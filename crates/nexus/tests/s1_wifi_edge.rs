//! Seam S1 for the awkward Wi-Fi cases: hidden Networks, SSIDs that are not
//! UTF-8, several Wi-Fi cards, OWE and WPA2/WPA3 transition mode, and access
//! point details. A real Daemon against a fake NetworkManager.

use nexus_testkit::{Ap, DeviceId, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

const KEY_MGMT: &str = "802-11-wireless-security.key-mgmt";

fn id(ssid: &[u8]) -> String {
    nexus_ipc::network_id(ssid)
}

fn join(world: &World, ssid: &[u8], extra: Value) -> Value {
    let mut op = json!({"op": "wifi_join", "network": id(ssid)});
    for (k, v) in extra.as_object().unwrap() {
        op[k] = v.clone();
    }
    world.wire().call(op)
}

fn list(world: &World) -> Value {
    world.wire().call(json!({"op": "wifi_list"}))["ok"].clone()
}

fn names(list: &Value) -> Vec<String> {
    list["networks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["ssid"].as_str().unwrap().to_owned())
        .collect()
}

fn two_cards() -> (World, DeviceId, DeviceId) {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    let wlan1 = world.nm.add_wifi_device("wlan1");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:01", 80));
    world
        .nm
        .add_ap(wlan1, Ap::wpa2("Home", "AA:00:00:00:00:01", 55));
    world
        .nm
        .add_ap(wlan0, Ap::open("Only0", "AA:00:00:00:00:02", 70));
    (world, wlan0, wlan1)
}

// -- hidden Networks --------------------------------------------------------

#[test]
fn a_hidden_network_is_not_listed_but_joins_by_name_and_security() {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Ghost", "AA:00:00:00:00:09", 66).hidden());
    world.nm.set_psk("Ghost", "boo-boo-boo");

    assert!(
        names(&list(&world)).is_empty(),
        "a hidden network cannot be listed"
    );
    let resp = join(
        &world,
        b"Ghost",
        json!({"hidden": "wpa2", "password": "boo-boo-boo"}),
    );
    assert_eq!(resp["ok"]["ssid"], "Ghost", "{resp}");
    assert_eq!(world.nm.joined_ssid(wlan0).as_deref(), Some("Ghost"));
    assert_eq!(world.nm.persisted_profiles(), 1);
    assert_eq!(
        world.nm.added_setting(b"Ghost", "802-11-wireless.hidden"),
        Some("true".into()),
        "NetworkManager must probe for it"
    );
    assert_eq!(
        world.nm.added_setting(b"Ghost", KEY_MGMT).as_deref(),
        Some("wpa-psk")
    );
    // Once joined its name is known, so it lists like any other.
    assert_eq!(names(&list(&world)), ["Ghost"]);
}

#[test]
fn a_hidden_network_nobody_answers_for_is_out_of_range_and_leaves_nothing() {
    let world = World::start(NEXUS);
    world.nm.add_wifi_device("wlan0");
    let resp = join(&world, b"Nowhere", json!({"hidden": "open"}));
    assert_eq!(resp["err"]["code"], "out_of_range", "{resp}");
    assert_eq!(world.nm.profile_count(), 0);
}

#[test]
fn a_hidden_join_needs_a_security_nexus_can_make_a_profile_for() {
    let world = World::start(NEXUS);
    world.nm.add_wifi_device("wlan0");
    for bad in ["enterprise", "wep", "unknown"] {
        let resp = join(&world, b"Ghost", json!({"hidden": bad}));
        assert_eq!(resp["err"]["code"], "invalid_settings", "{bad}: {resp}");
    }
    assert_eq!(world.nm.profile_count(), 0);
}

#[test]
fn an_open_hidden_network_joins_without_a_password() {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world.nm.add_ap(
        wlan0,
        Ap::open("QuietCafe", "AA:00:00:00:00:0a", 50).hidden(),
    );
    let resp = join(&world, b"QuietCafe", json!({"hidden": "open"}));
    assert!(resp.get("ok").is_some(), "{resp}");
    assert_eq!(world.nm.added_setting(b"QuietCafe", KEY_MGMT), None);
}

#[test]
fn the_cli_joins_a_hidden_network() {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world.nm.add_ap(
        wlan0,
        Ap::open("QuietCafe", "AA:00:00:00:00:0a", 50).hidden(),
    );
    let out = world.run_nexus(&["wifi", "join", "QuietCafe", "--hidden", "open"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Joined QuietCafe.\n");
}

// -- odd SSIDs --------------------------------------------------------------

#[test]
fn a_non_utf8_ssid_displays_safely_and_stays_joinable() {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    let ssid: &[u8] = b"caf\xe9 \x1b[2Jbar";
    world.nm.add_ap(
        wlan0,
        Ap::open("x", "AA:00:00:00:00:0b", 60).ssid_bytes(ssid),
    );
    let l = list(&world);
    let row = &l["networks"][0];
    assert_eq!(row["id"], id(ssid));
    let shown = row["ssid"].as_str().unwrap();
    assert!(!shown.chars().any(char::is_control), "{shown:?}");
    assert!(shown.contains('\u{fffd}'), "{shown:?}");

    let resp = join(&world, ssid, json!({}));
    assert!(resp.get("ok").is_some(), "{resp}");
    let joined = resp["ok"]["ssid"].as_str().unwrap();
    assert!(!joined.chars().any(char::is_control), "{joined:?}");
    assert_eq!(world.nm.persisted_profiles(), 1);
    // The saved Profile matches the row again, by bytes.
    let row = list(&world)["networks"][0].clone();
    assert_eq!(row["joined"], true);
}

#[test]
fn the_cli_joins_an_ssid_given_as_hex() {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    let ssid: &[u8] = b"\xff\xfeodd";
    world.nm.add_ap(
        wlan0,
        Ap::open("x", "AA:00:00:00:00:0b", 60).ssid_bytes(ssid),
    );
    let out = world.run_nexus(&["wifi", "join", &id(ssid), "--hex"]);
    assert!(out.status.success(), "{out:?}");
    let printed = String::from_utf8_lossy(&out.stdout);
    assert!(!printed.chars().any(|c| c.is_control() && c != '\n'));
    let out = world.run_nexus(&["wifi", "join", "zz", "--hex"]);
    assert!(!out.status.success());
}

// -- several cards ----------------------------------------------------------

#[test]
fn a_network_joins_through_the_chosen_card() {
    let (world, wlan0, wlan1) = two_cards();
    world.nm.set_psk("Home", "hunter2hunter2");
    let resp = join(
        &world,
        b"Home",
        json!({"device": "wlan1", "password": "hunter2hunter2"}),
    );
    assert!(resp.get("ok").is_some(), "{resp}");
    assert_eq!(world.nm.joined_ssid(wlan1).as_deref(), Some("Home"));
    assert_eq!(world.nm.joined_ssid(wlan0), None);
}

#[test]
fn without_a_choice_the_card_that_hears_it_best_joins() {
    let (world, wlan0, wlan1) = two_cards();
    let resp = join(&world, b"Only0", json!({}));
    assert!(resp.get("ok").is_some(), "{resp}");
    assert_eq!(world.nm.joined_ssid(wlan0).as_deref(), Some("Only0"));
    assert_eq!(world.nm.joined_ssid(wlan1), None);
}

#[test]
fn a_card_that_cannot_hear_the_network_says_so_and_an_unknown_card_is_refused() {
    let (world, _, wlan1) = two_cards();
    let resp = join(&world, b"Only0", json!({"device": "wlan1"}));
    assert_eq!(resp["err"]["code"], "out_of_range", "{resp}");
    assert!(
        resp["err"]["message"].as_str().unwrap().contains("wlan1"),
        "{resp}"
    );
    assert_eq!(world.nm.joined_ssid(wlan1), None);
    let resp = join(&world, b"Home", json!({"device": "wlan9"}));
    assert_eq!(resp["err"]["code"], "unknown_device", "{resp}");
    assert_eq!(world.nm.profile_count(), 0);
}

#[test]
fn a_card_can_also_be_chosen_by_its_id() {
    let (world, wlan0, wlan1) = two_cards();
    let devices = list(&world)["devices"].clone();
    let wlan1_id = devices
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "wlan1")
        .unwrap()["id"]
        .clone();
    world.nm.set_psk("Home", "hunter2hunter2");
    let resp = join(
        &world,
        b"Home",
        json!({"device": wlan1_id, "password": "hunter2hunter2"}),
    );
    assert!(resp.get("ok").is_some(), "{resp}");
    assert!(world.nm.joined_ssid(wlan1).is_some() && world.nm.joined_ssid(wlan0).is_none());
}

#[test]
fn the_cli_chooses_the_card_and_an_unknown_one_has_its_own_exit_code() {
    let (world, _, wlan1) = two_cards();
    let out = world.run_nexus(&["wifi", "join", "Only0", "--device", "wlan9"]);
    assert_eq!(out.status.code(), Some(40), "{out:?}");
    let out = world.run_nexus(&[
        "wifi",
        "join",
        "Home",
        "--device",
        "wlan1",
        "--password",
        "long-enough-pw",
    ]);
    // No password is set on the fake: the join works on the chosen card.
    assert!(out.status.success(), "{out:?}");
    assert_eq!(world.nm.joined_ssid(wlan1).as_deref(), Some("Home"));
}

#[test]
fn the_cli_list_names_the_card_when_there_are_several() {
    let (world, ..) = two_cards();
    let out = world.run_nexus(&["wifi", "list"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("DEVICE"), "{text}");
    assert!(text.contains("wlan0"), "{text}");
}

// -- OWE and transition mode ------------------------------------------------

#[test]
fn an_owe_network_is_listed_as_owe_and_joins_without_a_password() {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::owe("Airport Free", "AA:00:00:00:00:0c", 70));
    let l = list(&world);
    assert_eq!(l["networks"][0]["security"], "owe");
    let resp = join(&world, b"Airport Free", json!({}));
    assert!(resp.get("ok").is_some(), "{resp}");
    assert_eq!(
        world.nm.added_setting(b"Airport Free", KEY_MGMT).as_deref(),
        Some("owe")
    );
}

#[test]
fn a_transition_network_is_wpa2_3_and_joins_with_its_passphrase() {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::transition("Mixed", "AA:00:00:00:00:0d", 70));
    world.nm.set_psk("Mixed", "correct-horse-battery");
    assert_eq!(list(&world)["networks"][0]["security"], "wpa3_transition");
    let resp = join(
        &world,
        b"Mixed",
        json!({"password": "correct-horse-battery"}),
    );
    assert!(resp.get("ok").is_some(), "{resp}");
    // WPA2 clients and WPA3 clients both read a passphrase profile.
    assert_eq!(
        world.nm.added_setting(b"Mixed", KEY_MGMT).as_deref(),
        Some("wpa-psk")
    );
}

// -- access point details ---------------------------------------------------

#[test]
fn access_point_details_give_address_channel_frequency_band_and_rate() {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world.nm.add_ap(
        wlan0,
        Ap::wpa2("Mesh", "AA:00:00:00:00:21", 40)
            .frequency(2437)
            .bitrate(144_000),
    );
    world.nm.add_ap(
        wlan0,
        Ap::wpa2("Mesh", "AA:00:00:00:00:22", 85)
            .frequency(5180)
            .bitrate(866_000),
    );
    world.nm.add_ap(
        wlan0,
        Ap::wpa2("Mesh", "AA:00:00:00:00:23", 20)
            .frequency(6135)
            .bitrate(2_402_000),
    );
    let resp = world
        .wire()
        .call(json!({"op": "wifi_aps", "network": id(b"Mesh")}));
    let aps = &resp["ok"]["aps"];
    assert_eq!(resp["ok"]["ssid"], "Mesh");
    assert_eq!(aps.as_array().unwrap().len(), 3, "{resp}");
    // Strongest first.
    assert_eq!(aps[0]["bssid"], "AA:00:00:00:00:22");
    assert_eq!(aps[0]["channel"], 36);
    assert_eq!(aps[0]["frequency_mhz"], 5180);
    assert_eq!(aps[0]["band"], "5");
    assert_eq!(aps[0]["rate_mbps"], 866);
    assert_eq!(aps[0]["signal"], 85);
    assert_eq!(aps[0]["device"], "wlan0");
    assert_eq!(aps[1]["channel"], 6);
    assert_eq!(aps[1]["band"], "2.4");
    assert_eq!(aps[2]["band"], "6", "6 GHz is reported as such");
    assert_eq!(aps[2]["channel"], 37);
    assert_eq!(aps[2]["rate_mbps"], 2402);
}

#[test]
fn access_point_details_mark_the_joined_one_and_refuse_an_unheard_network() {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::open("Cafe", "AA:00:00:00:00:31", 60));
    let ok = join(&world, b"Cafe", json!({}));
    assert!(ok.get("ok").is_some(), "{ok}");
    let resp = world
        .wire()
        .call(json!({"op": "wifi_aps", "network": id(b"Cafe")}));
    assert_eq!(resp["ok"]["aps"][0]["joined"], true, "{resp}");
    let resp = world
        .wire()
        .call(json!({"op": "wifi_aps", "network": id(b"Nobody")}));
    assert_eq!(resp["err"]["code"], "out_of_range", "{resp}");
    let resp = world
        .wire()
        .call(json!({"op": "wifi_aps", "network": "not hex"}));
    assert_eq!(resp["err"]["code"], "bad_request", "{resp}");
}

#[test]
fn the_cli_prints_access_point_details() {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world.nm.add_ap(
        wlan0,
        Ap::wpa2("Mesh", "AA:00:00:00:00:22", 85)
            .frequency(5180)
            .bitrate(866_000),
    );
    let out = world.run_nexus(&["wifi", "aps", "Mesh"]);
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    for want in ["AA:00:00:00:00:22", "36", "5180", "5 GHz", "866"] {
        assert!(text.contains(want), "{want} in {text}");
    }
    let json = world.run_nexus(&["wifi", "aps", "Mesh", "--json"]);
    let v: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(v["aps"][0]["channel"], 36);
}
