//! Seam S1 for WPA-Enterprise (802.1X) Wi-Fi: a real `nexus daemon` against a
//! fake NetworkManager, over the socket and the CLI.

use std::io::Write as _;
use std::process::Stdio;

use nexus_testkit::{Ap, DeviceId, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

fn id(ssid: &str) -> String {
    nexus_ipc::network_id(ssid.as_bytes())
}

fn world() -> (World, DeviceId) {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::enterprise("eduroam", "AA:00:00:00:00:01", 70));
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:02", 80));
    (world, wlan0)
}

fn peap() -> Value {
    json!({
        "method": "peap",
        "phase2": "mschapv2",
        "identity": "alice@uni.example",
        "anonymous_identity": "anonymous@uni.example",
        "password": "hunter2",
        "domain_suffix": "radius.uni.example",
    })
}

fn join_eap(world: &World, ssid: &str, eap: Value) -> Value {
    world
        .wire()
        .call(json!({"op": "wifi_join_eap", "network": id(ssid), "eap": eap}))
}

#[test]
fn peap_joins_eduroam_and_sends_networkmanager_the_whole_setting() {
    let (world, wlan0) = world();
    world
        .nm
        .set_eap_user("eduroam", "alice@uni.example", "hunter2");
    let resp = join_eap(&world, "eduroam", peap());
    assert_eq!(resp["ok"]["ssid"], "eduroam", "{resp}");
    assert_eq!(world.nm.joined_ssid(wlan0).as_deref(), Some("eduroam"));
    assert_eq!(world.nm.persisted_profiles(), 1);
    let got = |k: &str| world.nm.eap_setting("eduroam", k);
    assert_eq!(got("eap").as_deref(), Some("peap"));
    assert_eq!(got("phase2-auth").as_deref(), Some("mschapv2"));
    assert_eq!(got("identity").as_deref(), Some("alice@uni.example"));
    assert_eq!(got("anonymous-identity").as_deref(), Some("anonymous@uni.example"));
    assert_eq!(got("domain-suffix-match").as_deref(), Some("radius.uni.example"));
    assert_eq!(got("system-ca-certs").as_deref(), Some("true"));
    assert_eq!(got("ca-cert"), None);
}

#[test]
fn a_refused_sign_in_is_a_wrong_password_and_leaves_no_profile() {
    let (world, _) = world();
    world
        .nm
        .set_eap_user("eduroam", "alice@uni.example", "something-else");
    let resp = join_eap(&world, "eduroam", peap());
    assert_eq!(resp["err"]["code"], "wrong_password", "{resp}");
    let message = resp["err"]["message"].as_str().unwrap();
    assert!(message.contains("eduroam") && message.contains("sign-in"), "{message}");
    assert!(!message.contains("hunter2"));
    assert_eq!(world.nm.profile_count(), 0);
}

#[test]
fn settings_that_fail_validation_never_reach_networkmanager() {
    let (world, _) = world();
    let mut eap = peap();
    eap["identity"] = json!("");
    eap["domain_suffix"] = json!(null);
    let resp = join_eap(&world, "eduroam", eap);
    assert_eq!(resp["err"]["code"], "invalid_settings", "{resp}");
    let message = resp["err"]["message"].as_str().unwrap();
    assert!(message.contains("identity") && message.contains("server name"), "{message}");
    assert_eq!(world.nm.profile_count(), 0);
}

#[test]
fn skipping_validation_must_be_said_out_loud_and_then_works() {
    let (world, _) = world();
    let mut eap = peap();
    eap["domain_suffix"] = json!(null);
    eap["no_ca_validation"] = json!(true);
    let resp = join_eap(&world, "eduroam", eap);
    assert_eq!(resp["ok"]["ssid"], "eduroam", "{resp}");
    assert_eq!(
        world.nm.eap_setting("eduroam", "system-ca-certs").as_deref(),
        Some("false")
    );
    assert_eq!(world.nm.eap_setting("eduroam", "domain-suffix-match"), None);
}

#[test]
fn tls_passes_certificate_files_as_paths() {
    let (world, _) = world();
    let dir = std::env::temp_dir().join(format!("nexus-s1-eap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let files: Vec<String> = ["ca.pem", "me.pem", "me.key"]
        .iter()
        .map(|n| {
            let p = dir.join(n);
            std::fs::write(&p, "pem").unwrap();
            p.to_str().unwrap().to_owned()
        })
        .collect();
    let resp = join_eap(
        &world,
        "eduroam",
        json!({
            "method": "tls",
            "identity": "alice@uni.example",
            "ca_cert": files[0],
            "client_cert": files[1],
            "private_key": files[2],
            "private_key_password": "keypass",
        }),
    );
    assert_eq!(resp["ok"]["ssid"], "eduroam", "{resp}");
    let got = |k: &str| world.nm.eap_setting("eduroam", k);
    assert_eq!(got("eap").as_deref(), Some("tls"));
    assert_eq!(got("ca-cert"), Some(format!("file://{}", files[0])));
    assert_eq!(got("client-cert"), Some(format!("file://{}", files[1])));
    assert_eq!(got("private-key"), Some(format!("file://{}", files[2])));
    assert_eq!(got("private-key-password").as_deref(), Some("keypass"));
    assert_eq!(got("password"), None);
    assert_eq!(got("system-ca-certs").as_deref(), Some("false"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_personal_network_refuses_enterprise_settings() {
    let (world, _) = world();
    let resp = join_eap(&world, "Home", peap());
    assert_eq!(resp["err"]["code"], "invalid_settings", "{resp}");
    assert!(resp["err"]["message"].as_str().unwrap().contains("Home"));
    assert_eq!(world.nm.profile_count(), 0);
}

#[test]
fn the_cli_joins_with_flags_and_reads_the_password_from_stdin() {
    let (world, wlan0) = world();
    world
        .nm
        .set_eap_user("eduroam", "alice@uni.example", "hunter2");
    let mut child = world
        .nexus()
        .args([
            "wifi",
            "join",
            "eduroam",
            "--eap",
            "peap",
            "--identity",
            "alice@uni.example",
            "--anonymous-identity",
            "anonymous@uni.example",
            "--domain",
            "radius.uni.example",
            "--password-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"hunter2\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(world.nm.joined_ssid(wlan0).as_deref(), Some("eduroam"));
}

#[test]
fn the_cli_explains_a_bad_form_and_exits_13() {
    let (world, _) = world();
    let out = world
        .nexus()
        .args(["wifi", "join", "eduroam", "--eap", "peap", "--password", "x"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(13));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("identity"), "{err}");
    assert_eq!(world.nm.profile_count(), 0);
}
