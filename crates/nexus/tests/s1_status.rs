//! Seam S1: a real `nexus daemon` against a fake NetworkManager, driven over
//! the socket and through the `nexus status` command.

use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use nexus_testkit::{ActiveConnection, NmConnectivity, NmState, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

fn status(world: &World) -> Value {
    world.wire().call(json!({"op": "status"}))
}

fn cli_json(world: &World) -> Value {
    let out = world.run_nexus(&["status", "--json"]);
    assert!(out.status.success(), "{out:?}");
    serde_json::from_slice(&out.stdout).expect("status --json prints JSON")
}

fn go_online(world: &World) {
    world.nm.set_primary(Some(ActiveConnection::wifi("Home")));
    world.nm.set_connectivity(NmConnectivity::Full);
    world.nm.set_state(NmState::ConnectedGlobal);
}

#[test]
fn online_status_names_the_primary_link() {
    let world = World::start(NEXUS);
    go_online(&world);

    let resp = status(&world);
    assert_eq!(resp["v"], 1);
    assert_eq!(resp["ok"]["state"], "online");
    assert_eq!(resp["ok"]["connectivity"], "full");
    assert_eq!(resp["ok"]["link"], json!({"name": "Home", "kind": "wifi"}));
    assert_eq!(resp["ok"]["backend"]["name"], "networkmanager");
    assert_eq!(resp["ok"]["backend"]["version"], "1.58.1");

    assert_eq!(cli_json(&world)["state"], "online");
    let human = world.run_nexus(&["status"]);
    assert_eq!(String::from_utf8_lossy(&human.stdout), "online via Home\n");
}

#[test]
fn ethernet_link_kind() {
    let world = World::start(NEXUS);
    world
        .nm
        .set_primary(Some(ActiveConnection::ethernet("Wired")));
    world.nm.set_state(NmState::ConnectedGlobal);
    assert_eq!(status(&world)["ok"]["link"]["kind"], "ethernet");
}

#[test]
fn offline_status_has_no_link() {
    let world = World::start(NEXUS);
    let resp = status(&world);
    assert_eq!(resp["ok"]["state"], "offline");
    assert!(resp["ok"].get("link").is_none(), "{resp}");
    let human = world.run_nexus(&["status"]);
    assert_eq!(String::from_utf8_lossy(&human.stdout), "offline\n");
}

#[test]
fn connecting_status() {
    let world = World::start(NEXUS);
    world.nm.set_state(NmState::Connecting);
    assert_eq!(status(&world)["ok"]["state"], "connecting");
}

#[test]
fn portal_connectivity_is_reported() {
    let world = World::start(NEXUS);
    go_online(&world);
    world.nm.set_connectivity(NmConnectivity::Portal);
    assert_eq!(status(&world)["ok"]["connectivity"], "portal");
}

#[test]
fn missing_networkmanager_is_a_state_not_a_crash() {
    let world = World::builder(NEXUS).nm_running(false).start();
    assert_eq!(status(&world)["ok"]["state"], "unavailable");
    let human = world.run_nexus(&["status"]);
    assert_eq!(
        String::from_utf8_lossy(&human.stdout),
        "NetworkManager is not running\n"
    );
}

#[test]
fn networkmanager_restart_is_followed() {
    let mut world = World::start(NEXUS);
    go_online(&world);
    assert_eq!(status(&world)["ok"]["state"], "online");

    world.nm.stop();
    assert_eq!(status(&world)["ok"]["state"], "unavailable");

    world.nm.restart();
    assert_eq!(status(&world)["ok"]["state"], "online");
}

#[test]
fn malformed_requests_get_coded_errors_and_the_connection_lives_on() {
    let world = World::start(NEXUS);
    let mut wire = world.wire();
    assert_eq!(wire.raw("garbage")["err"]["code"], "bad_request");
    assert_eq!(
        wire.raw(r#"{"v":7,"id":3,"op":"status"}"#)["err"]["code"],
        "unsupported_version"
    );
    let unknown = wire.raw(r#"{"v":1,"id":4,"op":"teleport"}"#);
    assert_eq!(unknown["err"]["code"], "unknown_op");
    assert_eq!(unknown["id"], 4);
    let extra = wire.raw(r#"{"v":1,"id":5,"op":"status","future":true}"#);
    assert_eq!(extra["id"], 5);
    assert!(extra.get("ok").is_some(), "{extra}");
}

#[test]
fn the_socket_directory_is_private() {
    let world = World::start(NEXUS);
    let dir = world.socket().parent().unwrap().to_owned();
    let mode = std::fs::metadata(dir).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[test]
fn unreachable_daemon_gets_a_clear_message_and_exit_code_3() {
    let world = World::builder(NEXUS).daemon(false).start();
    let out = world.run_nexus(&["status"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("daemon"), "stderr was: {err}");
    assert!(out.stdout.is_empty());
}

#[test]
fn socket_activation_starts_the_daemon_on_first_connect() {
    let Ok(probe) = Command::new("systemd-socket-activate")
        .arg("--version")
        .output()
    else {
        eprintln!("skipped: systemd-socket-activate not installed");
        return;
    };
    drop(probe);
    let mut world = World::builder(NEXUS).daemon(false).start();
    world.nm.set_state(NmState::Connecting);
    let socket = world.socket();
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    std::fs::set_permissions(
        socket.parent().unwrap(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    // systemd-socket-activate owns the listening socket and passes it as fd 3
    // (LISTEN_FDS/LISTEN_PID), exactly like nexus.socket does.
    // The activated child only gets the environment named with -E.
    let mut cmd = Command::new("systemd-socket-activate");
    cmd.env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default());
    for (k, v) in world.nexus().get_envs() {
        if let Some(v) = v {
            let mut kv = k.to_owned();
            kv.push("=");
            kv.push(v);
            cmd.arg("-E").arg(kv);
        }
    }
    cmd.arg("-l").arg(&socket).arg(NEXUS).arg("daemon");
    let child = cmd
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn systemd-socket-activate");
    world.adopt_daemon_group(child);
    assert_eq!(status(&world)["ok"]["state"], "connecting");
}

/// Needs a second user: set `NEXUS_TEST_INTRUDER=<user>` and passwordless
/// `sudo -u <user>` with `socat` installed (CI creates such a user).
#[test]
fn connections_from_another_uid_are_refused() {
    let Ok(intruder) = std::env::var("NEXUS_TEST_INTRUDER") else {
        eprintln!("skipped: set NEXUS_TEST_INTRUDER=<user> to run the peer-credential test");
        return;
    };
    let world = World::start(NEXUS);
    // Open every door so only the uid check is under test.
    let socket = world.socket();
    for path in [
        world.runtime_dir().parent().unwrap().to_owned(),
        world.runtime_dir(),
        socket.parent().unwrap().into(),
        socket.clone(),
    ] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o777)).unwrap();
    }
    let ask = |user: Option<&str>| {
        let mut cmd = match user {
            Some(u) => {
                let mut c = Command::new("sudo");
                c.args(["-n", "-u", u, "socat", "-t", "2", "-"]);
                c
            }
            None => {
                let mut c = Command::new("socat");
                c.args(["-t", "2", "-"]);
                c
            }
        };
        cmd.arg(format!("UNIX-CONNECT:{}", socket.display()));
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn socat");
        std::io::Write::write_all(
            child.stdin.as_mut().unwrap(),
            b"{\"v\":1,\"id\":1,\"op\":\"status\"}\n",
        )
        .unwrap();
        drop(child.stdin.take());
        let out = child.wait_with_output().unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    // socat succeeds only if it reached the socket: the Daemon hung up on it.
    let (connected, reply) = ask(Some(&intruder));
    assert!(connected, "the intruder could not even reach the socket");
    assert_eq!(reply, "", "another uid must get silence");
    let (_, reply) = ask(None);
    assert!(reply.contains("\"ok\""), "our own uid must still work");
}
