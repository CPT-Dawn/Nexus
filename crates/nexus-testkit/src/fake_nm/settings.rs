//! The fake `Settings` service and its `Settings.Connection` Profiles.

#![allow(clippy::wildcard_imports)]

use super::*;

pub(super) struct Settings(pub(super) Shared);

#[interface(name = "org.freedesktop.NetworkManager.Settings")]
impl Settings {
    /// Saves a VPN Profile (the only kind a client adds this way here).
    async fn add_connection(
        &self,
        connection: SettingsDict,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        #[zbus(object_server)] os: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let text = |key: &str| -> Option<String> {
            connection
                .get("connection")?
                .get(key)?
                .try_clone()
                .ok()?
                .try_into()
                .ok()
        };
        let kind = text("type").unwrap_or_default();
        if kind != "wireguard" && kind != "vpn" {
            return Err(zbus::fdo::Error::InvalidArgs(format!(
                "the fake only adds VPN profiles, not {kind}"
            )));
        }
        let n = {
            let mut m = self.0.lock().unwrap();
            m.profiles.push(Some(ProfileModel {
                uuid: text("uuid").unwrap_or_default(),
                id: text("id").unwrap_or_default(),
                ssid: Vec::new(),
                autoconnect: false,
                persisted: true,
                agent_secrets: false,
                secret: None,
                settings: Default::default(),
                version: 1,
                bdaddr: None,
                wired: false,
                hotspot: None,
                vpn: Some(VpnModel {
                    kind,
                    settings: clone_settings(&connection),
                    active: 0,
                    fails: false,
                    asks: None,
                    answers: None,
                }),
            }));
            m.profiles.len() - 1
        };
        os.at(profile_path(n), Profile(self.0.clone(), n)).await?;
        Self::new_connection(&emitter, opath(&profile_path(n))).await?;
        Ok(opath(&profile_path(n)))
    }

    #[zbus(signal)]
    async fn new_connection(
        emitter: &SignalEmitter<'_>,
        connection: OwnedObjectPath,
    ) -> zbus::Result<()>;

    fn list_connections(&self) -> Vec<OwnedObjectPath> {
        let m = self.0.lock().unwrap();
        (0..m.profiles.len())
            .filter(|n| m.profiles[*n].is_some())
            .map(|n| opath(&profile_path(n)))
            .collect()
    }
}

/// Rows of an `aa{sv}` value.
pub(super) fn rows_of(v: &OwnedVariant) -> Vec<HashMap<String, OwnedVariant>> {
    v.try_clone()
        .ok()
        .and_then(|v| v.try_into().ok())
        .unwrap_or_default()
}

pub(super) fn rows_value(rows: Vec<HashMap<String, OwnedVariant>>) -> OwnedVariant {
    let rows: Vec<HashMap<String, zbus::Variant<'static>>> = rows
        .into_iter()
        .map(|r| r.into_iter().map(|(k, v)| (k, v.into())).collect())
        .collect();
    OwnedVariant::try_from(zbus::Variant::new(rows)).expect("plain rows")
}

/// What `GetSettings` shows of a VPN Profile: no private key, no pre-shared keys.
pub(super) fn vpn_visible(settings: &SettingsDict) -> SettingsDict {
    let mut all = clone_settings(settings);
    if let Some(wg) = all.get_mut("wireguard") {
        wg.remove("private-key");
        if let Some(peers) = wg.get("peers") {
            let rows = rows_of(peers)
                .into_iter()
                .map(|mut r| {
                    r.remove("preshared-key");
                    r
                })
                .collect();
            wg.insert("peers".into(), rows_value(rows));
        }
    }
    all
}

/// What `GetSecrets` hands over for a VPN Profile's `setting`.
pub(super) fn vpn_secrets(settings: &SettingsDict, setting: &str) -> SettingsDict {
    let mut out = HashMap::new();
    if setting != "wireguard" {
        return out;
    }
    let Some(wg) = settings.get("wireguard") else {
        return out;
    };
    let mut values = HashMap::new();
    if let Some(k) = wg.get("private-key").and_then(|k| k.try_clone().ok()) {
        values.insert("private-key".to_owned(), k);
    }
    if let Some(peers) = wg.get("peers") {
        let rows: Vec<_> = rows_of(peers)
            .into_iter()
            .filter(|r| r.contains_key("preshared-key"))
            .map(|r| {
                r.into_iter()
                    .filter(|(k, _)| k == "public-key" || k == "preshared-key")
                    .collect()
            })
            .collect();
        if !rows.is_empty() {
            values.insert("peers".to_owned(), rows_value(rows));
        }
    }
    out.insert("wireguard".to_owned(), values);
    out
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.NetworkManager.Settings")]
pub(super) enum SettingsError {
    #[zbus(error)]
    ZBus(zbus::Error),
    VersionIdMismatch(String),
    PermissionDenied(String),
}

pub(super) struct Profile(pub(super) Shared, pub(super) usize);

impl Profile {
    /// Replaces what the Profile holds with what a client wrote.
    fn store(&self, settings: SettingsDict) {
        let auto = flag(&settings, "connection", "autoconnect");
        if let Some(p) = self.0.lock().unwrap().profiles[self.1].as_mut() {
            p.autoconnect = auto.unwrap_or(true);
            p.persisted = true;
            p.version += 1;
            p.settings = settings;
            // Like NetworkManager: the secret itself is not kept with the
            // settings, but its flags are, and a secret that is not the
            // agent's moves to the stored one.
            let psk = p
                .settings
                .get_mut(SECURITY)
                .and_then(|s| s.remove("psk"))
                .and_then(|v| String::try_from(v).ok());
            let agent_owned = p
                .settings
                .get(SECURITY)
                .and_then(|s| s.get("psk-flags"))
                .and_then(|v| u32::try_from(v.try_clone().ok()?).ok())
                .is_some_and(|f| f & 1 != 0);
            if agent_owned {
                p.agent_secrets = true;
                if let Some((_, secret)) = p.secret.as_mut() {
                    secret.clear();
                }
            } else if let (Some(psk), Some((_, secret))) = (psk, p.secret.as_mut()) {
                *secret = psk;
            }
        }
    }
}

#[interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl Profile {
    /// True while the Profile only lives in NetworkManager's memory.
    #[zbus(property)]
    fn unsaved(&self) -> bool {
        let m = self.0.lock().unwrap();
        m.profiles[self.1].as_ref().is_some_and(|p| !p.persisted)
    }

    fn get_settings(&self) -> HashMap<String, HashMap<String, OwnedVariant>> {
        let m = self.0.lock().unwrap();
        let p = m.profiles[self.1].as_ref().expect("served profile exists");
        if let Some(vpn) = &p.vpn {
            return vpn_visible(&vpn.settings);
        }
        if let Some(h) = &p.hotspot {
            return hotspot::visible(&h.settings);
        }
        let v = |v: Variant<'_>| OwnedVariant::try_from(v).expect("plain value");
        let mut connection = HashMap::new();
        connection.insert("id".to_owned(), v(Variant::from(p.id.clone())));
        connection.insert("uuid".to_owned(), v(Variant::from(p.uuid.clone())));
        connection.insert(
            "type".to_owned(),
            v(Variant::from(if p.wired {
                "802-3-ethernet"
            } else {
                "802-11-wireless"
            })),
        );
        connection.insert("autoconnect".to_owned(), v(Variant::from(p.autoconnect)));
        if let Some(addr) = &p.bdaddr {
            connection.insert("type".to_owned(), v(Variant::from("bluetooth")));
            let mut bluetooth = HashMap::new();
            bluetooth.insert("bdaddr".to_owned(), v(Variant::from(addr.clone())));
            bluetooth.insert("type".to_owned(), v(Variant::from("panu")));
            return HashMap::from([
                ("connection".to_owned(), connection),
                ("bluetooth".to_owned(), bluetooth),
            ]);
        }
        let mut wireless = HashMap::new();
        wireless.insert("ssid".to_owned(), v(Variant::from(p.ssid.clone())));
        let mut all: HashMap<String, HashMap<String, OwnedVariant>> = if p.wired {
            HashMap::from([
                ("connection".to_owned(), connection),
                ("802-3-ethernet".to_owned(), HashMap::new()),
            ])
        } else {
            HashMap::from([
                ("connection".to_owned(), connection),
                ("802-11-wireless".to_owned(), wireless),
            ])
        };
        // What clients wrote comes back, except the identity they cannot change.
        for (section, values) in &p.settings {
            let into = all.entry(section.clone()).or_default();
            for (key, value) in values {
                let identity = matches!(key.as_str(), "id" | "uuid" | "type" | "ssid");
                if !identity && let Ok(value) = value.try_clone() {
                    into.insert(key.clone(), value);
                }
            }
        }
        // Like NetworkManager, GetSettings leaves the secret itself out.
        if let Some((key_mgmt, _)) = &p.secret {
            all.entry(SECURITY.to_owned())
                .or_default()
                .insert("key-mgmt".to_owned(), v(Variant::from(key_mgmt.clone())));
        }
        all
    }

    /// The stored secret of a setting. Refused while
    /// [`FakeNm::deny_secret_reads`] says polkit said no.
    fn get_secrets(
        &self,
        setting: &str,
    ) -> Result<HashMap<String, HashMap<String, OwnedVariant>>, SettingsError> {
        let mut m = self.0.lock().unwrap();
        if m.deny_secret_reads {
            // NM_SETTINGS_ERROR_PERMISSION_DENIED, as nm-errors.h names it.
            return Err(SettingsError::PermissionDenied(
                "Insufficient privileges.".into(),
            ));
        }
        m.secret_reads += 1;
        let p = m.profiles[self.1].as_ref().expect("served profile exists");
        if let Some(vpn) = &p.vpn {
            return Ok(vpn_secrets(&vpn.settings, setting));
        }
        if let Some(h) = &p.hotspot {
            return Ok(hotspot::secrets(&h.settings, setting));
        }
        let mut out = HashMap::new();
        if let Some((_, secret)) = &p.secret
            && setting == "802-11-wireless-security"
            && !agent_owned(p)
        {
            let mut values = HashMap::new();
            values.insert(
                "psk".to_owned(),
                OwnedVariant::try_from(Variant::from(secret.clone())).expect("plain value"),
            );
            out.insert(setting.to_owned(), values);
        }
        Ok(out)
    }

    #[zbus(property)]
    fn version_id(&self) -> u64 {
        self.0.lock().unwrap().profiles[self.1]
            .as_ref()
            .map_or(0, |p| p.version)
    }

    /// Like `Update`, but a nonzero `version-id` in `args` must match the
    /// Profile's `VersionId`, or nothing changes.
    async fn update2(
        &self,
        settings: HashMap<String, HashMap<String, OwnedVariant>>,
        _flags: u32,
        args: HashMap<String, OwnedVariant>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<HashMap<String, OwnedVariant>, SettingsError> {
        let wanted: u64 = args
            .get("version-id")
            .and_then(|v| v.try_clone().ok()?.try_into().ok())
            .unwrap_or(0);
        let have = self.0.lock().unwrap().profiles[self.1]
            .as_ref()
            .map_or(0, |p| p.version);
        if wanted != 0 && wanted != have {
            return Err(SettingsError::VersionIdMismatch(
                "The profile changed since it was read".into(),
            ));
        }
        self.store(settings);
        Self::updated(&emitter).await?;
        Ok(HashMap::new())
    }

    /// Like NetworkManager, replaces the settings; the fake keeps only the
    /// `connection.autoconnect` flag, the one thing the Backend changes.
    async fn update(
        &self,
        settings: HashMap<String, HashMap<String, OwnedVariant>>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.store(settings);
        Self::updated(&emitter).await?;
        Ok(())
    }

    #[zbus(signal)]
    pub(super) async fn updated(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    fn save(&self) {
        if let Some(p) = self.0.lock().unwrap().profiles[self.1].as_mut() {
            p.persisted = true;
        }
    }

    async fn delete(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        #[zbus(object_server)] os: &zbus::ObjectServer,
        #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<()> {
        end_vpn(conn, &self.0, self.1).await;
        let hotspot_dev = self.0.lock().unwrap().profiles[self.1]
            .as_ref()
            .and_then(|p| p.hotspot.as_ref())
            .filter(|h| h.active > 0)
            .map(|h| h.device);
        if let Some(dev) = hotspot_dev {
            set_device_state(conn, &self.0, dev, 30, 38).await; // CONNECTION_REMOVED
        }
        // Like NetworkManager, deleting a Profile in use drops its Link.
        let (in_use, told_agent): (Vec<usize>, Option<SettingsDict>) = {
            let mut m = self.0.lock().unwrap();
            let gone = m.profiles[self.1].take();
            let told_agent = gone.as_ref().filter(|p| p.agent_secrets).map(agent_view);
            let bdaddr = gone.as_ref().and_then(|p| p.bdaddr.clone());
            let was_wired = gone.as_ref().is_some_and(|p| p.wired);
            let ssid = gone.map(|p| p.ssid).unwrap_or_default();
            let in_use = (0..m.devices.len())
                .filter(|d| match (&bdaddr, &m.devices[*d].bt) {
                    _ if was_wired => {
                        m.devices[*d]
                            .wired
                            .as_ref()
                            .is_some_and(|w| w.active == Some(self.1))
                            && m.devices[*d].state >= 40
                    }
                    (Some(addr), Some(b)) => {
                        *addr == bt::bdaddr(&b.address) && m.devices[*d].state >= 40
                    }
                    (Some(_), None) | (None, Some(_)) => false,
                    (None, None) => m.devices[*d]
                        .active_ap
                        .is_some_and(|ap| m.aps[&ap].ssid == ssid),
                })
                .collect();
            (in_use, told_agent)
        };
        if let Some(shown) = told_agent {
            // The agent may hold secrets of its own for this profile.
            let body = (shown, opath(&profile_path(self.1)));
            let _ = call_agent(conn, &self.0, "DeleteSecrets", &body).await;
        }
        for dev in in_use {
            set_device_state(conn, &self.0, dev, 30, 38).await; // CONNECTION_REMOVED
        }
        Self::removed(&emitter).await?;
        os.remove::<Profile, _>(profile_path(self.1)).await?;
        Ok(())
    }

    #[zbus(signal)]
    async fn removed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}
