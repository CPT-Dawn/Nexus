//! Seam S1 for desktop notifications: a real `nexus daemon` talks to a fake
//! notification server on a private session bus. The terminal launcher is a
//! stand-in `xdg-terminal-exec` first on the Daemon's `PATH`, so no real
//! terminal ever opens.

use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nexus_testkit::{ActiveConnection, Ap, NmConnectivity, NmState, Wire, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

fn id(ssid: &str) -> String {
    nexus_ipc::network_id(ssid.as_bytes())
}

fn world_with(config: Option<&str>) -> World {
    let mut world = World::builder(NEXUS)
        .notifications(true)
        .daemon(false)
        .start();
    if let Some(config) = config {
        world.write_config(config);
    }
    world.start_daemon();
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:01", 80));
    world.nm.set_psk("Home", "right-password");
    world
}

fn go_online(world: &World, ssid: &str) {
    world.nm.set_primary(Some(ActiveConnection::wifi(ssid)));
    world.nm.set_connectivity(NmConnectivity::Full);
    world.nm.set_state(NmState::ConnectedGlobal);
}

fn go_offline(world: &World) {
    world.nm.set_primary(None);
    world.nm.set_state(NmState::Disconnected);
}

fn join_in_background(world: &World, password: Option<&str>) -> JoinHandle<Value> {
    let mut wire = world.wire();
    let mut op = json!({"op": "wifi_join", "network": id("Home")});
    if let Some(p) = password {
        op["password"] = json!(p);
    }
    std::thread::spawn(move || wire.call(op))
}

fn front_end(world: &World) -> Wire {
    let mut wire = world.wire();
    wire.subscribe();
    wire
}

fn eventually(what: &str, cond: impl Fn() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "never happened: {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A stand-in terminal launcher that records its arguments, one per line.
fn terminal_log(world: &World) -> PathBuf {
    let log = world.dir().join("terminal.log");
    world.install_tool(
        "xdg-terminal-exec",
        &format!("printf '%s\\n' \"$@\" >> '{}'", log.display()),
    );
    log
}

#[test]
fn joining_and_losing_a_network_are_announced() {
    let world = world_with(None);
    let notes = world.notifications.as_ref().unwrap();

    go_online(&world, "Home");
    let joined = notes.notified(1);
    assert_eq!(joined.app_name, "Nexus");
    assert_eq!(joined.summary, "Joined Home (Wi-Fi)");
    assert_eq!(joined.hints["desktop-entry"], "nexus");
    assert!(joined.actions.is_empty(), "{joined:?}");

    go_offline(&world);
    let lost = notes.notified(2);
    assert_eq!(lost.summary, "Lost Home");
}

#[test]
fn a_wrong_password_is_announced() {
    let world = world_with(None);
    let notes = world.notifications.as_ref().unwrap();

    let resp = join_in_background(&world, Some("typo-password"))
        .join()
        .unwrap();
    assert_eq!(resp["err"]["code"], "wrong_password", "{resp}");
    let note = notes.notified(1);
    assert_eq!(note.summary, "Wrong password");
    assert_eq!(note.body, "Wrong password for Home.");
}

#[test]
fn each_kind_can_be_switched_off() {
    let world = world_with(Some(
        "[notifications]\njoined = false\nwrong_password = false\n",
    ));
    let notes = world.notifications.as_ref().unwrap();
    let mut events = front_end(&world);

    go_online(&world, "Home");
    events.next_event("activity"); // Joined, which is switched off
    join_in_background(&world, Some("typo-password"))
        .join()
        .unwrap();
    go_offline(&world);
    events.next_event("activity"); // Lost
    // The loss is the one kind left on, and it comes after the other two
    // would have: so it is the first and only notification.
    assert_eq!(notes.notified(1).summary, "Lost Home");
    assert_eq!(notes.count(), 1);
}

#[test]
fn live_config_changes_apply_without_a_restart() {
    let world = world_with(Some("[notifications]\nlost = false\n"));
    let notes = world.notifications.as_ref().unwrap();
    let mut events = front_end(&world);

    go_online(&world, "Home");
    events.next_event("activity");
    go_offline(&world);
    events.next_event("activity"); // the Lost that stays quiet
    world.write_config("");
    events.next_event("config_reloaded");
    go_online(&world, "Home");
    events.next_event("activity");
    go_offline(&world);
    events.next_event("activity");
    assert_eq!(notes.notified(1).summary, "Joined Home (Wi-Fi)");
    assert_eq!(notes.notified(2).summary, "Joined Home (Wi-Fi)");
    assert_eq!(notes.notified(3).summary, "Lost Home");
    assert_eq!(notes.count(), 3);
}

#[test]
fn a_prompt_with_no_front_end_offers_to_open_the_tui_and_the_action_launches_it() {
    let world = world_with(None);
    let log = terminal_log(&world);
    let notes = world.notifications.as_ref().unwrap();

    let join = join_in_background(&world, None);
    let note = notes.notified(1);
    assert_eq!(note.summary, "Password needed");
    assert!(note.body.contains("Home"), "{note:?}");
    assert_eq!(note.actions, ["enter", "Enter password"]);
    assert_eq!(note.hints["urgency"], "2");
    assert!(note.expire_timeout > 0, "{note:?}");

    notes.invoke(note.id, "enter");
    eventually("the terminal launcher ran", || log.exists());
    let args = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        args.trim(),
        NEXUS,
        "the launcher is handed the nexus binary"
    );

    // The TUI that opens subscribes; the prompt is replayed to it.
    let mut fe = front_end(&world);
    let opened = fe.next_event("secret_prompt_opened");
    assert_eq!(opened["prompt"]["network"], "Home");
    let resp = fe.call(json!({
        "op": "secret_answer",
        "prompt": opened["prompt"]["id"],
        "secret": "right-password",
    }));
    assert!(resp.get("ok").is_some(), "{resp}");
    assert_eq!(join.join().unwrap()["ok"]["ssid"], "Home");
    // Answered elsewhere: the notification goes away by itself.
    notes.wait_closed(note.id);
}

#[test]
fn a_prompt_nobody_answers_expires_and_the_notification_is_withdrawn() {
    let world = world_with(Some("[prompts]\ntimeout_secs = 1\n"));
    terminal_log(&world);
    let notes = world.notifications.as_ref().unwrap();

    let join = join_in_background(&world, None);
    let note = notes.notified(1);
    assert_eq!(note.expire_timeout, 1000);
    assert_eq!(join.join().unwrap()["err"]["code"], "password_required");
    notes.wait_closed(note.id);
}

#[test]
fn ignoring_the_notification_leaves_the_prompt_open() {
    let world = world_with(None);
    let log = terminal_log(&world);
    let notes = world.notifications.as_ref().unwrap();

    let join = join_in_background(&world, None);
    let note = notes.notified(1);
    notes.dismiss(note.id, 1); // the server timed it out
    assert!(!log.exists(), "nothing was clicked");

    // The Daemon is fine and a Front end that arrives later still sees it.
    let mut fe = front_end(&world);
    let opened = fe.next_event("secret_prompt_opened");
    let resp = fe.call(json!({"op": "secret_cancel", "prompt": opened["prompt"]["id"]}));
    assert!(resp.get("ok").is_some(), "{resp}");
    assert_eq!(join.join().unwrap()["err"]["code"], "password_required");
}

#[test]
fn a_connected_front_end_gets_the_prompt_and_no_notification() {
    let world = world_with(None);
    let notes = world.notifications.as_ref().unwrap();
    let mut fe = front_end(&world);

    let join = join_in_background(&world, None);
    let opened = fe.next_event("secret_prompt_opened");
    let resp = fe.call(json!({
        "op": "secret_answer",
        "prompt": opened["prompt"]["id"],
        "secret": "right-password",
    }));
    assert!(resp.get("ok").is_some(), "{resp}");
    join.join().unwrap();

    // Announce something else, so there is a notification to wait for; the
    // prompt's would have come before it.
    go_offline(&world);
    go_online(&world, "Home");
    notes.notified(1);
    assert!(
        notes.all().iter().all(|n| n.actions.is_empty()),
        "{:?}",
        notes.all()
    );
}

#[test]
fn a_front_end_that_left_means_the_next_prompt_is_announced() {
    let world = world_with(None);
    terminal_log(&world);
    let notes = world.notifications.as_ref().unwrap();
    drop(front_end(&world)); // came and went

    let join = join_in_background(&world, None);
    let note = notes.notified(1);
    assert_eq!(note.actions, ["enter", "Enter password"]);
    let mut fe = front_end(&world);
    let opened = fe.next_event("secret_prompt_opened");
    fe.call(json!({"op": "secret_cancel", "prompt": opened["prompt"]["id"]}));
    join.join().unwrap();
}

#[test]
fn a_vpn_prompt_with_no_front_end_says_what_the_vpn_wants() {
    let world = world_with(None);
    terminal_log(&world);
    world.install_vpn_plugin("openvpn");
    world
        .nm
        .add_plugin_vpn_of("uuid-ovpn", "Office", Some("openvpn"));
    world.nm.script_vpn_asks("uuid-ovpn", &["otp"]);
    let notes = world.notifications.as_ref().unwrap();

    let mut wire = world.wire();
    let up = std::thread::spawn(move || wire.call(json!({"op": "vpn_up", "vpn": "uuid-ovpn"})));
    let note = notes.notified(1);
    assert_eq!(note.summary, "One-time code needed");
    assert!(note.body.contains("Office"), "{note:?}");
    assert_eq!(note.actions, ["enter", "Enter password"]);

    let mut fe = front_end(&world);
    let opened = fe.next_event("secret_prompt_opened");
    assert_eq!(opened["prompt"]["kind"], "vpn_secret");
    fe.call(json!({"op": "secret_answer", "prompt": opened["prompt"]["id"], "secret": "123456"}));
    assert_eq!(up.join().unwrap()["ok"]["state"], "up");
    notes.wait_closed(note.id);
}
