//! A stand-in `org.freedesktop.Notifications` server on a private session
//! bus, so tests see what the Daemon announces and can click its buttons
//! without a notification ever reaching the real desktop.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::OwnedVariant;
use zbus::connection;
use zbus::object_server::SignalEmitter;

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// One `Notify` call the server received.
#[derive(Debug, Clone)]
pub struct Notified {
    /// The id the server handed back (1-based, in arrival order).
    pub id: u32,
    pub app_name: String,
    pub replaces_id: u32,
    pub summary: String,
    pub body: String,
    /// Flat `[key, label, key, label, ...]`, as on the wire.
    pub actions: Vec<String>,
    /// Hint values as text (`urgency` is `"1"`, `desktop-entry` is the name).
    pub hints: HashMap<String, String>,
    pub expire_timeout: i32,
}

#[derive(Debug, Default)]
struct Log {
    notified: Vec<Notified>,
    closed: Vec<u32>,
}

#[derive(Debug)]
struct Server {
    log: Arc<Mutex<Log>>,
}

fn hint_text(v: OwnedVariant) -> String {
    if let Ok(s) = String::try_from(v.clone()) {
        return s;
    }
    if let Ok(n) = u8::try_from(&v) {
        return n.to_string();
    }
    format!("{v:?}")
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    fn get_capabilities(&self) -> Vec<String> {
        ["actions", "body"].map(String::from).to_vec()
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "stand-in".into(),
            "nexus-testkit".into(),
            "0".into(),
            "1.2".into(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedVariant>,
        expire_timeout: i32,
    ) -> u32 {
        let mut log = self.log.lock().unwrap();
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            log.notified.len() as u32 + 1
        };
        log.notified.push(Notified {
            id,
            app_name,
            replaces_id,
            summary,
            body,
            actions,
            hints: hints.into_iter().map(|(k, v)| (k, hint_text(v))).collect(),
            expire_timeout,
        });
        id
    }

    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.log.lock().unwrap().closed.push(id);
        Self::notification_closed(&emitter, id, 3).await?;
        Ok(())
    }

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;
}

/// Lets the test click a button: the signal must come from the server's own
/// connection or the Daemon (rightly) ignores it.
#[derive(Debug)]
struct Remote;

#[zbus::interface(name = "org.nexus.Test")]
impl Remote {
    async fn invoke(
        &self,
        id: u32,
        key: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        Server::action_invoked(&emitter, id, &key).await?;
        Ok(())
    }

    /// The server dismisses the notification (expiry or the user closing it).
    async fn dismiss(
        &self,
        id: u32,
        reason: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        Server::notification_closed(&emitter, id, reason).await?;
        Ok(())
    }
}

/// The notification server on a private session bus.
#[derive(Debug)]
pub struct FakeNotifications {
    log: Arc<Mutex<Log>>,
    address: String,
    _conn: zbus::Connection,
    rt: tokio::runtime::Runtime,
}

impl FakeNotifications {
    /// Serves the fake on the (private) bus at `address`.
    pub fn start(address: &str) -> FakeNotifications {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let log = Arc::new(Mutex::new(Log::default()));
        let served = log.clone();
        let address = address.to_owned();
        let bus = address.clone();
        let conn = rt.block_on(async move {
            connection::Builder::address(bus.as_str())
                .serve_at(PATH, Server { log: served })
                .serve_at(PATH, Remote)
                .name(NAME)
                .build()
                .await
                .expect("serve fake notifications")
        });
        FakeNotifications {
            log,
            address,
            _conn: conn,
            rt,
        }
    }

    /// Waits for the `n`th notification (1-based) and returns it.
    pub fn notified(&self, n: usize) -> Notified {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(found) = self.log.lock().unwrap().notified.get(n - 1) {
                return found.clone();
            }
            assert!(
                Instant::now() < deadline,
                "notification {n} never arrived (saw {})",
                self.count()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// How many `Notify` calls arrived so far.
    pub fn count(&self) -> usize {
        self.log.lock().unwrap().notified.len()
    }

    /// Every notification so far, oldest first.
    pub fn all(&self) -> Vec<Notified> {
        self.log.lock().unwrap().notified.clone()
    }

    /// Ids the Daemon asked to close, in order.
    pub fn closed(&self) -> Vec<u32> {
        self.log.lock().unwrap().closed.clone()
    }

    /// Waits until the Daemon has closed notification `id`.
    pub fn wait_closed(&self, id: u32) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.closed().contains(&id) {
            assert!(
                Instant::now() < deadline,
                "notification {id} was never closed"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Clicks button `key` on notification `id`, as the user would.
    pub fn invoke(&self, id: u32, key: &str) {
        self.call("Invoke", &(id, key));
    }

    /// The server drops notification `id` by itself (it expired: reason 1).
    pub fn dismiss(&self, id: u32, reason: u32) {
        self.call("Dismiss", &(id, reason));
    }

    fn call<B: serde::Serialize + zbus::zvariant::DynamicType>(&self, method: &str, body: &B) {
        self.rt.block_on(async {
            // A second connection: the signal must be sent by the server's.
            let caller = connection::Builder::address(self.address.as_str())
                .build()
                .await
                .expect("test helper connection");
            caller
                .call_method(Some(NAME), PATH, Some("org.nexus.Test"), method, body)
                .await
                .expect("test helper call");
        });
    }
}
