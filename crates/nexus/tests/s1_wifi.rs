//! Seam S1 for the Wi-Fi Network list and scan: a real `nexus daemon` against
//! a fake NetworkManager, driven over the socket and through `nexus wifi`.

use std::time::{Duration, Instant};

use nexus_testkit::{Ap, SCAN_RATE_LIMIT_MS, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

// NM80211ApSecurityFlags
const PSK: u32 = 0x100;
const EAP: u32 = 0x200;
const SAE: u32 = 0x400;
const OWE: u32 = 0x800;

fn wifi_list(world: &World) -> Value {
    let resp = world.wire().call(json!({"op": "wifi_list"}));
    assert!(resp.get("ok").is_some(), "wifi_list failed: {resp}");
    resp["ok"].clone()
}

fn ssids(list: &Value) -> Vec<&str> {
    list["networks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["ssid"].as_str().unwrap())
        .collect()
}

fn network<'a>(list: &'a Value, ssid: &str) -> &'a Value {
    list["networks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["ssid"] == ssid)
        .unwrap_or_else(|| panic!("no network {ssid} in {list}"))
}

#[test]
fn duplicate_bssids_fold_into_one_row_and_the_strongest_wins() {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:01", 40));
    world.nm.add_ap(
        wlan0,
        Ap::wpa2("Home", "AA:00:00:00:00:02", 82).frequency(5180),
    );
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:03", 55));
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Cafe", "BB:00:00:00:00:01", 60));

    let list = wifi_list(&world);
    assert_eq!(ssids(&list), ["Home", "Cafe"], "strongest first");
    let home = network(&list, "Home");
    assert_eq!(home["signal"], 82);
    assert_eq!(home["band"], "5");
    assert_eq!(home["access_points"], 3);
    assert_eq!(home["security"], "wpa2");
    assert_eq!(home["device"], list["devices"][0]["id"]);
    assert_eq!(list["devices"][0]["name"], "wlan0");
}

#[test]
fn security_and_band_are_classified() {
    let world = World::start(NEXUS);
    let d = world.nm.add_wifi_device("wlan0");
    let ap = |ssid: &str, n: u8| Ap::open(ssid, &format!("CC:00:00:00:00:{n:02x}"), 50 + n);
    world.nm.add_ap(d, ap("open", 1));
    world
        .nm
        .add_ap(d, ap("owe", 2).security(0, 0, OWE).frequency(5955));
    world.nm.add_ap(d, ap("wep", 3).security(0x1, 0, 0));
    world.nm.add_ap(d, ap("wpa", 4).security(0x1, PSK | 0x8, 0));
    world
        .nm
        .add_ap(d, ap("wpa2", 5).security(0x1, 0, PSK | 0x8));
    world
        .nm
        .add_ap(d, ap("wpa3", 6).security(0x1, 0, SAE | 0x8).frequency(5975));
    world
        .nm
        .add_ap(d, ap("transition", 7).security(0x1, 0, PSK | SAE | 0x8));
    world
        .nm
        .add_ap(d, ap("corp", 8).security(0x1, 0, EAP | 0x8).frequency(5500));

    let list = wifi_list(&world);
    let sec = |s: &str| network(&list, s)["security"].clone();
    assert_eq!(sec("open"), "open");
    assert_eq!(sec("owe"), "owe");
    assert_eq!(sec("wep"), "wep");
    assert_eq!(sec("wpa"), "wpa");
    assert_eq!(sec("wpa2"), "wpa2");
    assert_eq!(sec("wpa3"), "wpa3");
    assert_eq!(sec("transition"), "wpa3_transition");
    assert_eq!(sec("corp"), "enterprise");
    assert_eq!(network(&list, "open")["band"], "2.4");
    assert_eq!(network(&list, "owe")["band"], "6");
    assert_eq!(network(&list, "wpa3")["band"], "6");
    assert_eq!(network(&list, "corp")["band"], "5");
}

#[test]
fn ssids_are_keyed_by_bytes_and_survive_invalid_utf8() {
    let world = World::start(NEXUS);
    let d = world.nm.add_wifi_device("wlan0");
    let odd = [0x66, 0xff, 0xfe, 0x6f];
    world
        .nm
        .add_ap(d, Ap::wpa2("x", "DD:00:00:00:00:01", 70).ssid_bytes(&odd));
    world
        .nm
        .add_ap(d, Ap::wpa2("x", "DD:00:00:00:00:02", 30).ssid_bytes(&odd));
    // Hidden network: empty SSID, never listed.
    world
        .nm
        .add_ap(d, Ap::wpa2("x", "DD:00:00:00:00:03", 90).ssid_bytes(&[]));

    let list = wifi_list(&world);
    let nets = list["networks"].as_array().unwrap();
    assert_eq!(nets.len(), 1, "{list}");
    assert_eq!(nets[0]["id"], "66fffe6f");
    assert_eq!(nets[0]["ssid"], "f\u{fffd}\u{fffd}o");
    assert_eq!(nets[0]["access_points"], 2);
}

#[test]
fn saved_profiles_split_into_in_range_and_out_of_range() {
    let world = World::start(NEXUS);
    let d = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(d, Ap::wpa2("Home", "EE:00:00:00:00:01", 70));
    world
        .nm
        .add_ap(d, Ap::wpa2("Cafe", "EE:00:00:00:00:02", 50));
    world
        .nm
        .add_wifi_profile("uuid-home", "Home", b"Home", true);
    world
        .nm
        .add_wifi_profile("uuid-office", "Office", b"Office", false);

    let list = wifi_list(&world);
    assert_eq!(network(&list, "Home")["profile"], "uuid-home");
    assert!(network(&list, "Cafe").get("profile").is_none());
    assert_eq!(
        list["saved"],
        json!([{"uuid": "uuid-office", "id": "Office", "ssid": "Office", "autoconnect": false}])
    );
}

#[test]
fn multiple_devices_are_listed_and_networks_fold_across_them() {
    let world = World::start(NEXUS);
    let a = world.nm.add_wifi_device("wlan0");
    let b = world.nm.add_wifi_device("wlan1");
    world
        .nm
        .add_ap(a, Ap::wpa2("Home", "FF:00:00:00:00:01", 30));
    world
        .nm
        .add_ap(b, Ap::wpa2("Home", "FF:00:00:00:00:02", 75));

    let list = wifi_list(&world);
    assert_eq!(list["devices"].as_array().unwrap().len(), 2);
    let home = network(&list, "Home");
    assert_eq!(home["signal"], 75);
    assert_eq!(home["device"], list["devices"][1]["id"]);
}

#[test]
fn cli_lists_networks_as_a_table_and_as_json() {
    let world = World::start(NEXUS);
    let d = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(d, Ap::wpa2("Home", "AB:00:00:00:00:01", 82).frequency(5180));
    world.nm.add_wifi_profile("u-1", "Office", b"Office", true);

    let out = world.run_nexus(&["wifi", "list", "--json"]);
    assert!(out.status.success(), "{out:?}");
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(network(&json, "Home")["signal"], 82);

    let out = world.run_nexus(&["wifi", "list"]);
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    let home = text.lines().find(|l| l.contains("Home")).expect(&text);
    assert!(
        home.contains("82") && home.contains("wpa2") && home.contains("5 GHz"),
        "{text}"
    );
    assert!(
        text.contains("Saved, out of range") && text.contains("Office"),
        "{text}"
    );
}

#[test]
fn listing_without_networkmanager_is_a_clear_error() {
    let world = World::builder(NEXUS).nm_running(false).start();
    let resp = world.wire().call(json!({"op": "wifi_list"}));
    assert_eq!(resp["err"]["code"], "backend_unavailable", "{resp}");
    let out = world.run_nexus(&["wifi", "list"]);
    assert_eq!(out.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&out.stderr).contains("NetworkManager"));
}

#[test]
fn a_scan_inside_the_rate_limit_reuses_the_last_scan() {
    let world = World::start(NEXUS);
    let d = world.nm.add_wifi_device("wlan0");
    world.nm.set_last_scan_ago(d, 2_000);

    let resp = world.wire().call(json!({"op": "wifi_scan"}));
    assert_eq!(resp["ok"]["requested"], 0, "{resp}");
    assert_eq!(resp["ok"]["skipped"], 1);
    let ago = resp["ok"]["last_scan_ms_ago"].as_u64().unwrap();
    assert!((2_000..SCAN_RATE_LIMIT_MS as u64).contains(&ago), "{resp}");
    assert_eq!(
        world.nm.scan_requests(),
        0,
        "NetworkManager was never bothered"
    );

    let out = world.run_nexus(&["wifi", "scan"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(world.nm.scan_requests(), 0);
}

#[test]
fn an_old_scan_is_refreshed() {
    let world = World::start(NEXUS);
    let d = world.nm.add_wifi_device("wlan0");
    world.nm.set_last_scan_ago(d, 60_000);

    let resp = world.wire().call(json!({"op": "wifi_scan"}));
    assert_eq!(resp["ok"]["requested"], 1, "{resp}");
    assert_eq!(world.nm.scan_requests(), 1);
}

#[test]
fn a_never_scanned_device_is_scanned() {
    let world = World::start(NEXUS);
    world.nm.add_wifi_device("wlan0");
    let resp = world.wire().call(json!({"op": "wifi_scan"}));
    assert_eq!(resp["ok"]["requested"], 1, "{resp}");
    assert!(
        resp["ok"].get("last_scan_ms_ago").is_none() || resp["ok"]["last_scan_ms_ago"].is_u64()
    );
}

#[test]
fn a_refused_scan_is_not_an_error() {
    let world = World::start(NEXUS);
    let d = world.nm.add_wifi_device("wlan0");
    world.nm.set_last_scan_ago(d, 60_000);
    world.nm.refuse_scans(true); // e.g. polkit, or the device is busy

    let resp = world.wire().call(json!({"op": "wifi_scan"}));
    assert_eq!(resp["ok"]["requested"], 0, "{resp}");
    assert_eq!(resp["ok"]["skipped"], 1);
    assert!(world.run_nexus(&["wifi", "scan"]).status.success());
}

fn subscribe(world: &World) -> nexus_testkit::Wire {
    let mut wire = world.wire();
    let resp = wire.call(json!({"op": "subscribe"}));
    assert!(resp.get("ok").is_some(), "{resp}");
    wire
}

fn next_event(wire: &mut nexus_testkit::Wire) -> Value {
    wire.try_read_line(Duration::from_secs(5))
        .expect("an event within 5s")
}

#[test]
fn subscribers_hear_when_a_network_appears_and_disappears() {
    let world = World::start(NEXUS);
    let d = world.nm.add_wifi_device("wlan0");
    let mut wire = subscribe(&world);

    let ap = world
        .nm
        .add_ap(d, Ap::wpa2("Home", "AA:00:00:00:00:01", 70));
    let ev = next_event(&mut wire);
    assert_eq!(ev["v"], 1);
    assert_eq!(ev["event"], "networks_changed");
    assert_eq!(ssids(&ev["wifi"]), ["Home"]);

    world.nm.remove_ap(d, ap);
    let ev = next_event(&mut wire);
    assert_eq!(ev["event"], "networks_changed");
    assert!(ssids(&ev["wifi"]).is_empty());
}

#[test]
fn a_signal_storm_is_coalesced_into_few_updates_and_ends_correct() {
    let world = World::start(NEXUS);
    let d = world.nm.add_wifi_device("wlan0");
    let ap = world
        .nm
        .add_ap(d, Ap::wpa2("Home", "AA:00:00:00:00:01", 10));
    let mut wire = subscribe(&world);

    const STORM: u8 = 90;
    for strength in 11..11 + STORM {
        world.nm.set_strength(ap, strength);
    }
    let last = 10 + STORM;

    let mut events = 0;
    let mut seen = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while seen != last && Instant::now() < deadline {
        let Some(ev) = wire.try_read_line(Duration::from_millis(500)) else {
            continue;
        };
        events += 1;
        seen = ev["wifi"]["networks"][0]["signal"].as_u64().unwrap() as u8;
    }
    assert_eq!(
        seen, last,
        "the final signal is what subscribers end up with"
    );
    assert!(events <= 10, "{STORM} changes became {events} events");
    // Quiet again afterwards.
    assert!(wire.try_read_line(Duration::from_millis(500)).is_none());
}

#[test]
fn a_change_that_alters_nothing_visible_sends_no_event() {
    let world = World::start(NEXUS);
    let d = world.nm.add_wifi_device("wlan0");
    let mut wire = subscribe(&world);
    let ap = world
        .nm
        .add_ap(d, Ap::wpa2("Home", "AA:00:00:00:00:01", 50));
    next_event(&mut wire); // the new network itself
    world.nm.set_strength(ap, 50);
    assert!(wire.try_read_line(Duration::from_millis(700)).is_none());
}
