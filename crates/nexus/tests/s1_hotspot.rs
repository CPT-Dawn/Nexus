//! Seam S1 for the Hotspot: a real `nexus daemon` against a fake
//! NetworkManager whose Wi-Fi card may or may not be able to host one, over
//! the socket and the CLI. The devices that join are counted with `iw`, so
//! the tests put a stand-in `iw` on the world's PATH.

use nexus_testkit::{DeviceId, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

/// A world with a Wi-Fi card that can host a Hotspot.
fn world() -> (World, DeviceId) {
    let world = World::builder(NEXUS).start();
    let wlan0 = world.nm.add_wifi_device("wlan0");
    (world, wlan0)
}

fn home(world: &World) -> std::path::PathBuf {
    world
        .config_path()
        .ancestors()
        .nth(3)
        .expect("a home directory")
        .to_owned()
}

/// `iw` as a stand-in: it lists the stations written to `$HOME/stations`,
/// and fails (like a missing `iw`) while that file does not exist.
fn install_iw(world: &World) {
    world.install_tool(
        "iw",
        "[ -f \"$HOME/stations\" ] || exit 1\necho \"$@\" > \"$HOME/iw-args\"\ncat \"$HOME/stations\"",
    );
}

fn set_stations(world: &World, n: usize) {
    let text: String = (0..n)
        .map(|i| format!("Station 02:00:00:00:00:{i:02x} (on wlan0)\n\tinactive time:\t120 ms\n"))
        .collect();
    std::fs::create_dir_all(home(world)).expect("home");
    std::fs::write(home(world).join("stations"), text).expect("stations");
}

fn hotspot(world: &World) -> Value {
    world.wire().call(json!({"op": "hotspot"}))["ok"].clone()
}

fn start(world: &World, extra: Value) -> Value {
    let mut op = json!({"op": "hotspot_start"});
    for (k, v) in extra.as_object().into_iter().flatten() {
        op[k] = v.clone();
    }
    world.wire().call(op)
}

#[test]
fn a_card_that_can_host_says_so_before_anything_runs() {
    let (world, _) = world();
    let h = hotspot(&world);
    assert_eq!(h["available"], true, "{h}");
    assert_eq!(h["on"], false);
    assert!(h.get("why").is_none(), "{h}");
}

#[test]
fn a_card_that_cannot_host_gets_an_explanation_and_a_refusal() {
    let (world, wlan0) = world();
    world.nm.set_ap_support(wlan0, false);
    let h = hotspot(&world);
    assert_eq!(h["available"], false, "{h}");
    let why = h["why"].as_str().expect("a sentence");
    assert!(why.contains("cannot host a Hotspot"), "{why}");
    assert!(why.contains("access point mode"), "{why}");
    let resp = start(&world, json!({}));
    assert_eq!(resp["err"]["code"], "hotspot_unsupported", "{resp}");
    assert_eq!(world.nm.hotspot_profiles(), 0);
    // The CLI says it too, with its own exit code.
    let out = world.run_nexus(&["hotspot", "start"]);
    assert_eq!(out.status.code(), Some(26));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot host a Hotspot"));
    let out = world.run_nexus(&["hotspot", "status"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("cannot host a Hotspot"));
}

#[test]
fn a_machine_without_wifi_says_so() {
    let world = World::builder(NEXUS).start();
    let h = hotspot(&world);
    assert_eq!(h["available"], false, "{h}");
    assert!(h["why"].as_str().unwrap().contains("no Wi-Fi card"), "{h}");
}

#[test]
fn wifi_switched_off_says_so() {
    let (world, _) = world();
    world.nm.set_radio(false, false);
    let h = hotspot(&world);
    assert_eq!(h["available"], false, "{h}");
    assert!(h["why"].as_str().unwrap().contains("switched off"), "{h}");
}

#[test]
fn starting_makes_up_a_name_and_a_strong_password() {
    let (world, wlan0) = world();
    let resp = start(&world, json!({}));
    let ok = &resp["ok"];
    let ssid = ok["ssid"].as_str().expect("an ssid");
    let password = ok["password"].as_str().expect("a password");
    assert!(ssid.starts_with("Nexus-") && ssid.len() == 10, "{resp}");
    assert_eq!(password.len(), 16, "{resp}");
    assert!(
        password
            .chars()
            .all(|c| c.is_ascii_alphanumeric() && !"0O1Il".contains(c)),
        "{password}"
    );
    assert_eq!(ok["generated"], true);
    // What NetworkManager was told: an access point, shared, WPA2.
    assert_eq!(world.nm.hotspots(), vec![ssid.to_owned()]);
    assert_eq!(world.nm.hotspot_psk(ssid).as_deref(), Some(password));
    assert_eq!(
        world
            .nm
            .hotspot_setting(ssid, "802-11-wireless", "mode")
            .as_deref(),
        Some("ap")
    );
    assert_eq!(
        world.nm.hotspot_setting(ssid, "ipv4", "method").as_deref(),
        Some("shared")
    );
    assert_eq!(
        world
            .nm
            .hotspot_setting(ssid, "802-11-wireless-security", "key-mgmt")
            .as_deref(),
        Some("wpa-psk")
    );
    // In memory only, and it is the card's Link.
    assert_eq!(world.nm.persisted_profiles(), 0);
    assert!(world.nm.joined_ssid(wlan0).is_none());
    let h = hotspot(&world);
    assert_eq!(h["on"], true, "{h}");
    assert_eq!(h["ssid"], ssid);
    assert_eq!(h["starting"], false);
    // Nobody asked for the password back through the secrets door.
    assert_eq!(world.nm.secret_reads(), 0);
}

#[test]
fn two_hotspots_get_different_passwords() {
    let (world, _) = world();
    let a = start(&world, json!({}))["ok"]["password"].clone();
    assert!(
        world
            .wire()
            .call(json!({"op": "hotspot_stop"}))
            .get("ok")
            .is_some()
    );
    let b = start(&world, json!({}))["ok"]["password"].clone();
    assert_ne!(a, b);
}

#[test]
fn a_custom_name_and_password_are_used() {
    let (world, _) = world();
    let resp = start(
        &world,
        json!({"ssid": "Garage Wi-Fi", "password": "open sesame 42"}),
    );
    assert_eq!(resp["ok"]["ssid"], "Garage Wi-Fi", "{resp}");
    assert_eq!(resp["ok"]["generated"], false);
    assert_eq!(
        world.nm.hotspot_psk("Garage Wi-Fi").as_deref(),
        Some("open sesame 42")
    );
}

#[test]
fn unusable_names_and_passwords_are_refused_before_nm_hears_of_them() {
    let (world, _) = world();
    for extra in [
        json!({"password": "short"}),
        json!({"ssid": "", "password": "longenough"}),
        json!({"ssid": "x".repeat(33), "password": "longenough"}),
        json!({"password": "naïve-password"}),
    ] {
        let resp = start(&world, extra.clone());
        assert_eq!(resp["err"]["code"], "invalid_settings", "{extra} {resp}");
    }
    assert_eq!(world.nm.hotspot_profiles(), 0);
    let out = world.run_nexus(&["hotspot", "start", "--password", "short"]);
    assert_eq!(out.status.code(), Some(15));
}

#[test]
fn a_second_hotspot_is_refused_while_one_runs() {
    let (world, _) = world();
    assert!(start(&world, json!({"ssid": "One"})).get("ok").is_some());
    let resp = start(&world, json!({"ssid": "Two"}));
    assert_eq!(resp["err"]["code"], "join_failed", "{resp}");
    assert!(resp["err"]["message"].as_str().unwrap().contains("One"));
    assert_eq!(world.nm.hotspots(), vec!["One".to_owned()]);
}

#[test]
fn stopping_ends_the_link_and_forgets_the_profile() {
    let (world, wlan0) = world();
    start(&world, json!({"ssid": "Garage"}));
    let resp = world.wire().call(json!({"op": "hotspot_stop"}));
    assert_eq!(resp["ok"]["ssid"], "Garage", "{resp}");
    assert!(world.nm.hotspots().is_empty());
    assert_eq!(world.nm.hotspot_profiles(), 0);
    assert!(!world.nm.tethered(wlan0), "the card is free again");
    assert_eq!(hotspot(&world)["on"], false);
    // Nothing to stop the second time.
    let again = world.wire().call(json!({"op": "hotspot_stop"}));
    assert_eq!(again["err"]["code"], "not_connected", "{again}");
    let out = world.run_nexus(&["hotspot", "stop"]);
    assert_eq!(out.status.code(), Some(11));
}

#[test]
fn a_hotspot_that_dies_on_the_way_up_leaves_nothing_behind() {
    let (world, wlan0) = world();
    world.nm.script_hotspot_failure(true);
    let resp = start(&world, json!({"ssid": "Doomed"}));
    assert_eq!(resp["err"]["code"], "join_failed", "{resp}");
    assert!(resp["err"]["message"].as_str().unwrap().contains("Doomed"));
    assert_eq!(world.nm.hotspot_profiles(), 0);
    assert!(!world.nm.tethered(wlan0));
    assert_eq!(
        world.run_nexus(&["hotspot", "start"]).status.code(),
        Some(8)
    );
}

#[test]
fn the_hotspot_is_not_a_saved_wifi_network() {
    let (world, _) = world();
    start(&world, json!({"ssid": "Garage"}));
    let list = world.wire().call(json!({"op": "wifi_list"}));
    let text = list.to_string();
    assert!(!text.contains("Garage"), "{text}");
}

#[test]
fn devices_are_counted_with_iw_and_unknown_without_it() {
    let (world, _) = world();
    start(&world, json!({"ssid": "Garage"}));
    // No `iw` answer yet: the count is unknown, not zero.
    let h = hotspot(&world);
    assert!(h.get("devices").is_none(), "{h}");
    install_iw(&world);
    set_stations(&world, 3);
    let h = hotspot(&world);
    assert_eq!(h["devices"], 3, "{h}");
    let args = std::fs::read_to_string(home(&world).join("iw-args")).expect("iw ran");
    assert_eq!(args.trim(), "dev wlan0 station dump");
    set_stations(&world, 0);
    assert_eq!(hotspot(&world)["devices"], 0);
}

#[test]
fn the_device_count_is_pushed_while_the_hotspot_runs() {
    let mut world = World::builder(NEXUS).daemon(false).start();
    world.write_config("[hotspot]\npoll_secs = 1\n");
    world.start_daemon();
    world.nm.add_wifi_device("wlan0");
    install_iw(&world);
    set_stations(&world, 0);
    let mut ev = world.wire();
    ev.subscribe();
    start(&world, json!({"ssid": "Garage"}));
    let seen = |ev: &mut nexus_testkit::Wire, want: u64| loop {
        let e = ev.next_event("hotspot_changed");
        if e["hotspot"]["devices"] == want && e["hotspot"]["on"] == true {
            return e;
        }
    };
    seen(&mut ev, 0);
    set_stations(&world, 2);
    let e = seen(&mut ev, 2);
    assert_eq!(e["hotspot"]["ssid"], "Garage");
}

#[test]
fn stopping_pushes_an_event_and_stops_counting() {
    let (world, _) = world();
    let mut ev = world.wire();
    ev.subscribe();
    start(&world, json!({"ssid": "Garage"}));
    // Starting and stopping inside one coalesced burst would be no change at
    // all, so hear the start first.
    loop {
        let e = ev.next_event("hotspot_changed");
        if e["hotspot"]["on"] == true {
            assert_eq!(e["hotspot"]["ssid"], "Garage");
            break;
        }
    }
    world.wire().call(json!({"op": "hotspot_stop"}));
    loop {
        let e = ev.next_event("hotspot_changed");
        if e["hotspot"]["on"] == false {
            assert_eq!(e["hotspot"]["available"], true);
            break;
        }
    }
}

#[test]
fn the_qr_details_come_only_from_an_explicit_reveal() {
    let (world, _) = world();
    let started = start(
        &world,
        json!({"ssid": "Garage", "password": "open sesame 42"}),
    );
    assert!(started.get("ok").is_some());
    assert_eq!(world.nm.secret_reads(), 0, "starting reads no secrets");
    let resp = world.wire().call(json!({"op": "hotspot_reveal"}));
    assert_eq!(resp["ok"]["ssid"], "Garage", "{resp}");
    assert_eq!(resp["ok"]["auth"], "wpa");
    assert_eq!(resp["ok"]["password"], "open sesame 42");
    assert_eq!(world.nm.secret_reads(), 1);
}

#[test]
fn a_refused_reveal_is_reveal_denied() {
    let (world, _) = world();
    start(&world, json!({"ssid": "Garage"}));
    world.nm.deny_secret_reads(true);
    let resp = world.wire().call(json!({"op": "hotspot_reveal"}));
    assert_eq!(resp["err"]["code"], "reveal_denied", "{resp}");
    assert_eq!(world.run_nexus(&["hotspot", "qr"]).status.code(), Some(14));
}

#[test]
fn revealing_needs_a_running_hotspot() {
    let (world, _) = world();
    let resp = world.wire().call(json!({"op": "hotspot_reveal"}));
    assert_eq!(resp["err"]["code"], "not_connected", "{resp}");
}

#[test]
fn the_cli_starts_shows_and_stops() {
    let (world, _) = world();
    let out = world.run_nexus(&[
        "hotspot",
        "start",
        "--ssid",
        "Garage",
        "--password",
        "open sesame 42",
    ]);
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Hotspot Garage is on."), "{text}");
    assert!(text.contains("Garage: open sesame 42"), "{text}");
    let out = world.run_nexus(&["hotspot", "status"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("Hotspot Garage is on, device count unknown"),
        "{text}"
    );
    let json = world.run_nexus(&["hotspot", "status", "--json"]);
    let v: Value = serde_json::from_slice(&json.stdout).expect("json");
    assert_eq!(v["on"], true);
    assert_eq!(v["ssid"], "Garage");
    let out = world.run_nexus(&["hotspot", "qr"]);
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains('\u{2580}') || text.contains('\u{2588}') || text.contains('\u{2584}'));
    assert!(
        !text.contains("open sesame"),
        "the code only, not the password"
    );
    let out = world.run_nexus(&["hotspot", "stop"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("Hotspot Garage stopped."));
}

#[test]
fn the_cli_reads_the_password_from_stdin() {
    use std::io::Write as _;
    use std::process::Stdio;
    let (world, _) = world();
    let mut child = world
        .nexus()
        .args([
            "hotspot",
            "start",
            "--ssid",
            "Garage",
            "--password-stdin",
            "--json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"from standard in\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        world.nm.hotspot_psk("Garage").as_deref(),
        Some("from standard in")
    );
}
