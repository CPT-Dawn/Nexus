//! Seam S1 in real mode: the same wire and CLI as `s1_*.rs`, but against real
//! NetworkManager, virtual radios and hostapd access points (`rig/run.sh`).
//! Without the rig every test skips. The APs are defined in `rig/aps.d/`.

use std::time::{Duration, Instant};

use nexus_testkit::{RealWorld, Wire};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

// Keep in step with rig/aps.d/*.conf.
const OPEN: &str = "NexusOpen";
const WPA2: &str = "NexusWpa2";
const WPA2_PSK: &str = "correct-horse-2";
const SAE: &str = "NexusSae";
const SAE_PSK: &str = "correct-horse-3";

fn id(ssid: &str) -> String {
    nexus_ipc::network_id(ssid.as_bytes())
}

/// Retries `f` until it returns `Some`, for up to 30 s (NetworkManager scans
/// on its own schedule).
fn eventually<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out: {what}"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn list(world: &RealWorld) -> Value {
    world.wire().call(json!({"op": "wifi_list"}))["ok"].clone()
}

fn row(list: &Value, ssid: &str) -> Option<Value> {
    list["networks"]
        .as_array()?
        .iter()
        .find(|n| n["ssid"] == ssid)
        .cloned()
}

/// A world with a Daemon, every AP in range and no saved Profiles.
fn world() -> Option<RealWorld> {
    let world = RealWorld::start(NEXUS)?;
    let _ = world.wire().call(json!({"op": "wifi_scan"}));
    for ssid in [OPEN, WPA2, SAE] {
        eventually(&format!("{ssid} in range"), || row(&list(&world), ssid));
    }
    // Leftovers from an earlier test (the rig is shared, so are its Profiles).
    let _ = world.wire().call(json!({"op": "wifi_disconnect"}));
    for ssid in [OPEN, WPA2, SAE] {
        let _ = world
            .wire()
            .call(json!({"op": "wifi_forget", "network": id(ssid)}));
    }
    Some(world)
}

fn join(world: &RealWorld, ssid: &str, password: Option<&str>) -> Value {
    let mut op = json!({"op": "wifi_join", "network": id(ssid)});
    if let Some(p) = password {
        op["password"] = json!(p);
    }
    world.wire().call(op)
}

#[test]
fn status_reports_networkmanager() {
    let Some(world) = world() else { return };
    let resp = world.wire().call(json!({"op": "status"}));
    assert_eq!(resp["ok"]["backend"]["name"], "networkmanager", "{resp}");
    let out = world.run_nexus(&["status", "--json"]);
    assert!(out.status.success(), "{out:?}");
}

#[test]
fn list_and_scan_see_the_virtual_access_points() {
    let Some(world) = world() else { return };
    let scan = world.wire().call(json!({"op": "wifi_scan"}));
    assert!(scan.get("ok").is_some(), "{scan}");
    let l = list(&world);
    let secs = [(OPEN, "open"), (WPA2, "wpa2"), (SAE, "wpa3")];
    for (ssid, security) in secs {
        let r = row(&l, ssid).unwrap_or_else(|| panic!("no {ssid} in {l}"));
        assert_eq!(r["security"], security, "{r}");
        assert_eq!(r["joined"], false);
    }
    let out = world.run_nexus(&["wifi", "list"]);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains(WPA2));
}

#[test]
fn joining_open_wpa2_and_sae_works_and_forget_removes_the_profile() {
    let Some(world) = world() else { return };
    for (ssid, psk) in [(OPEN, None), (WPA2, Some(WPA2_PSK)), (SAE, Some(SAE_PSK))] {
        let resp = join(&world, ssid, psk);
        assert_eq!(resp["ok"]["ssid"], ssid, "{resp}\n{}", world.daemon_log());
        let r = row(&list(&world), ssid).expect("still listed");
        assert_eq!(r["joined"], true, "{r}");
        assert!(r["profile"].is_string(), "{r}");

        let gone = world
            .wire()
            .call(json!({"op": "wifi_forget", "network": id(ssid)}));
        assert_eq!(gone["ok"]["ssid"], ssid, "{gone}");
        // Never joined, not even for a moment, once the Profile is gone.
        let r = row(&list(&world), ssid).expect("still listed");
        assert_eq!(r["joined"], false, "{r}");
        assert!(r.get("profile").is_none(), "{r}");
    }
}

/// A subscribed Front end, and a join (with or without a password) running on
/// its own connection, the way a TUI and the CLI would.
fn front_end(world: &RealWorld) -> Wire {
    let mut wire = world.wire();
    wire.subscribe();
    wire
}

fn join_in_background(
    world: &RealWorld,
    ssid: &str,
    password: Option<&str>,
) -> std::thread::JoinHandle<Value> {
    let mut wire = world.wire();
    let mut op = json!({"op": "wifi_join", "network": id(ssid)});
    if let Some(p) = password {
        op["password"] = json!(p);
    }
    std::thread::spawn(move || wire.call(op))
}

#[test]
fn a_wrong_password_is_reported_as_one_and_leaves_no_profile() {
    let Some(world) = world() else { return };
    for ssid in [WPA2, SAE] {
        let mut fe = front_end(&world);
        let join = join_in_background(&world, ssid, Some("not-the-password"));
        // The password came with the join, so no Secret prompt may open: the
        // caller gets the error and decides what to ask next.
        while !join.is_finished() {
            if let Some(ev) = fe.try_read_line(Duration::from_millis(500)) {
                assert_ne!(ev["event"], "secret_prompt_opened", "{ssid}: {ev}");
            }
        }
        let resp = join.join().unwrap();
        assert_eq!(resp["err"]["code"], "wrong_password", "{ssid}: {resp}");
        assert!(resp["err"]["message"].as_str().unwrap().contains(ssid));
        let r = row(&list(&world), ssid).expect("still listed");
        assert!(
            r.get("profile").is_none(),
            "a failed join must not linger: {r}"
        );
        assert_eq!(r["joined"], false);
    }
}

#[test]
fn a_join_without_a_password_prompts_and_the_answer_joins() {
    let Some(world) = world() else { return };
    let mut fe = front_end(&world);
    let join = join_in_background(&world, WPA2, None);
    let opened = fe.next_event("secret_prompt_opened");
    assert_eq!(opened["prompt"]["retry"], false, "{opened}");
    let ok = fe
        .call(json!({"op": "secret_answer", "prompt": opened["prompt"]["id"], "secret": WPA2_PSK}));
    assert!(ok.get("ok").is_some(), "{ok}");
    let resp = join.join().unwrap();
    assert_eq!(resp["ok"]["ssid"], WPA2, "{resp}");
}

#[test]
fn cancelling_the_first_prompt_reads_as_needing_a_password() {
    let Some(world) = world() else { return };
    let mut fe = front_end(&world);
    let join = join_in_background(&world, WPA2, None);
    let opened = fe.next_event("secret_prompt_opened");
    let cancel = fe.call(json!({"op": "secret_cancel", "prompt": opened["prompt"]["id"]}));
    assert!(cancel.get("ok").is_some(), "{cancel}");
    let resp = join.join().unwrap();
    assert_eq!(resp["err"]["code"], "password_required", "{resp}");
}

// ---- WPA-Enterprise (ticket #16): hostapd's built-in EAP server ----------------

const EAP: &str = "NexusEap";
const EAP_USER: &str = "student";
const EAP_PASSWORD: &str = "correct-horse-4";
/// Where `rig/inner.sh` puts the throwaway CA and the "student" certificate.
const PKI: &str = "/run/nexus-rig/pki";

fn eap_world() -> Option<RealWorld> {
    let world = RealWorld::start(NEXUS)?;
    let _ = world.wire().call(json!({"op": "wifi_scan"}));
    eventually("NexusEap in range", || row(&list(&world), EAP));
    let _ = world.wire().call(json!({"op": "wifi_disconnect"}));
    let _ = world
        .wire()
        .call(json!({"op": "wifi_forget", "network": id(EAP)}));
    Some(world)
}

fn join_eap(world: &RealWorld, eap: Value) -> Value {
    world
        .wire()
        .call(json!({"op": "wifi_join_eap", "network": id(EAP), "eap": eap}))
}

fn peap(password: &str) -> Value {
    json!({
        "method": "peap",
        "phase2": "mschapv2",
        "identity": EAP_USER,
        "password": password,
        "ca_cert": format!("{PKI}/ca.pem"),
        "domain_suffix": "nexus.test",
    })
}

#[test]
fn peap_joins_the_enterprise_network_and_forget_removes_it() {
    let Some(world) = eap_world() else { return };
    let resp = join_eap(&world, peap(EAP_PASSWORD));
    assert_eq!(resp["ok"]["ssid"], EAP, "{resp}\n{}", world.daemon_log());
    let r = row(&list(&world), EAP).expect("still listed");
    assert_eq!(r["joined"], true, "{r}");
    assert_eq!(r["security"], "enterprise", "{r}");
    let gone = world
        .wire()
        .call(json!({"op": "wifi_forget", "network": id(EAP)}));
    assert_eq!(gone["ok"]["ssid"], EAP, "{gone}");
}

#[test]
fn a_wrong_enterprise_password_is_a_refused_sign_in_with_no_profile() {
    let Some(world) = eap_world() else { return };
    let resp = join_eap(&world, peap("not-the-password"));
    assert_eq!(
        resp["err"]["code"],
        "wrong_password",
        "{resp}\n{}",
        world.daemon_log()
    );
    assert!(resp["err"]["message"].as_str().unwrap().contains("sign-in"));
    let r = row(&list(&world), EAP).expect("still listed");
    assert!(
        r.get("profile").is_none(),
        "a failed join must not linger: {r}"
    );
}

#[test]
fn the_server_certificate_is_really_checked() {
    let Some(world) = eap_world() else { return };
    // The right password, but the system's CAs do not know the rig's CA.
    let mut eap = peap(EAP_PASSWORD);
    eap.as_object_mut().unwrap().remove("ca_cert");
    let resp = join_eap(&world, eap);
    assert!(resp.get("err").is_some(), "{resp}\n{}", world.daemon_log());
    let r = row(&list(&world), EAP).expect("still listed");
    assert!(r.get("profile").is_none(), "{r}");
}

#[test]
fn tls_joins_with_a_client_certificate() {
    let Some(world) = eap_world() else { return };
    let resp = join_eap(
        &world,
        json!({
            "method": "tls",
            "identity": EAP_USER,
            "ca_cert": format!("{PKI}/ca.pem"),
            "domain_suffix": "nexus.test",
            "client_cert": format!("{PKI}/student.pem"),
            "private_key": format!("{PKI}/student.key"),
        }),
    );
    assert_eq!(resp["ok"]["ssid"], EAP, "{resp}\n{}", world.daemon_log());
}
