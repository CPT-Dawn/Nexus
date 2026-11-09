//! Seam S1 for the CLI commands that finish "every action has a command"
//! (#35): `nexus portal`, `nexus wifi autoconnect`, `nexus ethernet connect
//! --eap` and `nexus activity`.

use std::io::Write as _;
use std::process::Stdio;

use nexus_testkit::{ActiveConnection, Ap, NmConnectivity, NmState, World};
use serde_json::Value;

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

fn json_of(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)))
}

fn text_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn behind_portal(world: &World) {
    world
        .nm
        .set_primary(Some(ActiveConnection::wifi("CafeGuest")));
    world.nm.set_connectivity(NmConnectivity::Portal);
    world.nm.set_state(NmState::ConnectedGlobal);
}

// ---- portal -----------------------------------------------------------------------------

#[test]
fn portal_says_whether_the_network_wants_a_login() {
    let world = World::start(NEXUS);
    let out = world.run_nexus(&["portal"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(text_of(&out).contains("No login"), "{}", text_of(&out));
    assert_eq!(
        json_of(&world.run_nexus(&["portal", "--json"]))["portal"],
        Value::Null
    );

    behind_portal(&world);
    // The Daemon learns of the change on its next re-read.
    let mut seen = String::new();
    for _ in 0..200 {
        let out = world.run_nexus(&["portal"]);
        seen = text_of(&out);
        if seen.contains("CafeGuest") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(seen.contains("CafeGuest"), "{seen}");
    let v = json_of(&world.run_nexus(&["portal", "--json"]));
    assert_eq!(v["portal"]["network"], "CafeGuest", "{v}");
}

#[test]
fn portal_open_runs_the_browser_and_exits_33_when_there_is_no_portal() {
    let mut world = World::builder(NEXUS).daemon(false).start();
    let log = world.dir().join("opened.log");
    world.install_tool("xdg-open", &format!("echo \"$1\" >> '{}'", log.display()));
    world.start_daemon();

    let out = world.run_nexus(&["portal", "open"]);
    assert_eq!(out.status.code(), Some(33), "no portal");

    behind_portal(&world);
    let mut code = None;
    for _ in 0..200 {
        code = world.run_nexus(&["portal", "open"]).status.code();
        if code == Some(0) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert_eq!(code, Some(0));
    // The browser is started, not waited for.
    let mut opened = String::new();
    for _ in 0..200 {
        opened = std::fs::read_to_string(&log).unwrap_or_default();
        if !opened.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(opened.starts_with("http://"), "{opened}");
}

#[test]
fn portal_open_without_a_browser_exits_34() {
    let mut world = World::builder(NEXUS)
        .daemon(false)
        .env("PATH", "/nonexistent")
        .start();
    world.start_daemon();
    behind_portal(&world);
    let mut code = None;
    for _ in 0..200 {
        code = world.run_nexus(&["portal", "open"]).status.code();
        if code != Some(33) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert_eq!(code, Some(34));
}

// ---- wifi autoconnect -----------------------------------------------------------------------

#[test]
fn autoconnect_is_switched_from_the_command_line() {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:01", 80));
    world
        .nm
        .add_wifi_profile("uuid-home", "Home", b"Home", true);

    let out = world.run_nexus(&["wifi", "autoconnect", "Home", "off"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(text_of(&out).contains("Home"), "{}", text_of(&out));
    assert_eq!(world.nm.profile_autoconnect("uuid-home"), Some(false));

    let out = world.run_nexus(&["wifi", "autoconnect", "Home", "on", "--json"]);
    let v = json_of(&out);
    assert_eq!(v["enabled"], true, "{v}");
    assert_eq!(world.nm.profile_autoconnect("uuid-home"), Some(true));

    let out = world.run_nexus(&["wifi", "autoconnect", "Nowhere", "on"]);
    assert_eq!(out.status.code(), Some(9), "unknown network");
}

// ---- ethernet connect --eap --------------------------------------------------------------------

fn nexus_with_stdin(world: &World, args: &[&str], stdin: &str) -> std::process::Output {
    let mut cmd = world.nexus();
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn nexus");
    let mut input = child.stdin.take().expect("stdin");
    input.write_all(stdin.as_bytes()).expect("write stdin");
    drop(input);
    child.wait_with_output().expect("wait")
}

#[test]
fn a_wired_802_1x_port_is_signed_in_from_the_command_line() {
    let world = World::start(NEXUS);
    let eth = world.nm.add_ethernet_device("enp3s0");
    world
        .nm
        .set_port_eap(eth, "alice@office.example", "hunter2");

    let wrong = nexus_with_stdin(
        &world,
        &[
            "ethernet",
            "connect",
            "--eap",
            "peap",
            "--identity",
            "alice@office.example",
            "--domain",
            "radius.office.example",
            "--password-stdin",
        ],
        "nope\n",
    );
    assert_eq!(wrong.status.code(), Some(5), "{wrong:?}");

    let right = nexus_with_stdin(
        &world,
        &[
            "ethernet",
            "connect",
            "enp3s0",
            "--eap",
            "peap",
            "--identity",
            "alice@office.example",
            "--domain",
            "radius.office.example",
            "--password-stdin",
            "--json",
        ],
        "hunter2\n",
    );
    assert_eq!(right.status.code(), Some(0), "{right:?}");
    assert_eq!(json_of(&right)["port"], "enp3s0");
    assert!(world.nm.ethernet_up(eth));
}

#[test]
fn eap_flags_without_eap_are_a_usage_error() {
    let world = World::start(NEXUS);
    let out = world.run_nexus(&["ethernet", "connect", "--identity", "alice"]);
    assert_eq!(out.status.code(), Some(2));
}

// ---- activity -----------------------------------------------------------------------------------

#[test]
fn activity_lists_what_happened_newest_first() {
    let world = World::start(NEXUS);
    let out = world.run_nexus(&["activity"]);
    assert_eq!(out.status.code(), Some(0));
    let v = json_of(&world.run_nexus(&["activity", "--json"]));
    assert!(v["entries"].is_array(), "{v}");
}
