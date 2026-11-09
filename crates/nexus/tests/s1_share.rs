//! Seam S1 for sharing a saved Wi-Fi Network (`wifi_reveal`, `nexus wifi
//! share`): a real `nexus daemon` against a fake NetworkManager. Reading a
//! stored secret needs the Backend's authorization, which the fake can refuse.

use nexus_testkit::{Ap, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

fn id(ssid: &str) -> String {
    nexus_ipc::network_id(ssid.as_bytes())
}

fn world() -> World {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:01", 80));
    world
        .nm
        .add_wifi_profile("uuid-home", "Home", b"Home", true);
    world
        .nm
        .set_profile_secret("uuid-home", "wpa-psk", "correct horse; battery");
    world
        .nm
        .add_wifi_profile("uuid-cafe", "Cafe", b"Cafe", true);
    world
}

fn reveal(world: &World, ssid: &str) -> Value {
    world
        .wire()
        .call(json!({"op": "wifi_reveal", "network": id(ssid)}))
}

#[test]
fn reveal_returns_the_stored_password_and_the_scheme() {
    let world = world();
    let resp = reveal(&world, "Home");
    assert_eq!(
        resp["ok"],
        json!({"ssid": "Home", "auth": "wpa", "password": "correct horse; battery"}),
        "{resp}"
    );
    assert_eq!(world.nm.secret_reads(), 1);
}

#[test]
fn an_open_network_reveals_no_password_and_reads_no_secret() {
    let world = world();
    let resp = reveal(&world, "Cafe");
    assert_eq!(
        resp["ok"],
        json!({"ssid": "Cafe", "auth": "open"}),
        "{resp}"
    );
    assert_eq!(world.nm.secret_reads(), 0);
}

#[test]
fn wpa3_networks_say_sae() {
    let world = world();
    world
        .nm
        .add_wifi_profile("uuid-work", "Work", b"Work", true);
    world
        .nm
        .set_profile_secret("uuid-work", "sae", "hunter22hunter");
    let resp = reveal(&world, "Work");
    assert_eq!(resp["ok"]["auth"], "sae", "{resp}");
}

/// The fake refuses with `org.freedesktop.NetworkManager.Settings.PermissionDenied`,
/// the name NetworkManager really uses, not a bus-level AccessDenied.
#[test]
fn denied_authorization_is_a_clear_refusal_and_no_secret_leaks() {
    let world = world();
    world.nm.deny_secret_reads(true);
    let resp = reveal(&world, "Home");
    assert_eq!(resp["err"]["code"], "reveal_denied", "{resp}");
    let message = resp["err"]["message"].as_str().unwrap();
    assert!(message.contains("authorization"), "{message}");
    assert!(!resp.to_string().contains("battery"), "{resp}");

    world.nm.deny_secret_reads(false);
    assert!(reveal(&world, "Home").get("ok").is_some());
}

#[test]
fn an_unsaved_network_cannot_be_revealed() {
    let world = world();
    let resp = reveal(&world, "Nowhere");
    assert_eq!(resp["err"]["code"], "unknown_network", "{resp}");
}

#[test]
fn nothing_reads_a_secret_until_asked() {
    let world = world();
    let _ = world.wire().call(json!({"op": "wifi_list"}));
    let _ = world.wire().call(json!({"op": "status"}));
    assert_eq!(world.nm.secret_reads(), 0);
}

fn cli(world: &World, args: &[&str]) -> std::process::Output {
    world.nexus().args(args).output().expect("runs nexus")
}

#[test]
fn the_cli_prints_the_password_or_a_qr_or_json() {
    let world = world();

    let plain = cli(&world, &["wifi", "share", "Home"]);
    assert!(plain.status.success(), "{plain:?}");
    let out = String::from_utf8_lossy(&plain.stdout);
    assert!(out.contains("correct horse; battery"), "{out}");

    let qr = cli(&world, &["wifi", "share", "Home", "--qr"]);
    assert!(qr.status.success(), "{qr:?}");
    let out = String::from_utf8_lossy(&qr.stdout);
    assert!(
        out.contains('\u{2588}') || out.contains('\u{2580}') || out.contains('\u{2584}'),
        "{out}"
    );
    assert!(
        !out.contains("battery"),
        "the QR view does not print the password"
    );

    let json = cli(&world, &["wifi", "share", "Home", "--json"]);
    let value: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["password"], "correct horse; battery");
}

#[test]
fn the_cli_says_so_and_exits_14_when_authorization_is_denied() {
    let world = world();
    world.nm.deny_secret_reads(true);
    let out = cli(&world, &["wifi", "share", "Home", "--qr"]);
    assert_eq!(out.status.code(), Some(14), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("authorization"), "{err}");
    assert!(out.stdout.is_empty());
}
