//! Profile settings on NetworkManager: reading and writing a Wi-Fi Profile's
//! IP, DNS, MAC, metered and priority settings. Edits to the Profile carrying
//! the current Link are wrapped in a checkpoint (`Manager.CheckpointCreate`),
//! so NetworkManager itself puts the old settings back unless the change is
//! confirmed, even if the Daemon dies meanwhile. Writes use `Update2` with the
//! Profile's `version-id`, so a concurrent edit is detected instead of lost.
//!
//! Update replaces a Profile whole: everything is read back (secrets
//! best effort), only the settings the person changed are touched, and the
//! rest is written as it was.

use std::collections::HashMap;

use nexus_ipc::{
    AddressEntry, Ipv4, Ipv4Method, Ipv6, Ipv6Method, MacMode, ProfileKey, ProfileSettings,
    ProfileView,
};
use zbus::proxy::CacheProperties;
use zbus::{Connection, OwnedObjectPath, OwnedVariant, Variant};

use super::{Settings, SettingsConnectionProxy, WirelessProxy};
use super::{active_ssid, profiles, radios, variant};
use crate::text::{self, Failure};

/// `NM_SETTINGS_UPDATE2_FLAG_TO_DISK`.
const TO_DISK: u32 = 0x1;

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Versioned {
    #[zbus(property)]
    fn version_id(&self) -> zbus::Result<u64>;
    fn update2(
        &self,
        settings: Settings,
        flags: u32,
        args: HashMap<String, OwnedVariant>,
    ) -> zbus::Result<HashMap<String, OwnedVariant>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait Checkpoints {
    fn checkpoint_create(
        &self,
        devices: &[OwnedObjectPath],
        rollback_timeout: u32,
        flags: u32,
    ) -> zbus::Result<OwnedObjectPath>;
    fn checkpoint_destroy(&self, checkpoint: &OwnedObjectPath) -> zbus::Result<()>;
    fn checkpoint_rollback(
        &self,
        checkpoint: &OwnedObjectPath,
    ) -> zbus::Result<HashMap<String, u32>>;
}

/// What an update did.
#[derive(Debug)]
pub struct Changed {
    pub ssid: String,
    pub version: u64,
    /// The checkpoint to confirm or roll back, for an active Profile.
    pub checkpoint: Option<String>,
}

type Section = HashMap<String, OwnedVariant>;

fn text(section: &Section, key: &str) -> Option<String> {
    section.get(key)?.try_clone().ok()?.try_into().ok()
}

fn int(section: &Section, key: &str) -> Option<i32> {
    section.get(key)?.try_clone().ok()?.try_into().ok()
}

fn strings(section: &Section, key: &str) -> Vec<String> {
    section
        .get(key)
        .and_then(|v| v.try_clone().ok()?.try_into().ok())
        .unwrap_or_default()
}

fn entries(section: &Section) -> Vec<AddressEntry> {
    let rows: Vec<Section> = section
        .get("address-data")
        .and_then(|v| v.try_clone().ok()?.try_into().ok())
        .unwrap_or_default();
    rows.iter()
        .filter_map(|row| {
            Some(AddressEntry {
                address: text(row, "address")?,
                prefix: u8::try_from(u32::try_from(row.get("prefix")?.try_clone().ok()?).ok()?)
                    .ok()?,
            })
        })
        .collect()
}

/// The setting the Profile's MAC address option lives in: a wired Profile has
/// `802-3-ethernet`, a Wi-Fi one `802-11-wireless`.
fn mac_section(all: &Settings) -> &'static str {
    if all.contains_key("802-3-ethernet") {
        "802-3-ethernet"
    } else {
        "802-11-wireless"
    }
}

/// What a Profile's settings say, in the terms of the wire.
pub(super) fn read_settings(all: &Settings) -> ProfileSettings {
    let empty = Section::new();
    let get = |name: &str| all.get(name).unwrap_or(&empty);
    let (v4, v6) = (get("ipv4"), get("ipv6"));
    let mut dns = strings(v4, "dns-data");
    dns.extend(strings(v6, "dns-data"));
    let wireless = get(mac_section(all));
    let mac = text(wireless, "assigned-mac-address")
        .or_else(|| text(wireless, "cloned-mac-address"))
        .as_deref()
        .map_or(MacMode::Permanent, |m| match m {
            "random" => MacMode::EveryTime,
            "stable" | "stable-ssid" => MacMode::PerNetwork,
            "" | "permanent" | "preserve" => MacMode::Permanent,
            _ => MacMode::Unknown, // a fixed address, or something newer
        });
    let connection = get("connection");
    ProfileSettings {
        ipv4: Ipv4 {
            method: match text(v4, "method").as_deref() {
                None | Some("auto") => Ipv4Method::Auto,
                Some("manual") => Ipv4Method::Manual,
                Some(_) => Ipv4Method::Unknown,
            },
            addresses: entries(v4),
            gateway: text(v4, "gateway").filter(|g| !g.is_empty()),
        },
        ipv6: Ipv6 {
            method: match text(v6, "method").as_deref() {
                None | Some("auto") => Ipv6Method::Auto,
                Some("manual") => Ipv6Method::Manual,
                Some("disabled" | "ignore") => Ipv6Method::Disabled,
                Some(_) => Ipv6Method::Unknown,
            },
            addresses: entries(v6),
            gateway: text(v6, "gateway").filter(|g| !g.is_empty()),
        },
        dns,
        mac,
        // 1 = yes, 3 = guessed yes.
        metered: matches!(int(connection, "metered"), Some(1 | 3)),
        priority: int(connection, "autoconnect-priority").unwrap_or(0),
    }
}

fn address_rows(entries: &[AddressEntry]) -> OwnedVariant {
    let rows: Vec<HashMap<String, Variant<'static>>> = entries
        .iter()
        .map(|e| {
            HashMap::from([
                ("address".to_owned(), Variant::from(e.address.clone())),
                ("prefix".to_owned(), Variant::from(u32::from(e.prefix))),
            ])
        })
        .collect();
    variant(rows)
}

fn is_v6(server: &str) -> bool {
    server.contains(':')
}

/// Writes the settings that differ from what `all` already says into `all`.
/// What the person did not touch (and what Nexus cannot express, such as a
/// "link-local" method) is left exactly as it was.
pub(super) fn apply_settings(all: &mut Settings, new: &ProfileSettings) {
    let before = read_settings(all);
    if before.ipv4 != new.ipv4 && new.ipv4.method != Ipv4Method::Unknown {
        let s = all.entry("ipv4".to_owned()).or_default();
        for stale in ["addresses", "address-data", "gateway", "routes"] {
            s.remove(stale);
        }
        match new.ipv4.method {
            Ipv4Method::Manual => {
                s.insert("method".into(), variant("manual"));
                s.insert("address-data".into(), address_rows(&new.ipv4.addresses));
                if let Some(g) = &new.ipv4.gateway {
                    s.insert("gateway".into(), variant(g.clone()));
                }
            }
            _ => {
                s.insert("method".into(), variant("auto"));
            }
        }
    }
    if before.ipv6 != new.ipv6 && new.ipv6.method != Ipv6Method::Unknown {
        let s = all.entry("ipv6".to_owned()).or_default();
        for stale in ["addresses", "address-data", "gateway", "routes"] {
            s.remove(stale);
        }
        match new.ipv6.method {
            Ipv6Method::Manual => {
                s.insert("method".into(), variant("manual"));
                s.insert("address-data".into(), address_rows(&new.ipv6.addresses));
                if let Some(g) = &new.ipv6.gateway {
                    s.insert("gateway".into(), variant(g.clone()));
                }
            }
            Ipv6Method::Disabled => {
                s.insert("method".into(), variant("disabled"));
            }
            _ => {
                s.insert("method".into(), variant("auto"));
            }
        }
    }
    if before.dns != new.dns {
        for (name, v6) in [("ipv4", false), ("ipv6", true)] {
            let servers: Vec<String> = new.dns.iter().filter(|d| is_v6(d) == v6).cloned().collect();
            let s = all.entry(name.to_owned()).or_default();
            s.remove("dns");
            if servers.is_empty() {
                s.remove("dns-data");
                s.remove("ignore-auto-dns");
            } else {
                s.insert("dns-data".into(), variant(servers));
                // Custom servers replace the Network's, not join them.
                s.insert("ignore-auto-dns".into(), variant(true));
            }
        }
    }
    if before.mac != new.mac && new.mac != MacMode::Unknown {
        let wired = mac_section(all) == "802-3-ethernet";
        let s = all.entry(mac_section(all).to_owned()).or_default();
        s.remove("cloned-mac-address");
        let mode = match new.mac {
            MacMode::EveryTime => "random",
            MacMode::Permanent => "permanent",
            // Ethernet has no SSID to be stable per.
            _ if wired => "stable",
            _ => "stable-ssid",
        };
        s.insert("assigned-mac-address".into(), variant(mode));
    }
    let connection = all.entry("connection".to_owned()).or_default();
    if before.metered != new.metered {
        connection.insert("metered".into(), variant(i32::from(new.metered)));
    }
    if before.priority != new.priority {
        connection.insert("autoconnect-priority".into(), variant(new.priority));
    }
}

/// The Wi-Fi Devices that carry `ssid` right now.
async fn carrying(conn: &Connection, ssid: &[u8]) -> zbus::Result<Vec<OwnedObjectPath>> {
    let mut out = Vec::new();
    for radio in radios(conn).await? {
        let wireless = WirelessProxy::builder(conn)
            .path(radio.path.as_str())
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        if active_ssid(conn, &wireless).await.as_deref() == Some(ssid) {
            out.push(radio.path);
        }
    }
    Ok(out)
}

async fn versioned<'a>(conn: &Connection, path: &'a str) -> zbus::Result<VersionedProxy<'a>> {
    VersionedProxy::builder(conn)
        .path(path)
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

async fn manager(conn: &Connection) -> zbus::Result<CheckpointsProxy<'_>> {
    CheckpointsProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

fn named(network: &str) -> Result<(Vec<u8>, String), Failure> {
    let ssid = nexus_ipc::parse_network_id(network).ok_or_else(text::bad_network_id)?;
    let name = String::from_utf8_lossy(&ssid).into_owned();
    Ok((ssid, name))
}

/// The saved Profile a key names, and what is needed to checkpoint it.
struct Target {
    path: OwnedObjectPath,
    uuid: String,
    /// What the Profile is called to the person: the SSID, or the wired
    /// Profile's name.
    name: String,
    /// The Devices that carry it right now.
    devices: Vec<OwnedObjectPath>,
    /// The setting whose secrets must travel with an update.
    secrets: &'static str,
}

async fn target(conn: &Connection, key: &ProfileKey) -> zbus::Result<Result<Target, Failure>> {
    match key {
        ProfileKey::Network(network) => {
            let (ssid, name) = match named(network) {
                Ok(n) => n,
                Err(f) => return Ok(Err(f)),
            };
            let Some(profile) = profiles(conn).await?.into_iter().find(|p| p.ssid == ssid) else {
                return Ok(Err(text::unknown_network(&name)));
            };
            Ok(Ok(Target {
                devices: carrying(conn, &ssid).await?,
                path: profile.path,
                uuid: profile.uuid,
                name,
                secrets: "802-11-wireless-security",
            }))
        }
        ProfileKey::Uuid(uuid) => {
            let Some(wired) = super::super::ethernet::find_profile(conn, uuid).await? else {
                return Ok(Err(text::unknown_profile()));
            };
            Ok(Ok(Target {
                devices: super::super::ethernet::carrying(conn, uuid).await?,
                path: wired.path,
                uuid: wired.uuid,
                name: wired.name,
                secrets: "802-1x",
            }))
        }
    }
}

pub async fn get(
    conn: &Connection,
    key: &ProfileKey,
) -> zbus::Result<Result<ProfileView, Failure>> {
    let t = match target(conn, key).await? {
        Ok(t) => t,
        Err(f) => return Ok(Err(f)),
    };
    let proxy = SettingsConnectionProxy::builder(conn)
        .path(t.path.as_str())
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let all = proxy.get_settings().await?;
    let version = versioned(conn, t.path.as_str()).await?.version_id().await?;
    Ok(Ok(ProfileView {
        ssid: t.name,
        uuid: t.uuid,
        version,
        active: !t.devices.is_empty(),
        settings: read_settings(&all),
    }))
}

/// Applies `new` to the Profile `key` names if it is still at `version`.
/// An active Profile is checkpointed first, for `rollback_secs`.
pub async fn update(
    conn: &Connection,
    key: &ProfileKey,
    version: u64,
    new: &ProfileSettings,
    rollback_secs: u32,
) -> zbus::Result<Result<Changed, Failure>> {
    let t = match target(conn, key).await? {
        Ok(t) => t,
        Err(f) => return Ok(Err(f)),
    };
    let (path, name, devices) = (t.path.as_str(), t.name.clone(), t.devices.clone());
    let proxy = SettingsConnectionProxy::builder(conn)
        .path(path)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let mut all = proxy.get_settings().await?;
    if let Ok(secrets) = proxy.get_secrets(t.secrets).await {
        for (setting, values) in secrets {
            all.entry(setting).or_default().extend(values);
        }
    }
    let mac_changes = read_settings(&all).mac != new.mac;
    apply_settings(&mut all, new);

    let nm = manager(conn).await?;
    let checkpoint = if devices.is_empty() {
        None
    } else {
        match nm.checkpoint_create(&devices, rollback_secs, 0).await {
            Ok(cp) => Some(cp),
            Err(e) => return Ok(Err(text::checkpoint_refused(&e.to_string()))),
        }
    };

    let args = HashMap::from([("version-id".to_owned(), variant(version))]);
    let write = versioned(conn, path).await?;
    if let Err(e) = write.update2(all, TO_DISK, args).await {
        if let Some(cp) = &checkpoint {
            let _ = nm.checkpoint_destroy(cp).await; // nothing changed
        }
        return Ok(Err(match e {
            zbus::Error::MethodError(ref n, _, _) if n.as_str().ends_with("VersionIdMismatch") => {
                text::profile_conflict(&name)
            }
            other => return Err(other),
        }));
    }
    // NetworkManager reapplies what it can on the live Link; a different MAC
    // address needs the Link brought up again.
    if mac_changes && let Some(device) = devices.first() {
        let reactivate = async {
            let nm = super::super::nm::NetworkManagerProxy::builder(conn)
                .cache_properties(CacheProperties::No)
                .build()
                .await?;
            let any = OwnedObjectPath::try_from("/")?;
            nm.activate_connection(&t.path, device, &any).await
        };
        if let Err(e) = reactivate.await {
            tracing::debug!("cannot re-activate {name} after a MAC change: {e}");
        }
    }
    let version = write.version_id().await?;
    Ok(Ok(Changed {
        ssid: name,
        version,
        checkpoint: checkpoint.map(|c| c.to_string()),
    }))
}

/// Keeps the change: the checkpoint is dropped without rolling anything back.
pub async fn confirm(conn: &Connection, checkpoint: &str) -> zbus::Result<()> {
    let path = OwnedObjectPath::try_from(checkpoint)?;
    manager(conn).await?.checkpoint_destroy(&path).await
}

/// Puts the old settings back now.
pub async fn rollback(conn: &Connection, checkpoint: &str) -> zbus::Result<()> {
    let path = OwnedObjectPath::try_from(checkpoint)?;
    manager(conn).await?.checkpoint_rollback(&path).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(sections: &[(&str, Vec<(&str, OwnedVariant)>)]) -> Settings {
        sections
            .iter()
            .map(|(name, kv)| {
                (
                    (*name).to_owned(),
                    kv.iter()
                        .map(|(k, v)| ((*k).to_owned(), v.try_clone().unwrap()))
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn nothing_changed_writes_nothing() {
        let mut all = dict(&[
            ("ipv4", vec![("method", variant("link-local"))]),
            ("connection", vec![("metered", variant(2i32))]),
        ]);
        let same = read_settings(&all);
        let before = format!("{all:?}");
        apply_settings(&mut all, &same);
        assert_eq!(format!("{all:?}"), before);
        assert_eq!(same.ipv4.method, Ipv4Method::Unknown);
        assert!(!same.metered);
    }

    #[test]
    fn static_settings_round_trip() {
        let mut all = dict(&[("connection", vec![])]);
        let new = ProfileSettings {
            ipv4: Ipv4 {
                method: Ipv4Method::Manual,
                addresses: vec![AddressEntry {
                    address: "192.168.1.20".into(),
                    prefix: 24,
                }],
                gateway: Some("192.168.1.1".into()),
            },
            ipv6: Ipv6 {
                method: Ipv6Method::Disabled,
                ..Ipv6::default()
            },
            dns: vec!["1.1.1.1".into(), "2606:4700:4700::1111".into()],
            mac: MacMode::EveryTime,
            metered: true,
            priority: -5,
        };
        apply_settings(&mut all, &new);
        assert_eq!(read_settings(&all), new);
        assert_eq!(
            all["ipv4"]
                .get("ignore-auto-dns")
                .map(|v| bool::try_from(v.try_clone().unwrap())),
            Some(Ok(true))
        );
        // Taking the DNS servers away hands DNS back to the Network.
        let mut cleared = new.clone();
        cleared.dns.clear();
        apply_settings(&mut all, &cleared);
        assert!(!all["ipv4"].contains_key("dns-data"));
        assert!(!all["ipv4"].contains_key("ignore-auto-dns"));
    }
}
