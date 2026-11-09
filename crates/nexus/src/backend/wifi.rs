//! Wi-Fi on NetworkManager: Devices, access points, saved Profiles, scanning.
//!
//! Member names follow NetworkManager's D-Bus API reference:
//! `org.freedesktop.NetworkManager` (`GetDevices`), `.Device` (`Interface`,
//! `DeviceType`), `.Device.Wireless` (`GetAllAccessPoints`, `RequestScan`,
//! `LastScan`), `.AccessPoint` (`Ssid`, `Strength`, `Frequency`, `Flags`,
//! `WpaFlags`, `RsnFlags`), `.Settings` (`ListConnections`) and
//! `.Settings.Connection` (`GetSettings`).

use std::collections::HashMap;
use std::fmt::Write as _;

use nexus_ipc::{Band, SavedWifi, ScanOutcome, Security, WifiDevice, WifiList, WifiNetwork};
use zbus::proxy::CacheProperties;
use zbus::{Connection, OwnedObjectPath, OwnedVariant};

/// NetworkManager refuses `RequestScan` when the last scan finished less than
/// this long ago (research 03, edge case 6). Asking anyway only earns an error.
pub const SCAN_RATE_LIMIT_MS: i64 = 10_000;

/// `NM_DEVICE_TYPE_WIFI`.
const DEVICE_TYPE_WIFI: u32 = 2;

// NM80211ApFlags
const AP_PRIVACY: u32 = 0x1;
// NM80211ApSecurityFlags (key management part)
const KEY_PSK: u32 = 0x100;
const KEY_802_1X: u32 = 0x200;
const KEY_SAE: u32 = 0x400;
const KEY_OWE: u32 = 0x800;
const KEY_OWE_TM: u32 = 0x1000;
const KEY_EAP_SUITE_B_192: u32 = 0x2000;

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Device {
    #[zbus(property)]
    fn interface(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Wireless {
    fn get_all_access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn request_scan(&self, options: HashMap<String, OwnedVariant>) -> zbus::Result<()>;
    #[zbus(property)]
    fn last_scan(&self) -> zbus::Result<i64>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.AccessPoint",
    default_service = "org.freedesktop.NetworkManager"
)]
trait AccessPoint {
    #[zbus(property)]
    fn ssid(&self) -> zbus::Result<Vec<u8>>;
    #[zbus(property)]
    fn strength(&self) -> zbus::Result<u8>;
    #[zbus(property)]
    fn frequency(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn flags(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn wpa_flags(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn rsn_flags(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
trait Settings {
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
trait SettingsConnection {
    fn get_settings(&self) -> zbus::Result<HashMap<String, HashMap<String, OwnedVariant>>>;
}

/// One access point, as read.
#[derive(Debug, Clone)]
struct Ap {
    device: String,
    ssid: Vec<u8>,
    strength: u8,
    security: Security,
    band: Band,
}

struct Profile {
    uuid: String,
    id: String,
    ssid: Vec<u8>,
    autoconnect: bool,
}

/// A Wi-Fi Device found on the bus.
struct Radio {
    path: OwnedObjectPath,
    name: String,
}

macro_rules! no_cache {
    ($proxy:ident, $conn:expr, $path:expr) => {
        $proxy::builder($conn)
            .path($path)
            .cache_properties(CacheProperties::No)
            .build()
            .await?
    };
}

async fn radios(conn: &Connection) -> zbus::Result<Vec<Radio>> {
    let nm = super::nm::NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let mut out = Vec::new();
    for path in nm.get_devices().await? {
        // A Device can vanish between the two calls: skip it.
        let device = no_cache!(DeviceProxy, conn, path.as_str());
        let (Ok(kind), Ok(name)) = (device.device_type().await, device.interface().await) else {
            continue;
        };
        if kind == DEVICE_TYPE_WIFI {
            out.push(Radio { path, name });
        }
    }
    Ok(out)
}

pub async fn list(conn: &Connection) -> zbus::Result<WifiList> {
    let radios = radios(conn).await?;
    let mut aps = Vec::new();
    for radio in &radios {
        let wireless = no_cache!(WirelessProxy, conn, radio.path.as_str());
        let Ok(paths) = wireless.get_all_access_points().await else {
            continue;
        };
        for path in paths {
            if let Some(ap) = read_ap(conn, radio.path.as_str(), path.as_str()).await {
                aps.push(ap);
            }
        }
    }
    let profiles = profiles(conn).await?;
    let devices = radios
        .into_iter()
        .map(|r| WifiDevice {
            id: r.path.to_string(),
            name: r.name,
        })
        .collect();
    Ok(assemble(devices, aps, profiles))
}

/// Reads one access point; `None` if it vanished or hides its SSID.
async fn read_ap(conn: &Connection, device: &str, path: &str) -> Option<Ap> {
    let ap = AccessPointProxy::builder(conn)
        .path(path)
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .ok()?;
    let ssid = ap.ssid().await.ok()?;
    if ssid.is_empty() {
        return None; // hidden: it cannot be shown or joined from a list
    }
    Some(Ap {
        device: device.to_owned(),
        ssid,
        strength: ap.strength().await.ok()?,
        band: band(ap.frequency().await.ok()?),
        security: security(
            ap.flags().await.ok()?,
            ap.wpa_flags().await.ok()?,
            ap.rsn_flags().await.ok()?,
        ),
    })
}

async fn profiles(conn: &Connection) -> zbus::Result<Vec<Profile>> {
    let settings = SettingsProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let mut out = Vec::new();
    for path in settings.list_connections().await? {
        let proxy = no_cache!(SettingsConnectionProxy, conn, path.as_str());
        if let Ok(all) = proxy.get_settings().await
            && let Some(p) = parse_profile(&all)
        {
            out.push(p);
        }
    }
    Ok(out)
}

fn parse_profile(all: &HashMap<String, HashMap<String, OwnedVariant>>) -> Option<Profile> {
    let connection = all.get("connection")?;
    let text =
        |key: &str| -> Option<String> { connection.get(key)?.try_clone().ok()?.try_into().ok() };
    if text("type")? != "802-11-wireless" {
        return None;
    }
    let ssid: Vec<u8> = all
        .get("802-11-wireless")?
        .get("ssid")?
        .try_clone()
        .ok()?
        .try_into()
        .ok()?;
    let autoconnect = connection
        .get("autoconnect")
        .and_then(|v| v.try_clone().ok()?.try_into().ok())
        .unwrap_or(true); // NetworkManager's default
    Some(Profile {
        uuid: text("uuid")?,
        id: text("id")?,
        ssid,
        autoconnect,
    })
}

/// Asks every Wi-Fi Device to scan, unless it scanned too recently.
pub async fn scan(conn: &Connection) -> zbus::Result<ScanOutcome> {
    let now = super::boottime_ms();
    let (mut requested, mut skipped) = (0, 0);
    let mut freshest: Option<i64> = None;
    for radio in radios(conn).await? {
        let wireless = no_cache!(WirelessProxy, conn, radio.path.as_str());
        let last = wireless.last_scan().await.unwrap_or(-1);
        if last >= 0 {
            freshest = Some(freshest.map_or(now - last, |f| f.min(now - last)));
            if now - last < SCAN_RATE_LIMIT_MS {
                skipped += 1;
                continue;
            }
        }
        match wireless.request_scan(HashMap::new()).await {
            Ok(()) => requested += 1,
            Err(e) => {
                // Refused anyway (polkit, busy Device): the last results stand.
                tracing::debug!("{} refused a scan: {e}", radio.name);
                skipped += 1;
            }
        }
    }
    Ok(ScanOutcome {
        requested,
        skipped,
        last_scan_ms_ago: freshest.map(|ms| ms.max(0) as u64),
    })
}

/// Folds access points into Networks by SSID bytes, strongest first, and
/// splits the saved Profiles into in-range and out-of-range.
fn assemble(devices: Vec<WifiDevice>, aps: Vec<Ap>, profiles: Vec<Profile>) -> WifiList {
    let mut by_ssid: HashMap<Vec<u8>, (Ap, u32)> = HashMap::new();
    for ap in aps {
        by_ssid
            .entry(ap.ssid.clone())
            .and_modify(|(best, n)| {
                *n += 1;
                if ap.strength > best.strength {
                    *best = ap.clone();
                }
            })
            .or_insert((ap, 1));
    }
    // One Profile per SSID is enough to mark a row; the first one wins.
    let mut profile_of: HashMap<&[u8], &str> = HashMap::new();
    for p in &profiles {
        profile_of.entry(&p.ssid).or_insert(&p.uuid);
    }
    let mut networks: Vec<WifiNetwork> = by_ssid
        .into_iter()
        .map(|(ssid, (best, access_points))| WifiNetwork {
            id: hex(&ssid),
            ssid: String::from_utf8_lossy(&ssid).into_owned(),
            signal: best.strength.min(100),
            security: best.security,
            band: best.band,
            access_points,
            device: best.device,
            profile: profile_of.get(ssid.as_slice()).map(|u| (*u).to_owned()),
        })
        .collect();
    networks.sort_by(|a, b| {
        b.signal
            .cmp(&a.signal)
            .then_with(|| a.ssid.cmp(&b.ssid))
            .then_with(|| a.id.cmp(&b.id))
    });
    let saved = profiles
        .iter()
        .filter(|p| !networks.iter().any(|n| n.id == hex(&p.ssid)))
        .map(|p| SavedWifi {
            uuid: p.uuid.clone(),
            id: p.id.clone(),
            ssid: String::from_utf8_lossy(&p.ssid).into_owned(),
            autoconnect: p.autoconnect,
        })
        .collect();
    WifiList {
        devices,
        networks,
        saved,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Channel frequency in MHz to band. 6 GHz starts at channel 1 (5955 MHz).
fn band(mhz: u32) -> Band {
    match mhz {
        2400..=2500 => Band::Ghz2_4,
        4900..=5900 => Band::Ghz5,
        5925..=7125 => Band::Ghz6,
        _ => Band::Unknown,
    }
}

/// Reads `Flags`, `WpaFlags` and `RsnFlags`. Transition modes show up as
/// several key-management bits at once.
fn security(flags: u32, wpa: u32, rsn: u32) -> Security {
    let any = wpa | rsn;
    if any & (KEY_802_1X | KEY_EAP_SUITE_B_192) != 0 {
        Security::Enterprise
    } else if rsn & (KEY_OWE | KEY_OWE_TM) != 0 {
        Security::Owe
    } else if rsn & KEY_SAE != 0 {
        if rsn & KEY_PSK != 0 {
            Security::Wpa3Transition
        } else {
            Security::Wpa3
        }
    } else if rsn & KEY_PSK != 0 {
        Security::Wpa2
    } else if wpa & KEY_PSK != 0 {
        Security::Wpa
    } else if any == 0 && flags & AP_PRIVACY != 0 {
        Security::Wep
    } else if any == 0 {
        Security::Open
    } else {
        Security::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_edges() {
        assert_eq!(band(2412), Band::Ghz2_4);
        assert_eq!(band(5180), Band::Ghz5);
        assert_eq!(band(5955), Band::Ghz6);
        assert_eq!(band(60_000), Band::Unknown);
    }
}
