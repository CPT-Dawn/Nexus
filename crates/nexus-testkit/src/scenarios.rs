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

/// An `.ovpn` the plugin scenario can import (documentation host).
pub const OVPN_CONF: &str =
    "client\ndev tun\nproto udp\nremote vpn.branch.example 1194\nauth-user-pass\n";

/// Plugin VPNs: the OpenVPN plugin is installed, `Office OpenVPN` asks for a
/// user name, a password and a one-time code when it starts, and `Legacy L2TP`
/// belongs to a plugin that is not installed. A stand-in `nmcli` imports
/// `.ovpn` files; one waits at the returned path.
pub fn vpn_plugins(world: &World) -> std::path::PathBuf {
    use crate::{ActiveConnection, NmConnectivity, NmState};
    let nm = &world.nm;
    nm.set_primary(Some(ActiveConnection::wifi("Home-5G")));
    nm.set_connectivity(NmConnectivity::Full);
    nm.set_state(NmState::ConnectedGlobal);
    world.install_vpn_plugin("openvpn");
    world.install_fake_nmcli();
    nm.add_plugin_vpn_of("uuid-office", "Office OpenVPN", Some("openvpn"));
    nm.script_vpn_asks("uuid-office", &["username", "password", "otp"]);
    nm.add_plugin_vpn_of("uuid-l2tp", "Legacy L2TP", Some("l2tp"));
    let file = world.dir().join("Branch.ovpn");
    std::fs::write(&file, OVPN_CONF).expect("write the sample .ovpn");
    file
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

/// Two Ethernet ports. `enp3s0` has its cable in and a saved Profile
/// ("Office wall socket"), but the port wants 802.1X, so the network gave it
/// no address: it reads as plugged in and misconfigured. The login that gets
/// past it is `alice@office.example` / `hunter2`, server name
/// `radius.office.example`. `enp4s0` has no cable. Shared by the S2 tests and
/// `demo ethernet`. Returns the two Devices.
pub fn ethernet_busy(world: &World) -> (DeviceId, DeviceId) {
    let nm = &world.nm;
    let enp3s0 = nm.add_ethernet_device("enp3s0");
    let enp4s0 = nm.add_ethernet_device("enp4s0");
    nm.set_port_eap(enp3s0, "alice@office.example", "hunter2");
    nm.set_lease(
        enp3s0,
        crate::Lease {
            address: "10.20.30.41".into(),
            prefix: 24,
            gateway: "10.20.30.1".into(),
            dns: vec!["10.20.30.2".into(), "10.20.30.3".into()],
        },
    );
    nm.add_ethernet_profile(
        "55555555-5555-4555-8555-555555555555",
        "Office wall socket",
        "enp3s0",
    );
    nm.set_carrier(enp4s0, false);
    // The first try fails, as it would on the real thing: no sign-in, no address.
    let tried = world
        .wire()
        .call(json!({"op": "ethernet_connect", "port": "enp3s0"}));
    assert!(
        tried.get("err").is_some(),
        "joining without a sign-in: {tried}"
    );
    (enp3s0, enp4s0)
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

/// The awkward Wi-Fi cases, nothing joined: two cards (`wlan0`, `wlan1`), a
/// mesh (`Mesh-Home`: 2.4, 5 and 6 GHz access points, heard by both cards),
/// an OWE and a WPA2/WPA3 transition Network, a name that is not valid UTF-8
/// (with an escape sequence inside), and `Ghost-Net`, a hidden WPA2 Network
/// that is not listed and takes the password `boo-boo-boo`.
pub fn wifi_edge(world: &World) -> (DeviceId, DeviceId) {
    let nm = &world.nm;
    let wlan0 = nm.add_wifi_device("wlan0");
    let wlan1 = nm.add_wifi_device("wlan1");
    nm.add_ap(
        wlan0,
        Ap::wpa2("Mesh-Home", "AA:00:00:00:10:01", 48)
            .frequency(2437)
            .bitrate(144_000),
    );
    nm.add_ap(
        wlan0,
        Ap::wpa2("Mesh-Home", "AA:00:00:00:10:02", 82)
            .frequency(5180)
            .bitrate(866_000),
    );
    nm.add_ap(
        wlan0,
        Ap::wpa2("Mesh-Home", "AA:00:00:00:10:03", 31)
            .frequency(6135)
            .bitrate(2_402_000),
    );
    nm.add_ap(
        wlan1,
        Ap::wpa2("Mesh-Home", "AA:00:00:00:10:02", 60)
            .frequency(5180)
            .bitrate(866_000),
    );
    nm.add_ap(wlan0, Ap::owe("Airport Free", "AA:00:00:00:20:01", 64));
    nm.add_ap(
        wlan1,
        Ap::transition("Mixed-Mode", "AA:00:00:00:30:01", 70).frequency(5240),
    );
    nm.add_ap(
        wlan0,
        Ap::open("x", "AA:00:00:00:40:01", 40).ssid_bytes(b"caf\xe9 \x1b[2Jwifi"),
    );
    nm.add_ap(
        wlan0,
        Ap::wpa2("Ghost-Net", "AA:00:00:00:50:01", 57).hidden(),
    );
    nm.set_psk("Ghost-Net", "boo-boo-boo");
    (wlan0, wlan1)
}
