//! Wi-Fi on NetworkManager: Devices, access points, saved Profiles, scanning.
//!
//! Member names follow NetworkManager's D-Bus API reference:
//! `org.freedesktop.NetworkManager` (`GetDevices`), `.Device` (`Interface`,
//! `DeviceType`), `.Device.Wireless` (`GetAllAccessPoints`, `RequestScan`,
//! `LastScan`), `.AccessPoint` (`Ssid`, `Strength`, `Frequency`, `Flags`,
//! `WpaFlags`, `RsnFlags`), `.Settings` (`ListConnections`) and
//! `.Settings.Connection` (`GetSettings`).

use std::collections::{HashMap, HashSet};

use nexus_ipc::{
    ApInfo, ApList, AutoconnectSet, Band, Disconnected, Eap, Forgotten, Joined, SavedWifi,
    ScanOutcome, Secret, Security, ShareAuth, WifiDevice, WifiList, WifiNetwork, WifiShare,
};

use super::JoinTarget;
use zbus::proxy::CacheProperties;
use zbus::{Connection, OwnedObjectPath, OwnedVariant};

use crate::secrets::{Broker, Ended};
use crate::text::{self, Failure};

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
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    fn disconnect(&self) -> zbus::Result<()>;
    #[zbus(signal, name = "StateChanged")]
    fn device_state_changed(&self, new_state: u32, old_state: u32, reason: u32)
    -> zbus::Result<()>;
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
    #[zbus(property)]
    fn active_access_point(&self) -> zbus::Result<OwnedObjectPath>;
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
    #[zbus(property)]
    fn hw_address(&self) -> zbus::Result<String>;
    /// Best link rate, in kbit/s.
    #[zbus(property)]
    fn max_bitrate(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
pub(super) trait Settings {
    fn add_connection(&self, connection: Settings) -> zbus::Result<OwnedObjectPath>;
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
pub(super) trait SettingsConnection {
    fn get_settings(&self) -> zbus::Result<HashMap<String, HashMap<String, OwnedVariant>>>;
    fn get_secrets(&self, setting: &str) -> zbus::Result<Settings>;
    fn update(&self, settings: Settings) -> zbus::Result<()>;
    fn save(&self) -> zbus::Result<()>;
    fn delete(&self) -> zbus::Result<()>;
}

/// One access point, as read.
#[derive(Debug, Clone)]
struct Ap {
    device: String,
    /// The object path, to tell which one a Device is associated with.
    path: String,
    ssid: Vec<u8>,
    bssid: String,
    mhz: u32,
    /// `MaxBitrate` in kbit/s; 0 when unknown.
    kbps: u32,
    strength: u8,
    security: Security,
    band: Band,
}

struct Profile {
    path: OwnedObjectPath,
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

pub(super) mod profile;

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

/// Everything the radios can hear right now.
struct Survey {
    radios: Vec<Radio>,
    aps: Vec<Ap>,
    /// SSIDs a Device is associated with.
    joined: HashSet<Vec<u8>>,
    /// Object paths of the access points a Device is associated with.
    active: HashSet<String>,
}

impl Survey {
    fn devices(&self) -> Vec<WifiDevice> {
        self.radios
            .iter()
            .map(|r| WifiDevice {
                id: r.path.to_string(),
                name: r.name.clone(),
            })
            .collect()
    }

    /// The Device called `key`: its id (object path) or its interface name.
    fn device(&self, key: &str) -> Option<&Radio> {
        self.radios
            .iter()
            .find(|r| r.path.as_str() == key || r.name == key)
    }
}

async fn survey(conn: &Connection) -> zbus::Result<Survey> {
    let radios = radios(conn).await?;
    let mut aps = Vec::new();
    let mut joined = HashSet::new();
    let mut active = HashSet::new();
    for radio in &radios {
        let wireless = no_cache!(WirelessProxy, conn, radio.path.as_str());
        if let Some((path, ssid)) = active_ap(conn, &wireless).await {
            joined.insert(ssid);
            active.insert(path);
        }
        let Ok(paths) = wireless.get_all_access_points().await else {
            continue;
        };
        for path in paths {
            if let Some(ap) = read_ap(conn, radio.path.as_str(), path.as_str()).await {
                aps.push(ap);
            }
        }
    }
    Ok(Survey {
        radios,
        aps,
        joined,
        active,
    })
}

pub async fn list(conn: &Connection) -> zbus::Result<WifiList> {
    let survey = survey(conn).await?;
    let profiles = profiles(conn).await?;
    let devices = survey.devices();
    Ok(assemble(devices, survey.aps, profiles, &survey.joined))
}

/// The access points that broadcast a Network, strongest first.
pub async fn access_points(
    conn: &Connection,
    network: &str,
) -> zbus::Result<Result<ApList, Failure>> {
    let Some(ssid) = nexus_ipc::parse_network_id(network) else {
        return Ok(Err(text::bad_network_id()));
    };
    let survey = survey(conn).await?;
    let mut aps: Vec<ApInfo> = survey
        .aps
        .iter()
        .filter(|ap| ap.ssid == ssid)
        .map(|ap| ApInfo {
            bssid: ap.bssid.clone(),
            channel: nexus_ipc::channel_of(ap.mhz),
            frequency_mhz: ap.mhz,
            band: ap.band,
            signal: ap.strength.min(100),
            rate_mbps: ap.kbps / 1000,
            device: survey
                .radios
                .iter()
                .find(|r| r.path.as_str() == ap.device)
                .map(|r| r.name.clone())
                .unwrap_or_default(),
            joined: survey.active.contains(&ap.path),
        })
        .collect();
    let name = nexus_ipc::display_ssid(&ssid);
    if aps.is_empty() {
        return Ok(Err(text::out_of_range(&name)));
    }
    aps.sort_by(|a, b| b.signal.cmp(&a.signal).then_with(|| a.bssid.cmp(&b.bssid)));
    Ok(Ok(ApList { ssid: name, aps }))
}

/// The SSID `wireless` is associated with, if any.
async fn active_ssid(conn: &Connection, wireless: &WirelessProxy<'_>) -> Option<Vec<u8>> {
    active_ap(conn, wireless).await.map(|(_, ssid)| ssid)
}

/// The access point `wireless` is associated with: its path and SSID.
async fn active_ap(conn: &Connection, wireless: &WirelessProxy<'_>) -> Option<(String, Vec<u8>)> {
    let path = wireless.active_access_point().await.ok()?;
    if path.as_str() == "/" {
        return None;
    }
    let ap = AccessPointProxy::builder(conn)
        .path(path.as_str())
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .ok()?;
    let ssid = ap.ssid().await.ok().filter(|s| !s.is_empty())?;
    Some((path.to_string(), ssid))
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
    let mhz = ap.frequency().await.ok()?;
    Some(Ap {
        device: device.to_owned(),
        path: path.to_owned(),
        ssid,
        bssid: ap.hw_address().await.unwrap_or_default(),
        mhz,
        kbps: ap.max_bitrate().await.unwrap_or(0),
        strength: ap.strength().await.ok()?,
        band: band(mhz),
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
            && let Some(p) = parse_profile(path, &all)
        {
            out.push(p);
        }
    }
    Ok(out)
}

fn parse_profile(
    path: OwnedObjectPath,
    all: &HashMap<String, HashMap<String, OwnedVariant>>,
) -> Option<Profile> {
    let connection = all.get("connection")?;
    let text =
        |key: &str| -> Option<String> { connection.get(key)?.try_clone().ok()?.try_into().ok() };
    if text("type")? != "802-11-wireless" {
        return None;
    }
    // A Hotspot is not a Network to join.
    let mode: Option<String> = all
        .get("802-11-wireless")?
        .get("mode")
        .and_then(|v| v.try_clone().ok()?.try_into().ok());
    if mode.as_deref() == Some("ap") {
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
        path,
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
fn assemble(
    devices: Vec<WifiDevice>,
    aps: Vec<Ap>,
    profiles: Vec<Profile>,
    joined: &HashSet<Vec<u8>>,
) -> WifiList {
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
    let mut profile_of: HashMap<&[u8], (&str, bool)> = HashMap::new();
    for p in &profiles {
        profile_of
            .entry(&p.ssid)
            .or_insert((&p.uuid, p.autoconnect));
    }
    let mut networks: Vec<WifiNetwork> = by_ssid
        .into_iter()
        .map(|(ssid, (best, access_points))| WifiNetwork {
            id: hex(&ssid),
            ssid: nexus_ipc::display_ssid(&ssid),
            signal: best.strength.min(100),
            security: best.security,
            band: best.band,
            access_points,
            // NetworkManager keeps reporting the active access point for a
            // moment after its Profile is deleted: no Profile, not joined.
            joined: joined.contains(&ssid) && profile_of.contains_key(ssid.as_slice()),
            device: best.device,
            profile: profile_of
                .get(ssid.as_slice())
                .map(|(u, _)| (*u).to_owned()),
            autoconnect: profile_of.get(ssid.as_slice()).map(|(_, a)| *a),
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
            ssid: nexus_ipc::display_ssid(&p.ssid),
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
    nexus_ipc::network_id(bytes)
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

/// How long a join may take before Nexus gives up on it. NetworkManager's
/// own activation timeout is 60 s; a wrong password is reported far sooner.
const JOIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

// NMDeviceState
const STATE_DISCONNECTED: u32 = 30;
const STATE_PREPARE: u32 = 40;
const STATE_ACTIVATED: u32 = 100;
const STATE_FAILED: u32 = 120;
/// `NM_DEVICE_STATE_REASON_NEW_ACTIVATION`: the old Link is making way.
const REASON_NEW_ACTIVATION: u32 = 60;

pub(super) type Settings = HashMap<String, HashMap<String, OwnedVariant>>;

pub(super) fn variant<'a>(v: impl Into<zbus::Variant<'a>>) -> OwnedVariant {
    OwnedVariant::try_from(v.into()).expect("plain values have no file descriptors")
}

/// A random (version 4) UUID for a new Profile.
pub(super) fn new_uuid() -> String {
    let mut b = [0u8; 16];
    // Blocking on the entropy pool only happens before it is seeded at boot.
    let _ = rustix::rand::getrandom(&mut b, rustix::rand::GetRandomFlags::empty());
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = nexus_ipc::network_id(&b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// The settings for a new Profile. The password goes only into this
/// dictionary, which is handed to NetworkManager and dropped; it is never
/// logged.
fn new_profile(
    ssid: &[u8],
    name: &str,
    uuid: &str,
    security: Security,
    password: Option<&str>,
    eap: Option<&Eap>,
    hidden: bool,
) -> Settings {
    let mut connection = HashMap::new();
    connection.insert("id".to_owned(), variant(name));
    connection.insert("type".to_owned(), variant("802-11-wireless"));
    connection.insert("uuid".to_owned(), variant(uuid));
    connection.insert("autoconnect".to_owned(), variant(true));
    let mut wireless = HashMap::new();
    wireless.insert("ssid".to_owned(), variant(ssid.to_vec()));
    wireless.insert("mode".to_owned(), variant("infrastructure"));
    if hidden {
        // NetworkManager then probes for the name instead of waiting to hear it.
        wireless.insert("hidden".to_owned(), variant(true));
    }
    let auto = |_: &str| HashMap::from([("method".to_owned(), variant("auto"))]);
    let mut all = HashMap::from([
        ("connection".to_owned(), connection),
        ("802-11-wireless".to_owned(), wireless),
        ("ipv4".to_owned(), auto("ipv4")),
        ("ipv6".to_owned(), auto("ipv6")),
    ]);
    if let Some(eap) = eap {
        all.insert(
            "802-11-wireless-security".to_owned(),
            HashMap::from([("key-mgmt".to_owned(), variant("wpa-eap"))]),
        );
        all.insert("802-1x".to_owned(), super::eap::setting(eap));
        return all;
    }
    let key_mgmt = match security {
        Security::Wpa3 => Some("sae"),
        Security::Owe => Some("owe"),
        Security::Open => None,
        _ => Some("wpa-psk"), // WPA, WPA2 and WPA2/3 transition
    };
    if let Some(key_mgmt) = key_mgmt {
        let mut sec = HashMap::from([("key-mgmt".to_owned(), variant(key_mgmt))]);
        if let Some(pw) = password {
            sec.insert("psk".to_owned(), variant(pw));
        }
        all.insert("802-11-wireless-security".to_owned(), sec);
    }
    all
}

/// Joins a Network and waits for the outcome. The inner error is something to
/// tell the person; the outer one is a bus problem.
///
/// Without a password for an unsaved secured Network, NetworkManager asks our
/// secret agent, which opens a Secret prompt; this call waits on it. A
/// password rejected after the person typed it (reasons 7, 8, 11) reopens the
/// prompt, saying so, and the join starts over.
pub async fn join(
    conn: &Connection,
    broker: &Broker,
    network: &str,
    password: Option<&Secret>,
    eap: Option<&Eap>,
    target: &JoinTarget,
) -> zbus::Result<Result<Joined, Failure>> {
    broker.forget_ended(network);
    loop {
        let result = join_once(conn, broker, network, password, eap, target).await?;
        let Err(failure) = &result else {
            return Ok(result);
        };
        if failure.code != nexus_ipc::ErrorCode::WrongPassword || password.is_some() {
            return Ok(result);
        }
        let name = nexus_ipc::parse_network_id(network)
            .map(|s| nexus_ipc::display_ssid(&s))
            .unwrap_or_default();
        match broker.take_ended(network) {
            Some(Ended::Cancelled) => return Ok(Err(text::prompt_cancelled(&name))),
            Some(Ended::Expired) => return Ok(Err(text::prompt_expired(&name))),
            Some(Ended::Withdrawn) => return Ok(Err(text::join_withdrawn(&name))),
            Some(Ended::Answered) => broker.mark_rejected(network),
            None => return Ok(result),
        }
    }
}

async fn join_once(
    conn: &Connection,
    broker: &Broker,
    network: &str,
    password: Option<&Secret>,
    eap: Option<&Eap>,
    target: &JoinTarget,
) -> zbus::Result<Result<Joined, Failure>> {
    let Some(ssid) = nexus_ipc::parse_network_id(network) else {
        return Ok(Err(text::bad_network_id()));
    };
    let name = nexus_ipc::display_ssid(&ssid);
    if let Some(eap) = eap {
        let errors = eap.validate();
        if !errors.is_empty() {
            return Ok(Err(text::invalid_eap(&name, &errors)));
        }
    }
    if let Some(sec) = target.hidden
        && !matches!(
            sec,
            Security::Open
                | Security::Owe
                | Security::Wpa
                | Security::Wpa2
                | Security::Wpa3
                | Security::Wpa3Transition
        )
    {
        return Ok(Err(text::hidden_security(&name, sec.label())));
    }
    let survey = survey(conn).await?;
    let chosen = match &target.device {
        Some(key) => match survey.device(key) {
            Some(radio) => Some((radio.path.to_string(), radio.name.clone())),
            None => return Ok(Err(text::unknown_device(key))),
        },
        None => None,
    };
    let all_profiles = profiles(conn).await?;
    let devices = survey.devices();
    // With a card chosen, only what that card hears counts.
    let heard: Vec<Ap> = match &chosen {
        Some((path, _)) => survey
            .aps
            .iter()
            .filter(|ap| &ap.device == path)
            .cloned()
            .collect(),
        None => survey.aps.clone(),
    };
    let list = assemble(devices.clone(), heard, all_profiles, &survey.joined);
    let listed = list.networks.iter().find(|n| n.id == network);
    let (security, device_path, hidden) = match (listed, target.hidden) {
        (_, Some(sec)) => {
            // Named by the person: it need not be in the list at all.
            let device = listed
                .map(|n| n.device.clone())
                .or_else(|| chosen.as_ref().map(|(p, _)| p.clone()))
                .or_else(|| survey.radios.first().map(|r| r.path.to_string()));
            let Some(device) = device else {
                return Ok(Err(text::out_of_range(&name)));
            };
            (sec, device, true)
        }
        (Some(n), None) => (n.security, n.device.clone(), false),
        (None, None) => {
            // Heard by another card than the chosen one?
            if let Some((_, card)) = &chosen
                && survey.aps.iter().any(|ap| ap.ssid == ssid)
            {
                return Ok(Err(text::not_heard_by(&name, card)));
            }
            return Ok(Err(text::out_of_range(&name)));
        }
    };
    if eap.is_some() && security != Security::Enterprise {
        return Ok(Err(text::not_enterprise(&name, security.label())));
    }
    let secured = !matches!(security, Security::Open | Security::Owe);
    // One Profile per SSID is enough; the first one wins, as in the list.
    let saved = profiles(conn)
        .await?
        .into_iter()
        .find(|p| p.ssid == ssid)
        .map(|p| (p.path, p.uuid));
    let nm = super::nm::NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let device = no_cache!(DeviceProxy, conn, device_path.as_str());
    // Subscribe before asking: the Device may move on before the call returns.
    let mut states = device.receive_device_state_changed().await?;
    // NetworkManager restarting forgets the join; do not wait it out.
    let mut owner = zbus::fdo::DBusProxy::new(conn)
        .await?
        .receive_name_owner_changed_with_args(&[(0, "org.freedesktop.NetworkManager")])
        .await?;
    let none = OwnedObjectPath::try_from("/").expect("root path");
    let device_obj = OwnedObjectPath::try_from(device_path.as_str()).expect("device path");

    let mut inline = None;
    let (profile_path, uuid, created) = match saved {
        // A saved Profile brings its own secrets.
        Some((path, uuid)) => {
            if let Err(e) = nm.activate_connection(&path, &device_obj, &none).await {
                return Ok(Err(text::backend_refused(&name, &e.to_string())));
            }
            (path, uuid, false)
        }
        None => {
            match security {
                Security::Enterprise if eap.is_none() => {
                    return Ok(Err(text::needs_eap(&name)));
                }
                Security::Wep | Security::Unknown => {
                    return Ok(Err(text::unsupported_security(&name, security.label())));
                }
                _ => {}
            }
            let password = password.map(Secret::expose).filter(|_| secured);
            if let Some(pw) = password
                && security != Security::Wpa3
                && !(8..=63).contains(&pw.chars().count())
            {
                return Ok(Err(text::bad_passphrase_length(&name)));
            }
            let uuid = new_uuid();
            inline = password.map(|_| broker.inline_password(&uuid));
            let settings = new_profile(&ssid, &name, &uuid, security, password, eap, hidden);
            let options = HashMap::from([("persist".to_owned(), variant("memory"))]);
            match nm
                .add_and_activate_connection2(settings, &device_obj, &none, options)
                .await
            {
                Ok((path, _active, _result)) => (path, uuid, true),
                Err(e) => return Ok(Err(text::backend_refused(&name, &e.to_string()))),
            }
        }
    };

    let outcome = wait_for_link(&mut states, &mut owner, &name, secured).await;
    drop(inline);
    let settings = no_cache!(SettingsConnectionProxy, conn, profile_path.as_str());
    match outcome {
        Ok(()) => {
            if created && let Err(e) = settings.save().await {
                // Up, but not remembered: say so rather than pretend.
                let _ = settings.delete().await;
                return Ok(Err(text::backend_refused(&name, &e.to_string())));
            }
            Ok(Ok(Joined {
                ssid: name,
                profile: uuid,
            }))
        }
        Err(mut failure) => {
            if eap.is_some() && failure.code == nexus_ipc::ErrorCode::WrongPassword {
                failure = text::eap_refused(&name);
            }
            if created {
                // No lingering Profile for a join that never worked.
                let _ = settings.delete().await;
            }
            // Stop NetworkManager retrying a join nobody wants any more.
            let _ = device.disconnect().await;
            Ok(Err(failure))
        }
    }
}

/// Watches a Device until the join it started is up or dead.
async fn wait_for_link(
    states: &mut StateChangedStream,
    owner: &mut zbus::fdo::NameOwnerChangedStream,
    name: &str,
    secured: bool,
) -> Result<(), Failure> {
    use futures_util::StreamExt as _;
    let deadline = tokio::time::sleep(JOIN_TIMEOUT);
    tokio::pin!(deadline);
    let mut started = false;
    loop {
        let signal = tokio::select! {
            s = states.next() => s,
            _ = owner.next() => return Err(text::join_interrupted(name)),
            () = &mut deadline => return Err(text::join_timed_out(name)),
        };
        let Some(signal) = signal else {
            return Err(text::backend_refused(name, "NetworkManager went away"));
        };
        let Ok(args) = signal.args() else { continue };
        let (new, old, reason) = (args.new_state, args.old_state, args.reason);
        match new {
            STATE_ACTIVATED => return Ok(()),
            STATE_FAILED => return Err(text::join_failure(name, old, reason, secured)),
            n if n >= STATE_PREPARE => started = true,
            // Back to rest after the join began: stopped by someone, not
            // the old Link making way for this one.
            n if n <= STATE_DISCONNECTED && started && reason != REASON_NEW_ACTIVATION => {
                return Err(text::join_failure(name, old, reason, secured));
            }
            _ => {}
        }
    }
}

/// Drops every Wi-Fi Link that is up or coming up, keeping the Profiles.
pub async fn disconnect(conn: &Connection) -> zbus::Result<Result<Disconnected, Failure>> {
    let mut dropped = None;
    for radio in radios(conn).await? {
        let device = no_cache!(DeviceProxy, conn, radio.path.as_str());
        let state = device.state().await.unwrap_or(0);
        if !(STATE_PREPARE..STATE_ACTIVATED + 10).contains(&state) {
            continue;
        }
        let wireless = no_cache!(WirelessProxy, conn, radio.path.as_str());
        let ssid = active_ssid(conn, &wireless).await;
        if device.disconnect().await.is_ok() {
            let name = ssid.map_or_else(|| radio.name.clone(), |s| nexus_ipc::display_ssid(&s));
            dropped.get_or_insert(name);
        }
    }
    Ok(match dropped {
        Some(ssid) => Ok(Disconnected { ssid }),
        None => Err(text::not_connected()),
    })
}

/// Deletes every Profile saved for a Network.
pub async fn forget(conn: &Connection, network: &str) -> zbus::Result<Result<Forgotten, Failure>> {
    let Some(ssid) = nexus_ipc::parse_network_id(network) else {
        return Ok(Err(text::bad_network_id()));
    };
    let name = nexus_ipc::display_ssid(&ssid);
    let mut removed = 0;
    for p in profiles(conn).await?.into_iter().filter(|p| p.ssid == ssid) {
        let proxy = no_cache!(SettingsConnectionProxy, conn, p.path.as_str());
        proxy.delete().await?;
        removed += 1;
    }
    Ok(if removed == 0 {
        Err(text::unknown_network(&name))
    } else {
        Ok(Forgotten { ssid: name })
    })
}

/// Turns automatic joining on or off for every Profile saved for a Network.
/// `Update` replaces a Profile's settings whole, so this reads them (and the
/// Wi-Fi secrets, best effort) back, flips `connection.autoconnect` and
/// writes them again.
pub async fn set_autoconnect(
    conn: &Connection,
    network: &str,
    enabled: bool,
) -> zbus::Result<Result<AutoconnectSet, Failure>> {
    let Some(ssid) = nexus_ipc::parse_network_id(network) else {
        return Ok(Err(text::bad_network_id()));
    };
    let name = nexus_ipc::display_ssid(&ssid);
    let mut changed = 0;
    for p in profiles(conn).await?.into_iter().filter(|p| p.ssid == ssid) {
        let proxy = no_cache!(SettingsConnectionProxy, conn, p.path.as_str());
        let mut all = proxy.get_settings().await?;
        if let Ok(secrets) = proxy.get_secrets("802-11-wireless-security").await {
            for (setting, values) in secrets {
                all.entry(setting).or_default().extend(values);
            }
        }
        all.entry("connection".to_owned())
            .or_default()
            .insert("autoconnect".to_owned(), variant(enabled));
        proxy.update(all).await?;
        changed += 1;
    }
    Ok(if changed == 0 {
        Err(text::unknown_network(&name))
    } else {
        Ok(AutoconnectSet {
            ssid: name,
            enabled,
        })
    })
}

/// Whether NetworkManager (or the bus) refused for lack of authorization.
pub(super) fn is_denial(e: &zbus::Error) -> bool {
    match e {
        zbus::Error::FDO(inner) => matches!(**inner, zbus::fdo::Error::AccessDenied(_)),
        zbus::Error::MethodError(name, ..) => name.as_str().ends_with(".PermissionDenied"),
        _ => false,
    }
}

/// Reads what a phone needs to join a saved Network. The password comes from
/// `GetSecrets`, which NetworkManager answers only for callers polkit lets
/// read secrets: a refusal is `reveal_denied`. Open Networks have no secret,
/// so nothing is asked of NetworkManager for them.
pub async fn reveal(conn: &Connection, network: &str) -> zbus::Result<Result<WifiShare, Failure>> {
    let Some(ssid) = nexus_ipc::parse_network_id(network) else {
        return Ok(Err(text::bad_network_id()));
    };
    let name = nexus_ipc::display_ssid(&ssid);
    let Some(p) = profiles(conn).await?.into_iter().find(|p| p.ssid == ssid) else {
        return Ok(Err(text::unknown_network(&name)));
    };
    let proxy = no_cache!(SettingsConnectionProxy, conn, p.path.as_str());
    let all = proxy.get_settings().await?;
    let key_mgmt: Option<String> = all
        .get("802-11-wireless-security")
        .and_then(|s| s.get("key-mgmt"))
        .and_then(|v| v.try_clone().ok()?.try_into().ok());
    let (auth, key) = match key_mgmt.as_deref() {
        None | Some("owe") => {
            return Ok(Ok(WifiShare {
                ssid: name,
                auth: ShareAuth::Open,
                password: None,
            }));
        }
        Some("wpa-psk") => (ShareAuth::Wpa, "psk"),
        Some("sae") => (ShareAuth::Sae, "psk"),
        Some("none") => (ShareAuth::Wep, "wep-key0"),
        Some(_) => return Ok(Err(text::cannot_share(&name))),
    };
    // A Profile kept in the keyring (agent-owned secret) is read from there.
    let kept = all
        .get("802-11-wireless-security")
        .and_then(|s| s.get("psk-flags"))
        .and_then(|v| u32::try_from(v.try_clone().ok()?).ok())
        .is_some_and(|f| f & 0x1 != 0);
    if kept {
        return Ok(match crate::keyring::Keyring.lookup(&p.uuid).await {
            Ok(Some(pw)) if !pw.is_empty() => Ok(WifiShare {
                ssid: name,
                auth,
                password: Some(Secret::new(pw.to_string())),
            }),
            Ok(_) => Err(text::no_stored_password(&name)),
            Err(e) => Err(text::keyring_unavailable(&name, &e.0)),
        });
    }
    let secrets = match proxy.get_secrets("802-11-wireless-security").await {
        Ok(s) => s,
        Err(e) if is_denial(&e) => return Ok(Err(text::reveal_denied(&name))),
        Err(e) => return Err(e),
    };
    let password: Option<String> = secrets
        .get("802-11-wireless-security")
        .and_then(|s| s.get(key))
        .and_then(|v| v.try_clone().ok()?.try_into().ok());
    Ok(match password {
        Some(pw) if !pw.is_empty() => Ok(WifiShare {
            ssid: name,
            auth,
            password: Some(Secret::new(pw)),
        }),
        _ => Err(text::no_stored_password(&name)),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn uuids_look_like_version_4() {
        let u = super::new_uuid();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(u, super::new_uuid());
    }

    use super::*;

    #[test]
    fn modern_security_reads_from_the_key_management_bits() {
        let ccmp = 0x8;
        assert_eq!(security(1, 0, KEY_OWE | ccmp), Security::Owe);
        // The open half of an OWE transition pair is joined as OWE too.
        assert_eq!(security(1, 0, KEY_OWE_TM | ccmp), Security::Owe);
        assert_eq!(security(1, 0, KEY_SAE | ccmp), Security::Wpa3);
        assert_eq!(
            security(1, 0, KEY_SAE | KEY_PSK | ccmp),
            Security::Wpa3Transition
        );
        assert_eq!(security(1, 0, KEY_PSK | ccmp), Security::Wpa2);
        assert_eq!(security(1, 0, KEY_EAP_SUITE_B_192), Security::Enterprise);
        assert_eq!(security(0, 0, 0), Security::Open);
    }

    #[test]
    fn band_edges() {
        assert_eq!(band(2412), Band::Ghz2_4);
        assert_eq!(band(5180), Band::Ghz5);
        assert_eq!(band(5955), Band::Ghz6);
        assert_eq!(band(60_000), Band::Unknown);
    }
}
