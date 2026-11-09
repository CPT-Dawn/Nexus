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

/// The Settings tab's world: a config with comments, a custom theme
/// (`sunrise`, from Gruvbox) and a mistaken one (`broken`). With `bad_theme`
/// the config selects `broken`, so the TUI starts on the fallback with its
/// warning. Press `6` in the demo.
pub fn settings_world(bin: impl Into<std::path::PathBuf>, bad_theme: bool) -> World {
    let mut world = World::builder(bin).daemon(false).start();
    let theme = if bad_theme {
        "broken"
    } else {
        "midnight-bloom"
    };
    world.write_config(&format!(
        "# Nexus settings, edited by hand and by the Settings tab.\n\
         theme = \"{theme}\"  # try `nexus config set theme nord`\n\
         \n\
         [notifications]\n\
         lost = false  # too chatty on the train\n\
         \n\
         [themes.sunrise]\n\
         base = \"gruvbox\"\n\
         accent = \"#ff7a59\"\n\
         \n\
         [themes.broken]\n\
         accent = \"teal\"\n"
    ));
    world.start_daemon();
    world
}
