//! Seam S1 for Profile settings: a real `nexus daemon` against a fake
//! NetworkManager. Edits to the Profile carrying the current Link are wrapped
//! in a NetworkManager checkpoint and roll back unless confirmed.

use nexus_testkit::{Ap, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");
const UUID: &str = "11111111-2222-4333-8444-555555555555";

fn id(ssid: &str) -> String {
    nexus_ipc::network_id(ssid.as_bytes())
}

/// A saved Profile for Home (not joined) and one for Cafe.
fn world() -> World {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:01", 80));
    world.nm.add_wifi_profile(UUID, "Home", b"Home", true);
    world
}

/// The same, with Home joined so its Profile carries the Link.
fn joined_world() -> World {
    let world = world();
    let resp = world
        .wire()
        .call(json!({"op": "wifi_join", "network": id("Home")}));
    assert!(resp.get("ok").is_some(), "{resp}");
    world
}

fn get(world: &World) -> Value {
    let resp = world
        .wire()
        .call(json!({"op": "profile_get", "network": id("Home")}));
    resp["ok"].clone()
}

fn update(world: &World, view: &Value, edit: impl FnOnce(&mut Value)) -> Value {
    let mut settings = view["settings"].clone();
    edit(&mut settings);
    world.wire().call(json!({
        "op": "profile_update",
        "network": id("Home"),
        "version": view["version"],
        "settings": settings,
        "rollback_secs": 1,
    }))
}

fn static_ip(s: &mut Value) {
    s["ipv4"] = json!({
        "method": "manual",
        "addresses": [{"address": "192.168.1.20", "prefix": 24}],
        "gateway": "192.168.1.1",
    });
    s["dns"] = json!(["1.1.1.1", "2606:4700:4700::1111"]);
}

#[test]
fn get_reads_the_defaults_a_version_and_whether_the_profile_is_in_use() {
    let world = world();
    let view = get(&world);
    assert_eq!(view["ssid"], "Home", "{view}");
    assert_eq!(view["uuid"], UUID);
    assert_eq!(view["active"], false);
    assert!(view["version"].as_u64().unwrap() > 0);
    let s = &view["settings"];
    assert_eq!(s["ipv4"]["method"], "auto");
    assert_eq!(s["ipv6"]["method"], "auto");
    assert_eq!(s["mac"], "permanent");
    assert_eq!(s["metered"], false);
    assert_eq!(s["priority"], 0);
    assert_eq!(s["dns"], json!([]));

    let joined = joined_world();
    assert_eq!(get(&joined)["active"], true);
}

#[test]
fn an_unknown_network_has_no_profile_to_read() {
    let world = world();
    let resp = world
        .wire()
        .call(json!({"op": "profile_get", "network": id("Nowhere")}));
    assert_eq!(resp["err"]["code"], "unknown_network", "{resp}");
}

#[test]
fn editing_a_profile_that_is_not_in_use_applies_at_once_without_a_checkpoint() {
    let world = world();
    let view = get(&world);
    let resp = update(&world, &view, |s| {
        static_ip(s);
        s["ipv6"]["method"] = json!("disabled");
        s["mac"] = json!("every_time");
        s["metered"] = json!(true);
        s["priority"] = json!(50);
    });
    assert!(resp["ok"].get("pending").is_none(), "{resp}");
    assert_eq!(resp["ok"]["ssid"], "Home");
    assert_eq!(world.nm.checkpoints_open(), 0);

    assert_eq!(
        world.nm.profile_text(UUID, "ipv4", "method").as_deref(),
        Some("manual")
    );
    assert_eq!(
        world.nm.profile_text(UUID, "ipv6", "method").as_deref(),
        Some("disabled")
    );
    assert_eq!(
        world
            .nm
            .profile_text(UUID, "802-11-wireless", "assigned-mac-address")
            .as_deref(),
        Some("random")
    );
    assert_eq!(world.nm.profile_int(UUID, "connection", "metered"), Some(1));
    assert_eq!(
        world
            .nm
            .profile_int(UUID, "connection", "autoconnect-priority"),
        Some(50)
    );

    // Reading it back gives what was written, and a newer version.
    let back = get(&world);
    assert_eq!(back["settings"]["ipv4"]["method"], "manual");
    assert_eq!(
        back["settings"]["ipv4"]["addresses"],
        json!([{"address": "192.168.1.20", "prefix": 24}])
    );
    assert_eq!(back["settings"]["ipv4"]["gateway"], "192.168.1.1");
    assert_eq!(
        back["settings"]["dns"],
        json!(["1.1.1.1", "2606:4700:4700::1111"])
    );
    assert_eq!(back["settings"]["mac"], "every_time");
    assert_eq!(back["settings"]["metered"], true);
    assert_eq!(back["settings"]["priority"], 50);
    assert!(back["version"].as_u64() > view["version"].as_u64());
}

#[test]
fn invalid_settings_are_refused_before_anything_is_written() {
    let world = joined_world();
    let view = get(&world);
    let resp = update(&world, &view, |s| {
        s["ipv4"] = json!({
            "method": "manual",
            "addresses": [{"address": "192.168.1.20", "prefix": 24}],
            "gateway": "10.9.9.9",
        });
    });
    assert_eq!(resp["err"]["code"], "invalid_settings", "{resp}");
    assert!(
        resp["err"]["message"]
            .as_str()
            .unwrap()
            .contains("not on the same network"),
        "{resp}"
    );
    assert_eq!(world.nm.checkpoints_open(), 0);
    assert_eq!(get(&world)["version"], view["version"]);
}

#[test]
fn a_stale_version_is_a_conflict_and_changes_nothing() {
    let world = world();
    let view = get(&world);
    world.nm.edit_profile_externally(UUID);
    let resp = update(&world, &view, |s| s["metered"] = json!(true));
    assert_eq!(resp["err"]["code"], "conflict", "{resp}");
    assert_eq!(world.nm.profile_int(UUID, "connection", "metered"), None);
    // Reading again gets the new version, and the edit then goes through.
    let fresh = get(&world);
    let resp = update(&world, &fresh, |s| s["metered"] = json!(true));
    assert!(resp.get("ok").is_some(), "{resp}");
}

#[test]
fn a_confirmed_change_to_the_active_profile_stays() {
    let world = joined_world();
    let mut events = world.wire();
    events.subscribe();
    let view = get(&world);
    let resp = update(&world, &view, static_ip);
    let pending = resp["ok"]["pending"].clone();
    assert!(pending["id"].as_u64().is_some(), "{resp}");
    assert_eq!(pending["rollback_secs"], 1);
    assert_eq!(world.nm.checkpoints_open(), 1);
    let opened = events.next_event("profile_pending");
    assert_eq!(opened["change"]["id"], pending["id"]);

    let ok = world
        .wire()
        .call(json!({"op": "profile_confirm", "pending": pending["id"]}));
    assert!(ok.get("ok").is_some(), "{ok}");
    let settled = events.next_event("profile_settled");
    assert_eq!(settled["outcome"], "confirmed");
    assert_eq!(settled["ssid"], "Home");
    assert_eq!(world.nm.checkpoints_open(), 0);
    assert_eq!(world.nm.checkpoint_rollbacks(), 0);
    assert_eq!(get(&world)["settings"]["ipv4"]["method"], "manual");

    // Too late to confirm it twice.
    let again = world
        .wire()
        .call(json!({"op": "profile_confirm", "pending": pending["id"]}));
    assert_eq!(again["err"]["code"], "nothing_pending", "{again}");
}

#[test]
fn no_confirmation_rolls_the_change_back() {
    let world = joined_world();
    let mut events = world.wire();
    events.subscribe();
    let view = get(&world);
    let resp = update(&world, &view, static_ip);
    assert!(resp["ok"]["pending"].is_object(), "{resp}");
    assert_eq!(get(&world)["settings"]["ipv4"]["method"], "manual");

    let settled = events.next_event("profile_settled");
    assert_eq!(settled["outcome"], "rolled_back", "{settled}");
    assert_eq!(world.nm.checkpoint_rollbacks(), 1);
    assert_eq!(world.nm.checkpoints_open(), 0);
    let back = get(&world);
    assert_eq!(back["settings"]["ipv4"]["method"], "auto");
    assert_eq!(back["settings"]["dns"], json!([]));

    // A late confirmation finds nothing to confirm.
    let late = world
        .wire()
        .call(json!({"op": "profile_confirm", "pending": resp["ok"]["pending"]["id"]}));
    assert_eq!(late["err"]["code"], "nothing_pending", "{late}");
}

#[test]
fn reverting_by_hand_does_not_wait_for_the_clock() {
    let world = joined_world();
    let mut events = world.wire();
    events.subscribe();
    let view = get(&world);
    let resp = world.wire().call(json!({
        "op": "profile_update",
        "network": id("Home"),
        "version": view["version"],
        "settings": {
            "ipv4": view["settings"]["ipv4"], "ipv6": view["settings"]["ipv6"],
            "dns": ["9.9.9.9"], "mac": "permanent", "metered": false, "priority": 0,
        },
        "rollback_secs": 60,
    }));
    let id = resp["ok"]["pending"]["id"].clone();
    assert!(id.is_u64(), "{resp}");
    let reverted = world
        .wire()
        .call(json!({"op": "profile_revert", "pending": id}));
    assert!(reverted.get("ok").is_some(), "{reverted}");
    assert_eq!(
        events.next_event("profile_settled")["outcome"],
        "rolled_back"
    );
    assert_eq!(get(&world)["settings"]["dns"], json!([]));
    assert_eq!(world.nm.checkpoints_open(), 0);
}

#[test]
fn a_second_edit_waits_for_the_first_to_be_settled() {
    let world = joined_world();
    let view = get(&world);
    let first = world.wire().call(json!({
        "op": "profile_update", "network": id("Home"), "version": view["version"],
        "settings": view["settings"], "rollback_secs": 60,
    }));
    assert!(first["ok"]["pending"].is_object(), "{first}");
    let fresh = get(&world);
    let second = world.wire().call(json!({
        "op": "profile_update", "network": id("Home"), "version": fresh["version"],
        "settings": fresh["settings"], "rollback_secs": 60,
    }));
    assert_eq!(second["err"]["code"], "conflict", "{second}");
}

#[test]
fn the_cli_shows_and_sets_a_profile() {
    let world = world();
    let shown = world.run_nexus(&["profile", "show", "Home", "--json"]);
    assert!(shown.status.success(), "{shown:?}");
    let view: Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(view["settings"]["ipv4"]["method"], "auto");

    let set = world.run_nexus(&[
        "profile",
        "set",
        "Home",
        "--ipv4",
        "manual",
        "--ipv4-address",
        "192.168.1.20/24",
        "--ipv4-gateway",
        "192.168.1.1",
        "--dns",
        "1.1.1.1,9.9.9.9",
        "--mac",
        "permanent",
        "--metered",
        "yes",
        "--priority",
        "7",
    ]);
    assert!(set.status.success(), "{set:?}");
    let back = get(&world);
    assert_eq!(back["settings"]["ipv4"]["method"], "manual");
    assert_eq!(back["settings"]["dns"], json!(["1.1.1.1", "9.9.9.9"]));
    assert_eq!(back["settings"]["mac"], "permanent");
    assert_eq!(back["settings"]["priority"], 7);
}

#[test]
fn the_cli_refuses_a_typo_with_its_own_exit_code() {
    let world = world();
    let out = world.run_nexus(&[
        "profile",
        "set",
        "Home",
        "--ipv4",
        "manual",
        "--ipv4-address",
        "192.168.1.300/24",
    ]);
    assert_eq!(out.status.code(), Some(15), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("192.168.1.300"));
    assert_eq!(world.nm.profile_text(UUID, "ipv4", "method"), None);
}

#[test]
fn the_cli_keeps_an_active_change_with_keep_and_rolls_back_without_it() {
    let world = joined_world();
    let kept = world.run_nexus(&[
        "profile",
        "set",
        "Home",
        "--dns",
        "9.9.9.9",
        "--keep",
        "--rollback-secs",
        "30",
    ]);
    assert!(kept.status.success(), "{kept:?}");
    assert_eq!(world.nm.checkpoints_open(), 0);
    assert_eq!(get(&world)["settings"]["dns"], json!(["9.9.9.9"]));

    // No --keep and nobody answers on stdin: the change comes back out.
    let lost = world.run_nexus(&[
        "profile",
        "set",
        "Home",
        "--dns",
        "8.8.8.8",
        "--rollback-secs",
        "1",
    ]);
    assert!(lost.status.success(), "{lost:?}");
    assert!(
        String::from_utf8_lossy(&lost.stdout).contains("Reverted"),
        "{lost:?}"
    );
    assert_eq!(get(&world)["settings"]["dns"], json!(["9.9.9.9"]));
}

#[test]
fn answering_n_reverts_an_active_change_at_once() {
    use std::io::Write as _;
    use std::process::Stdio;
    let world = joined_world();
    let mut cmd = world.nexus();
    cmd.args([
        "profile",
        "set",
        "Home",
        "--dns",
        "8.8.8.8",
        "--rollback-secs",
        "60",
    ])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn nexus");
    child.stdin.take().unwrap().write_all(b"n\n").unwrap();
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("Reverted"),
        "{out:?}"
    );
    assert_eq!(world.nm.checkpoint_rollbacks(), 1);
    assert_eq!(get(&world)["settings"]["dns"], json!([]));
}
