//! Seam S1 for Secret prompts: a real `nexus daemon` registers as the secret
//! agent of a fake NetworkManager, which asks it for passwords like the real
//! AgentManager does. Front ends are raw wires.

use std::thread::JoinHandle;

use nexus_testkit::{Ap, DeviceId, Wire, World, eventually, network_id as id};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

fn world() -> (World, DeviceId) {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:01", 80));
    world.nm.set_psk("Home", "right-password");
    (world, wlan0)
}

/// A subscribed Front end.
fn front_end(world: &World) -> Wire {
    let mut wire = world.wire();
    wire.subscribe();
    wire
}

/// `nexus wifi join` without a password, on its own connection and thread.
fn join_in_background(world: &World, ssid: &str) -> JoinHandle<Value> {
    let mut wire = world.wire();
    let network = id(ssid);
    std::thread::spawn(move || wire.call(json!({"op": "wifi_join", "network": network})))
}

fn answer(fe: &mut Wire, prompt: &Value, secret: &str) -> Value {
    fe.call(json!({"op": "secret_answer", "prompt": prompt["prompt"]["id"], "secret": secret}))
}

#[test]
fn a_prompt_opens_for_the_front_end_and_the_answer_joins_the_network() {
    let (world, wlan0) = world();
    let mut fe = front_end(&world);
    let join = join_in_background(&world, "Home");

    let opened = fe.next_event("secret_prompt_opened");
    assert_eq!(opened["prompt"]["kind"], "wifi_password", "{opened}");
    assert_eq!(opened["prompt"]["network"], "Home");
    assert_eq!(opened["prompt"]["network_id"], id("Home"));
    assert_eq!(opened["prompt"]["retry"], false);
    assert!(opened["prompt"]["expires_in"].as_u64().unwrap() > 0);

    let resp = answer(&mut fe, &opened, "right-password");
    assert!(resp.get("ok").is_some(), "{resp}");
    let closed = fe.next_event("secret_prompt_closed");
    assert_eq!(closed["id"], opened["prompt"]["id"]);
    assert_eq!(closed["reason"], "answered");

    let joined = join.join().unwrap();
    assert_eq!(joined["ok"]["ssid"], "Home", "{joined}");
    assert_eq!(world.nm.joined_ssid(wlan0).as_deref(), Some("Home"));
    assert_eq!(world.nm.persisted_profiles(), 1);
}

/// Polls (no sleeping to a guess) until `cond` holds.
/// A world whose Daemon starts with this `config.toml` already in place.
fn world_with_config(config: &str) -> (World, DeviceId) {
    let mut world = World::builder(NEXUS).daemon(false).start();
    world.write_config(config);
    world.start_daemon();
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:01", 80));
    world.nm.set_psk("Home", "right-password");
    (world, wlan0)
}

#[test]
fn cancelling_a_prompt_leaves_the_network_unsaved_and_needing_a_password() {
    let (world, wlan0) = world();
    let mut fe = front_end(&world);
    let join = join_in_background(&world, "Home");
    let opened = fe.next_event("secret_prompt_opened");

    let resp = fe.call(json!({"op": "secret_cancel", "prompt": opened["prompt"]["id"]}));
    assert!(resp.get("ok").is_some(), "{resp}");
    assert_eq!(fe.next_event("secret_prompt_closed")["reason"], "cancelled");

    let joined = join.join().unwrap();
    assert_eq!(joined["err"]["code"], "password_required", "{joined}");
    assert!(joined["err"]["message"].as_str().unwrap().contains("Home"));
    assert_eq!(world.nm.profile_count(), 0);
    assert_eq!(world.nm.joined_ssid(wlan0), None);
}

#[test]
fn a_wrong_password_reopens_the_prompt_saying_so() {
    let (world, wlan0) = world();
    let mut fe = front_end(&world);
    let join = join_in_background(&world, "Home");

    let first = fe.next_event("secret_prompt_opened");
    assert_eq!(first["prompt"]["retry"], false);
    assert!(answer(&mut fe, &first, "typo-password").get("ok").is_some());

    let again = fe.next_event("secret_prompt_opened");
    assert_ne!(again["prompt"]["id"], first["prompt"]["id"]);
    assert_eq!(again["prompt"]["retry"], true, "{again}");
    assert_eq!(
        again["prompt"]["message"],
        "Wrong password for Home. Try again."
    );
    assert!(
        answer(&mut fe, &again, "right-password")
            .get("ok")
            .is_some()
    );

    let joined = join.join().unwrap();
    assert_eq!(joined["ok"]["ssid"], "Home", "{joined}");
    assert_eq!(world.nm.joined_ssid(wlan0).as_deref(), Some("Home"));
    // NetworkManager asked twice, the second time with REQUEST_NEW, and then
    // handed the agent what it learned.
    eventually("SaveSecrets reached the agent", || {
        world.nm.agent_calls().contains(&"SaveSecrets".to_owned())
    });
    let asked = world
        .nm
        .agent_calls()
        .iter()
        .filter(|c| *c == "GetSecrets")
        .count();
    assert_eq!(asked, 2);
}

#[test]
fn a_failed_handshake_after_an_answer_also_reopens_the_prompt() {
    let (world, _) = world();
    // This NetworkManager fails the join (reason 8) instead of asking again.
    world.nm.set_request_new(false);
    let mut fe = front_end(&world);
    let join = join_in_background(&world, "Home");

    let first = fe.next_event("secret_prompt_opened");
    answer(&mut fe, &first, "typo-password");
    let again = fe.next_event("secret_prompt_opened");
    assert_eq!(again["prompt"]["retry"], true, "{again}");
    answer(&mut fe, &again, "right-password");
    assert_eq!(join.join().unwrap()["ok"]["ssid"], "Home");
    assert_eq!(world.nm.persisted_profiles(), 1);
}

#[test]
fn an_unanswered_prompt_expires_and_the_network_reads_as_needing_a_password() {
    let (world, wlan0) = world_with_config("[prompts]\ntimeout_secs = 1\n");
    let mut fe = front_end(&world);
    let join = join_in_background(&world, "Home");

    let opened = fe.next_event("secret_prompt_opened");
    assert_eq!(opened["prompt"]["expires_in"], 1);
    let closed = fe.next_event("secret_prompt_closed");
    assert_eq!(closed["id"], opened["prompt"]["id"]);
    assert_eq!(closed["reason"], "expired");

    let joined = join.join().unwrap();
    assert_eq!(joined["err"]["code"], "password_required", "{joined}");
    assert!(
        joined["err"]["message"]
            .as_str()
            .unwrap()
            .contains("in time")
    );
    assert_eq!(world.nm.profile_count(), 0);
    assert_eq!(world.nm.joined_ssid(wlan0), None);
    // A late answer finds nothing.
    let late = answer(&mut fe, &opened, "right-password");
    assert_eq!(late["err"]["code"], "unknown_prompt", "{late}");
}

#[test]
fn a_prompt_with_no_front_end_waits_and_is_replayed_to_the_next_one() {
    let (world, _) = world();
    let join = join_in_background(&world, "Home");
    eventually("NetworkManager is waiting on the agent", || {
        world.nm.secrets_pending()
    });

    // A Front end that shows up later still gets the open prompt first.
    let mut fe = front_end(&world);
    let opened = fe.next_event("secret_prompt_opened");
    assert_eq!(opened["prompt"]["network"], "Home");
    assert!(
        answer(&mut fe, &opened, "right-password")
            .get("ok")
            .is_some()
    );
    assert_eq!(join.join().unwrap()["ok"]["ssid"], "Home");
}

#[test]
fn networkmanager_cancelling_the_request_closes_the_prompt() {
    let (world, wlan0) = world();
    let mut fe = front_end(&world);
    let join = join_in_background(&world, "Home");
    let opened = fe.next_event("secret_prompt_opened");

    world.nm.cancel_pending_secrets();
    let closed = fe.next_event("secret_prompt_closed");
    assert_eq!(closed["id"], opened["prompt"]["id"]);
    assert_eq!(closed["reason"], "withdrawn");
    let joined = join.join().unwrap();
    assert!(joined.get("err").is_some(), "{joined}");
    assert_eq!(world.nm.profile_count(), 0);
    assert_eq!(world.nm.joined_ssid(wlan0), None);
}

#[test]
fn the_daemon_registers_again_when_networkmanager_restarts() {
    let (mut world, _) = world();
    eventually("the first registration", || world.nm.agent_registered());
    world.nm.restart();
    // The fake forgets its agent on restart, so seeing one again means the
    // Daemon registered anew.
    eventually("registration after the restart", || {
        world.nm.agent_registered()
    });
}

#[test]
fn answering_a_prompt_that_never_existed_is_a_plain_error() {
    let (world, _) = world();
    let mut wire = world.wire();
    let resp = wire.call(json!({"op": "secret_answer", "prompt": 999, "secret": "x"}));
    assert_eq!(resp["err"]["code"], "unknown_prompt", "{resp}");
    let resp = wire.call(json!({"op": "secret_cancel", "prompt": 999}));
    assert_eq!(resp["err"]["code"], "unknown_prompt", "{resp}");
}

#[test]
fn secrets_never_reach_the_log_or_an_event() {
    let (world, _) = world();
    let mut fe = front_end(&world);
    let join = join_in_background(&world, "Home");
    let first = fe.next_event("secret_prompt_opened");
    let mut seen = vec![first.to_string()];
    answer(&mut fe, &first, "typo-hunter2-secret");
    seen.push(fe.next_event("secret_prompt_closed").to_string());
    let again = fe.next_event("secret_prompt_opened");
    seen.push(again.to_string());
    answer(&mut fe, &again, "right-password");
    seen.push(fe.next_event("secret_prompt_closed").to_string());
    join.join().unwrap();
    for secret in ["typo-hunter2-secret", "right-password"] {
        assert!(!world.daemon_log().contains(secret), "log leaked {secret}");
        assert!(
            seen.iter().all(|line| !line.contains(secret)),
            "an event leaked {secret}"
        );
    }
}

// ---- the CLI, on a real terminal ----

/// A pseudo-terminal: the master end the test types on, the slave end for the
/// child's stdin.
fn pty() -> (std::fs::File, std::os::fd::OwnedFd) {
    use rustix::fs::{Mode, OFlags};
    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).expect("openpt");
    grantpt(&master).expect("grantpt");
    unlockpt(&master).expect("unlockpt");
    let name = ptsname(&master, Vec::new()).expect("ptsname");
    let slave = rustix::fs::open(
        name.as_c_str(),
        OFlags::RDWR | OFlags::NOCTTY,
        Mode::empty(),
    )
    .expect("open the slave side");
    (std::fs::File::from(master), slave)
}

/// Reads `from` until `needle` shows up in what has been read so far.
fn read_until(from: &mut impl std::io::Read, seen: &mut String, needle: &str) {
    let mut chunk = [0u8; 256];
    while !seen.contains(needle) {
        let n = from.read(&mut chunk).expect("read the child's stderr");
        assert!(n > 0, "the child ended before printing `{needle}`: {seen}");
        seen.push_str(&String::from_utf8_lossy(&chunk[..n]));
    }
}

#[test]
fn the_cli_asks_on_the_terminal_with_the_password_masked() {
    use std::io::{Read as _, Write as _};
    use std::process::Stdio;
    let (world, wlan0) = world();
    let (mut master, slave) = pty();
    let mut child = {
        let mut cmd = world.nexus();
        cmd.args(["wifi", "join", "Home"])
            .stdin(Stdio::from(slave))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.spawn().expect("spawn nexus")
        // `cmd` drops here, closing our copy of the slave end.
    };
    let mut stderr = child.stderr.take().unwrap();
    let mut seen = String::new();

    read_until(&mut stderr, &mut seen, "Password for Home: ");
    master.write_all(b"typo-password\n").unwrap();
    read_until(
        &mut stderr,
        &mut seen,
        "Wrong password for Home. Try again.: ",
    );
    master.write_all(b"right-password\n").unwrap();

    let status = child.wait().expect("wait");
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    assert_eq!(status.code(), Some(0), "{seen}");
    assert!(out.contains("Joined Home"), "{out}");
    assert_eq!(world.nm.joined_ssid(wlan0).as_deref(), Some("Home"));

    // Nothing the person typed came back out of the terminal.
    let mut echoed = Vec::new();
    let _ = master.read_to_end(&mut echoed); // EIO once the child has gone
    let echoed = String::from_utf8_lossy(&echoed);
    assert!(
        !echoed.contains("password"),
        "the terminal echoed: {echoed:?}"
    );
    assert!(!seen.contains("right-password") && !out.contains("right-password"));
}

#[test]
fn an_empty_answer_on_the_terminal_cancels_the_join() {
    use std::io::Write as _;
    use std::process::Stdio;
    let (world, _) = world();
    let (mut master, slave) = pty();
    let mut child = {
        let mut cmd = world.nexus();
        cmd.args(["wifi", "join", "Home"])
            .stdin(Stdio::from(slave))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.spawn().expect("spawn nexus")
    };
    let mut stderr = child.stderr.take().unwrap();
    let mut seen = String::new();
    read_until(&mut stderr, &mut seen, "Password for Home: ");
    master.write_all(b"\n").unwrap();
    let status = child.wait().expect("wait");
    assert_eq!(status.code(), Some(10), "{seen}");
    assert_eq!(world.nm.profile_count(), 0);
}

#[test]
fn a_wrong_password_given_with_the_join_fails_without_opening_a_prompt() {
    let (world, wlan0) = world();
    let mut fe = front_end(&world);
    let resp = world.wire().call(json!({
        "op": "wifi_join", "network": id("Home"), "password": "typo-password"
    }));
    assert_eq!(resp["err"]["code"], "wrong_password", "{resp}");
    assert_eq!(resp["err"]["message"], "Wrong password for Home.");
    while let Some(ev) = fe.try_read_line(std::time::Duration::from_millis(300)) {
        assert_ne!(
            ev["event"], "secret_prompt_opened",
            "no prompt may open for a password the caller supplied: {ev}"
        );
    }
    assert_eq!(world.nm.profile_count(), 0);
    assert_eq!(world.nm.joined_ssid(wlan0), None);
}
