//! A Hotspot in the fake NetworkManager: a Wi-Fi Profile in `ap` mode that
//! activates on an AP-capable Device (the state walk 40, 50, 70, 100 of every
//! other Link) and is served as a `Connection.Active` while it is up or coming
//! up. The stations that joined are not NetworkManager's to report, so they
//! are not modelled here: tests stand in for `iw` instead.

use zbus::{Connection, interface};

use super::{
    FakeNm, Model, OwnedObjectPath, Profile, ProfileModel, SettingsDict, Shared, clone_settings,
    device_path, manager_changed, opath, profile_path, set_device_state,
};

/// `NM_WIFI_DEVICE_CAP_AP` and a cap every card has (`CIPHER_CCMP`).
const CAP_AP: u32 = 0x40;
const CAP_CCMP: u32 = 0x8;

/// What makes a Profile a Hotspot.
#[derive(Debug)]
pub(super) struct HotspotModel {
    /// The settings as added, the psk included.
    pub settings: SettingsDict,
    /// `NMActiveConnectionState`; 0 is not active.
    pub active: u32,
    /// The Device it runs on.
    pub device: usize,
}

pub(super) fn active_path(n: usize) -> String {
    format!("{}/ActiveConnection/hs{n}", super::MANAGER_PATH)
}

/// The Profile slot behind a Hotspot active-connection path.
pub(super) fn slot_of(path: &str) -> Option<usize> {
    path.rsplit('/').next()?.strip_prefix("hs")?.parse().ok()
}

/// Whether the settings describe a Hotspot (`802-11-wireless.mode = ap`).
pub(super) fn is_hotspot(connection: &SettingsDict) -> bool {
    connection
        .get("802-11-wireless")
        .and_then(|w| w.get("mode"))
        .and_then(|v| v.try_clone().ok()?.try_into().ok())
        .is_some_and(|m: String| m == "ap")
}

pub(super) fn capabilities(ap_capable: bool) -> u32 {
    if ap_capable {
        CAP_AP | CAP_CCMP
    } else {
        CAP_CCMP
    }
}

/// The `Connection.Active` of a Hotspot.
pub(super) struct HotspotActive(pub Shared, pub usize);

impl HotspotActive {
    fn read<T>(&self, f: impl FnOnce(&ProfileModel, &HotspotModel) -> T) -> Option<T> {
        let m = self.0.lock().unwrap();
        let p = m.profiles[self.1].as_ref()?;
        Some(f(p, p.hotspot.as_ref()?))
    }
}

#[interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl HotspotActive {
    #[zbus(property)]
    fn id(&self) -> String {
        self.read(|p, _| p.id.clone()).unwrap_or_default()
    }

    #[zbus(property, name = "Type")]
    fn kind(&self) -> String {
        "802-11-wireless".into()
    }

    #[zbus(property)]
    fn uuid(&self) -> String {
        self.read(|p, _| p.uuid.clone()).unwrap_or_default()
    }

    #[zbus(property)]
    fn state(&self) -> u32 {
        self.read(|_, h| h.active).unwrap_or(4)
    }

    #[zbus(property)]
    fn connection(&self) -> OwnedObjectPath {
        opath(&profile_path(self.1))
    }

    #[zbus(property)]
    fn devices(&self) -> Vec<OwnedObjectPath> {
        self.read(|_, h| vec![opath(&device_path(h.device))])
            .unwrap_or_default()
    }
}

/// Sets the Hotspot's `State` and tells whoever listens.
async fn set_state(conn: &Connection, model: &Shared, n: usize, state: u32) {
    if let Some(h) = model.lock().unwrap().profiles[n]
        .as_mut()
        .and_then(|p| p.hotspot.as_mut())
    {
        h.active = state;
    }
    if let Ok(iface) = conn
        .object_server()
        .interface::<_, HotspotActive>(active_path(n))
        .await
    {
        let guard = iface.get().await;
        let _ = guard.state_changed(iface.signal_emitter()).await;
    }
}

/// The Hotspot stops being active: observers see state 4, then the object
/// goes. The Device is left to the caller.
pub(super) async fn end(conn: &Connection, model: &Shared, n: usize) {
    let was_active = model.lock().unwrap().profiles[n]
        .as_ref()
        .and_then(|p| p.hotspot.as_ref())
        .is_some_and(|h| h.active > 0);
    if !was_active {
        return;
    }
    set_state(conn, model, n, 4).await;
    if let Some(h) = model.lock().unwrap().profiles[n]
        .as_mut()
        .and_then(|p| p.hotspot.as_mut())
    {
        h.active = 0;
    }
    let _ = conn
        .object_server()
        .remove::<HotspotActive, _>(active_path(n))
        .await;
    manager_changed(conn).await;
}

/// A Device left its Link (or failed): any Hotspot on it ends.
pub(super) async fn device_dropped(conn: &Connection, model: &Shared, dev: usize) {
    let slots: Vec<usize> = {
        let m = model.lock().unwrap();
        (0..m.profiles.len())
            .filter(|n| {
                m.profiles[*n]
                    .as_ref()
                    .and_then(|p| p.hotspot.as_ref())
                    .is_some_and(|h| h.device == dev && h.active > 0)
            })
            .collect()
    };
    for n in slots {
        end(conn, model, n).await;
    }
}

/// `AddAndActivateConnection2` for a Hotspot Profile: serves the Profile and
/// the active connection, then walks the Device up on the fake's own task.
pub(super) async fn add_and_activate(
    conn: &Connection,
    model: &Shared,
    connection: SettingsDict,
    device: usize,
    persist_disk: bool,
) -> zbus::fdo::Result<(String, String)> {
    let text = |section: &str, key: &str| -> String {
        connection
            .get(section)
            .and_then(|s| s.get(key))
            .and_then(|v| v.try_clone().ok()?.try_into().ok())
            .unwrap_or_default()
    };
    {
        let m = model.lock().unwrap();
        if !m.devices[device].ap_capable {
            return Err(zbus::fdo::Error::NotSupported(
                "The device does not support AP mode".into(),
            ));
        }
    }
    let ssid: Vec<u8> = connection
        .get("802-11-wireless")
        .and_then(|w| w.get("ssid"))
        .and_then(|v| v.try_clone().ok()?.try_into().ok())
        .unwrap_or_default();
    let n = {
        let mut m = model.lock().unwrap();
        m.profiles.push(Some(ProfileModel {
            uuid: text("connection", "uuid"),
            id: text("connection", "id"),
            ssid,
            autoconnect: false,
            persisted: persist_disk,
            agent_secrets: false,
            secret: Some((
                text("802-11-wireless-security", "key-mgmt"),
                text("802-11-wireless-security", "psk"),
            )),
            settings: Default::default(),
            version: 1,
            bdaddr: None,
            vpn: None,
            hotspot: Some(HotspotModel {
                settings: clone_settings(&connection),
                active: 1,
                device,
            }),
        }));
        m.profiles.len() - 1
    };
    let os = conn.object_server();
    os.at(profile_path(n), Profile(model.clone(), n)).await?;
    os.at(active_path(n), HotspotActive(model.clone(), n))
        .await?;
    manager_changed(conn).await;
    let task = tokio::spawn(walk(conn.clone(), model.clone(), device, n));
    model.lock().unwrap().tasks.push(task.abort_handle());
    Ok((profile_path(n), active_path(n)))
}

async fn walk(conn: Connection, model: Shared, dev: usize, n: usize) {
    let step = std::time::Duration::from_millis(15);
    for s in [40, 50, 70] {
        tokio::time::sleep(step).await;
        set_device_state(&conn, &model, dev, s, 0).await;
    }
    tokio::time::sleep(step).await;
    if model.lock().unwrap().hotspot_fails {
        // 120 ends the Hotspot through `device_dropped`.
        set_device_state(&conn, &model, dev, 120, 1).await;
        return;
    }
    set_device_state(&conn, &model, dev, 100, 0).await;
    set_state(&conn, &model, n, 2).await;
}

/// The Hotspot Profile's settings as `GetSettings` shows them: no psk.
pub(super) fn visible(settings: &SettingsDict) -> SettingsDict {
    let mut out = clone_settings(settings);
    if let Some(sec) = out.get_mut("802-11-wireless-security") {
        sec.remove("psk");
    }
    out
}

/// The psk, for `GetSecrets("802-11-wireless-security")`.
pub(super) fn secrets(settings: &SettingsDict, setting: &str) -> SettingsDict {
    let mut out = SettingsDict::new();
    if let Some(psk) = settings.get(setting).and_then(|s| s.get("psk"))
        && let Ok(psk) = psk.try_clone()
    {
        out.insert(setting.to_owned(), [("psk".to_owned(), psk)].into());
    }
    out
}

impl FakeNm {
    /// Says whether the Wi-Fi Device can be an access point (default: it can).
    pub fn set_ap_support(&self, device: super::DeviceId, yes: bool) {
        self.model.lock().unwrap().devices[device.0].ap_capable = yes;
    }

    /// Makes the next Hotspot fail on its way up.
    pub fn script_hotspot_failure(&self, yes: bool) {
        self.model.lock().unwrap().hotspot_fails = yes;
    }

    /// The names of the Hotspots that are up or coming up.
    pub fn hotspots(&self) -> Vec<String> {
        let m = self.model.lock().unwrap();
        hotspot_slots(&m)
            .into_iter()
            .filter_map(|n| m.profiles[n].as_ref())
            .map(|p| String::from_utf8_lossy(&p.ssid).into_owned())
            .collect()
    }

    /// Hotspot Profiles still held (running or not): a stopped Hotspot
    /// leaves none.
    pub fn hotspot_profiles(&self) -> usize {
        let m = self.model.lock().unwrap();
        m.profiles
            .iter()
            .flatten()
            .filter(|p| p.hotspot.is_some())
            .count()
    }

    /// The psk a running Hotspot was given.
    pub fn hotspot_psk(&self, ssid: &str) -> Option<String> {
        let m = self.model.lock().unwrap();
        m.profiles
            .iter()
            .flatten()
            .filter(|p| p.ssid == ssid.as_bytes())
            .find_map(|p| p.secret.as_ref().map(|(_, psk)| psk.clone()))
    }

    /// A setting of the Hotspot named `ssid`, as text (e.g. `ipv4`,
    /// `method`).
    pub fn hotspot_setting(&self, ssid: &str, section: &str, key: &str) -> Option<String> {
        let m = self.model.lock().unwrap();
        let h = m
            .profiles
            .iter()
            .flatten()
            .find(|p| p.hotspot.is_some() && p.ssid == ssid.as_bytes())?
            .hotspot
            .as_ref()?;
        h.settings.get(section)?.get(key).map(super::show_variant)
    }
}

pub(super) fn hotspot_slots(m: &Model) -> Vec<usize> {
    (0..m.profiles.len())
        .filter(|n| {
            m.profiles[*n]
                .as_ref()
                .and_then(|p| p.hotspot.as_ref())
                .is_some_and(|h| h.active > 0)
        })
        .collect()
}
