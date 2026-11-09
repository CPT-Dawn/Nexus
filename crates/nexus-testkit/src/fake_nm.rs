//! A scriptable fake `org.freedesktop.NetworkManager`.
//!
//! It serves only the slice of NetworkManager the Daemon reads. Members are
//! checked against NetworkManager's D-Bus API reference: `State` (u),
//! `Connectivity` (u), `PrimaryConnection` (o), `Version` (s), the
//! `StateChanged(u)` signal, and `Connection.Active` with `Id` (s), `Type`
//! (s), `State` (u). A tiny fake means a proxy member it lacks fails loudly
//! with `UnknownProperty`, which keeps the Backend narrow.
//!
//! To add a scenario for a new ticket: add the member to the interface below,
//! keep its value in [`Model`], and expose a `set_*` method on [`FakeNm`] that
//! mutates the model and then calls [`FakeNm::emit`]-style change signals.

use std::sync::{Arc, Mutex};

use tokio::runtime::Runtime;
use zbus::OwnedObjectPath;
use zbus::fdo::ObjectManager;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, connection, interface};

const MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
const ACTIVE_PATH: &str = "/org/freedesktop/NetworkManager/ActiveConnection/1";
const NAME: &str = "org.freedesktop.NetworkManager";

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
            connection::Builder::address(address.as_str())
                .serve_at("/org/freedesktop", ObjectManager)
                .serve_at(MANAGER_PATH, Manager(model.clone()))
                .serve_at(ACTIVE_PATH, Active(model))
                .name(NAME)
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
