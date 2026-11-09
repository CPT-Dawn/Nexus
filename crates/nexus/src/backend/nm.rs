//! The NetworkManager Backend over the system D-Bus.
//!
//! The proxies are `zbus-xmlgen` style `#[proxy]` traits, hand-trimmed to the
//! members the Daemon reads (research 02 recommends committing trimmed
//! output). Member names and types follow NetworkManager's D-Bus API
//! reference (`org.freedesktop.NetworkManager`,
//! `org.freedesktop.NetworkManager.Connection.Active`).

use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt as _;
use futures_util::stream::{self, Stream, StreamExt as _};
use nexus_ipc::{
    AutoconnectSet, BackendInfo, Connectivity, Disconnected, Forgotten, Joined, Link, LinkKind,
    Radio, Radios, ScanOutcome, Secret, State, Status, Tethered, Tethers, Untethered, WifiList,
    WifiShare,
};
use std::pin::Pin;

use tokio::sync::Mutex;
use zbus::Connection;
use zbus::OwnedObjectPath;
use zbus::fdo::DBusProxy;
use zbus::names::BusName;
use zbus::proxy::CacheProperties;

use super::{Backend, BackendError, Capabilities, Counters, NetFacts};
use crate::secrets::Broker;
use crate::text::{self, Failure};

const SERVICE: &str = "org.freedesktop.NetworkManager";

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
pub(super) trait NetworkManager {
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn connectivity(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn primary_connection(&self) -> zbus::Result<OwnedObjectPath>;
    #[zbus(property)]
    fn version(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn wireless_enabled(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_wireless_enabled(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn wireless_hardware_enabled(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn wwan_enabled(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_wwan_enabled(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn connectivity_check_enabled(&self) -> zbus::Result<bool>;
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn activate_connection(
        &self,
        connection: &OwnedObjectPath,
        device: &OwnedObjectPath,
        specific_object: &OwnedObjectPath,
    ) -> zbus::Result<OwnedObjectPath>;
    fn add_and_activate_connection2(
        &self,
        connection: std::collections::HashMap<
            String,
            std::collections::HashMap<String, zbus::OwnedVariant>,
        >,
        device: &OwnedObjectPath,
        specific_object: &OwnedObjectPath,
        options: std::collections::HashMap<String, zbus::OwnedVariant>,
    ) -> zbus::Result<(
        OwnedObjectPath,
        OwnedObjectPath,
        std::collections::HashMap<String, zbus::OwnedVariant>,
    )>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Connection.Active",
    default_service = "org.freedesktop.NetworkManager"
)]
trait ActiveConnection {
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;
    #[zbus(property, name = "Type")]
    fn kind(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    #[zbus(property, name = "Ip4Config")]
    fn ip4_config(&self) -> zbus::Result<OwnedObjectPath>;
    #[zbus(property, name = "Ip6Config")]
    fn ip6_config(&self) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.IP4Config",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Ip4Config {
    #[zbus(property)]
    fn gateway(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.IP6Config",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Ip6Config {
    #[zbus(property)]
    fn address_data(
        &self,
    ) -> zbus::Result<Vec<std::collections::HashMap<String, zbus::OwnedVariant>>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Statistics",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Statistics {
    #[zbus(property)]
    fn refresh_rate_ms(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn set_refresh_rate_ms(&self, ms: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn rx_bytes(&self) -> zbus::Result<u64>;
    #[zbus(property)]
    fn tx_bytes(&self) -> zbus::Result<u64>;
}

/// NetworkManager only updates a Device's counters while this is non-zero.
const STATISTICS_REFRESH_MS: u32 = 1000;

/// Property-change bursts are folded into one update per tick.
const TICK: Duration = Duration::from_millis(100);
/// How long to wait before re-opening a watch that could not be opened.
const RETRY: Duration = Duration::from_secs(2);

type Signals = Pin<Box<dyn Stream<Item = zbus::Result<zbus::Message>> + Send>>;

pub struct NetworkManager {
    broker: Arc<Broker>,
    conn: Mutex<Option<Connection>>,
    /// Every signal NetworkManager sends, plus it appearing and vanishing.
    watch: Mutex<Option<Signals>>,
    /// logind's `PrepareForSleep`, for noticing a resume.
    sleep_watch: Mutex<Option<Signals>>,
}

impl std::fmt::Debug for NetworkManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkManager").finish_non_exhaustive()
    }
}

impl NetworkManager {
    pub fn new(broker: Arc<Broker>) -> Self {
        Self {
            broker,
            conn: Mutex::default(),
            watch: Mutex::default(),
            sleep_watch: Mutex::default(),
        }
    }

    /// The system bus connection, made on first use and remade after a failure.
    async fn connection(&self) -> Result<Connection, zbus::Error> {
        let mut slot = self.conn.lock().await;
        if let Some(c) = &*slot {
            return Ok(c.clone());
        }
        let c = Connection::system().await?;
        *slot = Some(c.clone());
        Ok(c)
    }

    async fn forget_connection(&self) {
        *self.conn.lock().await = None;
    }

    async fn read(&self, conn: &Connection) -> zbus::Result<Status> {
        // No property cache: every read asks NetworkManager, so a restart of
        // NetworkManager can never leave stale answers behind.
        let nm = NetworkManagerProxy::builder(conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let state = map_state(nm.state().await?);
        let connectivity = map_connectivity(nm.connectivity().await?);
        let version = nm.version().await.ok();
        let primary = nm.primary_connection().await?;
        let link = if primary.as_str() == "/" {
            None
        } else {
            let active = ActiveConnectionProxy::builder(conn)
                .path(primary.as_str())
                .cache_properties(CacheProperties::No)
                .build()
                .await?;
            // The active connection can vanish between the two reads.
            match (active.id().await, active.kind().await) {
                (Ok(name), Ok(kind)) => Some(Link {
                    name,
                    kind: map_kind(&kind),
                }),
                _ => None,
            }
        };
        Ok(Status {
            state,
            connectivity,
            link,
            backend: BackendInfo {
                name: "networkmanager".into(),
                version,
            },
            radios: Self::read_radios(&nm).await?,
        })
    }

    async fn read_radios(nm: &NetworkManagerProxy<'_>) -> zbus::Result<Radios> {
        let wifi = nm.wireless_enabled().await?;
        let hardware_on = nm.wireless_hardware_enabled().await?;
        let wwan = nm.wwan_enabled().await?;
        Ok(Radios {
            wifi: Radio {
                enabled: wifi,
                hardware_blocked: !hardware_on,
            },
            airplane: !wifi && !wwan,
        })
    }

    /// Switches radios: Wi-Fi alone, or (airplane mode) Wi-Fi and mobile
    /// broadband together. A hardware switch is reported, not fought.
    async fn set_radios(
        conn: &Connection,
        wifi: bool,
        wwan: Option<bool>,
    ) -> zbus::Result<Result<Radios, Failure>> {
        let nm = NetworkManagerProxy::builder(conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        if wifi && !nm.wireless_hardware_enabled().await? {
            return Ok(Err(text::wifi_hard_blocked()));
        }
        nm.set_wireless_enabled(wifi).await?;
        if let Some(on) = wwan {
            nm.set_wwan_enabled(on).await?;
        }
        Ok(Ok(Self::read_radios(&nm).await?))
    }

    /// Runs `read` on the bus; a failure becomes `Unavailable` when
    /// NetworkManager is not on the bus, `Failed` when it is.
    async fn with_bus<T, F, Fut>(&self, read: F) -> Result<T, BackendError>
    where
        F: FnOnce(Connection) -> Fut,
        Fut: std::future::Future<Output = zbus::Result<T>>,
    {
        let conn = self.connection().await.map_err(|e| {
            tracing::debug!("system bus unreachable: {e}");
            BackendError::Unavailable
        })?;
        match read(conn.clone()).await {
            Ok(v) => Ok(v),
            Err(e) => {
                if Self::has_owner(&conn).await {
                    Err(BackendError::Failed(format!(
                        "NetworkManager read failed: {e}"
                    )))
                } else {
                    self.forget_connection().await;
                    Err(BackendError::Unavailable)
                }
            }
        }
    }

    async fn open_watch(&self) -> zbus::Result<Signals> {
        use zbus::message::Type;
        use zbus::{MatchRule, MessageStream};
        let conn = self.connection().await?;
        let nm_signals = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(SERVICE)
            .build()?;
        let owner = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender("org.freedesktop.DBus")
            .interface("org.freedesktop.DBus")
            .member("NameOwnerChanged")
            .arg(0, SERVICE)
            .build()?;
        // Bluetooth being switched, or its service coming and going, changes
        // what Tethering can do; NetworkManager says nothing about it.
        let adapter = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender("org.bluez")
            .interface("org.freedesktop.DBus.Properties")
            .member("PropertiesChanged")
            .arg(0, "org.bluez.Adapter1")
            .build()?;
        let bluez_owner = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender("org.freedesktop.DBus")
            .interface("org.freedesktop.DBus")
            .member("NameOwnerChanged")
            .arg(0, "org.bluez")
            .build()?;
        let a = MessageStream::for_match_rule(nm_signals, &conn, None).await?;
        let b = MessageStream::for_match_rule(owner, &conn, None).await?;
        let c = MessageStream::for_match_rule(adapter, &conn, None).await?;
        let d = MessageStream::for_match_rule(bluez_owner, &conn, None).await?;
        Ok(Box::pin(stream::select(
            stream::select(a, b),
            stream::select(c, d),
        )))
    }

    /// logind's `PrepareForSleep(b)`: true before suspend, false after resume.
    async fn open_sleep_watch(&self) -> zbus::Result<Signals> {
        use zbus::message::Type;
        use zbus::{MatchRule, MessageStream};
        let conn = self.connection().await?;
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender("org.freedesktop.login1")
            .interface("org.freedesktop.login1.Manager")
            .member("PrepareForSleep")
            .build()?;
        Ok(Box::pin(
            MessageStream::for_match_rule(rule, &conn, None).await?,
        ))
    }

    async fn has_owner(conn: &Connection) -> bool {
        let Ok(bus) = DBusProxy::new(conn).await else {
            return false;
        };
        let Ok(name) = BusName::try_from(SERVICE) else {
            return false;
        };
        bus.name_has_owner(name).await.unwrap_or(false)
    }
}

impl Backend for NetworkManager {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            connectivity_check: true,
            portal_detection: true,
        }
    }

    async fn wifi_list(&self) -> Result<WifiList, BackendError> {
        self.with_bus(|c| async move { super::wifi::list(&c).await })
            .await
    }

    async fn wifi_scan(&self) -> Result<ScanOutcome, BackendError> {
        self.with_bus(|c| async move { super::wifi::scan(&c).await })
            .await
    }

    async fn net_facts(&self) -> Result<NetFacts, BackendError> {
        self.with_bus(|c| async move { read_facts(&c).await }).await
    }

    async fn counters(&self) -> Result<Option<Counters>, BackendError> {
        self.with_bus(|c| async move { read_counters(&c).await })
            .await
    }

    async fn wifi_join(
        &self,
        network: String,
        password: Option<Secret>,
    ) -> Result<Joined, BackendError> {
        let broker = self.broker.clone();
        self.with_bus(|c| async move {
            super::wifi::join(&c, &broker, &network, password.as_ref(), None).await
        })
        .await?
        .map_err(BackendError::Refused)
    }

    async fn wifi_join_eap(
        &self,
        network: String,
        eap: nexus_ipc::Eap,
    ) -> Result<Joined, BackendError> {
        let broker = self.broker.clone();
        self.with_bus(|c| async move {
            super::wifi::join(&c, &broker, &network, None, Some(&eap)).await
        })
        .await?
        .map_err(BackendError::Refused)
    }

    async fn wifi_disconnect(&self) -> Result<Disconnected, BackendError> {
        self.with_bus(|c| async move { super::wifi::disconnect(&c).await })
            .await?
            .map_err(BackendError::Refused)
    }

    async fn wifi_forget(&self, network: String) -> Result<Forgotten, BackendError> {
        self.with_bus(|c| async move { super::wifi::forget(&c, &network).await })
            .await?
            .map_err(BackendError::Refused)
    }

    async fn wifi_autoconnect(
        &self,
        network: String,
        enabled: bool,
    ) -> Result<AutoconnectSet, BackendError> {
        self.with_bus(|c| async move { super::wifi::set_autoconnect(&c, &network, enabled).await })
            .await?
            .map_err(BackendError::Refused)
    }

    async fn profile_get(&self, network: String) -> Result<nexus_ipc::ProfileView, BackendError> {
        self.with_bus(|c| async move { super::wifi::profile::get(&c, &network).await })
            .await?
            .map_err(BackendError::Refused)
    }

    async fn profile_update(
        &self,
        network: String,
        version: u64,
        settings: nexus_ipc::ProfileSettings,
        rollback_secs: u32,
    ) -> Result<super::ProfileChanged, BackendError> {
        let changed = self
            .with_bus(|c| async move {
                super::wifi::profile::update(&c, &network, version, &settings, rollback_secs).await
            })
            .await?
            .map_err(BackendError::Refused)?;
        Ok(super::ProfileChanged {
            ssid: changed.ssid,
            version: changed.version,
            checkpoint: changed.checkpoint,
        })
    }

    async fn checkpoint_confirm(&self, checkpoint: String) -> Result<(), BackendError> {
        self.with_bus(|c| async move { super::wifi::profile::confirm(&c, &checkpoint).await })
            .await
    }

    async fn checkpoint_rollback(&self, checkpoint: String) -> Result<(), BackendError> {
        self.with_bus(|c| async move { super::wifi::profile::rollback(&c, &checkpoint).await })
            .await
    }

    async fn wifi_reveal(&self, network: String) -> Result<WifiShare, BackendError> {
        self.with_bus(|c| async move { super::wifi::reveal(&c, &network).await })
            .await?
            .map_err(BackendError::Refused)
    }

    async fn tethers(&self) -> Result<Tethers, BackendError> {
        self.with_bus(|c| async move { super::tether::list(&c).await })
            .await
    }

    async fn tether_connect(&self, phone: Option<String>) -> Result<Tethered, BackendError> {
        self.with_bus(|c| async move { super::tether::connect(&c, phone.as_deref()).await })
            .await?
            .map_err(BackendError::Refused)
    }

    async fn tether_disconnect(&self) -> Result<Untethered, BackendError> {
        self.with_bus(|c| async move { super::tether::disconnect(&c).await })
            .await?
            .map_err(BackendError::Refused)
    }

    async fn wifi_power(&self, on: bool) -> Result<Radios, BackendError> {
        self.with_bus(|c| async move { Self::set_radios(&c, on, None).await })
            .await?
            .map_err(BackendError::Refused)
    }

    async fn airplane(&self, on: bool) -> Result<Radios, BackendError> {
        self.with_bus(|c| async move { Self::set_radios(&c, !on, Some(!on)).await })
            .await?
            .map_err(BackendError::Refused)
    }

    async fn arm(&self) {
        let mut guard = self.watch.lock().await;
        if guard.is_none() {
            match self.open_watch().await {
                Ok(s) => *guard = Some(s),
                Err(e) => tracing::debug!("cannot arm the NetworkManager watch: {e}"),
            }
        }
        drop(guard);
        let mut guard = self.sleep_watch.lock().await;
        if guard.is_none() {
            match self.open_sleep_watch().await {
                Ok(s) => *guard = Some(s),
                Err(e) => tracing::debug!("cannot arm the sleep watch: {e}"),
            }
        }
    }

    async fn resumed(&self) {
        loop {
            let mut guard = self.sleep_watch.lock().await;
            if guard.is_none() {
                match self.open_sleep_watch().await {
                    Ok(s) => *guard = Some(s),
                    Err(e) => {
                        tracing::debug!("cannot watch for sleep yet: {e}");
                        self.forget_connection().await;
                        drop(guard);
                        tokio::time::sleep(RETRY).await;
                        continue;
                    }
                }
            }
            let signals = guard.as_mut().expect("just opened");
            let Some(msg) = signals.next().await else {
                *guard = None;
                drop(guard);
                self.forget_connection().await;
                tokio::time::sleep(RETRY).await;
                continue;
            };
            // `PrepareForSleep(false)` is the machine waking up.
            if let Ok(msg) = msg
                && msg.body().deserialize::<bool>() == Ok(false)
            {
                return;
            }
        }
    }

    async fn secrets(&self) {
        loop {
            match self.connection().await {
                Ok(conn) => {
                    if let Err(e) = super::agent::keep_registered(&conn, self.broker.clone()).await
                    {
                        tracing::debug!("secret agent stopped: {e}");
                    }
                }
                Err(e) => tracing::debug!("secret agent cannot reach the bus yet: {e}"),
            }
            self.forget_connection().await;
            tokio::time::sleep(RETRY).await;
        }
    }

    async fn changed(&self) {
        loop {
            let mut guard = self.watch.lock().await;
            if guard.is_none() {
                match self.open_watch().await {
                    Ok(s) => *guard = Some(s),
                    Err(e) => {
                        tracing::debug!("cannot watch NetworkManager yet: {e}");
                        self.forget_connection().await;
                        drop(guard);
                        tokio::time::sleep(RETRY).await;
                        continue;
                    }
                }
            }
            let signals = guard.as_mut().expect("just opened");
            if signals.next().await.is_none() {
                *guard = None; // the bus went away; reopen
                drop(guard);
                self.forget_connection().await;
                tokio::time::sleep(RETRY).await;
                continue;
            }
            // Let the burst land, then swallow it whole.
            tokio::time::sleep(TICK).await;
            while signals.next().now_or_never().flatten().is_some() {}
            return;
        }
    }

    async fn status(&self) -> Result<Status, BackendError> {
        let conn = match self.connection().await {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("system bus unreachable: {e}");
                return Ok(Status::unavailable());
            }
        };
        match self.read(&conn).await {
            Ok(status) => Ok(status),
            Err(e) => {
                if Self::has_owner(&conn).await {
                    Err(BackendError::Failed(format!(
                        "NetworkManager read failed: {e}"
                    )))
                } else {
                    // NetworkManager is not (yet) on the bus.
                    self.forget_connection().await;
                    Ok(Status::unavailable())
                }
            }
        }
    }
}

/// Reads what the Doctor needs. The primary connection's addressing is
/// best effort: it can vanish between reads, and "no gateway" is an answer.
async fn read_facts(conn: &Connection) -> zbus::Result<NetFacts> {
    let nm = NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let version = nm.version().await.ok();
    let wireless_enabled = nm.wireless_enabled().await?;
    let wireless_hw_enabled = nm.wireless_hardware_enabled().await?;
    let connectivity_check = nm.connectivity_check_enabled().await?;
    let mut gateway4 = None;
    let mut ipv6 = false;
    let primary = nm.primary_connection().await?;
    if primary.as_str() != "/"
        && let Ok(active) = ActiveConnectionProxy::builder(conn)
            .path(primary.as_str())
            .cache_properties(CacheProperties::No)
            .build()
            .await
    {
        if let Ok(path) = active.ip4_config().await
            && path.as_str() != "/"
            && let Ok(ip4) = Ip4ConfigProxy::builder(conn)
                .path(path.as_str())
                .cache_properties(CacheProperties::No)
                .build()
                .await
        {
            gateway4 = ip4.gateway().await.ok().filter(|g| !g.is_empty());
        }
        if let Ok(path) = active.ip6_config().await
            && path.as_str() != "/"
            && let Ok(ip6) = Ip6ConfigProxy::builder(conn)
                .path(path.as_str())
                .cache_properties(CacheProperties::No)
                .build()
                .await
        {
            ipv6 = ip6.address_data().await.is_ok_and(|a| !a.is_empty());
        }
    }
    Ok(NetFacts {
        version,
        wireless_enabled,
        wireless_hw_enabled,
        connectivity_check,
        gateway4,
        ipv6,
    })
}

async fn read_counters(conn: &Connection) -> zbus::Result<Option<Counters>> {
    let nm = NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let primary = nm.primary_connection().await?;
    if primary.as_str() == "/" {
        return Ok(None);
    }
    let active = ActiveConnectionProxy::builder(conn)
        .path(primary.as_str())
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    // The active connection can vanish between the two reads.
    let Ok(devices) = active.devices().await else {
        return Ok(None);
    };
    let Some(device) = devices.first() else {
        return Ok(None);
    };
    let stats = StatisticsProxy::builder(conn)
        .path(device.as_str())
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    if stats.refresh_rate_ms().await? == 0 {
        // Best effort: without it the counters never move.
        if let Err(e) = stats.set_refresh_rate_ms(STATISTICS_REFRESH_MS).await {
            tracing::debug!("cannot enable device statistics: {e}");
        }
    }
    Ok(Some(Counters {
        device: device.to_string(),
        rx_bytes: stats.rx_bytes().await?,
        tx_bytes: stats.tx_bytes().await?,
    }))
}

/// `NMState`. Values the Daemon does not know decode as `Unknown`.
fn map_state(nm: u32) -> State {
    match nm {
        10 => State::Asleep,
        20 => State::Offline,
        30 => State::Disconnecting,
        40 => State::Connecting,
        50 => State::Local,
        60 => State::Site,
        70 => State::Online,
        _ => State::Unknown,
    }
}

/// `NMConnectivityState`.
fn map_connectivity(nm: u32) -> Connectivity {
    match nm {
        1 => Connectivity::None,
        2 => Connectivity::Portal,
        3 => Connectivity::Limited,
        4 => Connectivity::Full,
        _ => Connectivity::Unknown,
    }
}

/// NetworkManager's connection type string.
fn map_kind(nm: &str) -> LinkKind {
    match nm {
        "802-11-wireless" => LinkKind::Wifi,
        "802-3-ethernet" => LinkKind::Ethernet,
        "vpn" => LinkKind::Vpn,
        "wireguard" => LinkKind::Wireguard,
        _ => LinkKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_numbers_map_to_unknown() {
        assert_eq!(map_state(999), State::Unknown);
        assert_eq!(map_connectivity(999), Connectivity::Unknown);
        assert_eq!(map_kind("carrier-pigeon"), LinkKind::Other);
    }
}
