//! A scriptable fake `org.freedesktop.NetworkManager`.
//!
//! It serves only the slice of NetworkManager the Daemon reads. Members are
//! checked against NetworkManager's D-Bus API reference: `State` (u),
//! `Connectivity` (u), `PrimaryConnection` (o), `Version` (s), the
//! `StateChanged(u)` signal, and `Connection.Active` with `Id` (s), `Type`
//! (s), `State` (u). A tiny fake means a proxy member it lacks fails loudly
//! with `UnknownProperty`, which keeps the Backend narrow.
//!
//! Wi-Fi (ticket #6): `GetDevices`, `Device` (`Interface`, `DeviceType`),
//! `Device.Wireless` (`GetAllAccessPoints`, `RequestScan`, `AccessPoints`,
//! `LastScan`, `AccessPointAdded`/`Removed`), `AccessPoint` (`Ssid`, `Strength`,
//! `Frequency`, `Flags`, `WpaFlags`, `RsnFlags`, `HwAddress`),
//! `Settings.ListConnections` and `Settings.Connection.GetSettings`.
//! `RequestScan` is refused inside NetworkManager's rate-limit window, like
//! the real one.
//!
//! Joining (ticket #7): `Manager.AddAndActivateConnection2` and
//! `ActivateConnection`, `Device.State`/`Disconnect`/`StateChanged`,
//! `Wireless.ActiveAccessPoint`, `Settings.Connection.Save`/`Delete`. A join
//! walks the Device through NetworkManager's real state sequence on its own
//! task; [`FakeNm::set_psk`] and [`FakeNm::script_join_failure`] decide how it
//! ends. A profile added with `persist: "memory"` stays out of the saved set
//! until `Save`.
//!
//! To add a scenario for a new ticket: add the member to the interface below,
//! keep its value in [`Model`], and expose a `set_*` method on [`FakeNm`] that
//! mutates the model and then calls [`FakeNm::emit`]-style change signals.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::runtime::Runtime;
use zbus::OwnedObjectPath;
use zbus::fdo::ObjectManager;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, connection, interface};
use zbus::{OwnedVariant, Variant};

mod bt;

const MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
const ACTIVE_PATH: &str = "/org/freedesktop/NetworkManager/ActiveConnection/1";
const NAME: &str = "org.freedesktop.NetworkManager";
const AGENT_MANAGER_PATH: &str = "/org/freedesktop/NetworkManager/AgentManager";
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const IP4_PATH: &str = "/org/freedesktop/NetworkManager/IP4Config/1";
const IP6_PATH: &str = "/org/freedesktop/NetworkManager/IP6Config/1";
/// NetworkManager refuses `RequestScan` this soon after the last scan.
pub const SCAN_RATE_LIMIT_MS: i64 = 10_000;

fn opath(p: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(p.to_owned()).expect("valid object path")
}

fn device_path(n: usize) -> String {
    format!("{MANAGER_PATH}/Devices/{n}")
}

fn ap_path(id: u32) -> String {
    format!("{MANAGER_PATH}/AccessPoint/{id}")
}

fn profile_path(n: usize) -> String {
    format!("{SETTINGS_PATH}/{n}")
}

/// Milliseconds of `CLOCK_BOOTTIME`, the clock NetworkManager's `LastScan` uses.
pub fn boottime_ms() -> i64 {
    let t = rustix::time::clock_gettime(rustix::time::ClockId::Boottime);
    t.tv_sec * 1000 + t.tv_nsec / 1_000_000
}

/// An access point to place in range of a Wi-Fi Device.
#[derive(Debug, Clone)]
pub struct Ap {
    pub ssid: Vec<u8>,
    pub bssid: String,
    pub strength: u8,
    pub frequency: u32,
    pub flags: u32,
    pub wpa_flags: u32,
    pub rsn_flags: u32,
}

impl Ap {
    /// A 2.4 GHz WPA2-PSK access point.
    pub fn wpa2(ssid: &str, bssid: &str, strength: u8) -> Self {
        Self {
            ssid: ssid.as_bytes().to_vec(),
            bssid: bssid.into(),
            strength,
            frequency: 2437,
            flags: 0x1,
            wpa_flags: 0,
            rsn_flags: 0x108, // KEY_MGMT_PSK (0x100) | PAIR_CCMP (0x8)
        }
    }

    pub fn frequency(mut self, mhz: u32) -> Self {
        self.frequency = mhz;
        self
    }

    pub fn ssid_bytes(mut self, ssid: &[u8]) -> Self {
        self.ssid = ssid.to_vec();
        self
    }

    /// Replaces the security flags: `(Flags, WpaFlags, RsnFlags)`.
    pub fn security(mut self, flags: u32, wpa: u32, rsn: u32) -> Self {
        (self.flags, self.wpa_flags, self.rsn_flags) = (flags, wpa, rsn);
        self
    }

    /// An open access point.
    pub fn open(ssid: &str, bssid: &str, strength: u8) -> Self {
        Self::wpa2(ssid, bssid, strength).security(0, 0, 0)
    }
}

/// Handle on an access point added with [`FakeNm::add_ap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApId(u32);

/// Handle on a Wi-Fi Device added with [`FakeNm::add_wifi_device`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceId(usize);

#[derive(Debug)]
struct DeviceModel {
    interface: String,
    /// `CLOCK_BOOTTIME` ms of the last finished scan; -1 never.
    last_scan: i64,
    aps: Vec<u32>,
    /// `Device.Statistics` byte counters.
    rx_bytes: u64,
    tx_bytes: u64,
    /// `NMDeviceState`.
    state: u32,
    active_ap: Option<u32>,
    /// Set for a paired phone (a Bluetooth Device).
    bt: Option<bt::BtModel>,
}

#[derive(Debug)]
struct ProfileModel {
    uuid: String,
    id: String,
    ssid: Vec<u8>,
    autoconnect: bool,
    /// False while the profile only lives in NetworkManager's memory.
    persisted: bool,
    /// Its secrets came from the secret agent, so deleting it tells the agent.
    agent_secrets: bool,
    /// Set for a Bluetooth Profile: the phone's address as six bytes.
    bdaddr: Option<Vec<u8>>,
}

/// `NMState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum NmState {
    Unknown = 0,
    Asleep = 10,
    Disconnected = 20,
    Disconnecting = 30,
    Connecting = 40,
    ConnectedLocal = 50,
    ConnectedSite = 60,
    ConnectedGlobal = 70,
}

/// `NMConnectivityState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum NmConnectivity {
    Unknown = 0,
    None = 1,
    Portal = 2,
    Limited = 3,
    Full = 4,
}

/// The primary active connection, as NetworkManager names it.
#[derive(Debug, Clone)]
pub struct ActiveConnection {
    pub id: String,
    /// NM connection type, e.g. `802-11-wireless`, `802-3-ethernet`.
    pub kind: String,
}

impl ActiveConnection {
    pub fn wifi(id: &str) -> Self {
        Self {
            id: id.into(),
            kind: "802-11-wireless".into(),
        }
    }

    pub fn vpn(id: &str) -> Self {
        Self {
            id: id.into(),
            kind: "vpn".into(),
        }
    }

    pub fn ethernet(id: &str) -> Self {
        Self {
            id: id.into(),
            kind: "802-3-ethernet".into(),
        }
    }
}

/// Everything the fake answers with. Survives [`FakeNm::stop`] and `restart`.
#[derive(Debug)]
struct Model {
    state: NmState,
    connectivity: NmConnectivity,
    primary: Option<ActiveConnection>,
    version: String,
    devices: Vec<DeviceModel>,
    aps: HashMap<u32, Ap>,
    next_ap: u32,
    /// Slots keep their object path for good; `None` is a deleted profile.
    profiles: Vec<Option<ProfileModel>>,
    /// The password a Network wants; a join with another one fails.
    psks: HashMap<Vec<u8>, String>,
    /// Joins that die on purpose: `(NMDeviceState died in, reason)`.
    join_failures: HashMap<Vec<u8>, (u32, u32)>,
    /// Tethering Links that die on purpose, by phone address.
    bt_failures: HashMap<String, (u32, u32)>,
    /// `RequestScan` calls NetworkManager accepted.
    scan_requests: u32,
    /// Refuse every `RequestScan`, whatever `LastScan` says.
    refuse_scans: bool,
    /// Reads of `Statistics.RxBytes`/`TxBytes`.
    statistics_reads: u32,
    /// What the Daemon wrote to `Statistics.RefreshRateMs`.
    refresh_rate_ms: u32,
    /// The registered secret agent: `(bus name, identifier)`.
    agent: Option<(String, String)>,
    /// Agent methods NetworkManager called and the agent answered without error.
    agent_calls: Vec<String>,
    /// Ask the agent again with `REQUEST_NEW` after a wrong password (what
    /// NetworkManager does); `false` makes the join fail with reason 8 instead.
    request_new: bool,
    /// The `GetSecrets` request waiting on the agent: `(connection path, setting)`.
    pending_secrets: Option<(String, String)>,
    /// The software Wi-Fi switch (`WirelessEnabled` as set by clients).
    wifi_soft: bool,
    /// A hardware switch holds the Wi-Fi radio off.
    wifi_hard_blocked: bool,
    wwan_soft: bool,
    /// Activations in flight; a stopped NetworkManager takes them with it.
    tasks: Vec<tokio::task::AbortHandle>,
    /// The state before [`FakeNm::sleep`], restored by [`FakeNm::resume`].
    before_sleep: NmState,
    /// `ConnectivityCheckEnabled`.
    check_enabled: bool,
    /// The primary connection's IPv4 gateway; `None` means no `Ip4Config`.
    gateway4: Option<String>,
    /// Whether the primary connection has an IPv6 address.
    ipv6: bool,
}

type Shared = Arc<Mutex<Model>>;

struct Manager(Shared);

#[interface(name = "org.freedesktop.NetworkManager")]
impl Manager {
    #[zbus(property)]
    fn state(&self) -> u32 {
        self.0.lock().unwrap().state as u32
    }

    #[zbus(property)]
    fn connectivity(&self) -> u32 {
        self.0.lock().unwrap().connectivity as u32
    }

    #[zbus(property)]
    fn primary_connection(&self) -> OwnedObjectPath {
        let path = match self.0.lock().unwrap().primary {
            Some(_) => ACTIVE_PATH,
            None => "/",
        };
        OwnedObjectPath::try_from(path).expect("static path")
    }

    #[zbus(property)]
    fn version(&self) -> String {
        self.0.lock().unwrap().version.clone()
    }

    /// Like NetworkManager: on only if the software switch is on and no
    /// hardware switch holds the radio.
    #[zbus(property)]
    fn wireless_enabled(&self) -> bool {
        let m = self.0.lock().unwrap();
        m.wifi_soft && !m.wifi_hard_blocked
    }

    #[zbus(property)]
    fn set_wireless_enabled(&mut self, on: bool) {
        self.0.lock().unwrap().wifi_soft = on;
    }

    #[zbus(property)]
    fn wireless_hardware_enabled(&self) -> bool {
        !self.0.lock().unwrap().wifi_hard_blocked
    }

    #[zbus(property)]
    fn wwan_enabled(&self) -> bool {
        self.0.lock().unwrap().wwan_soft
    }

    #[zbus(property)]
    fn set_wwan_enabled(&mut self, on: bool) {
        self.0.lock().unwrap().wwan_soft = on;
    }

    #[zbus(property)]
    fn connectivity_check_enabled(&self) -> bool {
        self.0.lock().unwrap().check_enabled
    }

    fn get_devices(&self) -> Vec<OwnedObjectPath> {
        let m = self.0.lock().unwrap();
        (0..m.devices.len())
            .map(|n| opath(&device_path(n)))
            .collect()
    }

    async fn add_and_activate_connection2(
        &self,
        connection: HashMap<String, HashMap<String, OwnedVariant>>,
        device: OwnedObjectPath,
        _specific_object: OwnedObjectPath,
        options: HashMap<String, OwnedVariant>,
        #[zbus(connection)] conn: &Connection,
        #[zbus(object_server)] os: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(
        OwnedObjectPath,
        OwnedObjectPath,
        HashMap<String, OwnedVariant>,
    )> {
        if connection.contains_key("bluetooth") {
            let persist_disk = options
                .get("persist")
                .and_then(|v| v.try_clone().ok()?.try_into().ok())
                .is_some_and(|p: String| p == "disk");
            let dev = device_index(device.as_str())?;
            let path = bt::add_and_activate(conn, &self.0, connection, dev, persist_disk).await?;
            return Ok((opath(&path), opath(ACTIVE_PATH), HashMap::new()));
        }
        let text = |section: &str, key: &str| -> Option<String> {
            connection
                .get(section)?
                .get(key)?
                .try_clone()
                .ok()?
                .try_into()
                .ok()
        };
        let ssid: Vec<u8> = connection
            .get("802-11-wireless")
            .and_then(|w| w.get("ssid"))
            .and_then(|v| v.try_clone().ok()?.try_into().ok())
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("no ssid".into()))?;
        let persist: Option<String> = options
            .get("persist")
            .and_then(|v| v.try_clone().ok()?.try_into().ok());
        // A secured profile without its secret: NetworkManager asks the agent.
        let needs_agent = connection.contains_key("802-11-wireless-security")
            && text("802-11-wireless-security", "psk").is_none();
        let n = {
            let mut m = self.0.lock().unwrap();
            m.profiles.push(Some(ProfileModel {
                uuid: text("connection", "uuid").unwrap_or_default(),
                id: text("connection", "id").unwrap_or_default(),
                ssid: ssid.clone(),
                autoconnect: true,
                persisted: persist.as_deref() == Some("disk"),
                agent_secrets: needs_agent,
                bdaddr: None,
            }));
            m.profiles.len() - 1
        };
        os.at(profile_path(n), Profile(self.0.clone(), n)).await?;
        let dev = device_index(device.as_str())?;
        let given = text("802-11-wireless-security", "psk").unwrap_or_default();
        let ask = needs_agent.then(|| Ask {
            settings: clone_settings(&connection),
            path: profile_path(n),
        });
        let task = tokio::spawn(activate(
            conn.clone(),
            self.0.clone(),
            dev,
            ssid,
            Some(given),
            ask,
        ));
        self.0.lock().unwrap().tasks.push(task.abort_handle());
        Ok((opath(&profile_path(n)), opath(ACTIVE_PATH), HashMap::new()))
    }

    async fn activate_connection(
        &self,
        connection: OwnedObjectPath,
        device: OwnedObjectPath,
        _specific_object: OwnedObjectPath,
        #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let n: usize = connection
            .as_str()
            .rsplit('/')
            .next()
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("bad connection path".into()))?;
        let (ssid, is_bt) = self
            .0
            .lock()
            .unwrap()
            .profiles
            .get(n)
            .and_then(|p| p.as_ref())
            .map(|p| (p.ssid.clone(), p.bdaddr.is_some()))
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("no such connection".into()))?;
        let dev = device_index(device.as_str())?;
        if is_bt {
            bt::start(conn, &self.0, dev);
            return Ok(opath(ACTIVE_PATH));
        }
        // A saved profile brings its own (right) secrets.
        let task = tokio::spawn(activate(
            conn.clone(),
            self.0.clone(),
            dev,
            ssid,
            None,
            None,
        ));
        self.0.lock().unwrap().tasks.push(task.abort_handle());
        Ok(opath(ACTIVE_PATH))
    }

    #[zbus(signal)]
    async fn device_added(emitter: &SignalEmitter<'_>, device: OwnedObjectPath)
    -> zbus::Result<()>;

    #[zbus(signal, name = "StateChanged")]
    async fn nm_state_changed(emitter: &SignalEmitter<'_>, state: u32) -> zbus::Result<()>;
}

struct Active(Shared);

#[interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl Active {
    #[zbus(property)]
    fn id(&self) -> String {
        self.primary().map(|p| p.id).unwrap_or_default()
    }

    #[zbus(property, name = "Type")]
    fn kind(&self) -> String {
        self.primary().map(|p| p.kind).unwrap_or_default()
    }

    #[zbus(property)]
    fn state(&self) -> u32 {
        2 // NM_ACTIVE_CONNECTION_STATE_ACTIVATED
    }

    #[zbus(property, name = "Ip4Config")]
    fn ip4_config(&self) -> OwnedObjectPath {
        let some = self.0.lock().unwrap().gateway4.is_some();
        opath(if some { IP4_PATH } else { "/" })
    }

    #[zbus(property, name = "Ip6Config")]
    fn ip6_config(&self) -> OwnedObjectPath {
        let some = self.0.lock().unwrap().ipv6;
        opath(if some { IP6_PATH } else { "/" })
    }

    /// The first Device carries the primary connection.
    #[zbus(property)]
    fn devices(&self) -> Vec<OwnedObjectPath> {
        if self.0.lock().unwrap().devices.is_empty() {
            Vec::new()
        } else {
            vec![opath(&device_path(0))]
        }
    }
}

struct Ip4Config(Shared);

#[interface(name = "org.freedesktop.NetworkManager.IP4Config")]
impl Ip4Config {
    #[zbus(property)]
    fn gateway(&self) -> String {
        self.0.lock().unwrap().gateway4.clone().unwrap_or_default()
    }
}

struct Ip6Config;

#[interface(name = "org.freedesktop.NetworkManager.IP6Config")]
impl Ip6Config {
    #[zbus(property)]
    fn address_data(&self) -> Vec<HashMap<String, OwnedVariant>> {
        let v = |x: Variant<'_>| OwnedVariant::try_from(x).expect("plain variant");
        vec![HashMap::from([
            ("address".to_owned(), v(Variant::from("2001:db8::2"))),
            ("prefix".to_owned(), v(Variant::from(64u32))),
        ])]
    }
}

impl Active {
    fn primary(&self) -> Option<ActiveConnection> {
        self.0.lock().unwrap().primary.clone()
    }
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.NetworkManager.Device")]
enum DeviceError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NotAllowed(String),
    NotActive(String),
}

struct Device(Shared, usize);

#[interface(name = "org.freedesktop.NetworkManager.Device")]
impl Device {
    #[zbus(property)]
    fn interface(&self) -> String {
        self.0.lock().unwrap().devices[self.1].interface.clone()
    }

    #[zbus(property)]
    fn device_type(&self) -> u32 {
        // NM_DEVICE_TYPE_BT or NM_DEVICE_TYPE_WIFI
        if self.0.lock().unwrap().devices[self.1].bt.is_some() {
            5
        } else {
            2
        }
    }

    #[zbus(property)]
    fn state(&self) -> u32 {
        self.0.lock().unwrap().devices[self.1].state
    }

    async fn disconnect(&self, #[zbus(connection)] conn: &Connection) -> Result<(), DeviceError> {
        let state = self.0.lock().unwrap().devices[self.1].state;
        if state <= 30 {
            return Err(DeviceError::NotActive("Device is not active".into()));
        }
        set_device_state(conn, &self.0, self.1, 30, 39).await; // USER_REQUESTED
        Ok(())
    }

    #[zbus(signal, name = "StateChanged")]
    async fn device_state_changed(
        emitter: &SignalEmitter<'_>,
        new_state: u32,
        old_state: u32,
        reason: u32,
    ) -> zbus::Result<()>;
}

fn device_index(path: &str) -> zbus::fdo::Result<usize> {
    path.rsplit('/')
        .next()
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| zbus::fdo::Error::InvalidArgs("bad device path".into()))
}

/// Moves a Device to `new`, like NetworkManager: model first, then the
/// `StateChanged` signal and the property change. Reaching 30 or 120 drops the
/// joined access point.
async fn set_device_state(conn: &Connection, model: &Shared, dev: usize, new: u32, reason: u32) {
    let (old, dropped_ap) = {
        let mut m = model.lock().unwrap();
        let old = m.devices[dev].state;
        m.devices[dev].state = new;
        let dropped = (new == 30 || new == 120) && m.devices[dev].active_ap.take().is_some();
        (old, dropped)
    };
    let os = conn.object_server();
    if dropped_ap && let Ok(i) = os.interface::<_, Wireless>(device_path(dev)).await {
        let _ = i
            .get()
            .await
            .active_access_point_changed(i.signal_emitter())
            .await;
    }
    if let Ok(i) = os.interface::<_, Device>(device_path(dev)).await {
        let _ = i.get().await.state_changed(i.signal_emitter()).await;
        let _ = Device::device_state_changed(i.signal_emitter(), new, old, reason).await;
    }
}

type SettingsDict = HashMap<String, HashMap<String, OwnedVariant>>;

fn clone_settings(s: &SettingsDict) -> SettingsDict {
    s.iter()
        .map(|(k, v)| {
            let inner = v
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.try_clone().ok()?)))
                .collect();
            (k.clone(), inner)
        })
        .collect()
}

/// A profile that has to get its secret from the agent.
struct Ask {
    settings: SettingsDict,
    path: String,
}

const AGENT_PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";
const AGENT_IFACE: &str = "org.freedesktop.NetworkManager.SecretAgent";
const SECURITY: &str = "802-11-wireless-security";
/// `NM_SECRET_AGENT_GET_SECRETS_FLAGS_ALLOW_INTERACTION`
const ALLOW_INTERACTION: u32 = 0x1;
/// `NM_SECRET_AGENT_GET_SECRETS_FLAGS_REQUEST_NEW`
const REQUEST_NEW: u32 = 0x2;

/// Calls a method on the registered agent. `Err(None)`: no agent is
/// registered; `Err(Some(_))`: the agent answered with an error.
async fn call_agent<B>(
    conn: &Connection,
    model: &Shared,
    method: &str,
    body: &B,
) -> Result<zbus::Message, Option<zbus::Error>>
where
    B: zbus::zvariant::DynamicType + serde::Serialize,
{
    let name = model
        .lock()
        .unwrap()
        .agent
        .as_ref()
        .map(|(name, _)| name.clone())
        .ok_or(None)?;
    let reply = conn
        .call_method(
            Some(name.as_str()),
            AGENT_PATH,
            Some(AGENT_IFACE),
            method,
            body,
        )
        .await
        .map_err(Some)?;
    model.lock().unwrap().agent_calls.push(method.to_owned());
    Ok(reply)
}

/// Asks the agent for the profile's password until it is right, the way
/// NetworkManager does: `REQUEST_NEW` marks every repeat. `Ok` carries the
/// settings to save; `Err` is the `NMDeviceStateReason` the activation dies with.
async fn negotiate(
    conn: &Connection,
    model: &Shared,
    ask: &Ask,
    wanted: Option<&str>,
) -> Result<SettingsDict, u32> {
    let mut flags = ALLOW_INTERACTION;
    loop {
        model.lock().unwrap().pending_secrets = Some((ask.path.clone(), SECURITY.to_owned()));
        let body = (
            clone_settings(&ask.settings),
            opath(&ask.path),
            SECURITY,
            vec![format!("{SECURITY}.psk")],
            flags,
        );
        let reply = call_agent(conn, model, "GetSecrets", &body).await;
        model.lock().unwrap().pending_secrets = None;
        let Ok(reply) = reply else {
            return Err(7); // NO_SECRETS: no agent, or it had nothing to give
        };
        let Ok(secrets) = reply.body().deserialize::<SettingsDict>() else {
            return Err(7);
        };
        let psk: Option<String> = secrets
            .get(SECURITY)
            .and_then(|s| s.get("psk"))
            .and_then(|v| v.try_clone().ok()?.try_into().ok());
        if wanted.is_none() || wanted == psk.as_deref() {
            return Ok(secrets);
        }
        if !model.lock().unwrap().request_new {
            return Err(8); // SUPPLICANT_DISCONNECT: the handshake failed
        }
        flags = ALLOW_INTERACTION | REQUEST_NEW;
    }
}

/// The life of one activation. `given` is the password sent with a new
/// profile; `None` means a saved profile with its stored (correct) secret.
/// `ask` is set when the secret has to come from the agent instead.
async fn activate(
    conn: Connection,
    model: Shared,
    dev: usize,
    ssid: Vec<u8>,
    given: Option<String>,
    ask: Option<Ask>,
) {
    let step = std::time::Duration::from_millis(15);
    let (script, wanted, ap) = {
        let m = model.lock().unwrap();
        let ap = m.devices[dev]
            .aps
            .iter()
            .copied()
            .find(|id| m.aps[id].ssid == ssid);
        (
            m.join_failures.get(&ssid).copied(),
            m.psks.get(&ssid).cloned(),
            ap,
        )
    };
    // (state to die in, reason), or a clean run.
    let failure = script.or_else(|| match (&wanted, &given) {
        _ if ask.is_some() => None, // the agent decides, at NEED_AUTH
        (Some(want), Some(got)) if want != got => Some((60, 7)), // NEED_AUTH, NO_SECRETS
        _ => None,
    });
    let mut agent_secrets = None;
    for s in [40, 50, 60, 70] {
        tokio::time::sleep(step).await;
        let asks = failure.is_none() && ask.is_some();
        if s == 60 && failure.map(|f| f.0) != Some(60) && !asks {
            continue; // only a failed password visits NEED_AUTH
        }
        if asks && s == 60 {
            set_device_state(&conn, &model, dev, 60, 0).await;
            let ask = ask.as_ref().expect("checked");
            match negotiate(&conn, &model, ask, wanted.as_deref()).await {
                Ok(secrets) => agent_secrets = Some(secrets),
                Err(reason) => {
                    tokio::time::sleep(step).await;
                    set_device_state(&conn, &model, dev, 120, reason).await;
                    return;
                }
            }
            continue;
        }
        set_device_state(&conn, &model, dev, s, 0).await;
        if let Some((died_in, reason)) = failure
            && died_in == s
        {
            tokio::time::sleep(step).await;
            set_device_state(&conn, &model, dev, 120, reason).await;
            return;
        }
    }
    tokio::time::sleep(step).await;
    model.lock().unwrap().devices[dev].active_ap = ap;
    if let Ok(i) = conn
        .object_server()
        .interface::<_, Wireless>(device_path(dev))
        .await
    {
        let _ = i
            .get()
            .await
            .active_access_point_changed(i.signal_emitter())
            .await;
    }
    set_device_state(&conn, &model, dev, 100, 0).await;
    if let (Some(secrets), Some(ask)) = (agent_secrets, ask) {
        // NetworkManager hands the agent what it learned, to store if it wants to.
        let _ = call_agent(&conn, &model, "SaveSecrets", &(secrets, opath(&ask.path))).await;
    }
}

/// `org.freedesktop.NetworkManager.AgentManager`: remembers who registered.
struct AgentManager(Shared);

#[interface(name = "org.freedesktop.NetworkManager.AgentManager")]
impl AgentManager {
    fn register(
        &self,
        identifier: String,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.register_with_capabilities(identifier, 0, hdr)
    }

    fn register_with_capabilities(
        &self,
        identifier: String,
        _capabilities: u32,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        if identifier.is_empty() || identifier.len() > 255 {
            return Err(zbus::fdo::Error::InvalidArgs("bad identifier".into()));
        }
        let sender = hdr
            .sender()
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("no sender".into()))?
            .to_string();
        self.0.lock().unwrap().agent = Some((sender, identifier));
        Ok(())
    }

    fn unregister(&self) {
        self.0.lock().unwrap().agent = None;
    }
}

struct Statistics(Shared, usize);

#[interface(name = "org.freedesktop.NetworkManager.Device.Statistics")]
impl Statistics {
    #[zbus(property)]
    fn refresh_rate_ms(&self) -> u32 {
        self.0.lock().unwrap().refresh_rate_ms
    }

    #[zbus(property)]
    fn set_refresh_rate_ms(&self, ms: u32) {
        self.0.lock().unwrap().refresh_rate_ms = ms;
    }

    #[zbus(property)]
    fn rx_bytes(&self) -> u64 {
        let mut m = self.0.lock().unwrap();
        m.statistics_reads += 1;
        m.devices[self.1].rx_bytes
    }

    #[zbus(property)]
    fn tx_bytes(&self) -> u64 {
        let mut m = self.0.lock().unwrap();
        m.statistics_reads += 1;
        m.devices[self.1].tx_bytes
    }
}

struct Wireless(Shared, usize);

#[interface(name = "org.freedesktop.NetworkManager.Device.Wireless")]
impl Wireless {
    #[zbus(property)]
    fn access_points(&self) -> Vec<OwnedObjectPath> {
        self.all()
    }

    #[zbus(property)]
    fn last_scan(&self) -> i64 {
        self.0.lock().unwrap().devices[self.1].last_scan
    }

    #[zbus(property)]
    fn active_access_point(&self) -> OwnedObjectPath {
        let m = self.0.lock().unwrap();
        match m.devices[self.1].active_ap {
            Some(id) => opath(&ap_path(id)),
            None => opath("/"),
        }
    }

    fn get_all_access_points(&self) -> Vec<OwnedObjectPath> {
        self.all()
    }

    async fn request_scan(
        &self,
        _options: HashMap<String, OwnedVariant>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), DeviceError> {
        {
            let mut m = self.0.lock().unwrap();
            let last = m.devices[self.1].last_scan;
            if m.refuse_scans || (last >= 0 && boottime_ms() - last < SCAN_RATE_LIMIT_MS) {
                return Err(DeviceError::NotAllowed(
                    "Scanning not allowed immediately following previous scan".into(),
                ));
            }
            m.scan_requests += 1;
            m.devices[self.1].last_scan = boottime_ms();
        }
        self.last_scan_changed(&emitter).await?;
        Ok(())
    }

    #[zbus(signal)]
    async fn access_point_added(
        emitter: &SignalEmitter<'_>,
        access_point: OwnedObjectPath,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn access_point_removed(
        emitter: &SignalEmitter<'_>,
        access_point: OwnedObjectPath,
    ) -> zbus::Result<()>;
}

impl Wireless {
    fn all(&self) -> Vec<OwnedObjectPath> {
        let m = self.0.lock().unwrap();
        m.devices[self.1]
            .aps
            .iter()
            .map(|id| opath(&ap_path(*id)))
            .collect()
    }
}

struct AccessPoint(Shared, u32);

impl AccessPoint {
    fn ap(&self) -> Ap {
        self.0.lock().unwrap().aps[&self.1].clone()
    }
}

#[interface(name = "org.freedesktop.NetworkManager.AccessPoint")]
impl AccessPoint {
    #[zbus(property)]
    fn ssid(&self) -> Vec<u8> {
        self.ap().ssid
    }

    #[zbus(property)]
    fn strength(&self) -> u8 {
        self.ap().strength
    }

    #[zbus(property)]
    fn frequency(&self) -> u32 {
        self.ap().frequency
    }

    #[zbus(property)]
    fn flags(&self) -> u32 {
        self.ap().flags
    }

    #[zbus(property)]
    fn wpa_flags(&self) -> u32 {
        self.ap().wpa_flags
    }

    #[zbus(property)]
    fn rsn_flags(&self) -> u32 {
        self.ap().rsn_flags
    }

    #[zbus(property)]
    fn hw_address(&self) -> String {
        self.ap().bssid
    }
}

struct Settings(Shared);

#[interface(name = "org.freedesktop.NetworkManager.Settings")]
impl Settings {
    fn list_connections(&self) -> Vec<OwnedObjectPath> {
        let m = self.0.lock().unwrap();
        (0..m.profiles.len())
            .filter(|n| m.profiles[*n].is_some())
            .map(|n| opath(&profile_path(n)))
            .collect()
    }
}

struct Profile(Shared, usize);

#[interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl Profile {
    fn get_settings(&self) -> HashMap<String, HashMap<String, OwnedVariant>> {
        let m = self.0.lock().unwrap();
        let p = m.profiles[self.1].as_ref().expect("served profile exists");
        let v = |v: Variant<'_>| OwnedVariant::try_from(v).expect("plain value");
        let mut connection = HashMap::new();
        connection.insert("id".to_owned(), v(Variant::from(p.id.clone())));
        connection.insert("uuid".to_owned(), v(Variant::from(p.uuid.clone())));
        connection.insert("type".to_owned(), v(Variant::from("802-11-wireless")));
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
        HashMap::from([
            ("connection".to_owned(), connection),
            ("802-11-wireless".to_owned(), wireless),
        ])
    }

    /// Like NetworkManager, replaces the settings; the fake keeps only the
    /// `connection.autoconnect` flag, the one thing the Backend changes.
    async fn update(
        &self,
        settings: HashMap<String, HashMap<String, OwnedVariant>>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let auto: Option<bool> = settings
            .get("connection")
            .and_then(|c| c.get("autoconnect"))
            .and_then(|v| v.try_clone().ok()?.try_into().ok());
        if let Some(p) = self.0.lock().unwrap().profiles[self.1].as_mut() {
            p.autoconnect = auto.unwrap_or(true);
            p.persisted = true;
        }
        Self::updated(&emitter).await?;
        Ok(())
    }

    #[zbus(signal)]
    async fn updated(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

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
        // Like NetworkManager, deleting a Profile in use drops its Link.
        let (in_use, told_agent): (Vec<usize>, bool) = {
            let mut m = self.0.lock().unwrap();
            let gone = m.profiles[self.1].take();
            let told_agent = gone.as_ref().is_some_and(|p| p.agent_secrets);
            let bdaddr = gone.as_ref().and_then(|p| p.bdaddr.clone());
            let ssid = gone.map(|p| p.ssid).unwrap_or_default();
            let in_use = (0..m.devices.len())
                .filter(|d| match (&bdaddr, &m.devices[*d].bt) {
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
        if told_agent {
            // The agent may hold secrets of its own for this profile.
            let body = (SettingsDict::new(), opath(&profile_path(self.1)));
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

/// Handle on the fake. Methods are synchronous so tests read like prose; the
/// fake runs on its own runtime and never nests inside the test's.
#[derive(Debug)]
pub struct FakeNm {
    rt: Runtime,
    address: String,
    model: Shared,
    conn: Option<Connection>,
    logind: Option<Connection>,
}

const LOGIND_NAME: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";

struct Logind;

#[interface(name = "org.freedesktop.login1.Manager")]
impl Logind {
    #[zbus(signal)]
    async fn prepare_for_sleep(emitter: &SignalEmitter<'_>, start: bool) -> zbus::Result<()>;
}

impl FakeNm {
    /// Serves the fake on the bus at `address`, state `Disconnected`.
    pub fn start(address: &str) -> FakeNm {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let model = Arc::new(Mutex::new(Model {
            state: NmState::Disconnected,
            connectivity: NmConnectivity::None,
            primary: None,
            version: "1.58.1".into(),
            devices: Vec::new(),
            aps: HashMap::new(),
            next_ap: 1,
            profiles: Vec::new(),
            psks: HashMap::new(),
            join_failures: HashMap::new(),
            bt_failures: HashMap::new(),
            scan_requests: 0,
            refuse_scans: false,
            statistics_reads: 0,
            refresh_rate_ms: 0,
            agent: None,
            agent_calls: Vec::new(),
            request_new: true,
            pending_secrets: None,
            wifi_soft: true,
            wifi_hard_blocked: false,
            wwan_soft: true,
            tasks: Vec::new(),
            before_sleep: NmState::Disconnected,
            check_enabled: true,
            gateway4: None,
            ipv6: false,
        }));
        let mut nm = FakeNm {
            rt,
            address: address.to_owned(),
            model,
            conn: None,
            logind: None,
        };
        nm.restart();
        nm.start_logind();
        nm
    }

    /// The fake `org.freedesktop.login1`, which only announces sleeping and
    /// waking. It stays up when NetworkManager goes away.
    fn start_logind(&mut self) {
        let address = self.address.clone();
        let conn = self.rt.block_on(async move {
            connection::Builder::address(address.as_str())
                .serve_at(LOGIND_PATH, Logind)
                .name(LOGIND_NAME)
                .build()
                .await
                .expect("serve fake logind")
        });
        self.logind = Some(conn);
    }

    fn prepare_for_sleep(&self, start: bool) {
        let Some(conn) = &self.logind else { return };
        let conn = conn.clone();
        self.rt.block_on(async move {
            if let Ok(iface) = conn
                .object_server()
                .interface::<_, Logind>(LOGIND_PATH)
                .await
            {
                let _ = Logind::prepare_for_sleep(iface.signal_emitter(), start).await;
            }
        });
    }

    /// The machine suspends: logind announces it, NetworkManager goes ASLEEP.
    pub fn sleep(&self) {
        let state = self.model.lock().unwrap().state;
        if state != NmState::Asleep {
            self.model.lock().unwrap().before_sleep = state;
        }
        self.prepare_for_sleep(true);
        self.set_state(NmState::Asleep);
    }

    /// The machine wakes: logind announces it first, then NetworkManager
    /// returns to the state it slept in.
    pub fn resume(&self) {
        self.prepare_for_sleep(false);
        let back = self.model.lock().unwrap().before_sleep;
        self.set_state(back);
    }

    /// Whether Wi-Fi is on, as NetworkManager's `WirelessEnabled` reads.
    pub fn wifi_enabled(&self) -> bool {
        let m = self.model.lock().unwrap();
        m.wifi_soft && !m.wifi_hard_blocked
    }

    pub fn wwan_enabled(&self) -> bool {
        self.model.lock().unwrap().wwan_soft
    }

    /// Flips a hardware kill switch on the Wi-Fi radio.
    pub fn set_wifi_hardware_blocked(&self, blocked: bool) {
        self.model.lock().unwrap().wifi_hard_blocked = blocked;
        self.notify(|m, e| Box::pin(async move { m.wireless_enabled_changed(e).await }));
        self.notify(|m, e| Box::pin(async move { m.wireless_hardware_enabled_changed(e).await }));
    }

    /// Whether the fake currently owns the NetworkManager name.
    pub fn is_running(&self) -> bool {
        self.conn.is_some()
    }

    /// NetworkManager going away: the name is released.
    pub fn stop(&mut self) {
        self.forget_activations();
        self.conn = None;
    }

    /// A NetworkManager that goes away forgets its joins in flight: their
    /// tasks end and their Devices fall back to rest.
    fn forget_activations(&self) {
        let mut m = self.model.lock().unwrap();
        for task in m.tasks.drain(..) {
            task.abort();
        }
        m.pending_secrets = None;
        // Profiles only in memory die with NetworkManager.
        for p in &mut m.profiles {
            if p.as_ref().is_some_and(|p| !p.persisted) {
                *p = None;
            }
        }
        for d in &mut m.devices {
            if (40..100).contains(&d.state) {
                d.state = 30;
                d.active_ap = None;
            }
        }
    }

    /// NetworkManager (re)appearing with its current model. Objects are
    /// served before the name is requested, so a client that watches the name
    /// never sees a half-built tree.
    pub fn restart(&mut self) {
        self.forget_activations();
        self.conn = None;
        // Agents have to register again with a new NetworkManager.
        self.model.lock().unwrap().agent = None;
        let (address, model) = (self.address.clone(), self.model.clone());
        let conn = self.rt.block_on(async move {
            let (devices, aps, profiles) = {
                let m = model.lock().unwrap();
                (
                    m.devices.len(),
                    m.aps.keys().copied().collect::<Vec<_>>(),
                    m.profiles
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| p.is_some())
                        .map(|(n, _)| n)
                        .collect::<Vec<_>>(),
                )
            };
            let mut b = connection::Builder::address(address.as_str())
                .serve_at("/org/freedesktop", ObjectManager)
                .serve_at(MANAGER_PATH, Manager(model.clone()))
                .serve_at(ACTIVE_PATH, Active(model.clone()))
                .serve_at(SETTINGS_PATH, Settings(model.clone()))
                .serve_at(AGENT_MANAGER_PATH, AgentManager(model.clone()))
                .serve_at(IP4_PATH, Ip4Config(model.clone()))
                .serve_at(IP6_PATH, Ip6Config);
            for n in 0..devices {
                b = b.serve_at(device_path(n), Device(model.clone(), n));
                if model.lock().unwrap().devices[n].bt.is_some() {
                    b = b.serve_at(device_path(n), bt::Bluetooth(model.clone(), n));
                } else {
                    b = b
                        .serve_at(device_path(n), Statistics(model.clone(), n))
                        .serve_at(device_path(n), Wireless(model.clone(), n));
                }
            }
            for id in aps {
                b = b.serve_at(ap_path(id), AccessPoint(model.clone(), id));
            }
            for n in profiles {
                b = b.serve_at(profile_path(n), Profile(model.clone(), n));
            }
            b.name(NAME)
                .build()
                .await
                .expect("serve fake NetworkManager")
        });
        self.conn = Some(conn);
    }

    pub fn set_state(&self, state: NmState) {
        self.model.lock().unwrap().state = state;
        self.notify(|m, e| Box::pin(async move { m.state_changed(e).await }));
        let Some(conn) = &self.conn else { return };
        let conn = conn.clone();
        self.rt.block_on(async move {
            if let Ok(iface) = conn
                .object_server()
                .interface::<_, Manager>(MANAGER_PATH)
                .await
            {
                let _ = Manager::nm_state_changed(iface.signal_emitter(), state as u32).await;
            }
        });
    }

    pub fn set_connectivity(&self, connectivity: NmConnectivity) {
        self.model.lock().unwrap().connectivity = connectivity;
        self.notify(|m, e| Box::pin(async move { m.connectivity_changed(e).await }));
    }

    /// Sets (or clears) the primary active connection.
    pub fn set_primary(&self, primary: Option<ActiveConnection>) {
        self.model.lock().unwrap().primary = primary;
        self.notify(|m, e| Box::pin(async move { m.primary_connection_changed(e).await }));
    }

    /// Radio switches as NetworkManager reports them: `soft` is the software
    /// switch (`WirelessEnabled`), `hard` the hardware one (`WirelessHardwareEnabled`).
    pub fn set_radio(&self, soft: bool, hard: bool) {
        let mut m = self.model.lock().unwrap();
        m.wifi_soft = soft;
        m.wifi_hard_blocked = !hard;
    }

    /// `ConnectivityCheckEnabled`.
    pub fn set_connectivity_check(&self, enabled: bool) {
        self.model.lock().unwrap().check_enabled = enabled;
    }

    /// The primary connection's IPv4 gateway (`None`: it has no `Ip4Config`).
    pub fn set_gateway(&self, gateway: Option<&str>) {
        self.model.lock().unwrap().gateway4 = gateway.map(str::to_owned);
    }

    /// Whether the primary connection has an IPv6 address.
    pub fn set_ipv6(&self, yes: bool) {
        self.model.lock().unwrap().ipv6 = yes;
    }

    /// Adds a Wi-Fi Device that has never scanned.
    pub fn add_wifi_device(&self, interface: &str) -> DeviceId {
        let n = {
            let mut m = self.model.lock().unwrap();
            m.devices.push(DeviceModel {
                interface: interface.into(),
                last_scan: -1,
                aps: Vec::new(),
                rx_bytes: 0,
                tx_bytes: 0,
                state: 30, // NM_DEVICE_STATE_DISCONNECTED
                active_ap: None,
                bt: None,
            });
            m.devices.len() - 1
        };
        let model = self.model.clone();
        self.with_conn(move |c| async move {
            let os = c.object_server();
            let _ = os.at(device_path(n), Device(model.clone(), n)).await;
            let _ = os.at(device_path(n), Statistics(model.clone(), n)).await;
            let _ = os.at(device_path(n), Wireless(model, n)).await;
            if let Ok(i) = os.interface::<_, Manager>(MANAGER_PATH).await {
                let _ = Manager::device_added(i.signal_emitter(), opath(&device_path(n))).await;
            }
        });
        DeviceId(n)
    }

    /// Puts an access point in range of `device`.
    pub fn add_ap(&self, device: DeviceId, ap: Ap) -> ApId {
        let id = {
            let mut m = self.model.lock().unwrap();
            let id = m.next_ap;
            m.next_ap += 1;
            m.aps.insert(id, ap);
            m.devices[device.0].aps.push(id);
            id
        };
        let model = self.model.clone();
        self.with_conn(move |c| async move {
            let _ = c
                .object_server()
                .at(ap_path(id), AccessPoint(model, id))
                .await;
            if let Ok(w) = c
                .object_server()
                .interface::<_, Wireless>(device_path(device.0))
                .await
            {
                let _ = Wireless::access_point_added(w.signal_emitter(), opath(&ap_path(id))).await;
            }
        });
        ApId(id)
    }

    /// Takes an access point out of range.
    pub fn remove_ap(&self, device: DeviceId, ap: ApId) {
        self.model.lock().unwrap().devices[device.0]
            .aps
            .retain(|i| *i != ap.0);
        self.with_conn(move |c| async move {
            if let Ok(w) = c
                .object_server()
                .interface::<_, Wireless>(device_path(device.0))
                .await
            {
                let _ =
                    Wireless::access_point_removed(w.signal_emitter(), opath(&ap_path(ap.0))).await;
            }
            let _ = c
                .object_server()
                .remove::<AccessPoint, _>(ap_path(ap.0))
                .await;
        });
    }

    /// Changes an access point's signal and emits `PropertiesChanged`.
    pub fn set_strength(&self, ap: ApId, strength: u8) {
        self.model
            .lock()
            .unwrap()
            .aps
            .get_mut(&ap.0)
            .expect("known ap")
            .strength = strength;
        self.with_conn(move |c| async move {
            if let Ok(i) = c
                .object_server()
                .interface::<_, AccessPoint>(ap_path(ap.0))
                .await
            {
                let _ = i.get().await.strength_changed(i.signal_emitter()).await;
            }
        });
    }

    /// [`set_strength`](Self::set_strength) for the first access point
    /// broadcasting `ssid`.
    pub fn set_strength_of(&self, ssid: &str, strength: u8) {
        let ap = {
            let m = self.model.lock().unwrap();
            let id = m
                .aps
                .iter()
                .find(|(_, ap)| ap.ssid == ssid.as_bytes())
                .map(|(id, _)| *id)
                .unwrap_or_else(|| panic!("no access point for {ssid}"));
            ApId(id)
        };
        self.set_strength(ap, strength);
    }

    /// Pretends `device` last scanned `ms_ago` milliseconds ago.
    pub fn set_last_scan_ago(&self, device: DeviceId, ms_ago: i64) {
        self.model.lock().unwrap().devices[device.0].last_scan = boottime_ms() - ms_ago;
    }

    /// Sets a Device's byte counters (`Statistics.RxBytes`/`TxBytes`). The
    /// first Device added carries the primary connection.
    pub fn set_traffic(&self, device: DeviceId, rx_bytes: u64, tx_bytes: u64) {
        let mut m = self.model.lock().unwrap();
        m.devices[device.0].rx_bytes = rx_bytes;
        m.devices[device.0].tx_bytes = tx_bytes;
    }

    /// How many times a counter was read, to prove nobody polls needlessly.
    pub fn statistics_reads(&self) -> u32 {
        self.model.lock().unwrap().statistics_reads
    }

    /// What was last written to `Statistics.RefreshRateMs` (0 = never).
    pub fn refresh_rate_ms(&self) -> u32 {
        self.model.lock().unwrap().refresh_rate_ms
    }

    /// Makes NetworkManager refuse every `RequestScan`.
    pub fn refuse_scans(&self, yes: bool) {
        self.model.lock().unwrap().refuse_scans = yes;
    }

    /// How many `RequestScan` calls NetworkManager accepted.
    pub fn scan_requests(&self) -> u32 {
        self.model.lock().unwrap().scan_requests
    }

    /// Adds a saved Wi-Fi Profile.
    pub fn add_wifi_profile(&self, uuid: &str, id: &str, ssid: &[u8], autoconnect: bool) {
        let n = {
            let mut m = self.model.lock().unwrap();
            m.profiles.push(Some(ProfileModel {
                uuid: uuid.into(),
                id: id.into(),
                ssid: ssid.to_vec(),
                autoconnect,
                persisted: true,
                agent_secrets: false,
                bdaddr: None,
            }));
            m.profiles.len() - 1
        };
        let model = self.model.clone();
        self.with_conn(move |c| async move {
            let _ = c
                .object_server()
                .at(profile_path(n), Profile(model, n))
                .await;
        });
    }

    /// Makes joining `ssid` need `psk`; any other password fails like a wrong one.
    pub fn set_psk(&self, ssid: &str, psk: &str) {
        self.model
            .lock()
            .unwrap()
            .psks
            .insert(ssid.as_bytes().to_vec(), psk.into());
    }

    /// Makes every join of `ssid` die: the Device walks up to `died_in`
    /// (an `NMDeviceState`: 40, 50, 60 or 70) and then fails with `reason`
    /// (an `NMDeviceStateReason`).
    pub fn script_join_failure(&self, ssid: &str, died_in: u32, reason: u32) {
        self.model
            .lock()
            .unwrap()
            .join_failures
            .insert(ssid.as_bytes().to_vec(), (died_in, reason));
    }

    /// Whether a secret agent is registered with the fake AgentManager.
    pub fn agent_registered(&self) -> bool {
        self.model.lock().unwrap().agent.is_some()
    }

    /// The agent methods NetworkManager called that the agent answered
    /// without an error, in order (`GetSecrets`, `SaveSecrets`, ...).
    pub fn agent_calls(&self) -> Vec<String> {
        self.model.lock().unwrap().agent_calls.clone()
    }

    /// After a wrong password, fail the join with reason 8 instead of asking
    /// the agent again with `REQUEST_NEW` (some NetworkManager versions do).
    pub fn set_request_new(&self, yes: bool) {
        self.model.lock().unwrap().request_new = yes;
    }

    /// Whether a `GetSecrets` call is waiting on the agent right now.
    pub fn secrets_pending(&self) -> bool {
        self.model.lock().unwrap().pending_secrets.is_some()
    }

    /// NetworkManager giving up on the `GetSecrets` request that is waiting:
    /// sends the agent `CancelGetSecrets`.
    pub fn cancel_pending_secrets(&self) {
        let Some((path, setting)) = self.model.lock().unwrap().pending_secrets.clone() else {
            return;
        };
        let model = self.model.clone();
        self.with_conn(|conn| async move {
            let _ = call_agent(&conn, &model, "CancelGetSecrets", &(opath(&path), setting)).await;
        });
    }

    /// Profiles NetworkManager would keep across a restart.
    pub fn persisted_profiles(&self) -> usize {
        let m = self.model.lock().unwrap();
        m.profiles.iter().flatten().filter(|p| p.persisted).count()
    }

    /// Whether the Profile with this UUID joins by itself; `None` if absent.
    pub fn profile_autoconnect(&self, uuid: &str) -> Option<bool> {
        let m = self.model.lock().unwrap();
        m.profiles
            .iter()
            .flatten()
            .find(|p| p.uuid == uuid)
            .map(|p| p.autoconnect)
    }

    /// Profiles that exist at all, in memory or on disk.
    pub fn profile_count(&self) -> usize {
        self.model.lock().unwrap().profiles.iter().flatten().count()
    }

    /// The SSID `device` is associated with, if any.
    pub fn joined_ssid(&self, device: DeviceId) -> Option<String> {
        let m = self.model.lock().unwrap();
        let id = m.devices[device.0].active_ap?;
        Some(String::from_utf8_lossy(&m.aps[&id].ssid).into_owned())
    }

    fn with_conn<F, Fut>(&self, f: F)
    where
        F: FnOnce(Connection) -> Fut,
        Fut: Future<Output = ()>,
    {
        if let Some(conn) = &self.conn {
            self.rt.block_on(f(conn.clone()));
        }
    }

    /// Emits a `PropertiesChanged` for one Manager property.
    fn notify<F>(&self, f: F)
    where
        F: for<'a> FnOnce(
            &'a Manager,
            &'a SignalEmitter<'a>,
        )
            -> std::pin::Pin<Box<dyn Future<Output = zbus::Result<()>> + Send + 'a>>,
    {
        let Some(conn) = &self.conn else { return };
        let conn = conn.clone();
        self.rt.block_on(async move {
            if let Ok(iface) = conn
                .object_server()
                .interface::<_, Manager>(MANAGER_PATH)
                .await
            {
                let guard = iface.get().await;
                let _ = f(&guard, iface.signal_emitter()).await;
            }
        });
    }
}
