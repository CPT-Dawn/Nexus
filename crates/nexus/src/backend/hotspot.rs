//! Hotspot on NetworkManager: a Wi-Fi Profile in access point mode
//! (`802-11-wireless.mode = ap`) with `ipv4.method = shared`, so NetworkManager
//! runs DHCP and NAT for the devices that join.
//!
//! Whether a card can host one is `Device.Wireless.WirelessCapabilities`
//! bit `NM_WIFI_DEVICE_CAP_AP`. NetworkManager does not list the stations
//! that joined, so they are counted with `iw dev <interface> station dump`
//! (read-only; without `iw` the count is simply unknown). The Profile lives
//! in memory only and is deleted when the Hotspot stops: its password is made
//! up for each run, so there is nothing worth keeping.

use std::collections::HashMap;
use std::time::Duration;

use futures_util::StreamExt as _;
use nexus_ipc::{Hotspot, HotspotStarted, HotspotStopped, Secret, ShareAuth, WifiShare};
use zbus::proxy::CacheProperties;
use zbus::{Connection, OwnedObjectPath};

use super::nm::{ActiveConnectionProxy, NetworkManagerProxy};
use super::wifi::{SettingsConnectionProxy, is_denial, new_uuid, variant};
use crate::text::{self, Failure};

/// `NM_DEVICE_TYPE_WIFI`.
const DEVICE_TYPE_WIFI: u32 = 2;
/// `NM_WIFI_DEVICE_CAP_AP`.
const CAP_AP: u32 = 0x40;
/// `NMActiveConnectionState`.
const ACTIVATING: u32 = 1;
const ACTIVATED: u32 = 2;
const DEACTIVATED: u32 = 4;

const UP_TIMEOUT: Duration = Duration::from_secs(45);

/// No look-alike characters (0/O, 1/l/I) and a power-of-two alphabet, so
/// picking a letter from a random byte has no bias.
const ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
/// 16 letters of 5 bits each: 80 bits.
const PASSWORD_LEN: usize = 16;

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
trait HotspotDevice {
    #[zbus(property)]
    fn interface(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager"
)]
trait HotspotWireless {
    #[zbus(property)]
    fn wireless_capabilities(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Connection.Active",
    default_service = "org.freedesktop.NetworkManager"
)]
trait HotspotActive {
    #[zbus(property)]
    fn connection(&self) -> zbus::Result<OwnedObjectPath>;
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

fn random(n: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; n];
    // Blocking on the entropy pool only happens before it is seeded at boot.
    let _ = rustix::rand::getrandom(&mut bytes, rustix::rand::GetRandomFlags::empty());
    bytes
}

/// A strong password: 16 characters, 80 bits, easy to read off a screen.
pub fn generate_password() -> String {
    random(PASSWORD_LEN)
        .into_iter()
        .map(|b| ALPHABET[usize::from(b & 31)] as char)
        .collect()
}

/// `Nexus-` and four letters, so two machines do not clash.
pub fn generate_ssid() -> String {
    let tail: String = random(4)
        .into_iter()
        .map(|b| ALPHABET[usize::from(b & 31)] as char)
        .collect();
    format!("Nexus-{tail}")
}

/// A name and password WPA2 accepts, or the sentence that says why not.
pub fn validate(ssid: &str, password: &str) -> Result<(), Failure> {
    if ssid.is_empty() || ssid.len() > 32 {
        return Err(text::invalid_hotspot(
            "A Hotspot name must be 1 to 32 bytes long.",
        ));
    }
    if ssid.chars().any(char::is_control) {
        return Err(text::invalid_hotspot(
            "A Hotspot name cannot contain control characters.",
        ));
    }
    let hex64 = password.len() == 64 && password.bytes().all(|b| b.is_ascii_hexdigit());
    if !hex64 && !(8..=63).contains(&password.len()) {
        return Err(text::invalid_hotspot(
            "A Hotspot password must be 8 to 63 characters long.",
        ));
    }
    if !password.is_ascii() || password.chars().any(char::is_control) {
        return Err(text::invalid_hotspot(
            "A Hotspot password can only use plain letters, digits and symbols.",
        ));
    }
    Ok(())
}

/// A Wi-Fi Device and whether it can be an access point.
struct Card {
    path: OwnedObjectPath,
    can_host: bool,
}

async fn cards(conn: &Connection) -> zbus::Result<Vec<Card>> {
    let nm = NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let mut out = Vec::new();
    for path in nm.get_devices().await? {
        let device = no_cache!(HotspotDeviceProxy, conn, path.as_str());
        if device.device_type().await.ok() != Some(DEVICE_TYPE_WIFI) {
            continue;
        }
        let wireless = no_cache!(HotspotWirelessProxy, conn, path.as_str());
        let caps = wireless.wireless_capabilities().await.unwrap_or(0);
        out.push(Card {
            path,
            can_host: caps & CAP_AP != 0,
        });
    }
    Ok(out)
}

/// Why this machine cannot host a Hotspot, if it cannot.
async fn unsupported(conn: &Connection) -> zbus::Result<Option<Failure>> {
    let nm = NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let cards = cards(conn).await?;
    Ok(if cards.is_empty() {
        Some(text::no_wifi_for_hotspot())
    } else if !nm.wireless_enabled().await.unwrap_or(true) {
        Some(text::wifi_off_for_hotspot())
    } else if cards.iter().all(|c| !c.can_host) {
        Some(text::card_cannot_host())
    } else {
        None
    })
}

/// The Hotspot that is up or coming up.
struct Running {
    active: OwnedObjectPath,
    profile: OwnedObjectPath,
    ssid: String,
    state: u32,
    device: Option<OwnedObjectPath>,
}

fn text_of(
    all: &HashMap<String, HashMap<String, zbus::OwnedVariant>>,
    section: &str,
    key: &str,
) -> Option<String> {
    all.get(section)?
        .get(key)?
        .try_clone()
        .ok()?
        .try_into()
        .ok()
}

async fn running(conn: &Connection) -> zbus::Result<Option<Running>> {
    let nm = NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    for path in nm.active_connections().await? {
        let active = no_cache!(ActiveConnectionProxy, conn, path.as_str());
        // An active connection can vanish between the reads.
        let (Ok(kind), Ok(state)) = (active.kind().await, active.state().await) else {
            continue;
        };
        if kind != "802-11-wireless" || !matches!(state, ACTIVATING | ACTIVATED) {
            continue;
        }
        let Ok(profile) = no_cache!(HotspotActiveProxy, conn, path.as_str())
            .connection()
            .await
        else {
            continue;
        };
        let proxy = no_cache!(SettingsConnectionProxy, conn, profile.as_str());
        let Ok(all) = proxy.get_settings().await else {
            continue;
        };
        if text_of(&all, "802-11-wireless", "mode").as_deref() != Some("ap") {
            continue;
        }
        let ssid: Vec<u8> = all
            .get("802-11-wireless")
            .and_then(|w| w.get("ssid"))
            .and_then(|v| v.try_clone().ok()?.try_into().ok())
            .unwrap_or_default();
        return Ok(Some(Running {
            device: active
                .devices()
                .await
                .ok()
                .and_then(|d| d.into_iter().next()),
            active: path,
            profile,
            ssid: String::from_utf8_lossy(&ssid).into_owned(),
            state,
        }));
    }
    Ok(None)
}

/// How many stations are joined to `interface`, from `iw`. `None` when `iw`
/// is missing or fails.
async fn count_stations(interface: &str) -> Option<u32> {
    let out = tokio::process::Command::new("iw")
        .args(["dev", interface, "station", "dump"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(3), out)
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let n = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("Station "))
        .count();
    u32::try_from(n).ok()
}

/// The picture for `hotspot` and `hotspot_changed`.
pub async fn state(conn: &Connection) -> zbus::Result<Hotspot> {
    if let Some(run) = running(conn).await? {
        let devices = match &run.device {
            Some(path) if run.state == ACTIVATED => {
                let device = no_cache!(HotspotDeviceProxy, conn, path.as_str());
                match device.interface().await {
                    Ok(interface) => count_stations(&interface).await,
                    Err(_) => None,
                }
            }
            _ => None,
        };
        return Ok(Hotspot {
            available: true,
            why: None,
            on: true,
            starting: run.state == ACTIVATING,
            ssid: Some(run.ssid),
            devices,
        });
    }
    Ok(match unsupported(conn).await? {
        Some(why) => Hotspot {
            available: false,
            why: Some(why.message),
            ..Hotspot::default()
        },
        None => Hotspot {
            available: true,
            ..Hotspot::default()
        },
    })
}

/// The settings of a Hotspot Profile, as `nmcli device wifi hotspot` writes them.
fn profile(ssid: &str, password: &str) -> HashMap<String, HashMap<String, zbus::OwnedVariant>> {
    let connection = HashMap::from([
        ("id".to_owned(), variant(format!("Hotspot {ssid}"))),
        ("type".to_owned(), variant("802-11-wireless")),
        ("uuid".to_owned(), variant(new_uuid())),
        ("autoconnect".to_owned(), variant(false)),
    ]);
    let wireless = HashMap::from([
        ("ssid".to_owned(), variant(ssid.as_bytes().to_vec())),
        ("mode".to_owned(), variant("ap")),
    ]);
    let security = HashMap::from([
        ("key-mgmt".to_owned(), variant("wpa-psk")),
        ("psk".to_owned(), variant(password)),
        ("proto".to_owned(), variant(vec!["rsn".to_owned()])),
        ("pairwise".to_owned(), variant(vec!["ccmp".to_owned()])),
        ("group".to_owned(), variant(vec!["ccmp".to_owned()])),
    ]);
    HashMap::from([
        ("connection".to_owned(), connection),
        ("802-11-wireless".to_owned(), wireless),
        ("802-11-wireless-security".to_owned(), security),
        (
            "ipv4".to_owned(),
            HashMap::from([("method".to_owned(), variant("shared"))]),
        ),
        (
            "ipv6".to_owned(),
            HashMap::from([("method".to_owned(), variant("ignore"))]),
        ),
    ])
}

/// Starts a Hotspot and waits until it is up. The inner error is for the
/// person; the outer one is a bus problem.
pub async fn start(
    conn: &Connection,
    ssid: Option<&str>,
    password: Option<&Secret>,
) -> zbus::Result<Result<HotspotStarted, Failure>> {
    let generated = password.is_none();
    let ssid = ssid.map_or_else(generate_ssid, str::to_owned);
    let password = password.map_or_else(|| Secret::new(generate_password()), Clone::clone);
    if let Err(failure) = validate(&ssid, password.expose()) {
        return Ok(Err(failure));
    }
    if let Some(run) = running(conn).await? {
        return Ok(Err(text::hotspot_running(&run.ssid)));
    }
    if let Some(why) = unsupported(conn).await? {
        return Ok(Err(why));
    }
    let Some(card) = cards(conn).await?.into_iter().find(|c| c.can_host) else {
        return Ok(Err(text::card_cannot_host()));
    };
    let nm = NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let none = OwnedObjectPath::try_from("/").expect("root path");
    let options = HashMap::from([("persist".to_owned(), variant("memory"))]);
    let settings = profile(&ssid, password.expose());
    let (profile_path, active_path) = match nm
        .add_and_activate_connection2(settings, &card.path, &none, options)
        .await
    {
        Ok((profile, active, _)) => (profile, active),
        Err(e) if is_denial(&e) => {
            return Ok(Err(text::hotspot_failed(
                &ssid,
                "NetworkManager did not authorize it",
            )));
        }
        Err(zbus::Error::MethodError(_, Some(why), _)) => {
            return Ok(Err(text::hotspot_failed(&ssid, why.trim_end_matches('.'))));
        }
        Err(e) => return Err(e),
    };
    let active = ActiveConnectionProxy::builder(conn)
        .path(active_path.as_str())
        .build()
        .await?;
    // Opened before the state is read, so no change slips by.
    let mut changes = active.receive_state_changed().await;
    let outcome = tokio::time::timeout(UP_TIMEOUT, async {
        let mut state = active.state().await.unwrap_or(DEACTIVATED);
        loop {
            match state {
                ACTIVATED => return true,
                ACTIVATING => {}
                _ => return false,
            }
            match changes.next().await {
                Some(change) => state = change.get().await.unwrap_or(DEACTIVATED),
                None => return false,
            }
        }
    })
    .await;
    let failure = match outcome {
        Ok(true) => {
            return Ok(Ok(HotspotStarted {
                ssid,
                password,
                generated,
            }));
        }
        Ok(false) => text::hotspot_failed(
            &ssid,
            "NetworkManager gave up on it; the driver may not allow a Hotspot right now",
        ),
        Err(_) => text::hotspot_timed_out(&ssid),
    };
    // Nothing is left behind by a Hotspot that never came up.
    let _ = nm.deactivate_connection(&active_path).await;
    let _ = no_cache!(SettingsConnectionProxy, conn, profile_path.as_str())
        .delete()
        .await;
    Ok(Err(failure))
}

/// Stops the running Hotspot and deletes its Profile.
pub async fn stop(conn: &Connection) -> zbus::Result<Result<HotspotStopped, Failure>> {
    let Some(run) = running(conn).await? else {
        return Ok(Err(text::hotspot_not_running()));
    };
    let nm = NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    nm.deactivate_connection(&run.active).await?;
    // The Profile is made up for one run; a failure to delete it is not worth
    // failing the stop for.
    let _ = no_cache!(SettingsConnectionProxy, conn, run.profile.as_str())
        .delete()
        .await;
    Ok(Ok(HotspotStopped { ssid: run.ssid }))
}

/// The running Hotspot's join details, for a QR code. The password comes
/// from `GetSecrets`, which polkit may gate: a refusal is `reveal_denied`.
pub async fn reveal(conn: &Connection) -> zbus::Result<Result<WifiShare, Failure>> {
    let Some(run) = running(conn).await? else {
        return Ok(Err(text::hotspot_not_running()));
    };
    let proxy = no_cache!(SettingsConnectionProxy, conn, run.profile.as_str());
    let secrets = match proxy.get_secrets("802-11-wireless-security").await {
        Ok(s) => s,
        Err(e) if is_denial(&e) => return Ok(Err(text::reveal_denied(&run.ssid))),
        Err(e) => return Err(e),
    };
    let password = text_of(&secrets, "802-11-wireless-security", "psk").filter(|p| !p.is_empty());
    Ok(match password {
        Some(pw) => Ok(WifiShare {
            ssid: run.ssid,
            auth: ShareAuth::Wpa,
            password: Some(Secret::new(pw)),
        }),
        None => Err(text::no_stored_password(&run.ssid)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_passwords_are_strong_and_unalike() {
        let a = generate_password();
        let b = generate_password();
        assert_eq!(a.len(), PASSWORD_LEN);
        assert_ne!(a, b);
        assert!(validate("Home", &a).is_ok());
        assert!(a.bytes().all(|c| ALPHABET.contains(&c)));
        assert!(!a.contains(['0', 'O', '1', 'I', 'l']));
    }

    #[test]
    fn generated_names_are_valid() {
        let s = generate_ssid();
        assert!(s.starts_with("Nexus-") && s.len() == 10, "{s}");
    }

    #[test]
    fn names_and_passwords_are_checked() {
        assert!(validate("", "longenough").is_err());
        assert!(validate(&"x".repeat(33), "longenough").is_err());
        assert!(validate("ok", "short").is_err());
        assert!(validate("ok", &"p".repeat(64)).is_err()); // 64 must be hex
        assert!(validate("ok", &"a".repeat(64)).is_ok());
        assert!(validate("ok", "pässwörd-long").is_err());
        assert!(validate("ok", "eightchr").is_ok());
        assert!(validate("tab\there", "eightchr").is_err());
    }
}
