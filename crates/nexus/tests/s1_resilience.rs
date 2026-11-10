//! Seam S1 for resilience and radio control: a real Daemon over `nexus-ipc`,
//! a fake NetworkManager (and fake logind) on a private bus.

use nexus_testkit::{
    ActiveConnection, Ap, DeviceId, NmState, Wire, World, eventually, network_id as id,
};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

fn front_end(world: &World) -> Wire {
    let mut wire = world.wire();
    wire.subscribe();
    wire
}

/// Waits for the first `name` event that `pred` accepts.
fn event_where(fe: &mut Wire, name: &str, pred: impl Fn(&Value) -> bool) -> Value {
    loop {
        let ev = fe.next_event(name);
        if pred(&ev) {
            return ev;
        }
    }
}

fn status_is(state: &'static str) -> impl Fn(&Value) -> bool {
    move |ev| ev["status"]["state"] == state
}

fn no_devices(ev: &Value) -> bool {
    ev["wifi"]["devices"].as_array().is_some_and(Vec::is_empty)
}

fn has_home(ev: &Value) -> bool {
    ev["wifi"]["networks"][0]["ssid"] == "Home"
}

trait Test: Fn(&Value) -> bool {}
impl<F: Fn(&Value) -> bool> Test for F {}

/// Reads events in whatever order they come until each `(name, test)` has
/// matched one. Order between a status and a networks event is not promised.
fn wait_for_all(fe: &mut Wire, wants: &[(&str, &dyn Test)]) {
    let mut done = vec![false; wants.len()];
    while !done.iter().all(|d| *d) {
        let line = fe.read_line();
        assert!(!line.is_null(), "events never arrived: {done:?}");
        for (n, (name, test)) in wants.iter().enumerate() {
            if line["event"] == *name && test(&line) {
                done[n] = true;
            }
        }
    }
}

fn online_world() -> (World, DeviceId) {
    let world = World::start(NEXUS);
    let wlan0 = set_up(&world);
    (world, wlan0)
}

/// Like `online_world`, but a Front end is subscribed first and has been told
/// the world is online, so it has seen every event since.
fn watched_world() -> (World, Wire) {
    let world = World::start(NEXUS);
    let mut fe = front_end(&world);
    set_up(&world);
    wait_for_all(
        &mut fe,
        &[
            ("status_changed", &status_is("online")),
            ("networks_changed", &has_home),
        ],
    );
    (world, fe)
}

fn set_up(world: &World) -> DeviceId {
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:01", 80));
    world.nm.set_psk("Home", "right-password");
    world.nm.set_primary(Some(ActiveConnection::wifi("Home")));
    world.nm.set_state(NmState::ConnectedGlobal);
    wlan0
}

#[test]
fn the_daemon_pushes_status_changed_when_the_backend_changes() {
    let (world, _) = online_world();
    let mut fe = front_end(&world);
    world.nm.set_state(NmState::Disconnected);
    let ev = event_where(&mut fe, "status_changed", status_is("offline"));
    assert_eq!(ev["status"]["backend"]["name"], "networkmanager");
}

#[test]
fn networkmanager_vanishing_is_reported_and_its_return_resyncs_everything() {
    let (mut world, mut fe) = watched_world();
    eventually("the agent registers", || world.nm.agent_registered());

    world.nm.stop();
    wait_for_all(
        &mut fe,
        &[
            ("status_changed", &status_is("unavailable")),
            ("networks_changed", &no_devices),
        ],
    );

    world.nm.restart();
    wait_for_all(
        &mut fe,
        &[
            ("status_changed", &status_is("online")),
            ("networks_changed", &has_home),
        ],
    );
    eventually("the secret agent registers again", || {
        world.nm.agent_registered()
    });
}

#[test]
fn a_networkmanager_restart_mid_join_fails_the_join_and_withdraws_the_prompt() {
    let (mut world, _) = online_world();
    let mut fe = front_end(&world);
    let mut joiner = world.wire();
    let network = id("Home");
    let join =
        std::thread::spawn(move || joiner.call(json!({"op": "wifi_join", "network": network})));
    let opened = fe.next_event("secret_prompt_opened");

    world.nm.restart();

    let closed = event_where(&mut fe, "secret_prompt_closed", |ev| {
        ev["id"] == opened["prompt"]["id"]
    });
    assert_eq!(closed["reason"], "withdrawn");
    eventually("the join gives up at once", || join.is_finished());
    let resp = join.join().unwrap();
    assert_eq!(resp["err"]["code"], "join_failed", "{resp}");
    assert!(
        resp["err"]["message"]
            .as_str()
            .unwrap()
            .contains("restarted"),
        "{resp}"
    );
    assert_eq!(world.nm.persisted_profiles() + world.nm.profile_count(), 0);

    // The Daemon is whole again: a fresh join prompts and works.
    eventually("the secret agent registers again", || {
        world.nm.agent_registered()
    });
    let mut joiner = world.wire();
    let network = id("Home");
    let join =
        std::thread::spawn(move || joiner.call(json!({"op": "wifi_join", "network": network})));
    let opened = fe.next_event("secret_prompt_opened");
    let resp = fe.call(json!({
        "op": "secret_answer", "prompt": opened["prompt"]["id"], "secret": "right-password"
    }));
    assert!(resp.get("ok").is_some(), "{resp}");
    let joined = join.join().unwrap();
    assert_eq!(joined["ok"]["ssid"], "Home", "{joined}");
}

#[test]
fn wifi_can_be_switched_off_and_on() {
    let (world, _) = online_world();
    let mut fe = front_end(&world);
    let mut wire = world.wire();

    let off = wire.call(json!({"op": "wifi_power", "on": false}));
    assert_eq!(off["ok"]["wifi"]["enabled"], false, "{off}");
    assert_eq!(off["ok"]["airplane"], false);
    assert!(!world.nm.wifi_enabled());
    let ev = event_where(&mut fe, "status_changed", |ev| {
        ev["status"]["radios"]["wifi"]["enabled"] == false
    });
    assert_eq!(ev["status"]["radios"]["wifi"]["hardware_blocked"], false);

    let on = wire.call(json!({"op": "wifi_power", "on": true}));
    assert_eq!(on["ok"]["wifi"]["enabled"], true, "{on}");
    assert!(world.nm.wifi_enabled());
    let status = wire.call(json!({"op": "status"}));
    assert_eq!(status["ok"]["radios"]["wifi"]["enabled"], true, "{status}");
}

#[test]
fn airplane_mode_switches_every_radio_and_back() {
    let (world, _) = online_world();
    let mut wire = world.wire();

    let on = wire.call(json!({"op": "airplane", "on": true}));
    assert_eq!(on["ok"]["airplane"], true, "{on}");
    assert!(!world.nm.wifi_enabled());
    assert!(!world.nm.wwan_enabled());
    let status = wire.call(json!({"op": "status"}));
    assert_eq!(status["ok"]["radios"]["airplane"], true, "{status}");

    let off = wire.call(json!({"op": "airplane", "on": false}));
    assert_eq!(off["ok"]["airplane"], false, "{off}");
    assert!(world.nm.wifi_enabled());
    assert!(world.nm.wwan_enabled());
}

#[test]
fn airplane_mode_turns_bluetooth_off_and_back_to_what_it_was() {
    let mut world = World::builder(NEXUS).bluez(true).start();
    world.nm.add_wifi_device("wlan0");
    let mut wire = world.wire();
    assert!(world.bluez().powered());
    wire.call(json!({"op": "airplane", "on": true}));
    assert!(!world.bluez().powered(), "Bluetooth stayed on");
    wire.call(json!({"op": "airplane", "on": false}));
    assert!(world.bluez().powered(), "Bluetooth was not restored");

    // Off before airplane mode: stays off after it.
    world.bluez().set_powered(false);
    wire.call(json!({"op": "airplane", "on": true}));
    wire.call(json!({"op": "airplane", "on": false}));
    assert!(
        !world.bluez().powered(),
        "airplane mode switched Bluetooth on"
    );
}

#[test]
fn a_hardware_block_is_told_apart_from_a_software_one() {
    // The Daemon has told its Front end the world is online before any switch
    // moves, so what follows is a change from a known picture.
    let (world, _watcher) = watched_world();
    let mut wire = world.wire();

    // Soft: Wi-Fi is off, but we may turn it on.
    wire.call(json!({"op": "wifi_power", "on": false}));
    let soft = wire.call(json!({"op": "status"}));
    assert_eq!(soft["ok"]["radios"]["wifi"]["enabled"], false);
    assert_eq!(soft["ok"]["radios"]["wifi"]["hardware_blocked"], false);
    wire.call(json!({"op": "wifi_power", "on": true}));

    // Hard: a switch on the machine; asking cannot help.
    world.nm.set_wifi_hardware_blocked(true);
    let hard = wire.call(json!({"op": "status"}));
    assert_eq!(hard["ok"]["radios"]["wifi"]["hardware_blocked"], true);
    assert_eq!(hard["ok"]["radios"]["wifi"]["enabled"], false);

    let refused = wire.call(json!({"op": "wifi_power", "on": true}));
    assert_eq!(refused["err"]["code"], "hard_blocked", "{refused}");
    assert!(
        refused["err"]["message"]
            .as_str()
            .unwrap()
            .contains("switch"),
        "{refused}"
    );
    let airplane = wire.call(json!({"op": "airplane", "on": false}));
    assert_eq!(airplane["err"]["code"], "hard_blocked", "{airplane}");

    let out = world.run_nexus(&["wifi", "on"]);
    assert_eq!(out.status.code(), Some(12), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("switch"));
    let human = world.run_nexus(&["status"]);
    assert!(
        String::from_utf8_lossy(&human.stdout).contains("hardware switch"),
        "{human:?}"
    );

    // Flip the switch back: the Daemon follows, and Wi-Fi can come on.
    let mut fe = front_end(&world);
    world.nm.set_wifi_hardware_blocked(false);
    event_where(&mut fe, "status_changed", |ev| {
        ev["status"]["radios"]["wifi"]["hardware_blocked"] == false
    });
    let on = wire.call(json!({"op": "wifi_power", "on": true}));
    assert_eq!(on["ok"]["wifi"]["enabled"], true, "{on}");
}

#[test]
fn the_cli_switches_wifi_and_airplane_mode() {
    let (world, _) = online_world();
    let out = world.run_nexus(&["wifi", "off"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(!world.nm.wifi_enabled());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Wi-Fi is off"));
    let out = world.run_nexus(&["airplane", "on"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(!world.nm.wwan_enabled());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Airplane mode is on"));
    let out = world.run_nexus(&["airplane", "off", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["airplane"], false);
    assert!(world.nm.wifi_enabled());
}

#[test]
fn sleep_and_resume_leave_the_state_correct_without_a_restart() {
    let (world, wlan0) = online_world();
    let mut fe = front_end(&world);

    world.nm.sleep();
    event_where(&mut fe, "status_changed", status_is("asleep"));

    // Scans are rate limited; pretend the last one was long ago.
    world.nm.set_last_scan_ago(wlan0, 60_000);
    let before = world.nm.scan_requests();
    world.nm.resume();
    wait_for_all(
        &mut fe,
        &[
            ("status_changed", &status_is("online")),
            ("networks_changed", &has_home),
        ],
    );
    eventually("a fresh scan is asked for after resume", || {
        world.nm.scan_requests() > before
    });
    let status = world.wire().call(json!({"op": "status"}));
    assert_eq!(status["ok"]["state"], "online", "{status}");
}

/// Joins Home for real, so a Profile that autoconnects exists.
fn joined_home(world: &World, wlan0: DeviceId) {
    let joined = world.wire().call(json!({
        "op": "wifi_join", "network": id("Home"), "password": "right-password",
    }));
    assert!(joined.get("ok").is_some(), "{joined}");
    eventually("Home is joined", || {
        world.nm.joined_ssid(wlan0).as_deref() == Some("Home")
    });
}

fn offline_without_a_link(ev: &Value) -> bool {
    ev["status"]["state"] == "offline" && ev["status"]["link"].is_null()
}

fn online_on_home(ev: &Value) -> bool {
    ev["status"]["state"] == "online" && ev["status"]["link"]["name"] == "Home"
}

#[test]
fn turning_wifi_off_drops_the_link_and_turning_it_on_brings_it_back() {
    let (world, wlan0) = online_world();
    joined_home(&world, wlan0);
    let mut fe = front_end(&world);

    let off = world.wire().call(json!({"op": "wifi_power", "on": false}));
    assert_eq!(off["ok"]["wifi"]["enabled"], false, "{off}");
    event_where(&mut fe, "status_changed", offline_without_a_link);
    eventually("the Device is unavailable", || {
        world.nm.device_state(wlan0) == 20
    });
    assert_eq!(world.nm.joined_ssid(wlan0), None);
    let status = world.wire().call(json!({"op": "status"}));
    assert!(status["ok"]["link"].is_null(), "{status}");

    world.wire().call(json!({"op": "wifi_power", "on": true}));
    event_where(&mut fe, "status_changed", online_on_home);
    assert_eq!(world.nm.device_state(wlan0), 100);
    assert_eq!(world.nm.joined_ssid(wlan0).as_deref(), Some("Home"));
}

#[test]
fn a_hardware_switch_drops_the_link_like_the_software_one() {
    let (world, wlan0) = online_world();
    joined_home(&world, wlan0);
    let mut fe = front_end(&world);

    world.nm.set_wifi_hardware_blocked(true);
    event_where(&mut fe, "status_changed", offline_without_a_link);
    assert_eq!(world.nm.device_state(wlan0), 20);

    world.nm.set_wifi_hardware_blocked(false);
    event_where(&mut fe, "status_changed", online_on_home);
    assert_eq!(world.nm.joined_ssid(wlan0).as_deref(), Some("Home"));
}
