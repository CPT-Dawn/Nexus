//! A scriptable fake `org.freedesktop.systemd1`, the seam for the Doctor's
//! look at the host's services (S1).
//!
//! It serves two `Manager` methods, the ones the Daemon calls:
//! `GetUnitFileState(s) -> s` (an error for a unit with no file, like the
//! real one) and `ListUnitsByPatterns(as, as) -> a(ssssssouso)` (loaded units
//! only). Units start absent; script them with [`FakeSystemd::set_unit`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::runtime::Runtime;
use zbus::{Connection, OwnedObjectPath, connection, interface};

const NAME: &str = "org.freedesktop.systemd1";
const PATH: &str = "/org/freedesktop/systemd1";

/// How a unit looks to the Daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// No unit file: the package is not installed.
    Absent,
    /// Installed, not running, not loaded.
    Inactive,
    /// Installed and running.
    Active,
}

type Units = Arc<Mutex<HashMap<String, Unit>>>;

struct Manager(Units);

type Row = (
    String,
    String,
    String,
    String,
    String,
    String,
    OwnedObjectPath,
    u32,
    String,
    OwnedObjectPath,
);

#[interface(name = "org.freedesktop.systemd1.Manager")]
impl Manager {
    fn get_unit_file_state(&self, name: String) -> zbus::fdo::Result<String> {
        match self.0.lock().unwrap().get(&name) {
            None | Some(Unit::Absent) => Err(zbus::fdo::Error::FileNotFound(format!(
                "unit file {name} does not exist"
            ))),
            Some(_) => Ok("enabled".into()),
        }
    }

    fn list_units_by_patterns(&self, _states: Vec<String>, patterns: Vec<String>) -> Vec<Row> {
        let none = || OwnedObjectPath::try_from("/").expect("static path");
        let mut rows: Vec<Row> = self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, unit)| **unit == Unit::Active && patterns.contains(name))
            .map(|(name, _)| {
                (
                    name.clone(),
                    name.clone(),
                    "loaded".into(),
                    "active".into(),
                    "running".into(),
                    String::new(),
                    none(),
                    0,
                    String::new(),
                    none(),
                )
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }
}

/// Handle on the fake. Fields drop in order: the service goes before its runtime.
#[derive(Debug)]
pub struct FakeSystemd {
    _conn: Connection,
    units: Units,
    _rt: Runtime,
}

impl FakeSystemd {
    /// Serves the fake on the bus at `address`, every unit absent.
    pub fn start(address: &str) -> FakeSystemd {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let units: Units = Arc::default();
        let served = units.clone();
        let address = address.to_owned();
        let conn = rt.block_on(async move {
            connection::Builder::address(address.as_str())
                .serve_at(PATH, Manager(served))
                .name(NAME)
                .build()
                .await
                .expect("serve fake systemd")
        });
        FakeSystemd {
            _conn: conn,
            units,
            _rt: rt,
        }
    }

    /// Sets how a unit looks, e.g. `"systemd-networkd.service"`.
    pub fn set_unit(&self, name: &str, unit: Unit) {
        self.units.lock().unwrap().insert(name.into(), unit);
    }
}
