//! Seam S1 for Bluetooth Tethering: a real `nexus daemon` against a fake
//! NetworkManager (phones are its Bluetooth Devices) and a fake BlueZ (the
//! adapter's power), over the socket and the CLI. No real Bluetooth is touched.

use nexus_testkit::{DeviceId, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

const PIXEL: &str = "AA:BB:CC:DD:EE:01";
const IPHONE: &str = "AA:BB:CC:DD:EE:02";

// NMDeviceState
const IP_CONFIG: u32 = 70;
// NMDeviceStateReason
const IP_CONFIG_UNAVAILABLE: u32 = 5;

/// Bluetooth on, one paired phone.
fn world() -> (World, DeviceId) {
    let world = World::builder(NEXUS).bluez(true).start();
    let pixel = world.nm.add_phone(PIXEL, "Pixel 8");
    (world, pixel)
}

fn tethers(world: &World) -> Value {
    world.wire().call(json!({"op": "tethers"}))["ok"].clone()
}

fn connect(world: &World, phone: Option<&str>) -> Value {
    let mut op = json!({"op": "tether_connect"});
    if let Some(p) = phone {
        op["phone"] = json!(p);
    }
    world.wire().call(op)
}

#[test]
fn lists_paired_phones_that_offer_network_access() {
    let (world, _) = world();
    // Dial-up only: NetworkManager has a Device, but there is no network to share.
    world.nm.add_dialup_only_phone(IPHONE, "Old Nokia");
    let t = tethers(&world);
    assert_eq!(t["bluetooth"], "on", "{t}");
    let phones = t["phones"].as_array().unwrap();
    assert_eq!(phones.len(), 1, "{t}");
    assert_eq!(phones[0]["id"], PIXEL);
    assert_eq!(phones[0]["name"], "Pixel 8");
    assert_eq!(phones[0]["connected"], false);
    assert_eq!(phones[0]["saved"], false);
}

#[test]
fn wifi_devices_are_not_phones() {
    let (world, _) = world();
    world.nm.add_wifi_device("wlan0");
    assert_eq!(tethers(&world)["phones"].as_array().unwrap().len(), 1);
}

#[test]
fn connecting_brings_the_link_up_and_saves_a_profile_afterwards() {
    let (world, pixel) = world();
    let resp = connect(&world, None);
    assert_eq!(resp["ok"]["phone"], "Pixel 8", "{resp}");
    assert_eq!(resp["ok"]["id"], PIXEL);
    assert!(world.nm.tethered(pixel));
    assert_eq!(world.nm.phone_profiles_persisted(PIXEL), 1);
    let t = tethers(&world);
    assert_eq!(t["phones"][0]["connected"], true, "{t}");
    assert_eq!(t["phones"][0]["saved"], true);
}

#[test]
fn connecting_again_reuses_the_saved_profile() {
    let (world, pixel) = world();
    assert!(connect(&world, None).get("ok").is_some());
    let down = world.wire().call(json!({"op": "tether_disconnect"}));
    assert_eq!(down["ok"]["phone"], "Pixel 8", "{down}");
    assert!(!world.nm.tethered(pixel));
    // Disconnecting keeps the Profile.
    assert_eq!(world.nm.phone_profiles(PIXEL), 1);
    assert!(connect(&world, None).get("ok").is_some());
    assert!(world.nm.tethered(pixel));
    assert_eq!(world.nm.phone_profiles(PIXEL), 1, "no second Profile");
}

#[test]
fn a_phone_can_be_named_by_address_or_by_name_in_any_case() {
    let (world, _) = world();
    world.nm.add_phone(IPHONE, "Sam's iPhone");
    for who in ["aa:bb:cc:dd:ee:02", "SAM'S IPHONE"] {
        let resp = connect(&world, Some(who));
        assert_eq!(resp["ok"]["id"], IPHONE, "{who}: {resp}");
    }
}

#[test]
fn several_phones_and_no_name_asks_which() {
    let (world, _) = world();
    world.nm.add_phone(IPHONE, "Sam's iPhone");
    let resp = connect(&world, None);
    assert_eq!(resp["err"]["code"], "unknown_phone", "{resp}");
    let message = resp["err"]["message"].as_str().unwrap();
    assert!(
        message.contains("Pixel 8") && message.contains("Sam's iPhone"),
        "{message}"
    );
}

#[test]
fn an_unknown_phone_name_is_refused_in_words() {
    let (world, _) = world();
    let resp = connect(&world, Some("Nokia"));
    assert_eq!(resp["err"]["code"], "unknown_phone", "{resp}");
    assert!(resp["err"]["message"].as_str().unwrap().contains("Nokia"));
}

#[test]
fn bluetooth_off_is_said_plainly_and_nothing_is_tried() {
    let (mut world, pixel) = world();
    world.bluez().set_powered(false);
    let t = tethers(&world);
    assert_eq!(t["bluetooth"], "off", "{t}");
    let resp = connect(&world, None);
    assert_eq!(resp["err"]["code"], "bluetooth_off", "{resp}");
    let message = resp["err"]["message"].as_str().unwrap();
    assert!(message.contains("Bluetooth is off"), "{message}");
    assert!(!world.nm.tethered(pixel));
    assert_eq!(world.nm.phone_profiles(PIXEL), 0);
}

#[test]
fn no_bluetooth_service_reads_as_absent() {
    // No fake BlueZ: nobody owns org.bluez, and asking must not start it.
    let world = World::start(NEXUS);
    world.nm.add_phone(PIXEL, "Pixel 8");
    let t = tethers(&world);
    assert_eq!(t["bluetooth"], "absent", "{t}");
    let resp = connect(&world, None);
    assert_eq!(resp["err"]["code"], "bluetooth_off", "{resp}");
    assert!(
        resp["err"]["message"]
            .as_str()
            .unwrap()
            .contains("no Bluetooth"),
        "{resp}"
    );
}

#[test]
fn no_paired_phone_says_how_to_pair() {
    let world = World::builder(NEXUS).bluez(true).start();
    let resp = connect(&world, None);
    assert_eq!(resp["err"]["code"], "no_phone", "{resp}");
    assert!(
        resp["err"]["message"].as_str().unwrap().contains("Pair"),
        "{resp}"
    );
}

#[test]
fn a_failed_link_leaves_no_profile_and_says_why() {
    let (world, pixel) = world();
    world
        .nm
        .script_tether_failure(PIXEL, IP_CONFIG, IP_CONFIG_UNAVAILABLE);
    let resp = connect(&world, None);
    assert_eq!(resp["err"]["code"], "join_failed", "{resp}");
    let message = resp["err"]["message"].as_str().unwrap();
    assert!(message.contains("Pixel 8"), "{message}");
    assert!(message.contains("tethering"), "{message}");
    assert!(!world.nm.tethered(pixel));
    assert_eq!(world.nm.phone_profiles(PIXEL), 0, "no lingering Profile");
}

#[test]
fn disconnect_without_a_link_is_not_connected() {
    let (world, _) = world();
    let resp = world.wire().call(json!({"op": "tether_disconnect"}));
    assert_eq!(resp["err"]["code"], "not_connected", "{resp}");
}

#[test]
fn subscribers_hear_about_bluetooth_and_links_changing() {
    let (mut world, _) = world();
    let mut ev = world.wire();
    ev.subscribe();
    // Drain the first push the world's own setup may have caused.
    world.bluez().set_powered(false);
    let e = ev.next_event("tethers_changed");
    assert_eq!(e["tethers"]["bluetooth"], "off", "{e}");
    world.bluez().set_powered(true);
    let e = ev.next_event("tethers_changed");
    assert_eq!(e["tethers"]["bluetooth"], "on", "{e}");
    assert!(connect(&world, None).get("ok").is_some());
    let mut up = false;
    for _ in 0..8 {
        let e = ev.next_event("tethers_changed");
        if e["tethers"]["phones"][0]["connected"] == true {
            up = true;
            break;
        }
    }
    assert!(up, "never heard the Link come up");
}

#[test]
fn cli_lists_connects_and_disconnects_in_words() {
    let (world, _) = world();
    let out = world.run_nexus(&["tether", "list"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Pixel 8") && text.contains(PIXEL), "{text}");

    let out = world.run_nexus(&["tether", "connect"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("Tethered through Pixel 8"));

    let out = world.run_nexus(&["tether", "list", "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["phones"][0]["connected"], true, "{v}");

    let out = world.run_nexus(&["tether", "disconnect"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("Dropped the tether to Pixel 8"));
}

#[test]
fn cli_exit_codes_tell_bluetooth_off_from_no_phone() {
    let (mut world, _) = world();
    world.bluez().set_powered(false);
    let out = world.run_nexus(&["tether", "connect"]);
    assert_eq!(out.status.code(), Some(14), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("Bluetooth is off"));
    world.bluez().set_powered(true);

    let empty = World::builder(NEXUS).bluez(true).start();
    let out = empty.run_nexus(&["tether", "connect"]);
    assert_eq!(out.status.code(), Some(15), "{out:?}");

    let out = empty.run_nexus(&["tether", "disconnect"]);
    assert_eq!(out.status.code(), Some(11), "{out:?}");
}
