//! Seam S1 for joining, disconnecting and Forgetting Wi-Fi Networks: a real
//! `nexus daemon` against a fake NetworkManager, over the socket and the CLI.

use std::io::Write as _;
use std::process::Stdio;

use nexus_testkit::{Ap, DeviceId, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

// NMDeviceState
const CONFIG: u32 = 50;
const NEED_AUTH: u32 = 60;
const IP_CONFIG: u32 = 70;
// NMDeviceStateReason
const SSID_NOT_FOUND: u32 = 53;
const IP_CONFIG_UNAVAILABLE: u32 = 5;

fn id(ssid: &str) -> String {
    nexus_ipc::network_id(ssid.as_bytes())
}

fn world() -> (World, DeviceId) {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:01", 80));
    world
        .nm
        .add_ap(wlan0, Ap::open("Cafe", "BB:00:00:00:00:01", 60));
    (world, wlan0)
}

fn join(world: &World, ssid: &str, password: Option<&str>) -> Value {
    let mut op = json!({"op": "wifi_join", "network": id(ssid)});
    if let Some(p) = password {
        op["password"] = json!(p);
    }
    world.wire().call(op)
}

fn list(world: &World) -> Value {
    world.wire().call(json!({"op": "wifi_list"}))["ok"].clone()
}

fn row(list: &Value, ssid: &str) -> Value {
    list["networks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["ssid"] == ssid)
        .cloned()
        .unwrap_or_else(|| panic!("no {ssid} in {list}"))
}

#[test]
fn joining_an_open_network_saves_a_profile_only_after_it_is_up() {
    let (world, wlan0) = world();
    let resp = join(&world, "Cafe", None);
    assert_eq!(resp["ok"]["ssid"], "Cafe", "{resp}");
    assert!(
        resp["ok"]["profile"]
            .as_str()
            .is_some_and(|u| !u.is_empty())
    );
    assert_eq!(world.nm.joined_ssid(wlan0).as_deref(), Some("Cafe"));
    assert_eq!(world.nm.persisted_profiles(), 1);
    let cafe = row(&list(&world), "Cafe");
    assert_eq!(cafe["joined"], true);
    assert_eq!(cafe["profile"], resp["ok"]["profile"]);
}

#[test]
fn joining_wpa2_with_the_right_password_works() {
    let (world, _) = world();
    world.nm.set_psk("Home", "hunter2hunter2");
    let resp = join(&world, "Home", Some("hunter2hunter2"));
    assert!(resp.get("ok").is_some(), "{resp}");
    assert_eq!(world.nm.persisted_profiles(), 1);
}

#[test]
fn a_secured_network_without_a_password_is_refused_before_anything_is_created() {
    let (world, _) = world();
    let resp = join(&world, "Home", None);
    assert_eq!(resp["err"]["code"], "password_required", "{resp}");
    assert_eq!(world.nm.profile_count(), 0);
}

#[test]
fn a_wrong_password_names_the_network_and_leaves_no_profile() {
    let (world, wlan0) = world();
    world.nm.set_psk("Home", "right-password");
    let resp = join(&world, "Home", Some("typo-password"));
    assert_eq!(resp["err"]["code"], "wrong_password", "{resp}");
    assert_eq!(resp["err"]["message"], "Wrong password for Home.");
    assert_eq!(world.nm.profile_count(), 0, "a failed join must not linger");
    assert_eq!(world.nm.joined_ssid(wlan0), None);
    assert_eq!(row(&list(&world), "Home").get("profile"), None);
}

#[test]
fn a_network_that_is_not_in_range_is_refused() {
    let (world, _) = world();
    let resp = join(&world, "Faraway", Some("whatever-password"));
    assert_eq!(resp["err"]["code"], "out_of_range", "{resp}");
    assert!(resp["err"]["message"].as_str().unwrap().contains("Faraway"));
    assert_eq!(world.nm.profile_count(), 0);
}

#[test]
fn a_network_that_vanishes_mid_join_is_out_of_range_too() {
    let (world, _) = world();
    world.nm.script_join_failure("Cafe", CONFIG, SSID_NOT_FOUND);
    let resp = join(&world, "Cafe", None);
    assert_eq!(resp["err"]["code"], "out_of_range", "{resp}");
    assert_eq!(world.nm.profile_count(), 0);
}

#[test]
fn a_dhcp_timeout_says_no_address_and_leaves_no_profile() {
    let (world, wlan0) = world();
    world
        .nm
        .script_join_failure("Cafe", IP_CONFIG, IP_CONFIG_UNAVAILABLE);
    let resp = join(&world, "Cafe", None);
    assert_eq!(resp["err"]["code"], "no_address", "{resp}");
    let message = resp["err"]["message"].as_str().unwrap();
    assert!(
        message.contains("Cafe") && message.contains("address"),
        "{message}"
    );
    assert_eq!(world.nm.profile_count(), 0);
    assert_eq!(world.nm.joined_ssid(wlan0), None);
}

#[test]
fn an_unknown_reason_number_still_reads_as_a_sentence() {
    let (world, _) = world();
    world.nm.script_join_failure("Cafe", NEED_AUTH + 10, 9999);
    let resp = join(&world, "Cafe", None);
    assert_eq!(resp["err"]["code"], "join_failed", "{resp}");
    assert!(
        resp["err"]["message"]
            .as_str()
            .unwrap()
            .contains("Unknown reason (9999)"),
        "{resp}"
    );
}

#[test]
fn disconnect_drops_the_link_and_keeps_the_profile() {
    let (world, wlan0) = world();
    assert!(join(&world, "Cafe", None).get("ok").is_some());
    let resp = world.wire().call(json!({"op": "wifi_disconnect"}));
    assert_eq!(resp["ok"]["ssid"], "Cafe", "{resp}");
    assert_eq!(world.nm.joined_ssid(wlan0), None);
    assert_eq!(world.nm.persisted_profiles(), 1);
    assert_eq!(row(&list(&world), "Cafe")["joined"], false);
    let again = world.wire().call(json!({"op": "wifi_disconnect"}));
    assert_eq!(again["err"]["code"], "not_connected", "{again}");
}

#[test]
fn rejoining_a_saved_network_needs_no_password() {
    let (world, wlan0) = world();
    world.nm.set_psk("Home", "right-password");
    assert!(
        join(&world, "Home", Some("right-password"))
            .get("ok")
            .is_some()
    );
    world.wire().call(json!({"op": "wifi_disconnect"}));
    let resp = join(&world, "Home", None);
    assert!(resp.get("ok").is_some(), "{resp}");
    assert_eq!(world.nm.joined_ssid(wlan0).as_deref(), Some("Home"));
    assert_eq!(world.nm.profile_count(), 1, "no duplicate Profile");
}

#[test]
fn forget_deletes_the_profile_and_unknown_networks_are_a_clear_error() {
    let (world, _) = world();
    assert!(join(&world, "Cafe", None).get("ok").is_some());
    let resp = world
        .wire()
        .call(json!({"op": "wifi_forget", "network": id("Cafe")}));
    assert_eq!(resp["ok"]["ssid"], "Cafe", "{resp}");
    assert_eq!(world.nm.profile_count(), 0);
    let cafe = row(&list(&world), "Cafe");
    assert_eq!(cafe.get("profile"), None);
    assert_eq!(cafe["joined"], false);

    let nope = world
        .wire()
        .call(json!({"op": "wifi_forget", "network": id("Nowhere")}));
    assert_eq!(nope["err"]["code"], "unknown_network", "{nope}");
    assert!(nope["err"]["message"].as_str().unwrap().contains("Nowhere"));
}

#[test]
fn forget_reaches_saved_networks_that_are_out_of_range() {
    let (world, _) = world();
    world
        .nm
        .add_wifi_profile("uuid-office", "Office", b"Office", true);
    let resp = world
        .wire()
        .call(json!({"op": "wifi_forget", "network": id("Office")}));
    assert_eq!(resp["ok"]["ssid"], "Office", "{resp}");
    assert_eq!(world.nm.profile_count(), 0);
}

#[test]
fn the_password_never_reaches_the_daemon_log() {
    let (world, _) = world();
    world.nm.set_psk("Home", "right-password");
    join(&world, "Home", Some("super-secret-typo"));
    assert!(!world.daemon_log().contains("super-secret-typo"));
}

// ---- the CLI ----

fn nexus_join(world: &World, args: &[&str], stdin: Option<&str>) -> std::process::Output {
    let mut cmd = world.nexus();
    cmd.args(["wifi", "join"]).args(args);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn nexus");
    let mut input = child.stdin.take().expect("stdin");
    if let Some(text) = stdin {
        input.write_all(text.as_bytes()).expect("write stdin");
    }
    drop(input);
    child.wait_with_output().expect("wait")
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn cli_joins_with_the_password_from_stdin() {
    let (world, wlan0) = world();
    world.nm.set_psk("Home", "right-password");
    let out = nexus_join(
        &world,
        &["Home", "--password-stdin"],
        Some("right-password\n"),
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("Joined Home"));
    assert_eq!(world.nm.joined_ssid(wlan0).as_deref(), Some("Home"));
}

#[test]
fn cli_joins_with_the_password_flag() {
    let (world, _) = world();
    world.nm.set_psk("Home", "right-password");
    let out = nexus_join(&world, &["Home", "--password", "right-password"], None);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
}

#[test]
fn cli_exit_codes_tell_the_failure_classes_apart() {
    let (world, _) = world();
    world.nm.set_psk("Home", "right-password");
    world
        .nm
        .script_join_failure("Cafe", IP_CONFIG, IP_CONFIG_UNAVAILABLE);

    let wrong = nexus_join(&world, &["Home", "--password", "nope-nope-nope"], None);
    assert_eq!(wrong.status.code(), Some(5), "{wrong:?}");
    assert!(
        stderr(&wrong).contains("Wrong password for Home."),
        "{wrong:?}"
    );

    let far = nexus_join(&world, &["Faraway", "--password", "whatever-ok"], None);
    assert_eq!(far.status.code(), Some(6), "{far:?}");

    let dhcp = nexus_join(&world, &["Cafe"], None);
    assert_eq!(dhcp.status.code(), Some(7), "{dhcp:?}");

    let missing = nexus_join(&world, &["Home"], None);
    assert_eq!(missing.status.code(), Some(10), "{missing:?}");
    assert!(stderr(&missing).contains("--password"), "{missing:?}");

    let gone = world.run_nexus(&["wifi", "forget", "Nowhere"]);
    assert_eq!(gone.status.code(), Some(9), "{gone:?}");

    let idle = world.run_nexus(&["wifi", "disconnect"]);
    assert_eq!(idle.status.code(), Some(11), "{idle:?}");

    let ok = world.run_nexus(&["wifi", "forget", "Cafe"]);
    assert_eq!(
        ok.status.code(),
        Some(9),
        "nothing was saved for Cafe: {ok:?}"
    );
}

#[test]
fn cli_disconnect_and_forget_report_in_words() {
    let (world, _) = world();
    assert_eq!(nexus_join(&world, &["Cafe"], None).status.code(), Some(0));
    let out = world.run_nexus(&["wifi", "disconnect"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("Disconnected from Cafe"));
    let out = world.run_nexus(&["wifi", "forget", "Cafe"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("Forgot Cafe"));
}

#[test]
fn cli_without_a_daemon_still_exits_3() {
    let world = World::builder(NEXUS).daemon(false).start();
    let out = nexus_join(&world, &["Cafe"], None);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
}
