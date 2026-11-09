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
