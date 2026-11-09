//! Seam S1 for "Keep in keyring": a real Daemon against the fake
//! NetworkManager and a fake Secret Service on the private session bus. A
//! Profile kept in the keyring is agent-owned: NetworkManager never stores its
//! password and asks the Daemon, which answers from the keyring.

use nexus_testkit::{Ap, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");
const UUID: &str = "11111111-2222-4333-8444-555555555555";
const PASSWORD: &str = "hunter2-hunter2";

fn id(ssid: &str) -> String {
    nexus_ipc::network_id(ssid.as_bytes())
}

/// A saved Profile for Home with a stored password, in a world with or
/// without a keyring.
fn world(keyring: bool) -> World {
    let world = World::builder(NEXUS).keyring(keyring).start();
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:01", 80));
    world.nm.add_wifi_profile(UUID, "Home", b"Home", true);
    world.nm.set_profile_secret(UUID, "wpa-psk", PASSWORD);
    world.nm.set_psk("Home", PASSWORD);
    world
}

fn get(world: &World) -> Value {
    let resp = world
        .wire()
        .call(json!({"op": "profile_get", "network": id("Home")}));
    resp["ok"].clone()
}

fn keep_in_keyring(world: &World, yes: bool) -> Value {
    let view = get(world);
    let mut settings = view["settings"].clone();
    settings["keyring"] = json!(yes);
    world.wire().call(json!({
        "op": "profile_update",
        "network": id("Home"),
        "version": view["version"],
        "settings": settings,
    }))
}

fn join(world: &World) -> Value {
    world
        .wire()
        .call(json!({"op": "wifi_join", "network": id("Home")}))
}

#[test]
fn the_system_store_stays_the_default() {
    let world = world(true);
    assert_eq!(get(&world)["settings"]["keyring"], false);
    assert_eq!(world.keyring.as_ref().unwrap().items(), 0);
}

#[test]
fn keeping_a_profile_in_the_keyring_moves_its_password_there() {
    let world = world(true);
    let resp = keep_in_keyring(&world, true);
    assert!(resp.get("ok").is_some(), "{resp}");

    let keyring = world.keyring.as_ref().unwrap();
    assert_eq!(keyring.secret_of(UUID).as_deref(), Some(PASSWORD));
    assert_eq!(get(&world)["settings"]["keyring"], true);
    // NetworkManager no longer holds the password: agent-owned (flag 1).
    assert_eq!(
        world
            .nm
            .profile_int(UUID, "802-11-wireless-security", "psk-flags"),
        Some(1)
    );
}

#[test]
fn nexus_serves_the_kept_password_to_networkmanager_without_asking_anyone() {
    let world = world(true);
    keep_in_keyring(&world, true);
    let reads = world.keyring.as_ref().unwrap().reads();

    // The fake checks the password against Home's, so a wrong answer fails.
    let resp = join(&world);
    assert_eq!(resp["ok"]["ssid"], "Home", "{resp}");
    assert!(world.nm.agent_calls().iter().any(|c| c == "GetSecrets"));
    assert!(world.keyring.as_ref().unwrap().reads() > reads);
}

#[test]
fn turning_it_off_brings_the_password_back_to_networkmanager() {
    let world = world(true);
    keep_in_keyring(&world, true);
    let resp = keep_in_keyring(&world, false);
    assert!(resp.get("ok").is_some(), "{resp}");

    assert_eq!(get(&world)["settings"]["keyring"], false);
    assert_eq!(world.keyring.as_ref().unwrap().items(), 0);
    assert_eq!(
        world
            .nm
            .profile_int(UUID, "802-11-wireless-security", "psk-flags"),
        Some(0)
    );
    // The system store serves it again: a join needs no agent.
    let calls = world.nm.agent_calls().len();
    let resp = join(&world);
    assert_eq!(resp["ok"]["ssid"], "Home", "{resp}");
    assert_eq!(world.nm.agent_calls().len(), calls);
}

#[test]
fn without_a_keyring_the_profile_is_left_as_it_was_and_the_message_says_why() {
    let world = world(false);
    let resp = keep_in_keyring(&world, true);
    assert_eq!(resp["err"]["code"], "keyring_unavailable", "{resp}");
    let message = resp["err"]["message"].as_str().unwrap();
    assert!(message.contains("keyring"), "{message}");
    assert!(message.contains("left where it is"), "{message}");

    // Not silently broken: still system-owned, still joins.
    assert_eq!(get(&world)["settings"]["keyring"], false);
    assert_eq!(
        world
            .nm
            .profile_int(UUID, "802-11-wireless-security", "psk-flags"),
        None
    );
    let resp = join(&world);
    assert_eq!(resp["ok"]["ssid"], "Home", "{resp}");
}

#[test]
fn a_keyring_that_vanished_falls_back_to_the_prompt_instead_of_breaking_the_join() {
    let mut world = world(true);
    keep_in_keyring(&world, true);
    world.keyring.as_mut().unwrap().stop();

    let mut fe = world.wire();
    fe.subscribe();
    let background = {
        let mut wire = world.wire();
        std::thread::spawn(move || wire.call(json!({"op": "wifi_join", "network": id("Home")})))
    };
    let opened = fe.next_event("secret_prompt_opened");
    assert_eq!(opened["prompt"]["network"], "Home", "{opened}");
    let resp = fe.call(json!({
        "op": "secret_answer",
        "prompt": opened["prompt"]["id"],
        "secret": PASSWORD,
    }));
    assert!(resp.get("ok").is_some(), "{resp}");
    let joined = background.join().unwrap();
    assert_eq!(joined["ok"]["ssid"], "Home", "{joined}");
}

#[test]
fn turning_it_off_without_the_keyring_is_refused_so_the_password_is_not_lost() {
    let mut world = world(true);
    keep_in_keyring(&world, true);
    world.keyring.as_mut().unwrap().stop();
    let resp = keep_in_keyring(&world, false);
    assert_eq!(resp["err"]["code"], "keyring_unavailable", "{resp}");
    assert_eq!(get(&world)["settings"]["keyring"], true);
}

#[test]
fn an_open_network_has_no_password_to_keep() {
    let world = World::builder(NEXUS).keyring(true).start();
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:01", 80));
    world.nm.add_wifi_profile(UUID, "Home", b"Home", true);
    let resp = keep_in_keyring(&world, true);
    assert_eq!(resp["err"]["code"], "invalid_settings", "{resp}");
    assert_eq!(world.keyring.as_ref().unwrap().items(), 0);
}

#[test]
fn forgetting_the_network_clears_its_keyring_entry() {
    let world = world(true);
    keep_in_keyring(&world, true);
    assert_eq!(world.keyring.as_ref().unwrap().items(), 1);
    let resp = world
        .wire()
        .call(json!({"op": "wifi_forget", "network": id("Home")}));
    assert!(resp.get("ok").is_some(), "{resp}");
    let keyring = world.keyring.as_ref().unwrap();
    let start = std::time::Instant::now();
    while keyring.items() != 0 {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "the keyring entry outlived its Profile"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn the_cli_keeps_a_profile_in_the_keyring_and_exits_33_without_one() {
    let keyed = world(true);
    let out = keyed.run_nexus(&["profile", "set", "Home", "--keyring", "yes", "--keep"]);
    assert!(out.status.success(), "{out:?}");
    let out = keyed.run_nexus(&["profile", "show", "Home"]);
    let shown = String::from_utf8_lossy(&out.stdout);
    assert!(shown.contains("Keyring   yes"), "{shown}");
    assert_eq!(
        keyed.keyring.as_ref().unwrap().secret_of(UUID).as_deref(),
        Some(PASSWORD)
    );

    let bare = world(false);
    let out = bare.run_nexus(&["profile", "set", "Home", "--keyring", "yes", "--keep"]);
    assert_eq!(out.status.code(), Some(33), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("keyring"));
}
