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

// A WireGuard Profile made by the Daemon has to satisfy real NetworkManager
// (its schema, its secrets, the kernel's wireguard module), which the fake
// cannot prove. No peer answers: WireGuard is connectionless, so NetworkManager
// calls the tunnel up once the device is configured. Only 10.77.0.0/24 is
// routed through it, so the rig's own traffic is untouched.
#[test]
fn a_wireguard_profile_round_trips_through_real_networkmanager() {
    let Some(world) = world() else { return };
    // RFC 7748 section 6.1 (Alice): a private key and the public key it gives.
    let private = "dwdtCnMYpX08FsFyUbJmRd9ML4frwJkqsXf7pR25LCo=";
    let public = "hSDwCYkwp1R0i33ctD73Wg2/Og0mOBr066SpjqqbTmo=";
    let peer = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=";
    let config = format!(
        "[Interface]\nPrivateKey = {private}\nAddress = 10.77.0.2/32\n\n[Peer]\nPublicKey = {peer}\nAllowedIPs = 10.77.0.0/24\nEndpoint = 10.99.1.1:51820\nPersistentKeepalive = 25\n"
    );
    let mut wire = world.wire();
    let added = wire.call(json!({"op": "vpn_import", "name": "rigwg", "config": config}));
    assert_eq!(added["ok"]["public_key"], public, "{added}");
    let vpn = added["ok"]["id"].as_str().unwrap().to_owned();

    let vpns = wire.call(json!({"op": "vpn_list"}))["ok"]["vpns"].clone();
    let mine = vpns
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == vpn.as_str())
        .unwrap_or_else(|| panic!("not listed: {vpns}"))
        .clone();
    assert_eq!(mine["kind"], "wireguard", "{mine}");
    assert_eq!(mine["state"], "down", "{mine}");
    assert_eq!(mine["addresses"], json!(["10.77.0.2/32"]), "{mine}");
    assert_eq!(mine["endpoint"], "10.99.1.1:51820", "{mine}");

    let key = wire.call(json!({"op": "vpn_key", "vpn": vpn}));
    assert_eq!(key["ok"]["public_key"], public, "{key}");
    let shown = wire.call(json!({"op": "vpn_reveal", "vpn": vpn}));
    let text = shown["ok"]["config"].as_str().unwrap_or_default();
    assert!(text.contains(&format!("PrivateKey = {private}")), "{shown}");
    assert!(text.contains("Endpoint = 10.99.1.1:51820"), "{shown}");

    let up = wire.call(json!({"op": "vpn_up", "vpn": vpn}));
    assert_eq!(up["ok"]["state"], "up", "{up}\n{}", world.daemon_log());
    let state = |wire: &mut Wire| {
        wire.call(json!({"op": "vpn_list"}))["ok"]["vpns"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == vpn.as_str())
            .map(|v| v["state"].clone())
    };
    assert_eq!(state(&mut wire), Some(json!("up")));

    let down = wire.call(json!({"op": "vpn_down", "vpn": vpn}));
    assert_eq!(down["ok"]["state"], "down", "{down}");
    eventually("the tunnel to be down", || {
        (state(&mut wire) == Some(json!("down"))).then_some(())
    });
    // NetworkManager removes the device it made, which later tests rely on.
    eventually("the wireguard device to go away", || {
        (!std::path::Path::new("/sys/class/net/rigwg").exists()).then_some(())
    });
}
