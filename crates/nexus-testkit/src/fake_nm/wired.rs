//! Ethernet in the fake NetworkManager: a wired Device (`NM_DEVICE_TYPE_ETHERNET`)
//! with `Device.Wired` (`HwAddress`, `Speed`, `Carrier`), its active connection
//! and IP configuration, and an activation that walks 40, 50, 70, 100 or dies
//! the way a real port does:
//!
//! - no `Carrier`: the Device is `Unavailable` (20) with reason 40, and
//!   activating it is refused;
//! - a port that wants 802.1X ([`FakeNm::set_port_eap`]) leaves a Profile
//!   without `802-1x` waiting for a DHCP answer that never comes (70, then
//!   failed with reason 5), and refuses a wrong identity or password from
//!   `NEED_AUTH` (60) with reason 7;
//! - a network that does not answer DHCP ([`FakeNm::set_dhcp`]) fails the same
//!   way as the first case, unless the Profile has a manual address.

use std::collections::HashMap;

use zbus::{Connection, OwnedObjectPath, interface};

use super::{
    Device, DeviceId, DeviceModel, Manager, Model, Profile, ProfileModel, SettingsDict, Shared,
    device_path, opath, profile_path, set_device_state,
};

/// The lease a network hands out (or the manual addresses stand in for).
#[derive(Debug, Clone)]
pub struct Lease {
    pub address: String,
    pub prefix: u8,
    pub gateway: String,
    pub dns: Vec<String>,
}

impl Default for Lease {
    fn default() -> Self {
        Self {
            address: "192.168.1.50".into(),
            prefix: 24,
            gateway: "192.168.1.1".into(),
            dns: vec!["192.168.1.1".into()],
        }
    }
}

/// What an Ethernet port looks like to NetworkManager.
#[derive(Debug, Clone)]
pub(super) struct WiredModel {
    pub mac: String,
    pub carrier: bool,
    /// `Device.Wired.Speed` in Mbit/s while there is a carrier.
    pub speed: u32,
    /// The reason of the last state change (`StateReason`).
    pub reason: u32,
    /// The Profile slot this Device is activating or has up.
    pub active: Option<usize>,
    /// Whether the network answers DHCP.
    pub dhcp: bool,
    /// The port demands 802.1X with this identity and password.
    pub eap_user: Option<(String, String)>,
    pub lease: Lease,
    /// The address configuration of a Link that is up.
    pub ip: Option<Lease>,
}

fn active_path(dev: usize) -> String {
    format!("{}/ActiveConnection/w{dev}", super::MANAGER_PATH)
}

fn ip4_path(dev: usize) -> String {
    format!("{}/IP4Config/w{dev}", super::MANAGER_PATH)
}

pub(super) struct Wired(pub Shared, pub usize);

#[interface(name = "org.freedesktop.NetworkManager.Device.Wired")]
impl Wired {
    #[zbus(property)]
    fn hw_address(&self) -> String {
        self.model().mac
    }

    #[zbus(property)]
    fn speed(&self) -> u32 {
        let w = self.model();
        if w.carrier { w.speed } else { 0 }
    }

    #[zbus(property)]
    fn carrier(&self) -> bool {
        self.model().carrier
    }
}

impl Wired {
    fn model(&self) -> WiredModel {
        self.0.lock().unwrap().devices[self.1]
            .wired
            .clone()
            .expect("a wired Device has its model")
    }
}

/// The wired Device's `Connection.Active`, served for the whole life of the
/// Device and pointed at by `Device.ActiveConnection` only while a Profile is
/// active.
pub(super) struct WiredActive(pub Shared, pub usize);

impl WiredActive {
    fn profile<T>(&self, f: impl FnOnce(&ProfileModel) -> T) -> Option<T> {
        let m = self.0.lock().unwrap();
        let slot = m.devices[self.1].wired.as_ref()?.active?;
        m.profiles[slot].as_ref().map(f)
    }
}

#[interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl WiredActive {
    #[zbus(property)]
    fn id(&self) -> String {
        self.profile(|p| p.id.clone()).unwrap_or_default()
    }

    #[zbus(property)]
    fn uuid(&self) -> String {
        self.profile(|p| p.uuid.clone()).unwrap_or_default()
    }

    #[zbus(property, name = "Type")]
    fn kind(&self) -> String {
        "802-3-ethernet".into()
    }

    #[zbus(property)]
    fn state(&self) -> u32 {
        // Activating while the Device walks, activated once it is up.
        match self.0.lock().unwrap().devices[self.1].state {
            100 => 2,
            _ => 1,
        }
    }

    #[zbus(property)]
    fn devices(&self) -> Vec<OwnedObjectPath> {
        vec![opath(&device_path(self.1))]
    }

    #[zbus(property, name = "Ip4Config")]
    fn ip4_config(&self) -> OwnedObjectPath {
        ip4_object(&self.0, self.1)
    }

    #[zbus(property, name = "Ip6Config")]
    fn ip6_config(&self) -> OwnedObjectPath {
        opath("/")
    }
}

fn ip4_object(model: &Shared, dev: usize) -> OwnedObjectPath {
    let up = model.lock().unwrap().devices[dev]
        .wired
        .as_ref()
        .is_some_and(|w| w.ip.is_some());
    opath(&if up { ip4_path(dev) } else { "/".into() })
}

/// The wired Device's `IP4Config`.
pub(super) struct WiredIp4(pub Shared, pub usize);

impl WiredIp4 {
    fn lease(&self) -> Lease {
        self.0.lock().unwrap().devices[self.1]
            .wired
            .as_ref()
            .and_then(|w| w.ip.clone())
            .unwrap_or_default()
    }
}

fn row(pairs: Vec<(&str, zbus::Variant<'_>)>) -> HashMap<String, zbus::OwnedVariant> {
    pairs
        .into_iter()
        .map(|(k, v)| {
            (
                k.to_owned(),
                zbus::OwnedVariant::try_from(v).expect("plain value"),
            )
        })
        .collect()
}

#[interface(name = "org.freedesktop.NetworkManager.IP4Config")]
impl WiredIp4 {
    #[zbus(property)]
    fn gateway(&self) -> String {
        self.lease().gateway
    }

    #[zbus(property)]
    fn address_data(&self) -> Vec<HashMap<String, zbus::OwnedVariant>> {
        let l = self.lease();
        vec![row(vec![
            ("address", zbus::Variant::from(l.address)),
            ("prefix", zbus::Variant::from(u32::from(l.prefix))),
        ])]
    }

    #[zbus(property)]
    fn nameserver_data(&self) -> Vec<HashMap<String, zbus::OwnedVariant>> {
        self.lease()
            .dns
            .into_iter()
            .map(|d| row(vec![("address", zbus::Variant::from(d))]))
            .collect()
    }
}

/// What `Device.ActiveConnection` says.
pub(super) fn active_connection(model: &Shared, dev: usize) -> OwnedObjectPath {
    let active = model.lock().unwrap().devices[dev]
        .wired
        .as_ref()
        .is_some_and(|w| w.active.is_some());
    opath(&if active { active_path(dev) } else { "/".into() })
}

/// What `Device.Ip4Config` says.
pub(super) fn ip4_config(model: &Shared, dev: usize) -> OwnedObjectPath {
    ip4_object(model, dev)
}

/// The text of a setting of a connection dictionary.
fn text(dict: &SettingsDict, section: &str, key: &str) -> Option<String> {
    dict.get(section)?
        .get(key)?
        .try_clone()
        .ok()?
        .try_into()
        .ok()
}

/// Creates the Profile for an `802-3-ethernet` connection and starts the
/// walk. Returns the Profile's path.
pub(super) async fn add_and_activate(
    conn: &Connection,
    model: &Shared,
    connection: SettingsDict,
    device: usize,
    persist_disk: bool,
) -> zbus::fdo::Result<String> {
    refuse_unplugged(model, device)?;
    let n = {
        let mut m = model.lock().unwrap();
        m.profiles.push(Some(ProfileModel {
            uuid: text(&connection, "connection", "uuid").unwrap_or_default(),
            id: text(&connection, "connection", "id").unwrap_or_default(),
            ssid: Vec::new(),
            autoconnect: true,
            persisted: persist_disk,
            agent_secrets: false,
            secret: None,
            settings: super::clone_settings(&connection),
            version: 1,
            bdaddr: None,
            vpn: None,
            hotspot: None,
            wired: true,
        }));
        m.profiles.len() - 1
    };
    conn.object_server()
        .at(profile_path(n), Profile(model.clone(), n))
        .await?;
    start(conn, model, device, n);
    Ok(profile_path(n))
}

/// NetworkManager will not activate a Device without a carrier.
pub(super) fn refuse_unplugged(model: &Shared, dev: usize) -> zbus::fdo::Result<()> {
    let plugged = model.lock().unwrap().devices[dev]
        .wired
        .as_ref()
        .is_none_or(|w| w.carrier);
    if plugged {
        Ok(())
    } else {
        Err(zbus::fdo::Error::Failed(
            "Connection is not available on the device because the device has no carrier".into(),
        ))
    }
}

/// Starts activating Profile slot `profile` on wired Device `device`.
pub(super) fn start(conn: &Connection, model: &Shared, device: usize, profile: usize) {
    let task = tokio::spawn(walk(conn.clone(), model.clone(), device, profile));
    model.lock().unwrap().tasks.push(task.abort_handle());
}

/// What the activation of `profile` will end in: `None` for a Link that comes
/// up, else the state it dies in and the reason.
fn verdict(m: &Model, dev: usize, profile: usize) -> Option<(u32, u32)> {
    let w = m.devices[dev].wired.as_ref()?;
    let p = m.profiles[profile].as_ref()?;
    let has_eap = p.settings.contains_key("802-1x");
    if let Some((identity, password)) = &w.eap_user {
        if !has_eap {
            return Some((70, 5)); // nobody answers the DHCP request
        }
        let given = (
            text(&p.settings, "802-1x", "identity"),
            text(&p.settings, "802-1x", "password"),
        );
        if given != (Some(identity.clone()), Some(password.clone())) {
            return Some((60, 7)); // NEED_AUTH, NO_SECRETS
        }
        return None;
    }
    let manual = text(&p.settings, "ipv4", "method").as_deref() == Some("manual");
    if !w.dhcp && !manual {
        return Some((70, 5));
    }
    None
}

/// The address configuration a Profile brings up.
fn lease_for(m: &Model, dev: usize, profile: usize) -> Lease {
    let w = m.devices[dev].wired.as_ref().expect("wired Device");
    let mut lease = w.lease.clone();
    let Some(p) = m.profiles[profile].as_ref() else {
        return lease;
    };
    if text(&p.settings, "ipv4", "method").as_deref() == Some("manual")
        && let Some(rows) = p.settings.get("ipv4").and_then(|s| s.get("address-data"))
        && let Some(first) = super::rows_of(rows).first()
    {
        let get = |k: &str| -> Option<String> { first.get(k)?.try_clone().ok()?.try_into().ok() };
        if let Some(a) = get("address") {
            lease.address = a;
        }
        if let Some(prefix) = first
            .get("prefix")
            .and_then(|v| u32::try_from(v.try_clone().ok()?).ok())
        {
            lease.prefix = u8::try_from(prefix).unwrap_or(24);
        }
        lease.gateway = text(&p.settings, "ipv4", "gateway").unwrap_or_default();
    }
    lease
}

async fn walk(conn: Connection, model: Shared, dev: usize, profile: usize) {
    let step = std::time::Duration::from_millis(15);
    let failure = verdict(&model.lock().unwrap(), dev, profile);
    {
        let mut m = model.lock().unwrap();
        if let Some(w) = m.devices[dev].wired.as_mut() {
            w.active = Some(profile);
            w.ip = None;
        }
    }
    announce(&conn, dev).await;
    for s in [40, 50, 60, 70] {
        // Only a refused sign-in visits NEED_AUTH.
        if s == 60 && failure.map(|f| f.0) != Some(60) {
            continue;
        }
        tokio::time::sleep(step).await;
        set_device_state(&conn, &model, dev, s, 0).await;
        if let Some((died_in, reason)) = failure
            && died_in == s
        {
            tokio::time::sleep(step).await;
            // A failed activation is gone from the Device, like the real one.
            if let Some(w) = model.lock().unwrap().devices[dev].wired.as_mut() {
                w.active = None;
            }
            set_device_state(&conn, &model, dev, 120, reason).await;
            announce(&conn, dev).await;
            return;
        }
    }
    tokio::time::sleep(step).await;
    {
        let mut m = model.lock().unwrap();
        let lease = lease_for(&m, dev, profile);
        if let Some(w) = m.devices[dev].wired.as_mut() {
            w.ip = Some(lease);
        }
    }
    set_device_state(&conn, &model, dev, 100, 0).await;
    announce(&conn, dev).await;
}

/// Tells watchers that the Device's connection and addressing moved. Their
/// objects are served for the whole life of the Device; only the pointers on
/// `Device` change.
pub(super) async fn announce(conn: &Connection, dev: usize) {
    let os = conn.object_server();
    if let Ok(i) = os.interface::<_, Device>(device_path(dev)).await {
        let guard = i.get().await;
        let _ = guard.active_connection_changed(i.signal_emitter()).await;
        let _ = guard.ip4_config_changed(i.signal_emitter()).await;
    }
}

impl super::FakeNm {
    /// Adds an Ethernet port with its cable plugged in and nothing joined
    /// (`Disconnected`), at 1 Gbps, on a network that answers DHCP.
    pub fn add_ethernet_device(&self, interface: &str) -> DeviceId {
        let n = {
            let mut m = self.model.lock().unwrap();
            let mac = format!("52:54:00:00:00:{:02X}", m.devices.len());
            m.devices.push(DeviceModel {
                interface: interface.into(),
                last_scan: -1,
                aps: Vec::new(),
                rx_bytes: 0,
                tx_bytes: 0,
                state: 30,
                active_ap: None,
                bt: None,
                ap_capable: false,
                wired: Some(WiredModel {
                    mac,
                    carrier: true,
                    speed: 1000,
                    reason: 0,
                    active: None,
                    dhcp: true,
                    eap_user: None,
                    lease: Lease::default(),
                    ip: None,
                }),
            });
            m.devices.len() - 1
        };
        let model = self.model.clone();
        self.with_conn(move |c| async move {
            serve(&c, &model, n).await;
            let os = c.object_server();
            if let Ok(i) = os.interface::<_, Manager>(super::MANAGER_PATH).await {
                let _ = Manager::device_added(i.signal_emitter(), opath(&device_path(n))).await;
            }
        });
        DeviceId(n)
    }

    /// Plugs or pulls the cable. Pulling drops a Link that was up (state 20,
    /// reason 40); plugging lets the Device rest at `Disconnected`.
    pub fn set_carrier(&self, device: DeviceId, plugged: bool) {
        let model = self.model.clone();
        self.with_conn(move |c| async move {
            let dev = device.0;
            {
                let mut m = model.lock().unwrap();
                let Some(w) = m.devices[dev].wired.as_mut() else {
                    return;
                };
                w.carrier = plugged;
                if !plugged {
                    w.active = None;
                    w.ip = None;
                }
            }
            for task in model.lock().unwrap().tasks.drain(..) {
                task.abort();
            }
            if let Ok(i) = c
                .object_server()
                .interface::<_, Wired>(device_path(dev))
                .await
            {
                let guard = i.get().await;
                let _ = guard.carrier_changed(i.signal_emitter()).await;
                let _ = guard.speed_changed(i.signal_emitter()).await;
            }
            if plugged {
                set_device_state(&c, &model, dev, 30, 41).await;
            } else {
                set_device_state(&c, &model, dev, 20, 40).await;
            }
            announce(&c, dev).await;
        });
    }

    /// The speed the port negotiates, in Mbit/s.
    pub fn set_link_speed(&self, device: DeviceId, mbps: u32) {
        let model = self.model.clone();
        self.with_conn(move |c| async move {
            if let Some(w) = model.lock().unwrap().devices[device.0].wired.as_mut() {
                w.speed = mbps;
            }
            if let Ok(i) = c
                .object_server()
                .interface::<_, Wired>(device_path(device.0))
                .await
            {
                let _ = i.get().await.speed_changed(i.signal_emitter()).await;
            }
        });
    }

    /// Whether the network behind the port answers DHCP.
    pub fn set_dhcp(&self, device: DeviceId, answers: bool) {
        if let Some(w) = self.model.lock().unwrap().devices[device.0].wired.as_mut() {
            w.dhcp = answers;
        }
    }

    /// The lease the network hands out.
    pub fn set_lease(&self, device: DeviceId, lease: Lease) {
        if let Some(w) = self.model.lock().unwrap().devices[device.0].wired.as_mut() {
            w.lease = lease;
        }
    }

    /// The port demands 802.1X: only this identity and password get past it.
    pub fn set_port_eap(&self, device: DeviceId, identity: &str, password: &str) {
        if let Some(w) = self.model.lock().unwrap().devices[device.0].wired.as_mut() {
            w.eap_user = Some((identity.into(), password.into()));
        }
    }

    /// A Profile (saved on disk) for a wired port: DHCP, no sign-in. Returns
    /// its UUID.
    pub fn add_ethernet_profile(&self, uuid: &str, id: &str, interface: &str) {
        let mut m = self.model.lock().unwrap();
        let v = |x: zbus::Variant<'_>| zbus::OwnedVariant::try_from(x).expect("plain value");
        let settings: SettingsDict = HashMap::from([
            (
                "connection".to_owned(),
                HashMap::from([(
                    "interface-name".to_owned(),
                    v(zbus::Variant::from(interface)),
                )]),
            ),
            ("802-3-ethernet".to_owned(), HashMap::new()),
            (
                "ipv4".to_owned(),
                HashMap::from([("method".to_owned(), v(zbus::Variant::from("auto")))]),
            ),
        ]);
        m.profiles.push(Some(ProfileModel {
            uuid: uuid.into(),
            id: id.into(),
            ssid: Vec::new(),
            autoconnect: true,
            persisted: true,
            agent_secrets: false,
            secret: None,
            settings,
            version: 1,
            bdaddr: None,
            vpn: None,
            hotspot: None,
            wired: true,
        }));
        let n = m.profiles.len() - 1;
        drop(m);
        let model = self.model.clone();
        self.with_conn(move |c| async move {
            let _ = c
                .object_server()
                .at(profile_path(n), Profile(model, n))
                .await;
        });
    }

    /// Whether the port's Link is up.
    pub fn ethernet_up(&self, device: DeviceId) -> bool {
        self.model.lock().unwrap().devices[device.0].state == 100
    }

    /// The UUIDs of the wired Profiles that exist.
    pub fn ethernet_profiles(&self) -> Vec<String> {
        let m = self.model.lock().unwrap();
        m.profiles
            .iter()
            .flatten()
            .filter(|p| p.wired)
            .map(|p| p.uuid.clone())
            .collect()
    }
}

/// Serves a wired Device's objects.
pub(super) async fn serve(c: &Connection, model: &Shared, n: usize) {
    let os = c.object_server();
    let _ = os.at(device_path(n), Device(model.clone(), n)).await;
    let _ = os
        .at(device_path(n), super::Statistics(model.clone(), n))
        .await;
    let _ = os.at(device_path(n), Wired(model.clone(), n)).await;
    let _ = os.at(active_path(n), WiredActive(model.clone(), n)).await;
    let _ = os.at(ip4_path(n), WiredIp4(model.clone(), n)).await;
}

/// The paths a restarted fake serves again for wired Device `n`.
pub(super) fn paths(n: usize) -> (String, String) {
    (active_path(n), ip4_path(n))
}
