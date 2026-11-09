//! A fake `org.bluez` on the private system bus: one adapter whose power can
//! be switched, and the ObjectManager the Daemon reads it through. It never
//! touches a real Bluetooth stack. Phones are the fake NetworkManager's
//! Bluetooth Devices (`FakeNm::add_phone`), because that is where Nexus
//! lists them.

use std::future::Future;
use std::sync::{Arc, Mutex};

use tokio::runtime::Runtime;
use zbus::fdo::ObjectManager;
use zbus::{Connection, connection, interface};

const ADAPTER_PATH: &str = "/org/bluez/hci0";

struct Adapter(Arc<Mutex<bool>>);

#[interface(name = "org.bluez.Adapter1")]
impl Adapter {
    #[zbus(property)]
    fn powered(&self) -> bool {
        *self.0.lock().unwrap()
    }

    #[zbus(property)]
    fn set_powered(&mut self, on: bool) {
        *self.0.lock().unwrap() = on;
    }
}

/// Handle on the fake BlueZ. Methods are synchronous, like [`crate::FakeNm`].
#[derive(Debug)]
pub struct FakeBluez {
    rt: Runtime,
    address: String,
    powered: Arc<Mutex<bool>>,
    conn: Option<Connection>,
}

impl FakeBluez {
    /// Serves one adapter on the bus at `address`, powered or not.
    pub fn start(address: &str, powered: bool) -> FakeBluez {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let mut bluez = FakeBluez {
            rt,
            address: address.to_owned(),
            powered: Arc::new(Mutex::new(powered)),
            conn: None,
        };
        bluez.restart();
        bluez
    }

    /// Switches the adapter on or off, announcing it like BlueZ does.
    pub fn set_powered(&self, on: bool) {
        *self.powered.lock().unwrap() = on;
        self.with_conn(|conn| async move {
            if let Ok(i) = conn
                .object_server()
                .interface::<_, Adapter>(ADAPTER_PATH)
                .await
            {
                let _ = i.get().await.powered_changed(i.signal_emitter()).await;
            }
        });
    }

    /// Whether the adapter is powered right now (the Daemon may have changed it).
    pub fn powered(&self) -> bool {
        *self.powered.lock().unwrap()
    }

    /// The Bluetooth service going away: the name is released.
    pub fn stop(&mut self) {
        self.conn = None;
    }

    pub fn is_running(&self) -> bool {
        self.conn.is_some()
    }

    /// The Bluetooth service (re)appearing.
    pub fn restart(&mut self) {
        self.conn = None;
        let (address, powered) = (self.address.clone(), self.powered.clone());
        let conn = self.rt.block_on(async move {
            connection::Builder::address(address.as_str())
                .serve_at("/", ObjectManager)
                .serve_at(ADAPTER_PATH, Adapter(powered))
                .name("org.bluez")
                .build()
                .await
                .expect("serve fake BlueZ")
        });
        self.conn = Some(conn);
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
}
