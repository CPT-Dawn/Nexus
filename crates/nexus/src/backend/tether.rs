//! Bluetooth Tethering on NetworkManager, with BlueZ read only for the
//! adapter's power.
//!
//! A paired phone shows up in NetworkManager as a Device of type
//! `NM_DEVICE_TYPE_BT` (`Device.Bluetooth`: `HwAddress`, `Name`,
//! `BtCapabilities`). Only phones whose capabilities include
//! `NM_BT_CAPABILITY_NAP` can share a network, so those are the ones listed.
//! The Link is a `bluetooth` Profile of type `panu` (we are the client of
//! the phone's network access point). BlueZ is asked once, through its
//! ObjectManager, whether any `org.bluez.Adapter1` is `Powered`; it is never
//! asked while it has no owner on the bus, so a closed Bluetooth service is
//! not started by being looked at.

use std::collections::HashMap;

use nexus_ipc::{Bluetooth, Phone, Tethered, Tethers, Untethered};
use zbus::proxy::CacheProperties;
use zbus::{Connection, OwnedObjectPath, OwnedVariant};

use super::wifi::{new_uuid, variant};
use crate::text::{self, Failure};

/// `NM_DEVICE_TYPE_BT`.
const DEVICE_TYPE_BT: u32 = 5;
/// `NM_BT_CAPABILITY_NAP`.
const CAP_NAP: u32 = 2;

const STATE_DISCONNECTED: u32 = 30;
const STATE_PREPARE: u32 = 40;
const STATE_ACTIVATED: u32 = 100;
const STATE_FAILED: u32 = 120;
/// `NM_DEVICE_STATE_REASON_NEW_ACTIVATION`.
const REASON_NEW_ACTIVATION: u32 = 60;

const LINK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

const BLUEZ: &str = "org.bluez";

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
trait PhoneDevice {
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
    interface = "org.freedesktop.NetworkManager.Device.Bluetooth",
    default_service = "org.freedesktop.NetworkManager"
)]
trait PhoneBluetooth {
    #[zbus(property)]
    fn hw_address(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn bt_capabilities(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
trait PhoneSettings {
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
trait PhoneProfile {
    #[zbus(property)]
    fn unsaved(&self) -> zbus::Result<bool>;
    fn get_settings(&self) -> zbus::Result<HashMap<String, HashMap<String, OwnedVariant>>>;
    fn save(&self) -> zbus::Result<()>;
    fn delete(&self) -> zbus::Result<()>;
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

/// A paired phone as NetworkManager sees it.
struct Found {
    path: OwnedObjectPath,
    phone: Phone,
}

/// Whether BlueZ has an adapter, and whether any is powered.
async fn bluetooth(conn: &Connection) -> Bluetooth {
    use zbus::fdo::{DBusProxy, ObjectManagerProxy};
    let Ok(bus) = DBusProxy::new(conn).await else {
        return Bluetooth::Absent;
    };
    let Ok(name) = zbus::names::BusName::try_from(BLUEZ) else {
        return Bluetooth::Absent;
    };
    // Looking must never start the Bluetooth service by D-Bus activation.
    if !bus.name_has_owner(name).await.unwrap_or(false) {
        return Bluetooth::Absent;
    }
    let managed = async {
        ObjectManagerProxy::builder(conn)
            .destination(BLUEZ)
            .path("/")
            .cache_properties(CacheProperties::No)
            .build()
            .await?
            .get_managed_objects()
            .await
    }
    .await;
    let Ok(objects) = managed else {
        return Bluetooth::Absent;
    };
    let mut adapters = 0;
    let mut powered = false;
    for interfaces in objects.values() {
        if let Some(props) = interfaces.get("org.bluez.Adapter1") {
            adapters += 1;
            powered |= props
                .get("Powered")
                .and_then(|v| bool::try_from(v.try_clone().ok()?).ok())
                .unwrap_or(false);
        }
    }
    match (adapters, powered) {
        (0, _) => Bluetooth::Absent,
        (_, true) => Bluetooth::On,
        (_, false) => Bluetooth::Off,
    }
}

async fn phones(conn: &Connection) -> zbus::Result<Vec<Found>> {
    let nm = super::nm::NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let saved = saved_addresses(conn).await?;
    let mut out = Vec::new();
    for path in nm.get_devices().await? {
        let device = no_cache!(PhoneDeviceProxy, conn, path.as_str());
        // A Device can vanish between the two calls: skip it.
        if device.device_type().await.ok() != Some(DEVICE_TYPE_BT) {
            continue;
        }
        let bt = no_cache!(PhoneBluetoothProxy, conn, path.as_str());
        let (Ok(address), Ok(name), Ok(caps), Ok(state)) = (
            bt.hw_address().await,
            bt.name().await,
            bt.bt_capabilities().await,
            device.state().await,
        ) else {
            continue;
        };
        if caps & CAP_NAP == 0 {
            continue;
        }
        let id = address.to_ascii_uppercase();
        out.push(Found {
            path,
            phone: Phone {
                saved: saved.contains(&id),
                connected: state == STATE_ACTIVATED,
                connecting: (STATE_PREPARE..STATE_ACTIVATED).contains(&state),
                id,
                name,
            },
        });
    }
    out.sort_by(|a, b| {
        (a.phone.name.to_lowercase(), &a.phone.id).cmp(&(b.phone.name.to_lowercase(), &b.phone.id))
    });
    Ok(out)
}

pub async fn list(conn: &Connection) -> zbus::Result<Tethers> {
    Ok(Tethers {
        bluetooth: bluetooth(conn).await,
        phones: phones(conn).await?.into_iter().map(|f| f.phone).collect(),
    })
}

/// `AA:BB:CC:DD:EE:FF` for the six bytes of a `bluetooth.bdaddr`.
fn format_address(bytes: &[u8]) -> Option<String> {
    (bytes.len() == 6).then(|| {
        bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":")
    })
}

fn parse_address(address: &str) -> Option<Vec<u8>> {
    let bytes: Option<Vec<u8>> = address
        .split(':')
        .map(|b| u8::from_str_radix(b, 16).ok())
        .collect();
    bytes.filter(|b| b.len() == 6)
}

/// A saved `bluetooth` Profile: where it is and which phone it is for.
struct Saved {
    path: OwnedObjectPath,
    address: String,
}

async fn saved_profiles(conn: &Connection) -> zbus::Result<Vec<Saved>> {
    let settings = PhoneSettingsProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let mut out = Vec::new();
    for path in settings.list_connections().await? {
        let proxy = no_cache!(PhoneProfileProxy, conn, path.as_str());
        // A Profile only in memory is a Link still coming up (or one that
        // failed): not something the person saved.
        if proxy.unsaved().await.unwrap_or(true) {
            continue;
        }
        let Ok(all) = proxy.get_settings().await else {
            continue;
        };
        let kind: Option<String> = all
            .get("connection")
            .and_then(|c| c.get("type"))
            .and_then(|v| v.try_clone().ok()?.try_into().ok());
        if kind.as_deref() != Some("bluetooth") {
            continue;
        }
        let bytes: Option<Vec<u8>> = all
            .get("bluetooth")
            .and_then(|b| b.get("bdaddr"))
            .and_then(|v| v.try_clone().ok()?.try_into().ok());
        if let Some(address) = bytes.as_deref().and_then(format_address) {
            out.push(Saved { path, address });
        }
    }
    Ok(out)
}

async fn saved_addresses(conn: &Connection) -> zbus::Result<Vec<String>> {
    Ok(saved_profiles(conn)
        .await?
        .into_iter()
        .map(|s| s.address)
        .collect())
}

/// Picks the phone a request means: by address or name, ignoring case, or the
/// only one there is.
fn pick<'a>(found: &'a [Found], who: Option<&str>) -> Result<&'a Found, Failure> {
    let names = || {
        found
            .iter()
            .map(|f| f.phone.name.clone())
            .collect::<Vec<_>>()
    };
    match who {
        Some(who) => found
            .iter()
            .find(|f| f.phone.id.eq_ignore_ascii_case(who))
            .or_else(|| {
                found
                    .iter()
                    .find(|f| f.phone.name.eq_ignore_ascii_case(who))
            })
            .ok_or_else(|| text::unknown_phone(who, &names())),
        None => match found {
            [only] => Ok(only),
            _ => Err(text::which_phone(&names())),
        },
    }
}

/// Brings a Tethering Link up and waits for the outcome. A Profile is saved
/// only once the Link works. The inner error is something to tell the
/// person; the outer one is a bus problem.
pub async fn connect(
    conn: &Connection,
    who: Option<&str>,
) -> zbus::Result<Result<Tethered, Failure>> {
    match bluetooth(conn).await {
        Bluetooth::On => {}
        Bluetooth::Off => return Ok(Err(text::bluetooth_off())),
        Bluetooth::Absent => return Ok(Err(text::bluetooth_absent())),
    }
    let found = phones(conn).await?;
    if found.is_empty() {
        return Ok(Err(text::no_phone()));
    }
    let chosen = match pick(&found, who) {
        Ok(f) => f,
        Err(failure) => return Ok(Err(failure)),
    };
    let phone = &chosen.phone;
    let done = Tethered {
        phone: phone.name.clone(),
        id: phone.id.clone(),
    };
    if phone.connected {
        return Ok(Ok(done));
    }
    let Some(addr) = parse_address(&phone.id) else {
        return Ok(Err(text::tether_refused(&phone.name, "its address is odd")));
    };
    let nm = super::nm::NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let device = no_cache!(PhoneDeviceProxy, conn, chosen.path.as_str());
    // Subscribe before asking: the Device may move on before the call returns.
    let mut states = device.receive_device_state_changed().await?;
    let mut owner = zbus::fdo::DBusProxy::new(conn)
        .await?
        .receive_name_owner_changed_with_args(&[(0, "org.freedesktop.NetworkManager")])
        .await?;
    let none = OwnedObjectPath::try_from("/").expect("root path");

    let saved = saved_profiles(conn)
        .await?
        .into_iter()
        .find(|s| s.address == phone.id);
    let (profile, created) = match saved {
        Some(s) => {
            if let Err(e) = nm.activate_connection(&s.path, &chosen.path, &none).await {
                return Ok(Err(text::tether_refused(&phone.name, &e.to_string())));
            }
            (s.path, false)
        }
        None => {
            let settings = new_profile(&phone.name, &addr);
            let options = HashMap::from([("persist".to_owned(), variant("memory"))]);
            match nm
                .add_and_activate_connection2(settings, &chosen.path, &none, options)
                .await
            {
                Ok((path, _active, _result)) => (path, true),
                Err(e) => return Ok(Err(text::tether_refused(&phone.name, &e.to_string()))),
            }
        }
    };

    let outcome = wait_for_link(&mut states, &mut owner, &phone.name).await;
    let profile = no_cache!(PhoneProfileProxy, conn, profile.as_str());
    match outcome {
        Ok(()) => {
            if created && let Err(e) = profile.save().await {
                let _ = profile.delete().await;
                return Ok(Err(text::tether_refused(&phone.name, &e.to_string())));
            }
            Ok(Ok(done))
        }
        Err(failure) => {
            if created {
                let _ = profile.delete().await;
            }
            // Stop NetworkManager retrying a Link nobody wants any more.
            let _ = device.disconnect().await;
            Ok(Err(failure))
        }
    }
}

/// The settings for a new Profile: a PANU client of the phone's access point.
fn new_profile(name: &str, bdaddr: &[u8]) -> HashMap<String, HashMap<String, OwnedVariant>> {
    let connection = HashMap::from([
        ("id".to_owned(), variant(name)),
        ("type".to_owned(), variant("bluetooth")),
        ("uuid".to_owned(), variant(new_uuid().as_str())),
        // Joining the phone's network by itself is a surprise: opt in later.
        ("autoconnect".to_owned(), variant(false)),
    ]);
    let bluetooth = HashMap::from([
        ("bdaddr".to_owned(), variant(bdaddr.to_vec())),
        ("type".to_owned(), variant("panu")),
    ]);
    let auto = || HashMap::from([("method".to_owned(), variant("auto"))]);
    HashMap::from([
        ("connection".to_owned(), connection),
        ("bluetooth".to_owned(), bluetooth),
        ("ipv4".to_owned(), auto()),
        ("ipv6".to_owned(), auto()),
    ])
}

async fn wait_for_link(
    states: &mut StateChangedStream,
    owner: &mut zbus::fdo::NameOwnerChangedStream,
    name: &str,
) -> Result<(), Failure> {
    use futures_util::StreamExt as _;
    let deadline = tokio::time::sleep(LINK_TIMEOUT);
    tokio::pin!(deadline);
    let mut started = false;
    loop {
        let signal = tokio::select! {
            s = states.next() => s,
            _ = owner.next() => return Err(text::tether_interrupted(name)),
            () = &mut deadline => return Err(text::tether_timed_out(name)),
        };
        let Some(signal) = signal else {
            return Err(text::tether_refused(name, "NetworkManager went away"));
        };
        let Ok(args) = signal.args() else { continue };
        let (new, old, reason) = (args.new_state, args.old_state, args.reason);
        match new {
            STATE_ACTIVATED => return Ok(()),
            STATE_FAILED => return Err(text::tether_failure(name, old, reason)),
            n if n >= STATE_PREPARE => started = true,
            n if n <= STATE_DISCONNECTED && started && reason != REASON_NEW_ACTIVATION => {
                return Err(text::tether_failure(name, old, reason));
            }
            _ => {}
        }
    }
}

/// Drops the Tethering Link that is up or coming up, keeping the Profile.
pub async fn disconnect(conn: &Connection) -> zbus::Result<Result<Untethered, Failure>> {
    let mut dropped = None;
    for found in phones(conn).await? {
        if !(found.phone.connected || found.phone.connecting) {
            continue;
        }
        let device = no_cache!(PhoneDeviceProxy, conn, found.path.as_str());
        if device.disconnect().await.is_ok() {
            dropped.get_or_insert(found.phone.name);
        }
    }
    Ok(match dropped {
        Some(phone) => Ok(Untethered { phone }),
        None => Err(text::not_tethered()),
    })
}
