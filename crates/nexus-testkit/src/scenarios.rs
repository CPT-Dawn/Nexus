//! Ready-made worlds, shared by the `demo` example and the S2 tests so the
//! frame you eyeball is the frame that is under test.

use std::thread::{self, JoinHandle};

use serde_json::json;

use crate::{Ap, DeviceId, World};

const WPA3: (u32, u32, u32) = (0x1, 0, 0x408); // privacy, SAE | CCMP

/// A busy Wi-Fi tab: six Networks in range (one joined, with a Profile), two
/// saved ones out of range. Returns the Device.
pub fn wifi_busy(world: &World) -> DeviceId {
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
    let wlan0 = wifi_busy(world);
    world
        .nm
        .add_ap(wlan0, Ap::enterprise("eduroam", "AA:00:00:00:00:07", 74));
    world
        .nm
        .set_eap_user("eduroam", "you@uni.example", "correct-horse");
    wlan0
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
