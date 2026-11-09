//! Seam S1 in real mode: the same wire and CLI as `s1_*.rs`, but against real
//! NetworkManager, virtual radios and hostapd access points (`rig/run.sh`).
//! Without the rig every test skips. The APs are defined in `rig/aps.d/`.

use std::time::{Duration, Instant};

use nexus_testkit::RealWorld;
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
        // NetworkManager drops the active access point a moment after
        // Forget returns, so `joined` settles rather than flips.
        let r = eventually(&format!("{ssid} no longer joined"), || {
            row(&list(&world), ssid).filter(|r| r["joined"] == false)
        });
        assert!(r.get("profile").is_none(), "{r}");
    }
}

#[test]
fn a_wrong_password_fails_by_name_and_leaves_no_profile() {
    let Some(world) = world() else { return };
    for ssid in [WPA2, SAE] {
        let resp = join(&world, ssid, Some("not-the-password"));
        assert_eq!(resp["err"]["code"], "wrong_password", "{ssid}: {resp}");
        assert_eq!(
            resp["err"]["message"],
            format!("Wrong password for {ssid}.")
        );
        let r = row(&list(&world), ssid).expect("still listed");
        assert!(
            r.get("profile").is_none(),
            "a failed join must not linger: {r}"
        );
        assert_eq!(r["joined"], false);
    }
}

#[test]
fn a_secured_network_without_a_password_is_refused() {
    let Some(world) = world() else { return };
    let resp = join(&world, WPA2, None);
    assert_eq!(resp["err"]["code"], "password_required", "{resp}");
}
