//! The NetworkManager Backend over the system D-Bus.
//!
//! The proxies are `zbus-xmlgen` style `#[proxy]` traits, hand-trimmed to the
//! members the Daemon reads (research 02 recommends committing trimmed
//! output). Member names and types follow NetworkManager's D-Bus API
//! reference (`org.freedesktop.NetworkManager`,
//! `org.freedesktop.NetworkManager.Connection.Active`).

use nexus_ipc::{BackendInfo, Connectivity, Link, LinkKind, State, Status};
use tokio::sync::Mutex;
use zbus::Connection;
use zbus::OwnedObjectPath;
use zbus::fdo::DBusProxy;
use zbus::names::BusName;
use zbus::proxy::CacheProperties;

use super::{Backend, BackendError, Capabilities};

const SERVICE: &str = "org.freedesktop.NetworkManager";

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait NetworkManager {
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn connectivity(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn primary_connection(&self) -> zbus::Result<OwnedObjectPath>;
    #[zbus(property)]
    fn version(&self) -> zbus::Result<String>;
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
}

#[derive(Debug, Default)]
pub struct NetworkManager {
    conn: Mutex<Option<Connection>>,
}

impl NetworkManager {
    pub fn new() -> Self {
        Self::default()
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
        })
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
                    Err(BackendError(format!("NetworkManager read failed: {e}")))
                } else {
                    // NetworkManager is not (yet) on the bus.
                    self.forget_connection().await;
                    Ok(Status::unavailable())
                }
            }
        }
    }
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
