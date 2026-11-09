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
//! To add a scenario for a new ticket: add the member to the interface below,
//! keep its value in [`Model`], and expose a `set_*` method on [`FakeNm`] that
//! mutates the model and then calls [`FakeNm::emit`]-style change signals.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::runtime::Runtime;
use zbus::OwnedObjectPath;
use zbus::{OwnedVariant, Variant};
use zbus::fdo::ObjectManager;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, connection, interface};

const MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
const ACTIVE_PATH: &str = "/org/freedesktop/NetworkManager/ActiveConnection/1";
const NAME: &str = "org.freedesktop.NetworkManager";
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
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
    t.tv_sec * 1000 + i64::from(t.tv_nsec) / 1_000_000
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
}

#[derive(Debug)]
struct ProfileModel {
    uuid: String,
    id: String,
    ssid: Vec<u8>,
    autoconnect: bool,
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
    profiles: Vec<ProfileModel>,
    /// `RequestScan` calls NetworkManager accepted.
    scan_requests: u32,
    /// Refuse every `RequestScan`, whatever `LastScan` says.
    refuse_scans: bool,
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

    fn get_devices(&self) -> Vec<OwnedObjectPath> {
        let m = self.0.lock().unwrap();
        (0..m.devices.len()).map(|n| opath(&device_path(n))).collect()
    }

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
        2 // NM_DEVICE_TYPE_WIFI
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
        m.devices[self.1].aps.iter().map(|id| opath(&ap_path(*id))).collect()
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
        (0..m.profiles.len()).map(|n| opath(&profile_path(n))).collect()
    }
}

struct Profile(Shared, usize);

#[interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl Profile {
    fn get_settings(&self) -> HashMap<String, HashMap<String, OwnedVariant>> {
        let m = self.0.lock().unwrap();
        let p = &m.profiles[self.1];
        let v = |v: Variant<'_>| OwnedVariant::try_from(v).expect("plain value");
        let mut connection = HashMap::new();
        connection.insert("id".to_owned(), v(Variant::from(p.id.clone())));
        connection.insert("uuid".to_owned(), v(Variant::from(p.uuid.clone())));
        connection.insert("type".to_owned(), v(Variant::from("802-11-wireless")));
        connection.insert("autoconnect".to_owned(), v(Variant::from(p.autoconnect)));
        let mut wireless = HashMap::new();
        wireless.insert("ssid".to_owned(), v(Variant::from(p.ssid.clone())));
        HashMap::from([
            ("connection".to_owned(), connection),
            ("802-11-wireless".to_owned(), wireless),
        ])
    }
}

/// Handle on the fake. Methods are synchronous so tests read like prose; the
/// fake runs on its own runtime and never nests inside the test's.
#[derive(Debug)]
pub struct FakeNm {
    rt: Runtime,
    address: String,
    model: Shared,
    conn: Option<Connection>,
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
            scan_requests: 0,
            refuse_scans: false,
        }));
        let mut nm = FakeNm {
            rt,
            address: address.to_owned(),
            model,
            conn: None,
        };
        nm.restart();
        nm
    }

    /// Whether the fake currently owns the NetworkManager name.
    pub fn is_running(&self) -> bool {
        self.conn.is_some()
    }

    /// NetworkManager going away: the name is released.
    pub fn stop(&mut self) {
        self.conn = None;
    }

    /// NetworkManager (re)appearing with its current model. Objects are
    /// served before the name is requested, so a client that watches the name
    /// never sees a half-built tree.
    pub fn restart(&mut self) {
        self.conn = None;
        let (address, model) = (self.address.clone(), self.model.clone());
        let conn = self.rt.block_on(async move {
            let (devices, aps, profiles) = {
                let m = model.lock().unwrap();
                (
                    m.devices.len(),
                    m.aps.keys().copied().collect::<Vec<_>>(),
                    m.profiles.len(),
                )
            };
            let mut b = connection::Builder::address(address.as_str())
                .serve_at("/org/freedesktop", ObjectManager)
                .serve_at(MANAGER_PATH, Manager(model.clone()))
                .serve_at(ACTIVE_PATH, Active(model.clone()))
                .serve_at(SETTINGS_PATH, Settings(model.clone()));
            for n in 0..devices {
                b = b
                    .serve_at(device_path(n), Device(model.clone(), n))
                    .serve_at(device_path(n), Wireless(model.clone(), n));
            }
            for id in aps {
                b = b.serve_at(ap_path(id), AccessPoint(model.clone(), id));
            }
            for n in 0..profiles {
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

    /// Adds a Wi-Fi Device that has never scanned.
    pub fn add_wifi_device(&self, interface: &str) -> DeviceId {
        let n = {
            let mut m = self.model.lock().unwrap();
            m.devices.push(DeviceModel {
                interface: interface.into(),
                last_scan: -1,
                aps: Vec::new(),
            });
            m.devices.len() - 1
        };
        let model = self.model.clone();
        self.with_conn(move |c| async move {
            let os = c.object_server();
            let _ = os.at(device_path(n), Device(model.clone(), n)).await;
            let _ = os.at(device_path(n), Wireless(model, n)).await;
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
            let _ = c.object_server().at(ap_path(id), AccessPoint(model, id)).await;
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
        self.model.lock().unwrap().devices[device.0].aps.retain(|i| *i != ap.0);
        self.with_conn(move |c| async move {
            if let Ok(w) = c
                .object_server()
                .interface::<_, Wireless>(device_path(device.0))
                .await
            {
                let _ = Wireless::access_point_removed(w.signal_emitter(), opath(&ap_path(ap.0)))
                    .await;
            }
            let _ = c.object_server().remove::<AccessPoint, _>(ap_path(ap.0)).await;
        });
    }

    /// Changes an access point's signal and emits `PropertiesChanged`.
    pub fn set_strength(&self, ap: ApId, strength: u8) {
        self.model.lock().unwrap().aps.get_mut(&ap.0).expect("known ap").strength = strength;
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

    /// Pretends `device` last scanned `ms_ago` milliseconds ago.
    pub fn set_last_scan_ago(&self, device: DeviceId, ms_ago: i64) {
        self.model.lock().unwrap().devices[device.0].last_scan = boottime_ms() - ms_ago;
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
            m.profiles.push(ProfileModel {
                uuid: uuid.into(),
                id: id.into(),
                ssid: ssid.to_vec(),
                autoconnect,
            });
            m.profiles.len() - 1
        };
        let model = self.model.clone();
        self.with_conn(move |c| async move {
            let _ = c.object_server().at(profile_path(n), Profile(model, n)).await;
        });
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
