//! A stand-in `org.freedesktop.secrets` (Secret Service) on a private session
//! bus: one unlocked `default` collection, plain sessions only (so clients
//! such as `oo7` fall back from their encrypted attempt), no prompts. Tests
//! seed it, read back what the Daemon kept and make it vanish like a keyring
//! that was never started or has been closed.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use zbus::connection;
use zbus::{ObjectServer, OwnedObjectPath, OwnedVariant, Variant};

const NAME: &str = "org.freedesktop.secrets";
const ROOT: &str = "/org/freedesktop/secrets";
const COLLECTION: &str = "/org/freedesktop/secrets/collection/login";
const SESSION: &str = "/org/freedesktop/secrets/session/s1";
const NO_PROMPT: &str = "/";

type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

fn path(p: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(p).expect("valid object path")
}

#[derive(Debug, Clone)]
struct Entry {
    label: String,
    attributes: HashMap<String, String>,
    secret: Vec<u8>,
}

#[derive(Debug, Default)]
struct Store {
    items: BTreeMap<u64, Entry>,
    next: u64,
    /// `GetSecret` calls served, so tests can tell a read from none.
    reads: u32,
}

type Shared = Arc<Mutex<Store>>;

fn item_path(id: u64) -> String {
    format!("{COLLECTION}/{id}")
}

fn matching(store: &Store, query: &HashMap<String, String>) -> Vec<u64> {
    store
        .items
        .iter()
        .filter(|(_, e)| query.iter().all(|(k, v)| e.attributes.get(k) == Some(v)))
        .map(|(id, _)| *id)
        .collect()
}

#[derive(Debug)]
struct Service(Shared);

#[zbus::interface(name = "org.freedesktop.Secret.Service")]
impl Service {
    async fn open_session(
        &self,
        algorithm: String,
        _input: Variant<'_>,
        #[zbus(object_server)] os: &ObjectServer,
    ) -> zbus::fdo::Result<(OwnedVariant, OwnedObjectPath)> {
        if algorithm != "plain" {
            // Like a keyring without the DH algorithm: the client retries plain.
            return Err(zbus::fdo::Error::NotSupported(
                "only the plain algorithm".into(),
            ));
        }
        os.at(SESSION, Session).await?;
        let empty = OwnedVariant::try_from(Variant::from("")).expect("plain value");
        Ok((empty, path(SESSION)))
    }

    #[zbus(property)]
    fn collections(&self) -> Vec<OwnedObjectPath> {
        vec![path(COLLECTION)]
    }

    fn read_alias(&self, name: String) -> OwnedObjectPath {
        path(if name == "default" {
            COLLECTION
        } else {
            NO_PROMPT
        })
    }

    fn search_items(
        &self,
        attributes: HashMap<String, String>,
    ) -> (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) {
        let store = self.0.lock().unwrap();
        let found = matching(&store, &attributes)
            .into_iter()
            .map(|id| path(&item_path(id)))
            .collect();
        (found, Vec::new())
    }

    fn unlock(&self, objects: Vec<OwnedObjectPath>) -> (Vec<OwnedObjectPath>, OwnedObjectPath) {
        (objects, path(NO_PROMPT))
    }
}

#[derive(Debug)]
struct Session;

#[zbus::interface(name = "org.freedesktop.Secret.Session")]
impl Session {
    fn close(&self) {}
}

#[derive(Debug)]
struct Collection(Shared);

#[zbus::interface(name = "org.freedesktop.Secret.Collection")]
impl Collection {
    #[zbus(property)]
    fn items(&self) -> Vec<OwnedObjectPath> {
        let store = self.0.lock().unwrap();
        store.items.keys().map(|id| path(&item_path(*id))).collect()
    }

    #[zbus(property)]
    fn label(&self) -> String {
        "Login".into()
    }

    #[zbus(property)]
    fn locked(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn created(&self) -> u64 {
        0
    }

    #[zbus(property)]
    fn modified(&self) -> u64 {
        0
    }

    fn search_items(&self, attributes: HashMap<String, String>) -> Vec<OwnedObjectPath> {
        let store = self.0.lock().unwrap();
        matching(&store, &attributes)
            .into_iter()
            .map(|id| path(&item_path(id)))
            .collect()
    }

    async fn create_item(
        &self,
        properties: HashMap<String, OwnedVariant>,
        secret: Secret,
        replace: bool,
        #[zbus(object_server)] os: &ObjectServer,
    ) -> zbus::fdo::Result<(OwnedObjectPath, OwnedObjectPath)> {
        let label = properties
            .get("org.freedesktop.Secret.Item.Label")
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .unwrap_or_default();
        let attributes: HashMap<String, String> = properties
            .get("org.freedesktop.Secret.Item.Attributes")
            .and_then(|v| v.try_clone().ok()?.try_into().ok())
            .unwrap_or_default();
        let id = {
            let mut store = self.0.lock().unwrap();
            let existing = replace
                .then(|| matching(&store, &attributes).first().copied())
                .flatten();
            let id = existing.unwrap_or_else(|| {
                store.next += 1;
                store.next
            });
            store.items.insert(
                id,
                Entry {
                    label,
                    attributes,
                    secret: secret.2,
                },
            );
            id
        };
        // Replacing keeps the object that is already served.
        let _ = os.at(item_path(id), Item(self.0.clone(), id)).await?;
        Ok((path(&item_path(id)), path(NO_PROMPT)))
    }
}

#[derive(Debug)]
struct Item(Shared, u64);

#[zbus::interface(name = "org.freedesktop.Secret.Item")]
impl Item {
    #[zbus(property)]
    fn locked(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn attributes(&self) -> HashMap<String, String> {
        let store = self.0.lock().unwrap();
        store
            .items
            .get(&self.1)
            .map(|e| e.attributes.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn label(&self) -> String {
        let store = self.0.lock().unwrap();
        store
            .items
            .get(&self.1)
            .map(|e| e.label.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn created(&self) -> u64 {
        0
    }

    #[zbus(property)]
    fn modified(&self) -> u64 {
        0
    }

    fn get_secret(&self, session: OwnedObjectPath) -> zbus::fdo::Result<Secret> {
        let mut store = self.0.lock().unwrap();
        store.reads += 1;
        let entry = store
            .items
            .get(&self.1)
            .ok_or_else(|| zbus::fdo::Error::Failed("no such item".into()))?;
        Ok((
            session,
            Vec::new(),
            entry.secret.clone(),
            "text/plain".into(),
        ))
    }

    fn set_secret(&self, secret: Secret) {
        if let Some(entry) = self.0.lock().unwrap().items.get_mut(&self.1) {
            entry.secret = secret.2;
        }
    }

    async fn delete(
        &self,
        #[zbus(object_server)] os: &ObjectServer,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        self.0.lock().unwrap().items.remove(&self.1);
        os.remove::<Item, _>(item_path(self.1)).await?;
        Ok(path(NO_PROMPT))
    }
}

/// The Secret Service on a private session bus.
#[derive(Debug)]
pub struct FakeSecretService {
    store: Shared,
    conn: Option<zbus::Connection>,
    rt: tokio::runtime::Runtime,
}

impl FakeSecretService {
    /// Serves the fake on the (private) bus at `address`.
    pub fn start(address: &str) -> FakeSecretService {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let store = Shared::default();
        let served = store.clone();
        let bus = address.to_owned();
        let conn = rt.block_on(async move {
            connection::Builder::address(bus.as_str())
                .serve_at(ROOT, Service(served.clone()))
                .serve_at(COLLECTION, Collection(served))
                .name(NAME)
                .build()
                .await
                .expect("start fake secret service")
        });
        FakeSecretService {
            store,
            conn: Some(conn),
            rt,
        }
    }

    /// Keeps `secret` for the Profile with this UUID, as the Daemon would.
    pub fn put(&self, uuid: &str, secret: &str) {
        let attributes = HashMap::from([
            ("application".to_owned(), "dev.cptdawn.Nexus".to_owned()),
            ("kind".to_owned(), "wifi-psk".to_owned()),
            ("profile-uuid".to_owned(), uuid.to_owned()),
        ]);
        let mut store = self.store.lock().unwrap();
        store.next += 1;
        let id = store.next;
        store.items.insert(
            id,
            Entry {
                label: "seeded".into(),
                attributes,
                secret: secret.as_bytes().to_vec(),
            },
        );
        drop(store);
        if let Some(conn) = &self.conn {
            self.rt.block_on(async {
                let _ = conn
                    .object_server()
                    .at(item_path(id), Item(self.store.clone(), id))
                    .await;
            });
        }
    }

    /// The secret the keyring holds for the Profile with this UUID.
    pub fn secret_of(&self, uuid: &str) -> Option<String> {
        let store = self.store.lock().unwrap();
        store
            .items
            .values()
            .find(|e| e.attributes.get("profile-uuid").map(String::as_str) == Some(uuid))
            .map(|e| String::from_utf8_lossy(&e.secret).into_owned())
    }

    /// How many items the keyring holds.
    pub fn items(&self) -> usize {
        self.store.lock().unwrap().items.len()
    }

    /// How many secrets were read (`GetSecret`) so far.
    pub fn reads(&self) -> u32 {
        self.store.lock().unwrap().reads
    }

    /// The keyring goes away (closed, crashed): the name is released.
    pub fn stop(&mut self) {
        if let Some(conn) = self.conn.take() {
            self.rt.block_on(conn.graceful_shutdown());
        }
    }
}
