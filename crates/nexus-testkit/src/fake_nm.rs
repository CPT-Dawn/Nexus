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
    /// `MaxBitrate` in kbit/s.
    pub bitrate: u32,
    /// Does not broadcast its name: `Ssid` reads empty until a Device joins it.
    pub hidden: bool,
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
            bitrate: 130_000,
            hidden: false,
        }
    }

    /// Hides the name: the access point is in range but cannot be listed.
    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Sets `MaxBitrate` in kbit/s.
    pub fn bitrate(mut self, kbps: u32) -> Self {
        self.bitrate = kbps;
        self
    }

    /// An OWE (Enhanced Open) access point.
    pub fn owe(ssid: &str, bssid: &str, strength: u8) -> Self {
        Self::wpa2(ssid, bssid, strength).security(0x1, 0, 0x808) // KEY_MGMT_OWE | PAIR_CCMP
    }

    /// A WPA2/WPA3 transition-mode access point (PSK and SAE together).
    pub fn transition(ssid: &str, bssid: &str, strength: u8) -> Self {
        Self::wpa2(ssid, bssid, strength).security(0x1, 0, 0x508) // PSK | SAE | PAIR_CCMP
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
    /// A WPA2-Enterprise (802.1X) access point, like eduroam.
    pub fn enterprise(ssid: &str, bssid: &str, strength: u8) -> Self {
        Self::wpa2(ssid, bssid, strength).security(0x1, 0, 0x208) // KEY_MGMT_802_1X | PAIR_CCMP
    }

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
    /// The stored Wi-Fi secret: `(key-mgmt, psk)`. `None` is an open Network.
    secret: Option<(String, String)>,
    /// Set for VPN Profiles.
    vpn: Option<VpnModel>,
    /// Set for a Bluetooth Profile: the phone's address as six bytes.
    bdaddr: Option<Vec<u8>>,
    /// Every setting a client wrote with `Update`/`Update2`, merged over the
    /// basics when read back.
    settings: SettingsDict,
    /// `VersionId`: bumped by every change.
    version: u64,
}

/// What makes a Profile a VPN.
#[derive(Debug)]
struct VpnModel {
    /// The connection type: `wireguard` or `vpn`.
    kind: String,
    /// The settings as added, secrets included.
    settings: SettingsDict,
    /// `NMActiveConnectionState`; 0 is not active.
    active: u32,
    /// Activation ends in failure (state 4) instead of success.
    fails: bool,
    /// The secret names (hints) NetworkManager asks the agent for before the
    /// VPN comes up, like a plugin that needs a password or a code.
    asks: Option<Vec<String>>,
    /// What the agent answered the last time (`vpn.secrets`).
    answers: Option<HashMap<String, String>>,
}

/// A checkpoint: the Profiles as they were, to put back on rollback.
#[derive(Debug)]
struct CheckpointModel {
    id: u32,
    profiles: Vec<(usize, SettingsDict, bool)>,
    timer: Option<tokio::task::AbortHandle>,
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
    /// The `802-1x` setting of the last Profile added for each SSID, as text.
    eap_settings: HashMap<Vec<u8>, HashMap<String, String>>,
    /// Every setting (`section.key`, secrets left out) of the last Profile
    /// added for each SSID, as text.
    added: HashMap<Vec<u8>, HashMap<String, String>>,
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
    /// Keep `ActiveAccessPoint` after the Link drops, as real NetworkManager
    /// does for a moment.
    ap_lag: bool,
    /// The `GetSecrets` request waiting on the agent: `(connection path, setting)`.
    pending_secrets: Option<(String, String)>,
    /// `GetSecrets` calls on a Profile that were let through.
    secret_reads: u32,
    /// Refuse `GetSecrets` on Profiles, like a polkit denial.
    deny_secret_reads: bool,
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
    /// Open checkpoints.
    checkpoints: Vec<CheckpointModel>,
    next_checkpoint: u32,
    /// Checkpoints that were rolled back, by timeout or by request.
    checkpoint_rollbacks: u32,
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

    /// The VPNs that are up or coming up. (The primary connection of the
    /// other fixtures is not listed: nothing here reads it.)
    #[zbus(property)]
    fn active_connections(&self) -> Vec<OwnedObjectPath> {
        let m = self.0.lock().unwrap();
        (0..m.profiles.len())
            .filter(|n| {
                m.profiles[*n]
                    .as_ref()
                    .and_then(|p| p.vpn.as_ref())
                    .is_some_and(|v| v.active > 0)
            })
            .map(|n| opath(&vpn_active_path(n)))
            .collect()
    }

    async fn deactivate_connection(
        &self,
        active: OwnedObjectPath,
        #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<()> {
        let n: usize = active
            .as_str()
            .rsplit('/')
            .next()
            .and_then(|s| s.strip_prefix("vpn"))
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("not an active connection".into()))?;
        end_vpn(conn, &self.0, n).await;
        Ok(())
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
        let shown: HashMap<String, String> = connection
            .iter()
            .flat_map(|(section, keys)| {
                keys.iter()
                    .filter(|(k, _)| !matches!(k.as_str(), "psk" | "password" | "wep-key0"))
                    .map(move |(k, v)| (format!("{section}.{k}"), show_variant(v)))
            })
            .collect();
        self.0.lock().unwrap().added.insert(ssid.clone(), shown);
        // A secured profile without its secret: NetworkManager asks the agent.
        let eap = connection.get("802-1x");
        let needs_agent = eap.is_none()
            && connection.contains_key("802-11-wireless-security")
            // OWE has no secret: nothing to ask for.
            && text("802-11-wireless-security", "key-mgmt").as_deref() != Some("owe")
            && text("802-11-wireless-security", "psk").is_none();
        if let Some(eap) = eap {
            let shown = eap
                .iter()
                .map(|(k, v)| (k.clone(), show_variant(v)))
                .collect();
            self.0
                .lock()
                .unwrap()
                .eap_settings
                .insert(ssid.clone(), shown);
        }
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
                settings: SettingsDict::new(),
                version: 1,
                secret: text("802-11-wireless-security", "key-mgmt")
                    .zip(text("802-11-wireless-security", "psk")),
                vpn: None,
            }));
            m.profiles.len() - 1
        };
        os.at(profile_path(n), Profile(self.0.clone(), n)).await?;
        let dev = device_index(device.as_str())?;
        let given = match eap {
            // Identity and password stand in for the psk of a Wi-Fi Network.
            Some(_) => format!(
                "{}\0{}",
                text("802-1x", "identity").unwrap_or_default(),
                text("802-1x", "password").unwrap_or_default()
            ),
            None => text("802-11-wireless-security", "psk").unwrap_or_default(),
        };
        let wrong_inline = !needs_agent
            && eap.is_none()
            && self
                .0
                .lock()
                .unwrap()
                .psks
                .get(&ssid)
                .is_some_and(|want| *want != given);
        let ask = (needs_agent || wrong_inline).then(|| Ask {
            settings: clone_settings(&connection),
            path: profile_path(n),
            wrong_inline,
        });
        // A hidden Network nobody answers the probe of: SSID_NOT_FOUND.
        let hidden = connection
            .get("802-11-wireless")
            .and_then(|w| w.get("hidden"))
            .and_then(|v| v.try_clone().ok()?.try_into().ok())
            .unwrap_or(false);
        let heard = {
            let m = self.0.lock().unwrap();
            m.devices[dev].aps.iter().any(|id| m.aps[id].ssid == ssid)
        };
        let forced = (hidden && !heard).then_some((40, 53));
        let task = tokio::spawn(activate(
            conn.clone(),
            self.0.clone(),
            dev,
            ssid,
            Some(given),
            ask,
            forced,
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
        let is_vpn = self
            .0
            .lock()
            .unwrap()
            .profiles
            .get(n)
            .and_then(|p| p.as_ref())
            .is_some_and(|p| p.vpn.is_some());
        if is_vpn {
            start_vpn(conn, &self.0, n).await?;
            return Ok(opath(&vpn_active_path(n)));
        }
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
        // A saved profile brings its own (right) secrets, unless they are the
        // agent's (a Profile kept in the keyring): then NetworkManager asks.
        let ask = {
            let m = self.0.lock().unwrap();
            m.profiles[n]
                .as_ref()
                .filter(|p| agent_owned(p))
                .map(|p| Ask {
                    settings: agent_view(p),
                    path: profile_path(n),
                    wrong_inline: false,
                })
        };
        let task = tokio::spawn(activate(
            conn.clone(),
            self.0.clone(),
            dev,
            ssid,
            None,
            ask,
            None,
        ));
        self.0.lock().unwrap().tasks.push(task.abort_handle());
        Ok(opath(ACTIVE_PATH))
    }

    /// Remembers every Profile; unless destroyed within `rollback_timeout`
    /// seconds (0 = never) the Profiles are put back, like NetworkManager.
    async fn checkpoint_create(
        &self,
        _devices: Vec<OwnedObjectPath>,
        rollback_timeout: u32,
        _flags: u32,
        #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let (id, profiles) = {
            let mut m = self.0.lock().unwrap();
            let id = m.next_checkpoint;
            m.next_checkpoint += 1;
            let profiles = m
                .profiles
                .iter()
                .enumerate()
                .filter_map(|(n, p)| {
                    let p = p.as_ref()?;
                    Some((n, clone_settings(&p.settings), p.autoconnect))
                })
                .collect();
            (id, profiles)
        };
        let timer = (rollback_timeout > 0).then(|| {
            let (conn, model) = (conn.clone(), self.0.clone());
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(u64::from(rollback_timeout)))
                    .await;
                roll_back(&conn, &model, id, false).await;
            })
            .abort_handle()
        });
        self.0.lock().unwrap().checkpoints.push(CheckpointModel {
            id,
            profiles,
            timer,
        });
        Ok(opath(&checkpoint_path(id)))
    }

    fn checkpoint_destroy(&self, checkpoint: OwnedObjectPath) -> zbus::fdo::Result<()> {
        let id = checkpoint_id(checkpoint.as_str())?;
        let mut m = self.0.lock().unwrap();
        let at = m
            .checkpoints
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("no such checkpoint".into()))?;
        if let Some(t) = m.checkpoints.remove(at).timer {
            t.abort();
        }
        Ok(())
    }

    async fn checkpoint_rollback(
        &self,
        checkpoint: OwnedObjectPath,
        #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<HashMap<String, u32>> {
        let id = checkpoint_id(checkpoint.as_str())?;
        if roll_back(conn, &self.0, id, true).await {
            Ok(HashMap::new())
        } else {
            Err(zbus::fdo::Error::InvalidArgs("no such checkpoint".into()))
        }
    }

    #[zbus(signal)]
    async fn device_added(emitter: &SignalEmitter<'_>, device: OwnedObjectPath)
    -> zbus::Result<()>;

    #[zbus(signal, name = "StateChanged")]
    async fn nm_state_changed(emitter: &SignalEmitter<'_>, state: u32) -> zbus::Result<()>;
}

fn checkpoint_path(id: u32) -> String {
    format!("{MANAGER_PATH}/Checkpoint/{id}")
}

fn checkpoint_id(path: &str) -> zbus::fdo::Result<u32> {
    path.rsplit('/')
        .next()
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| zbus::fdo::Error::InvalidArgs("bad checkpoint path".into()))
}

/// Puts the Profiles of checkpoint `id` back, bumping their versions, and
/// forgets it. False if there is no such checkpoint (it was destroyed or
/// rolled back already).
async fn roll_back(conn: &Connection, model: &Shared, id: u32, stop_timer: bool) -> bool {
    let restored: Vec<usize> = {
        let mut m = model.lock().unwrap();
        let Some(at) = m.checkpoints.iter().position(|c| c.id == id) else {
            return false;
        };
        let cp = m.checkpoints.remove(at);
        if stop_timer && let Some(t) = cp.timer {
            t.abort();
        }
        m.checkpoint_rollbacks += 1;
        let mut restored = Vec::new();
        for (n, dict, auto) in cp.profiles {
            if let Some(p) = m.profiles[n].as_mut() {
                p.autoconnect = auto;
                p.settings = dict;
                p.version += 1;
                restored.push(n);
            }
        }
        restored
    };
    for n in restored {
        if let Ok(i) = conn
            .object_server()
            .interface::<_, Profile>(profile_path(n))
            .await
        {
            let _ = Profile::updated(i.signal_emitter()).await;
        }
    }
    true
}

fn flag(dict: &SettingsDict, section: &str, key: &str) -> Option<bool> {
    dict.get(section)?
        .get(key)?
        .try_clone()
        .ok()?
        .try_into()
        .ok()
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

fn vpn_active_path(n: usize) -> String {
    format!("{MANAGER_PATH}/ActiveConnection/vpn{n}")
}

/// A VPN's `Connection.Active` object, served while it is up or coming up.
struct VpnActive(Shared, usize);

impl VpnActive {
    fn read<T>(&self, f: impl FnOnce(&ProfileModel, &VpnModel) -> T) -> Option<T> {
        let m = self.0.lock().unwrap();
        let p = m.profiles[self.1].as_ref()?;
        Some(f(p, p.vpn.as_ref()?))
    }
}

#[interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl VpnActive {
    #[zbus(property)]
    fn id(&self) -> String {
        self.read(|p, _| p.id.clone()).unwrap_or_default()
    }

    #[zbus(property, name = "Type")]
    fn kind(&self) -> String {
        self.read(|_, v| v.kind.clone()).unwrap_or_default()
    }

    #[zbus(property)]
    fn uuid(&self) -> String {
        self.read(|p, _| p.uuid.clone()).unwrap_or_default()
    }

    #[zbus(property)]
    fn state(&self) -> u32 {
        self.read(|_, v| v.active).unwrap_or(4)
    }
}

async fn manager_changed(conn: &Connection) {
    if let Ok(iface) = conn
        .object_server()
        .interface::<_, Manager>(MANAGER_PATH)
        .await
    {
        let guard = iface.get().await;
        let _ = guard
            .active_connections_changed(iface.signal_emitter())
            .await;
    }
}

/// Sets a VPN's `State` and tells whoever listens.
async fn vpn_state(conn: &Connection, model: &Shared, n: usize, state: u32) {
    if let Some(v) = model.lock().unwrap().profiles[n]
        .as_mut()
        .and_then(|p| p.vpn.as_mut())
    {
        v.active = state;
    }
    if let Ok(iface) = conn
        .object_server()
        .interface::<_, VpnActive>(vpn_active_path(n))
        .await
    {
        let guard = iface.get().await;
        let _ = guard.state_changed(iface.signal_emitter()).await;
    }
}

/// A VPN stops being active: observers see state 4, then the object goes.
async fn end_vpn(conn: &Connection, model: &Shared, n: usize) {
    let was_active = model.lock().unwrap().profiles[n]
        .as_ref()
        .and_then(|p| p.vpn.as_ref())
        .is_some_and(|v| v.active > 0);
    if !was_active {
        return;
    }
    vpn_state(conn, model, n, 4).await;
    if let Some(v) = model.lock().unwrap().profiles[n]
        .as_mut()
        .and_then(|p| p.vpn.as_mut())
    {
        v.active = 0;
    }
    let _ = conn
        .object_server()
        .remove::<VpnActive, _>(vpn_active_path(n))
        .await;
    manager_changed(conn).await;
}

/// `ActivateConnection` on a VPN Profile: coming up (1), then up (2) or
/// failed, on the fake's own task.
async fn start_vpn(conn: &Connection, model: &Shared, n: usize) -> zbus::fdo::Result<()> {
    {
        let mut m = model.lock().unwrap();
        let Some(v) = m.profiles[n].as_mut().and_then(|p| p.vpn.as_mut()) else {
            return Err(zbus::fdo::Error::InvalidArgs("no such connection".into()));
        };
        if v.active > 0 {
            return Ok(());
        }
        v.active = 1;
    }
    conn.object_server()
        .at(vpn_active_path(n), VpnActive(model.clone(), n))
        .await?;
    manager_changed(conn).await;
    let (c, m) = (conn.clone(), model.clone());
    let task = tokio::spawn(async move {
        tokio::task::yield_now().await;
        let (fails, asks, settings) = {
            let guard = m.lock().unwrap();
            let v = guard.profiles[n].as_ref().and_then(|p| p.vpn.as_ref());
            (
                v.is_some_and(|v| v.fails),
                v.and_then(|v| v.asks.clone()),
                v.map(|v| clone_settings(&v.settings)),
            )
        };
        let mut refused = false;
        if let (Some(hints), Some(settings)) = (asks, settings) {
            let body = (
                settings,
                opath(&profile_path(n)),
                "vpn",
                hints,
                ALLOW_INTERACTION,
            );
            match call_agent(&c, &m, "GetSecrets", &body).await {
                Ok(reply) => {
                    let secrets: Option<HashMap<String, String>> = reply
                        .body()
                        .deserialize::<SettingsDict>()
                        .ok()
                        .and_then(|mut all| {
                            all.remove("vpn")?
                                .remove("secrets")?
                                .try_clone()
                                .ok()?
                                .try_into()
                                .ok()
                        });
                    if let Some(v) = m.lock().unwrap().profiles[n]
                        .as_mut()
                        .and_then(|p| p.vpn.as_mut())
                    {
                        v.answers = secrets;
                    }
                }
                Err(_) => refused = true,
            }
        }
        if fails || refused {
            end_vpn(&c, &m, n).await;
        } else {
            vpn_state(&c, &m, n, 2).await;
        }
    });
    model.lock().unwrap().tasks.push(task.abort_handle());
    Ok(())
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
        let dropped =
            (new == 30 || new == 120) && !m.ap_lag && m.devices[dev].active_ap.take().is_some();
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

/// A settings value as readable text, for [`FakeNm::eap_setting`].
fn show_variant(v: &OwnedVariant) -> String {
    let Ok(v) = v.try_clone() else {
        return String::new();
    };
    if let Ok(s) = <String>::try_from(v.try_clone().expect("clone")) {
        return s;
    }
    if let Ok(b) = <bool>::try_from(v.try_clone().expect("clone")) {
        return b.to_string();
    }
    if let Ok(list) = <Vec<String>>::try_from(v.try_clone().expect("clone")) {
        return list.join(",");
    }
    if let Ok(bytes) = <Vec<u8>>::try_from(v.try_clone().expect("clone")) {
        return String::from_utf8_lossy(&bytes)
            .trim_end_matches('\0')
            .to_owned();
    }
    format!("{:?}", *v)
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

/// Whether the profile's Wi-Fi secret is the agent's (`psk-flags` has bit 1).
fn agent_owned(p: &ProfileModel) -> bool {
    p.settings
        .get(SECURITY)
        .and_then(|s| s.get("psk-flags"))
        .and_then(|v| u32::try_from(v.try_clone().ok()?).ok())
        .is_some_and(|f| f & 1 != 0)
}

/// What NetworkManager tells an agent about a profile: identity and the
/// secret flags, never the secret.
fn agent_view(p: &ProfileModel) -> SettingsDict {
    let v = |v: Variant<'_>| OwnedVariant::try_from(v).expect("plain value");
    let mut security = p
        .settings
        .get(SECURITY)
        .map(clone_section)
        .unwrap_or_default();
    if let Some((key_mgmt, _)) = &p.secret {
        security.insert("key-mgmt".to_owned(), v(Variant::from(key_mgmt.clone())));
    }
    SettingsDict::from([
        (
            "connection".to_owned(),
            HashMap::from([
                ("id".to_owned(), v(Variant::from(p.id.clone()))),
                ("uuid".to_owned(), v(Variant::from(p.uuid.clone()))),
                ("type".to_owned(), v(Variant::from("802-11-wireless"))),
            ]),
        ),
        (
            "802-11-wireless".to_owned(),
            HashMap::from([("ssid".to_owned(), v(Variant::from(p.ssid.clone())))]),
        ),
        (SECURITY.to_owned(), security),
    ])
}

fn clone_section(s: &HashMap<String, OwnedVariant>) -> HashMap<String, OwnedVariant> {
    s.iter()
        .filter_map(|(k, v)| Some((k.clone(), v.try_clone().ok()?)))
        .collect()
}

/// A profile that has to get its secret from the agent.
struct Ask {
    settings: SettingsDict,
    path: String,
    /// The profile came with a password that is wrong: like the real thing,
    /// NetworkManager asks the agent for a new one (`REQUEST_NEW`) at once.
    wrong_inline: bool,
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
    if ask.wrong_inline {
        flags |= REQUEST_NEW;
    }
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
    forced: Option<(u32, u32)>,
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
    let failure = script.or(forced).or_else(|| match (&wanted, &given) {
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
    {
        let mut m = model.lock().unwrap();
        m.devices[dev].active_ap = ap;
        // Associating teaches NetworkManager the name of a hidden network.
        if let Some(ap) = ap.and_then(|id| m.aps.get_mut(&id)) {
            ap.hidden = false;
        }
    }
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
        // NetworkManager hands the agent what it learned, to store if it wants
        // to: the whole connection, secrets included.
        let mut full = clone_settings(&ask.settings);
        for (setting, values) in secrets {
            full.entry(setting).or_default().extend(values);
        }
        let _ = call_agent(&conn, &model, "SaveSecrets", &(full, opath(&ask.path))).await;
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
        let ap = self.ap();
        if ap.hidden { Vec::new() } else { ap.ssid }
    }

    #[zbus(property)]
    fn max_bitrate(&self) -> u32 {
        self.ap().bitrate
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
fn rows_of(v: &OwnedVariant) -> Vec<HashMap<String, OwnedVariant>> {
    v.try_clone()
        .ok()
        .and_then(|v| v.try_into().ok())
        .unwrap_or_default()
}

fn rows_value(rows: Vec<HashMap<String, OwnedVariant>>) -> OwnedVariant {
    let rows: Vec<HashMap<String, zbus::Variant<'static>>> = rows
        .into_iter()
        .map(|r| r.into_iter().map(|(k, v)| (k, v.into())).collect())
        .collect();
    OwnedVariant::try_from(zbus::Variant::new(rows)).expect("plain rows")
}

/// What `GetSettings` shows of a VPN Profile: no private key, no pre-shared keys.
fn vpn_visible(settings: &SettingsDict) -> SettingsDict {
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
fn vpn_secrets(settings: &SettingsDict, setting: &str) -> SettingsDict {
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
#[zbus(prefix = "org.freedesktop.NetworkManager.Settings.Error")]
enum SettingsError {
    #[zbus(error)]
    ZBus(zbus::Error),
    VersionIdMismatch(String),
}

struct Profile(Shared, usize);

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
        let mut all: HashMap<String, HashMap<String, OwnedVariant>> = HashMap::from([
            ("connection".to_owned(), connection),
            ("802-11-wireless".to_owned(), wireless),
        ]);
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
    ) -> zbus::fdo::Result<HashMap<String, HashMap<String, OwnedVariant>>> {
        let mut m = self.0.lock().unwrap();
        if m.deny_secret_reads {
            return Err(zbus::fdo::Error::AccessDenied(
                "Insufficient privileges.".into(),
            ));
        }
        m.secret_reads += 1;
        let p = m.profiles[self.1].as_ref().expect("served profile exists");
        if let Some(vpn) = &p.vpn {
            return Ok(vpn_secrets(&vpn.settings, setting));
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
        end_vpn(conn, &self.0, self.1).await;
        // Like NetworkManager, deleting a Profile in use drops its Link.
        let (in_use, told_agent): (Vec<usize>, Option<SettingsDict>) = {
            let mut m = self.0.lock().unwrap();
            let gone = m.profiles[self.1].take();
            let told_agent = gone.as_ref().filter(|p| p.agent_secrets).map(agent_view);
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
            eap_settings: HashMap::new(),
            added: HashMap::new(),
            secret_reads: 0,
            deny_secret_reads: false,
            join_failures: HashMap::new(),
            bt_failures: HashMap::new(),
            scan_requests: 0,
            refuse_scans: false,
            statistics_reads: 0,
            refresh_rate_ms: 0,
            agent: None,
            agent_calls: Vec::new(),
            request_new: true,
            ap_lag: false,
            pending_secrets: None,
            wifi_soft: true,
            wifi_hard_blocked: false,
            wwan_soft: true,
            tasks: Vec::new(),
            before_sleep: NmState::Disconnected,
            check_enabled: true,
            gateway4: None,
            ipv6: false,
            checkpoints: Vec::new(),
            next_checkpoint: 1,
            checkpoint_rollbacks: 0,
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
        for p in m.profiles.iter_mut().flatten() {
            if let Some(v) = p.vpn.as_mut() {
                v.active = 0;
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
                settings: SettingsDict::new(),
                version: 1,
                secret: None,
                vpn: None,
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

    /// Makes joining the enterprise Network `ssid` need this identity and
    /// password; any other pair is refused like a wrong password.
    pub fn set_eap_user(&self, ssid: &str, identity: &str, password: &str) {
        self.model
            .lock()
            .unwrap()
            .psks
            .insert(ssid.as_bytes().to_vec(), format!("{identity}\0{password}"));
    }

    /// One setting, as `section.key` (e.g. `802-11-wireless-security.key-mgmt`),
    /// of the Profile the Daemon last added for `ssid`, as text. Secrets are
    /// not kept.
    pub fn added_setting(&self, ssid: &[u8], key: &str) -> Option<String> {
        let m = self.model.lock().unwrap();
        m.added.get(ssid)?.get(key).cloned()
    }

    /// One key of the `802-1x` setting the Daemon sent when it added the
    /// Profile for `ssid`, as text (lists joined by `,`, byte arrays as
    /// their UTF-8 text without the trailing NUL, so a certificate shows as `file://<path>`).
    pub fn eap_setting(&self, ssid: &str, key: &str) -> Option<String> {
        let m = self.model.lock().unwrap();
        m.eap_settings.get(ssid.as_bytes())?.get(key).cloned()
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

    /// While on, a Device that drops its Link keeps reporting the old
    /// `ActiveAccessPoint` (real NetworkManager lags a moment after Forget).
    pub fn set_active_ap_lag(&self, yes: bool) {
        self.model.lock().unwrap().ap_lag = yes;
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

    /// Gives the Profile with this UUID a stored secret: `key_mgmt` is
    /// NetworkManager's (`wpa-psk`, `sae`), `secret` the passphrase.
    pub fn set_profile_secret(&self, uuid: &str, key_mgmt: &str, secret: &str) {
        let mut m = self.model.lock().unwrap();
        if let Some(p) = m.profiles.iter_mut().flatten().find(|p| p.uuid == uuid) {
            p.secret = Some((key_mgmt.into(), secret.into()));
        }
    }

    /// Makes `GetSecrets` on Profiles fail the way a polkit denial does.
    pub fn deny_secret_reads(&self, yes: bool) {
        self.model.lock().unwrap().deny_secret_reads = yes;
    }

    /// How many `GetSecrets` calls on Profiles were let through.
    pub fn secret_reads(&self) -> u32 {
        self.model.lock().unwrap().secret_reads
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

    /// Adds a saved plugin VPN Profile (type `vpn`), e.g. an OpenVPN one.
    pub fn add_plugin_vpn(&self, uuid: &str, id: &str) {
        self.add_plugin_vpn_of(uuid, id, None);
    }

    /// Like [`FakeNm::add_plugin_vpn`], with the plugin it belongs to
    /// (`openvpn`: its `vpn.service-type` names the plugin).
    pub fn add_plugin_vpn_of(&self, uuid: &str, id: &str, plugin: Option<&str>) {
        let v = |x: &str| OwnedVariant::try_from(Variant::from(x)).expect("plain value");
        let connection = HashMap::from([
            ("id".to_owned(), v(id)),
            ("uuid".to_owned(), v(uuid)),
            ("type".to_owned(), v("vpn")),
        ]);
        let mut settings = HashMap::from([("connection".to_owned(), connection)]);
        if let Some(plugin) = plugin {
            settings.insert(
                "vpn".to_owned(),
                HashMap::from([(
                    "service-type".to_owned(),
                    v(&format!("org.freedesktop.NetworkManager.{plugin}")),
                )]),
            );
        }
        let n = {
            let mut m = self.model.lock().unwrap();
            m.profiles.push(Some(ProfileModel {
                uuid: uuid.into(),
                id: id.into(),
                ssid: Vec::new(),
                autoconnect: false,
                persisted: true,
                agent_secrets: false,
                secret: None,
                settings: Default::default(),
                version: 1,
                bdaddr: None,
                vpn: Some(VpnModel {
                    kind: "vpn".into(),
                    settings,
                    active: 0,
                    fails: false,
                    asks: None,
                    answers: None,
                }),
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

    fn vpn_slot(&self, uuid: &str) -> usize {
        let m = self.model.lock().unwrap();
        m.profiles
            .iter()
            .position(|p| {
                p.as_ref()
                    .is_some_and(|p| p.vpn.is_some() && p.uuid == uuid)
            })
            .unwrap_or_else(|| panic!("no VPN profile {uuid}"))
    }

    /// `NMActiveConnectionState` of a VPN Profile: 0 not active, 1 coming
    /// up, 2 up.
    pub fn vpn_active_state(&self, uuid: &str) -> u32 {
        let n = self.vpn_slot(uuid);
        let m = self.model.lock().unwrap();
        m.profiles[n].as_ref().unwrap().vpn.as_ref().unwrap().active
    }

    /// The tunnel dies on its own (the peer vanished, the link broke): no
    /// client asked for it.
    pub fn drop_vpn(&self, uuid: &str) {
        let n = self.vpn_slot(uuid);
        let model = self.model.clone();
        self.with_conn(move |c| async move { end_vpn(&c, &model, n).await });
    }

    /// Makes every activation of this VPN fail (state 4) instead of working.
    pub fn script_vpn_failure(&self, uuid: &str, fails: bool) {
        let n = self.vpn_slot(uuid);
        let mut m = self.model.lock().unwrap();
        m.profiles[n].as_mut().unwrap().vpn.as_mut().unwrap().fails = fails;
    }

    /// Makes activating this VPN ask the secret agent for these secrets
    /// first (`["password"]`, `["username", "password", "otp"]`, ...), the
    /// way a plugin that needs a login does. Without an answer it fails.
    pub fn script_vpn_asks(&self, uuid: &str, secrets: &[&str]) {
        let n = self.vpn_slot(uuid);
        let mut m = self.model.lock().unwrap();
        m.profiles[n].as_mut().unwrap().vpn.as_mut().unwrap().asks =
            Some(secrets.iter().map(|s| (*s).to_owned()).collect());
    }

    /// What the secret agent handed over for this VPN on its last
    /// activation (`vpn.secrets`), sorted by name.
    pub fn vpn_answers(&self, uuid: &str) -> Vec<(String, String)> {
        let n = self.vpn_slot(uuid);
        let m = self.model.lock().unwrap();
        let mut out: Vec<(String, String)> = m.profiles[n]
            .as_ref()
            .and_then(|p| p.vpn.as_ref())
            .and_then(|v| v.answers.clone())
            .unwrap_or_default()
            .into_iter()
            .collect();
        out.sort();
        out
    }

    /// The private key stored in a WireGuard Profile.
    pub fn vpn_private_key(&self, uuid: &str) -> Option<String> {
        let n = self.vpn_slot(uuid);
        let m = self.model.lock().unwrap();
        let v = m.profiles[n].as_ref()?.vpn.as_ref()?;
        v.settings
            .get("wireguard")?
            .get("private-key")?
            .try_clone()
            .ok()?
            .try_into()
            .ok()
    }

    /// The Profile's settings as NetworkManager stored them (secrets
    /// included): the value of `section`/`key` as text, for assertions.
    pub fn vpn_setting(&self, uuid: &str, section: &str, key: &str) -> Option<String> {
        let n = self.vpn_slot(uuid);
        let m = self.model.lock().unwrap();
        let v = m.profiles[n].as_ref()?.vpn.as_ref()?;
        let value = v.settings.get(section)?.get(key)?;
        Some(format!("{value:?}"))
    }

    /// The UUID of the first VPN Profile with this name.
    pub fn vpn_uuid(&self, name: &str) -> Option<String> {
        let m = self.model.lock().unwrap();
        m.profiles
            .iter()
            .flatten()
            .find(|p| p.vpn.is_some() && p.id == name)
            .map(|p| p.uuid.clone())
    }

    /// A text setting of the Profile with this UUID, as a client wrote it.
    pub fn profile_text(&self, uuid: &str, section: &str, key: &str) -> Option<String> {
        let m = self.model.lock().unwrap();
        let p = m.profiles.iter().flatten().find(|p| p.uuid == uuid)?;
        p.settings
            .get(section)?
            .get(key)?
            .try_clone()
            .ok()?
            .try_into()
            .ok()
    }

    /// A whole-number setting (`i32` or `u32`) of the Profile with this UUID.
    pub fn profile_int(&self, uuid: &str, section: &str, key: &str) -> Option<i64> {
        let m = self.model.lock().unwrap();
        let p = m.profiles.iter().flatten().find(|p| p.uuid == uuid)?;
        let v = p.settings.get(section)?.get(key)?.try_clone().ok()?;
        i32::try_from(v.try_clone().ok()?)
            .map(i64::from)
            .or_else(|_| u32::try_from(v).map(i64::from))
            .ok()
    }

    /// The Profile's `VersionId`.
    pub fn profile_version(&self, uuid: &str) -> Option<u64> {
        let m = self.model.lock().unwrap();
        m.profiles
            .iter()
            .flatten()
            .find(|p| p.uuid == uuid)
            .map(|p| p.version)
    }

    /// Somebody else (say `nmcli`) changed the Profile: its version moves on.
    pub fn edit_profile_externally(&self, uuid: &str) {
        let mut m = self.model.lock().unwrap();
        if let Some(p) = m.profiles.iter_mut().flatten().find(|p| p.uuid == uuid) {
            p.version += 1;
        }
    }

    /// Checkpoints that are open right now.
    pub fn checkpoints_open(&self) -> usize {
        self.model.lock().unwrap().checkpoints.len()
    }

    /// How many checkpoints were rolled back (by timeout or by request).
    pub fn checkpoint_rollbacks(&self) -> u32 {
        self.model.lock().unwrap().checkpoint_rollbacks
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
