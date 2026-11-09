//! Ethernet on NetworkManager: ports (Devices of type 1), their Link details,
//! joining with a saved or fresh Profile, and wired 802.1X.
//!
//! Member names follow NetworkManager's D-Bus API reference:
//! `org.freedesktop.NetworkManager.Device` (`Interface`, `DeviceType`, `State`,
//! `StateReason`, `ActiveConnection`, `Ip4Config`, `Ip6Config`),
//! `.Device.Wired` (`HwAddress`, `Speed`, `Carrier`), `.IP4Config`
//! (`AddressData`, `Gateway`, `NameserverData`) and `.IP6Config`
//! (`AddressData`).

use std::collections::HashMap;

use nexus_ipc::{
    AddressEntry, Eap, EthernetDropped, EthernetJoined, EthernetList, EthernetPort,
    EthernetProfile, EthernetState, EthernetTrouble,
};
use zbus::proxy::CacheProperties;
use zbus::{Connection, OwnedObjectPath, OwnedVariant};

use super::nm::{ActiveConnectionProxy, NetworkManagerProxy};
use super::wifi::{Settings, SettingsConnectionProxy, SettingsProxy, new_uuid, variant};
use crate::text::{self, Failure};

/// `NM_DEVICE_TYPE_ETHERNET`.
const DEVICE_TYPE_ETHERNET: u32 = 1;

// NMDeviceState
const STATE_DISCONNECTED: u32 = 30;
const STATE_PREPARE: u32 = 40;
const STATE_ACTIVATED: u32 = 100;
const STATE_FAILED: u32 = 120;
/// `NM_DEVICE_STATE_REASON_NEW_ACTIVATION`.
const REASON_NEW_ACTIVATION: u32 = 60;

/// How long a join may take. NetworkManager's own activation timeout is 60 s.
const JOIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Port {
    #[zbus(property)]
    fn interface(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn state_reason(&self) -> zbus::Result<(u32, u32)>;
    #[zbus(property)]
    fn active_connection(&self) -> zbus::Result<OwnedObjectPath>;
    #[zbus(property, name = "Ip4Config")]
    fn ip4_config(&self) -> zbus::Result<OwnedObjectPath>;
    #[zbus(property, name = "Ip6Config")]
    fn ip6_config(&self) -> zbus::Result<OwnedObjectPath>;
    fn disconnect(&self) -> zbus::Result<()>;
    #[zbus(signal, name = "StateChanged")]
    fn device_state_changed(&self, new_state: u32, old_state: u32, reason: u32)
    -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wired",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Wired {
    #[zbus(property)]
    fn hw_address(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn speed(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn carrier(&self) -> zbus::Result<bool>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.IP4Config",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Ip4 {
    #[zbus(property)]
    fn gateway(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn address_data(&self) -> zbus::Result<Vec<HashMap<String, OwnedVariant>>>;
    #[zbus(property)]
    fn nameserver_data(&self) -> zbus::Result<Vec<HashMap<String, OwnedVariant>>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.IP6Config",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Ip6 {
    #[zbus(property)]
    fn address_data(&self) -> zbus::Result<Vec<HashMap<String, OwnedVariant>>>;
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

/// A wired Profile as read.
#[derive(Debug, Clone)]
pub(super) struct WiredProfile {
    pub path: OwnedObjectPath,
    pub uuid: String,
    pub name: String,
    /// `connection.interface-name`, empty when the Profile fits any port.
    interface: String,
    enterprise: bool,
}

fn text_of(section: &HashMap<String, OwnedVariant>, key: &str) -> Option<String> {
    section.get(key)?.try_clone().ok()?.try_into().ok()
}

fn parse_profile(path: OwnedObjectPath, all: &Settings) -> Option<WiredProfile> {
    let connection = all.get("connection")?;
    if text_of(connection, "type")? != "802-3-ethernet" {
        return None;
    }
    Some(WiredProfile {
        path,
        uuid: text_of(connection, "uuid")?,
        name: text_of(connection, "id")?,
        interface: text_of(connection, "interface-name").unwrap_or_default(),
        enterprise: all.contains_key("802-1x"),
    })
}

async fn wired_profiles(conn: &Connection) -> zbus::Result<Vec<WiredProfile>> {
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

/// The wired Profile with this uuid, if one is saved.
pub(super) async fn find_profile(
    conn: &Connection,
    uuid: &str,
) -> zbus::Result<Option<WiredProfile>> {
    Ok(wired_profiles(conn)
        .await?
        .into_iter()
        .find(|p| p.uuid == uuid))
}

/// A port as found on the bus.
struct Port {
    path: OwnedObjectPath,
    name: String,
}

async fn ports(conn: &Connection) -> zbus::Result<Vec<Port>> {
    let nm = NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let mut out = Vec::new();
    for path in nm.get_devices().await? {
        // A Device can vanish between the two calls: skip it.
        let device = no_cache!(PortProxy, conn, path.as_str());
        let (Ok(kind), Ok(name)) = (device.device_type().await, device.interface().await) else {
            continue;
        };
        if kind == DEVICE_TYPE_ETHERNET {
            out.push(Port { path, name });
        }
    }
    Ok(out)
}

/// The Devices that carry the Profile `uuid` right now.
pub(super) async fn carrying(conn: &Connection, uuid: &str) -> zbus::Result<Vec<OwnedObjectPath>> {
    let mut out = Vec::new();
    for port in ports(conn).await? {
        let device = no_cache!(PortProxy, conn, port.path.as_str());
        if active_uuid(conn, &device).await.as_deref() == Some(uuid) {
            out.push(port.path);
        }
    }
    Ok(out)
}

/// The uuid of the Profile `device` has active (or is activating), if any.
async fn active_uuid(conn: &Connection, device: &PortProxy<'_>) -> Option<String> {
    let path = device.active_connection().await.ok()?;
    if path.as_str() == "/" {
        return None;
    }
    let active = ActiveConnectionProxy::builder(conn)
        .path(path.as_str())
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .ok()?;
    active.uuid().await.ok().filter(|u| !u.is_empty())
}

/// NetworkManager's reasons that mean "joining did not work", as opposed to
/// someone dropping the Link or a cable coming out.
fn is_failure_reason(reason: u32) -> bool {
    matches!(reason, 5..=11 | 15..=17)
}

/// Where a port stands, from what NetworkManager says. `enterprise` is whether
/// the Profile signs in with 802.1X.
pub(super) fn classify(
    name: &str,
    carrier: bool,
    state: u32,
    reason: u32,
    enterprise: bool,
) -> (EthernetState, Option<EthernetTrouble>) {
    if !carrier {
        return (EthernetState::Unplugged, None);
    }
    match state {
        STATE_ACTIVATED => (EthernetState::Connected, None),
        n if (STATE_PREPARE..STATE_ACTIVATED).contains(&n) => (EthernetState::Connecting, None),
        STATE_FAILED => (
            EthernetState::Misconfigured,
            Some(text::trouble(name, reason, enterprise)),
        ),
        // After a failed attempt NetworkManager rests at "disconnected"
        // and remembers why.
        _ if is_failure_reason(reason) && state <= STATE_DISCONNECTED && state >= 20 => (
            EthernetState::Misconfigured,
            Some(text::trouble(name, reason, enterprise)),
        ),
        _ => (EthernetState::Disconnected, None),
    }
}

fn rows(v: zbus::Result<Vec<HashMap<String, OwnedVariant>>>) -> Vec<HashMap<String, OwnedVariant>> {
    v.unwrap_or_default()
}

fn address_of(row: &HashMap<String, OwnedVariant>) -> Option<AddressEntry> {
    let address = text_of(row, "address")?;
    let prefix = row.get("prefix")?.try_clone().ok()?;
    let prefix = u32::try_from(prefix)
        .ok()
        .and_then(|p| u8::try_from(p).ok())?;
    Some(AddressEntry { address, prefix })
}

/// IPv6 link-local addresses (`fe80::/10`) are on every port and say nothing.
fn link_local(address: &str) -> bool {
    address
        .parse::<std::net::Ipv6Addr>()
        .is_ok_and(|a| a.segments()[0] & 0xffc0 == 0xfe80)
}

/// Addresses, gateway and DNS of a Link that is up. Best effort: the
/// configuration can vanish between reads.
async fn link_details(
    conn: &Connection,
    device: &PortProxy<'_>,
    port: &mut EthernetPort,
) -> zbus::Result<()> {
    let ip4 = device.ip4_config().await?;
    if ip4.as_str() != "/" {
        let cfg = no_cache!(Ip4Proxy, conn, ip4.as_str());
        port.addresses
            .extend(rows(cfg.address_data().await).iter().filter_map(address_of));
        port.gateway = cfg.gateway().await.ok().filter(|g| !g.is_empty());
        port.dns.extend(
            rows(cfg.nameserver_data().await)
                .iter()
                .filter_map(|r| text_of(r, "address")),
        );
    }
    let ip6 = device.ip6_config().await?;
    if ip6.as_str() != "/" {
        let cfg = no_cache!(Ip6Proxy, conn, ip6.as_str());
        port.addresses.extend(
            rows(cfg.address_data().await)
                .iter()
                .filter_map(address_of)
                .filter(|a| !link_local(&a.address)),
        );
    }
    Ok(())
}

/// The Profile a port uses, or would: the active one, else a saved one made
/// for this port, else one that fits any port.
fn profile_for<'a>(
    profiles: &'a [WiredProfile],
    port: &str,
    active: Option<&str>,
) -> Option<&'a WiredProfile> {
    active
        .and_then(|u| profiles.iter().find(|p| p.uuid == u))
        .or_else(|| profiles.iter().find(|p| p.interface == port))
        .or_else(|| profiles.iter().find(|p| p.interface.is_empty()))
}

pub async fn list(conn: &Connection) -> zbus::Result<EthernetList> {
    let found = ports(conn).await?;
    if found.is_empty() {
        // Most laptops with Wi-Fi only: read nothing more on every tick.
        return Ok(EthernetList::default());
    }
    let profiles = wired_profiles(conn).await?;
    let mut out = Vec::new();
    for port in found {
        let device = no_cache!(PortProxy, conn, port.path.as_str());
        let wired = no_cache!(WiredProxy, conn, port.path.as_str());
        // A port that vanishes mid-read is skipped, not an error.
        let (Ok(state), Ok(carrier)) = (device.state().await, wired.carrier().await) else {
            continue;
        };
        let reason = device.state_reason().await.map_or(0, |(_, r)| r);
        let active = active_uuid(conn, &device).await;
        let profile = profile_for(&profiles, &port.name, active.as_deref());
        let enterprise = profile.is_some_and(|p| p.enterprise);
        let (state, trouble) = classify(&port.name, carrier, state, reason, enterprise);
        let mut row = EthernetPort::bare(&port.name, state);
        row.mac = wired
            .hw_address()
            .await
            .ok()
            .filter(|m| !m.is_empty())
            .map(|m| m.to_ascii_uppercase());
        row.speed_mbps = if carrier {
            wired.speed().await.ok().filter(|s| *s > 0)
        } else {
            None
        };
        row.profile = profile.map(|p| EthernetProfile {
            uuid: p.uuid.clone(),
            name: p.name.clone(),
            enterprise: p.enterprise,
        });
        row.trouble = trouble;
        if state == EthernetState::Connected {
            let _ = link_details(conn, &device, &mut row).await;
        }
        out.push(row);
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(EthernetList { ports: out })
}

/// Picks the port the person means: by name, or the only one.
fn pick<'a>(all: &'a [Port], asked: Option<&str>) -> Result<&'a Port, Failure> {
    let names: Vec<String> = all.iter().map(|p| p.name.clone()).collect();
    match (asked, all) {
        (Some(name), _) => all
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| text::unknown_port(Some(name), &names)),
        (None, [only]) => Ok(only),
        (None, _) => Err(text::unknown_port(None, &names)),
    }
}

/// A new Profile for a port: DHCP, optionally signing in with 802.1X.
fn new_profile(port: &str, uuid: &str, eap: Option<&Eap>) -> Settings {
    let connection = HashMap::from([
        ("id".to_owned(), variant(format!("Wired {port}"))),
        ("type".to_owned(), variant("802-3-ethernet")),
        ("uuid".to_owned(), variant(uuid)),
        ("interface-name".to_owned(), variant(port)),
        ("autoconnect".to_owned(), variant(true)),
    ]);
    let auto = || HashMap::from([("method".to_owned(), variant("auto"))]);
    let mut all = HashMap::from([
        ("connection".to_owned(), connection),
        ("802-3-ethernet".to_owned(), HashMap::new()),
        ("ipv4".to_owned(), auto()),
        ("ipv6".to_owned(), auto()),
    ]);
    if let Some(eap) = eap {
        all.insert("802-1x".to_owned(), super::eap::setting(eap));
    }
    all
}

/// What `join` does to get the Profile in place.
enum Plan<'a> {
    /// Use the saved Profile as it is.
    Saved,
    /// Use the saved Profile, signing in with these settings first.
    SignIn(&'a Eap),
}

/// Brings a port up and waits for the outcome. The inner error is something to
/// tell the person; the outer one is a bus problem.
async fn join(
    conn: &Connection,
    asked: Option<&str>,
    plan: Plan<'_>,
) -> zbus::Result<Result<EthernetJoined, Failure>> {
    let all = ports(conn).await?;
    let port = match pick(&all, asked) {
        Ok(p) => p,
        Err(f) => return Ok(Err(f)),
    };
    let name = port.name.clone();
    let device = no_cache!(PortProxy, conn, port.path.as_str());
    let wired = no_cache!(WiredProxy, conn, port.path.as_str());
    if !wired.carrier().await? {
        return Ok(Err(text::unplugged(&name)));
    }
    let eap = match &plan {
        Plan::SignIn(eap) => {
            let errors = eap.validate();
            if !errors.is_empty() {
                return Ok(Err(text::invalid_eap(&name, &errors)));
            }
            Some(*eap)
        }
        Plan::Saved => None,
    };

    let profiles = wired_profiles(conn).await?;
    let active = active_uuid(conn, &device).await;
    let saved = profile_for(&profiles, &name, active.as_deref()).cloned();

    let nm = NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    // Subscribe before asking: the Device may move on before the call returns.
    let mut states = device.receive_device_state_changed().await?;
    // NetworkManager restarting forgets the join; do not wait it out.
    let mut owner = zbus::fdo::DBusProxy::new(conn)
        .await?
        .receive_name_owner_changed_with_args(&[(0, "org.freedesktop.NetworkManager")])
        .await?;
    let none = OwnedObjectPath::try_from("/").expect("root path");

    // `restore` puts a saved Profile back as it was if signing in fails.
    let mut restore: Option<(SettingsConnectionProxy<'_>, Settings)> = None;
    let (profile_path, uuid, created, enterprise) = match saved {
        Some(p) => {
            if let Some(eap) = eap {
                let proxy = no_cache!(SettingsConnectionProxy, conn, p.path.clone());
                let before = proxy.get_settings().await?;
                let mut after = before.clone();
                after.insert("802-1x".to_owned(), super::eap::setting(eap));
                proxy.update(after).await?;
                restore = Some((proxy, before));
            }
            if let Err(e) = nm.activate_connection(&p.path, &port.path, &none).await {
                return Ok(Err(text::ethernet_refused(&name, &e.to_string())));
            }
            let enterprise = p.enterprise || eap.is_some();
            (p.path, p.uuid, false, enterprise)
        }
        None => {
            let uuid = new_uuid();
            let settings = new_profile(&name, &uuid, eap);
            let options = HashMap::from([("persist".to_owned(), variant("memory"))]);
            match nm
                .add_and_activate_connection2(settings, &port.path, &none, options)
                .await
            {
                Ok((path, _active, _result)) => (path, uuid, true, eap.is_some()),
                Err(e) => return Ok(Err(text::ethernet_refused(&name, &e.to_string()))),
            }
        }
    };

    let outcome = wait_for_link(&mut states, &mut owner, &name, enterprise).await;
    let settings = no_cache!(SettingsConnectionProxy, conn, profile_path.as_str());
    match outcome {
        Ok(()) => {
            if created && let Err(e) = settings.save().await {
                // Up, but not remembered: say so rather than pretend.
                let _ = settings.delete().await;
                return Ok(Err(text::ethernet_refused(&name, &e.to_string())));
            }
            Ok(Ok(EthernetJoined {
                port: name,
                profile: uuid,
            }))
        }
        Err(failure) => {
            if created {
                // No lingering Profile for a join that never worked.
                let _ = settings.delete().await;
            } else if let Some((proxy, before)) = restore {
                // The sign-in did not work: the old Profile stays as it was.
                let _ = proxy.update(before).await;
            }
            // The Device is left to rest: NetworkManager remembers why it
            // failed, which is how the Ethernet tab explains it.
            Ok(Err(failure))
        }
    }
}

/// Watches a Device until the join it started is up or dead.
async fn wait_for_link(
    states: &mut StateChangedStream,
    owner: &mut zbus::fdo::NameOwnerChangedStream,
    name: &str,
    enterprise: bool,
) -> Result<(), Failure> {
    use futures_util::StreamExt as _;
    let deadline = tokio::time::sleep(JOIN_TIMEOUT);
    tokio::pin!(deadline);
    let mut started = false;
    loop {
        let signal = tokio::select! {
            s = states.next() => s,
            _ = owner.next() => return Err(text::ethernet_interrupted(name)),
            () = &mut deadline => return Err(text::ethernet_timed_out(name)),
        };
        let Some(signal) = signal else {
            return Err(text::ethernet_refused(name, "NetworkManager went away"));
        };
        let Ok(args) = signal.args() else { continue };
        let (new, reason) = (args.new_state, args.reason);
        match new {
            STATE_ACTIVATED => return Ok(()),
            STATE_FAILED => return Err(text::ethernet_failure(name, reason, enterprise)),
            n if n >= STATE_PREPARE => started = true,
            // Back to rest after the join began: stopped by someone, not
            // the old Link making way for this one.
            n if n <= STATE_DISCONNECTED && started && reason != REASON_NEW_ACTIVATION => {
                return Err(text::ethernet_failure(name, reason, enterprise));
            }
            _ => {}
        }
    }
}

pub async fn connect(
    conn: &Connection,
    port: Option<&str>,
) -> zbus::Result<Result<EthernetJoined, Failure>> {
    join(conn, port, Plan::Saved).await
}

pub async fn join_eap(
    conn: &Connection,
    port: Option<&str>,
    eap: &Eap,
) -> zbus::Result<Result<EthernetJoined, Failure>> {
    join(conn, port, Plan::SignIn(eap)).await
}

/// Drops a port's Link (or its attempt), keeping the Profile.
pub async fn disconnect(
    conn: &Connection,
    port: Option<&str>,
) -> zbus::Result<Result<EthernetDropped, Failure>> {
    let all = ports(conn).await?;
    let port = match pick(&all, port) {
        Ok(p) => p,
        Err(f) => return Ok(Err(f)),
    };
    let device = no_cache!(PortProxy, conn, port.path.as_str());
    if device.state().await? <= STATE_DISCONNECTED {
        return Ok(Err(text::ethernet_not_connected(&port.name)));
    }
    device.disconnect().await?;
    Ok(Ok(EthernetDropped {
        port: port.name.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_ipc::EthernetProblem;

    #[test]
    fn no_carrier_is_unplugged_whatever_else_is_said() {
        for state in [20, 30, 120] {
            assert_eq!(
                classify("eth0", false, state, 40, false),
                (EthernetState::Unplugged, None)
            );
        }
    }

    #[test]
    fn a_cable_that_is_in_but_got_no_address_is_misconfigured_not_unplugged() {
        let (state, trouble) = classify("eth0", true, 120, 5, false);
        assert_eq!(state, EthernetState::Misconfigured);
        assert_eq!(trouble.unwrap().kind, EthernetProblem::NoAddress);
        // The same after NetworkManager rested at "disconnected".
        let (state, trouble) = classify("eth0", true, 30, 17, false);
        assert_eq!(state, EthernetState::Misconfigured);
        assert_eq!(trouble.unwrap().kind, EthernetProblem::NoAddress);
    }

    #[test]
    fn a_dropped_link_is_disconnected_not_a_problem() {
        // 39: you disconnected it.
        assert_eq!(
            classify("eth0", true, 30, 39, false),
            (EthernetState::Disconnected, None)
        );
    }

    #[test]
    fn sign_in_trouble_reads_differently_with_and_without_a_sign_in() {
        let wants = classify("eth0", true, 120, 7, false).1.unwrap();
        let refused = classify("eth0", true, 120, 7, true).1.unwrap();
        assert_eq!(wants.kind, EthernetProblem::SignIn);
        assert_eq!(refused.kind, EthernetProblem::SignIn);
        assert_ne!(wants.message, refused.message);
        assert!(refused.message.contains("refused"));
    }

    #[test]
    fn states_in_between_are_connecting() {
        for state in [40, 50, 60, 70, 80, 90] {
            assert_eq!(
                classify("eth0", true, state, 0, false).0,
                EthernetState::Connecting
            );
        }
        assert_eq!(
            classify("eth0", true, 100, 0, false).0,
            EthernetState::Connected
        );
    }

    #[test]
    fn link_local_ipv6_is_not_worth_showing() {
        assert!(link_local("fe80::1"));
        assert!(!link_local("2001:db8::2"));
        assert!(!link_local("192.168.1.2"));
    }
}
