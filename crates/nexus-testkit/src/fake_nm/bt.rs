//! Bluetooth in the fake NetworkManager: paired phones appear as Devices of
//! type `NM_DEVICE_TYPE_BT` with the `Device.Bluetooth` interface, and a
//! Tethering Link is a state walk like a Wi-Fi join (40, 50, 70, 100).

use zbus::{Connection, interface};

use super::{
    DeviceId, FakeNm, Manager, Model, Profile, ProfileModel, SettingsDict, Shared, device_path,
    opath, profile_path, set_device_state,
};

/// `NMBluetoothCapabilities`.
const CAP_DUN: u32 = 1;
const CAP_NAP: u32 = 2;

/// What a paired phone looks like to NetworkManager.
#[derive(Debug, Clone)]
pub(super) struct BtModel {
    /// Upper-case `AA:BB:CC:DD:EE:FF`.
    pub address: String,
    pub name: String,
    pub caps: u32,
}

pub(super) struct Bluetooth(pub Shared, pub usize);

#[interface(name = "org.freedesktop.NetworkManager.Device.Bluetooth")]
impl Bluetooth {
    #[zbus(property)]
    fn hw_address(&self) -> String {
        self.model().address
    }

    #[zbus(property)]
    fn name(&self) -> String {
        self.model().name
    }

    #[zbus(property)]
    fn bt_capabilities(&self) -> u32 {
        self.model().caps
    }
}

impl Bluetooth {
    fn model(&self) -> BtModel {
        self.0.lock().unwrap().devices[self.1]
            .bt
            .clone()
            .expect("a Bluetooth Device has its model")
    }
}

/// `AA:BB:..` to the six bytes NetworkManager stores as `bluetooth.bdaddr`.
pub(super) fn bdaddr(address: &str) -> Vec<u8> {
    address
        .split(':')
        .map(|b| u8::from_str_radix(b, 16).expect("a hex byte"))
        .collect()
}

/// Reads the `bluetooth.bdaddr` of a connection dictionary.
pub(super) fn bdaddr_of(connection: &SettingsDict) -> Option<Vec<u8>> {
    connection
        .get("bluetooth")?
        .get("bdaddr")?
        .try_clone()
        .ok()?
        .try_into()
        .ok()
}

/// Creates the Profile for a `bluetooth` connection and starts the walk.
/// Returns the Profile's path.
pub(super) async fn add_and_activate(
    conn: &Connection,
    model: &Shared,
    connection: SettingsDict,
    device: usize,
    persist_disk: bool,
) -> zbus::fdo::Result<String> {
    let text = |section: &str, key: &str| -> String {
        connection
            .get(section)
            .and_then(|s| s.get(key))
            .and_then(|v| v.try_clone().ok()?.try_into().ok())
            .unwrap_or_default()
    };
    let addr = bdaddr_of(&connection)
        .ok_or_else(|| zbus::fdo::Error::InvalidArgs("no bluetooth.bdaddr".into()))?;
    let n = {
        let mut m = model.lock().unwrap();
        m.profiles.push(Some(ProfileModel {
            uuid: text("connection", "uuid"),
            id: text("connection", "id"),
            ssid: Vec::new(),
            autoconnect: true,
            persisted: persist_disk,
            agent_secrets: false,
            secret: None,
            settings: Default::default(),
            version: 1,
            bdaddr: Some(addr),
            wired: false,
            vpn: None,
        }));
        m.profiles.len() - 1
    };
    conn.object_server()
        .at(profile_path(n), Profile(model.clone(), n))
        .await?;
    start(conn, model, device);
    Ok(profile_path(n))
}

/// Starts the walk to an up Link on `device`.
pub(super) fn start(conn: &Connection, model: &Shared, device: usize) {
    let task = tokio::spawn(walk(conn.clone(), model.clone(), device));
    model.lock().unwrap().tasks.push(task.abort_handle());
}

async fn walk(conn: Connection, model: Shared, dev: usize) {
    let step = std::time::Duration::from_millis(15);
    let script = {
        let m = model.lock().unwrap();
        let address = m.devices[dev].bt.as_ref().map(|b| b.address.clone());
        address.and_then(|a| m.bt_failures.get(&a).copied())
    };
    for s in [40, 50, 70] {
        tokio::time::sleep(step).await;
        set_device_state(&conn, &model, dev, s, 0).await;
        if let Some((died_in, reason)) = script
            && died_in == s
        {
            tokio::time::sleep(step).await;
            set_device_state(&conn, &model, dev, 120, reason).await;
            return;
        }
    }
    tokio::time::sleep(step).await;
    set_device_state(&conn, &model, dev, 100, 0).await;
}

/// The Profile slots saved for this Bluetooth address.
pub(super) fn profile_slots(m: &Model, address: &str) -> Vec<usize> {
    let want = bdaddr(address);
    (0..m.profiles.len())
        .filter(|n| {
            m.profiles[*n]
                .as_ref()
                .is_some_and(|p| p.bdaddr.as_deref() == Some(want.as_slice()))
        })
        .collect()
}

impl FakeNm {
    /// Pairs a phone that offers network access (NAP). Its Device appears
    /// disconnected, ready for `tether_connect`.
    pub fn add_phone(&self, address: &str, name: &str) -> DeviceId {
        self.add_bt_device(address, name, CAP_NAP)
    }

    /// Pairs a phone that offers only dial-up (no network access):
    /// NetworkManager has a Device for it, but Nexus must not list it.
    pub fn add_dialup_only_phone(&self, address: &str, name: &str) -> DeviceId {
        self.add_bt_device(address, name, CAP_DUN)
    }

    fn add_bt_device(&self, address: &str, name: &str, caps: u32) -> DeviceId {
        let address = address.to_ascii_uppercase();
        let n = {
            let mut m = self.model.lock().unwrap();
            m.devices.push(super::DeviceModel {
                interface: address.clone(),
                last_scan: -1,
                aps: Vec::new(),
                rx_bytes: 0,
                tx_bytes: 0,
                state: 30,
                active_ap: None,
                bt: Some(BtModel {
                    address,
                    name: name.into(),
                    caps,
                }),
                wired: None,
            });
            m.devices.len() - 1
        };
        let model = self.model.clone();
        self.with_conn(move |c| async move {
            let os = c.object_server();
            let _ = os.at(device_path(n), super::Device(model.clone(), n)).await;
            let _ = os.at(device_path(n), Bluetooth(model, n)).await;
            if let Ok(i) = os.interface::<_, Manager>(super::MANAGER_PATH).await {
                let _ = Manager::device_added(i.signal_emitter(), opath(&device_path(n))).await;
            }
        });
        DeviceId(n)
    }

    /// Makes Tethering Links to this address die in `NMDeviceState`
    /// `died_in` with `NMDeviceStateReason` `reason`.
    pub fn script_tether_failure(&self, address: &str, died_in: u32, reason: u32) {
        self.model
            .lock()
            .unwrap()
            .bt_failures
            .insert(address.to_ascii_uppercase(), (died_in, reason));
    }

    /// Whether the phone's Link is up.
    pub fn tethered(&self, device: DeviceId) -> bool {
        self.model.lock().unwrap().devices[device.0].state == 100
    }

    /// Profiles (memory or disk) saved for this phone.
    pub fn phone_profiles(&self, address: &str) -> usize {
        let m = self.model.lock().unwrap();
        profile_slots(&m, &address.to_ascii_uppercase()).len()
    }

    /// Profiles for this phone that NetworkManager would keep across a restart.
    pub fn phone_profiles_persisted(&self, address: &str) -> usize {
        let m = self.model.lock().unwrap();
        profile_slots(&m, &address.to_ascii_uppercase())
            .into_iter()
            .filter(|n| m.profiles[*n].as_ref().is_some_and(|p| p.persisted))
            .count()
    }
}
