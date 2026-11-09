//! Ready-made worlds, shared by the `demo` example and the S2 tests so the
//! frame you eyeball is the frame that is under test.

use std::thread::{self, JoinHandle};

use serde_json::json;

use crate::{Ap, DeviceId, World};

const WPA3: (u32, u32, u32) = (0x1, 0, 0x408); // privacy, SAE | CCMP

/// A busy Wi-Fi tab: six Networks in range (one joined, with a Profile), two
/// saved ones out of range. Returns the Device.
pub fn wifi_busy(world: &World) -> DeviceId {
    busy(world, false)
}

fn busy(world: &World, enterprise: bool) -> DeviceId {
    let nm = &world.nm;
    let wlan0 = nm.add_wifi_device("wlan0");
    nm.add_ap(
        wlan0,
        Ap::wpa2("Home-5G", "AA:00:00:00:00:01", 82).frequency(5180),
    );
    nm.add_ap(wlan0, Ap::open("Cafe Guest", "AA:00:00:00:00:02", 58));
    nm.add_ap(
        wlan0,
        Ap::wpa2("Work-Secure", "AA:00:00:00:00:03", 64)
            .frequency(5745)
            .security(WPA3.0, WPA3.1, WPA3.2),
    );
    nm.add_ap(wlan0, Ap::wpa2("Neighbour-2G", "AA:00:00:00:00:04", 36));
    nm.add_ap(
        wlan0,
        Ap::wpa2(
            "A very long network name that will not fit",
            "AA:00:00:00:00:05",
            22,
        ),
    );
    nm.add_ap(
        wlan0,
        Ap::wpa2("Rooftop-6E", "AA:00:00:00:00:06", 14).frequency(6135),
    );
    if enterprise {
        // In range before the join, so the first list already has it.
        nm.add_ap(wlan0, Ap::enterprise("eduroam", "AA:00:00:00:00:07", 74));
        nm.set_eap_user("eduroam", "you@uni.example", "correct-horse");
    }
    nm.add_wifi_profile("uuid-parents", "Parents House", b"Parents House", true);
    nm.set_profile_secret("uuid-parents", "wpa-psk", "parents-wifi-2024");
    nm.add_wifi_profile("uuid-hotel", "Hotel Lobby", b"Hotel Lobby", false);
    // Join Home-5G for real, so it is joined and saved.
    let resp = world.wire().call(json!({
        "op": "wifi_join",
        "network": nexus_ipc::network_id(b"Home-5G"),
        "password": "demo-password",
    }));
    assert!(resp.get("ok").is_some(), "joining Home-5G: {resp}");
    wlan0
}

/// [`wifi_busy`], plus `eduroam`, a WPA-Enterprise Network in range that
/// accepts `you@uni.example` with `correct-horse`. Returns the Device.
pub fn wifi_enterprise(world: &World) -> DeviceId {
    busy(world, true)
}

/// [`wifi_busy`], plus a saved Profile (Parents House) that already has its
/// own settings: a static address, custom DNS, metered, a priority. The
/// joined Home-5G has defaults, so editing it shows the checkpoint countdown.
pub fn profile_busy(world: &World) -> DeviceId {
    let wlan0 = wifi_busy(world);
    let network = nexus_ipc::network_id(b"Parents House");
    let view = world
        .wire()
        .call(json!({"op": "profile_get", "network": network}));
    let resp = world.wire().call(json!({
        "op": "profile_update",
        "network": network,
        "version": view["ok"]["version"],
        "settings": {
            "ipv4": {
                "method": "manual",
                "addresses": [{"address": "192.168.7.20", "prefix": 24}],
                "gateway": "192.168.7.1",
            },
            "ipv6": {"method": "auto"},
            "dns": ["9.9.9.9", "149.112.112.112"],
            "mac": "per_network",
            "metered": true,
            "priority": 10,
        },
    }));
    assert!(resp.get("ok").is_some(), "editing Parents House: {resp}");
    wlan0
}

/// [`wifi_busy`], plus a join of `Work-Secure` that is waiting for its
/// password: the Daemon holds an open Secret prompt for any Front end that
/// subscribes. The returned thread ends when the prompt is answered.
pub fn wifi_with_open_prompt(world: &World) -> JoinHandle<()> {
    wifi_busy(world);
    let mut wire = world.wire();
    thread::spawn(move || {
        let _ = wire.call(json!({
            "op": "wifi_join",
            "network": nexus_ipc::network_id(b"Work-Secure"),
        }));
    })
}

/// A WireGuard config the VPN scenarios import (documentation keys).
pub const WG_CONF: &str = "[Interface]\nPrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=\nAddress = 10.8.0.2/32\nDNS = 10.8.0.1\n\n[Peer]\nPublicKey = xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = vpn.work.example:51820\nPersistentKeepalive = 25\n";

/// A filled VPN tab: `Work` (WireGuard, up), `Home Lab` (WireGuard, down) and
/// `Office OpenVPN` (a plugin VPN, down), on a machine that is online.
pub fn vpn_busy(world: &World) {
    use crate::{ActiveConnection, NmConnectivity, NmState};
    let nm = &world.nm;
    nm.set_primary(Some(ActiveConnection::wifi("Home-5G")));
    nm.set_connectivity(NmConnectivity::Full);
    nm.set_state(NmState::ConnectedGlobal);
    let mut wire = world.wire();
    let work = wire.call(json!({"op": "vpn_import", "name": "Work", "config": WG_CONF}));
    assert!(work.get("ok").is_some(), "importing Work: {work}");
    let lab = wire.call(json!({
        "op": "vpn_create", "name": "Home Lab", "addresses": ["10.9.0.2/32"],
        "peer_key": "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=",
        "endpoint": "lab.example.org:51820",
    }));
    assert!(lab.get("ok").is_some(), "creating Home Lab: {lab}");
    nm.add_plugin_vpn("uuid-office", "Office OpenVPN");
    let id = work["ok"]["id"].as_str().expect("an id").to_owned();
    let up = wire.call(json!({"op": "vpn_up", "vpn": id}));
    assert!(up.get("ok").is_some(), "bringing Work up: {up}");
}

/// A world with Bluetooth (powered or not) and three paired phones that offer
/// network access: `Pixel 8` (a Profile is saved, nothing connected),
/// `Sam's iPhone` (paired only) and one with a name too long for a row. Shared
/// by the S2 tests and `demo tether` / `demo tether-off`.
pub fn tether_world(bin: impl Into<std::path::PathBuf>, powered: bool) -> World {
    let mut world = World::builder(bin).bluez(true).start();
    world.nm.add_phone("AA:BB:CC:DD:EE:01", "Pixel 8");
    world.nm.add_phone("AA:BB:CC:DD:EE:02", "Sam's iPhone");
    world
        .nm
        .add_phone("AA:BB:CC:DD:EE:03", "Galaxy S24 Ultra of the whole family");
    // Tether once and let go, so Pixel 8 has a saved Profile.
    let resp = world
        .wire()
        .call(json!({"op": "tether_connect", "phone": "AA:BB:CC:DD:EE:01"}));
    assert!(resp.get("ok").is_some(), "tethering Pixel 8: {resp}");
    let resp = world.wire().call(json!({"op": "tether_disconnect"}));
    assert!(resp.get("ok").is_some(), "dropping the tether: {resp}");
    if !powered {
        world.bluez().set_powered(false);
    }
    world
}

/// What the Hotspot tab starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotspotStart {
    /// A card that can host one; nothing runs yet.
    Capable,
    /// A card that can host one, with `Garage Wi-Fi` already up and two
    /// devices joined.
    Running,
    /// A card whose driver cannot be an access point.
    NoCard,
}

/// Puts `n` stations on the stand-in `iw` of a world made by [`hotspot_world`].
pub fn hotspot_stations(world: &World, n: usize) {
    let text: String = (0..n)
        .map(|i| format!("Station 02:00:00:00:00:{i:02x} (on wlan0)\n\tinactive time:\t120 ms\n"))
        .collect();
    std::fs::create_dir_all(world.home()).expect("home dir");
    std::fs::write(world.home().join("stations"), text).expect("write stations");
}

/// The Hotspot tab's worlds: the Tethering phones (so the tab has both halves),
/// a Wi-Fi card, and a stand-in `iw` that lists the stations
/// [`hotspot_stations`] wrote. Shared by the S2 tests and `demo hotspot*`.
pub fn hotspot_world(bin: impl Into<std::path::PathBuf>, start: HotspotStart) -> World {
    let world = tether_world(bin, true);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world.install_tool(
        "iw",
        "[ -f \"$HOME/stations\" ] || exit 1\ncat \"$HOME/stations\"",
    );
    match start {
        HotspotStart::Capable => {}
        HotspotStart::NoCard => world.nm.set_ap_support(wlan0, false),
        HotspotStart::Running => {
            hotspot_stations(&world, 2);
            let resp = world.wire().call(json!({
                "op": "hotspot_start",
                "ssid": "Garage Wi-Fi",
                "password": "open sesame 42",
            }));
            assert!(resp.get("ok").is_some(), "starting the Hotspot: {resp}");
        }
    }
    world
}
