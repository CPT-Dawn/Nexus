//! Seam S1 for the Waybar output and the launcher menu: a real `nexus
//! daemon` against a fake NetworkManager, the real CLI, and a fake launcher
//! (a shell script named `fuzzel`, `rofi`, ...) first on the PATH.

use std::io::{BufRead as _, BufReader};
use std::process::{Child, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use nexus_testkit::{ActiveConnection, Ap, NmConnectivity, NmState, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");
const WAIT: Duration = Duration::from_secs(15);

fn go_online(world: &World) {
    world.nm.set_primary(Some(ActiveConnection::wifi("Home")));
    world.nm.set_connectivity(NmConnectivity::Full);
    world.nm.set_state(NmState::ConnectedGlobal);
}

fn waybar_once(world: &World) -> (Value, Option<i32>) {
    let out = world.run_nexus(&["status", "--waybar"]);
    let line = String::from_utf8_lossy(&out.stdout);
    (
        serde_json::from_str(line.trim()).unwrap_or_else(|e| panic!("{e}: {line:?}")),
        out.status.code(),
    )
}

/// `nexus status --waybar --follow`, its lines arriving on a channel.
struct Follow {
    child: Child,
    lines: Receiver<Value>,
}

impl Follow {
    fn start(world: &World) -> Self {
        let mut child = world
            .nexus()
            .args(["status", "--waybar", "--follow"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn follow");
        let stdout = child.stdout.take().expect("piped");
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str(&line)
                    && tx.send(v).is_err()
                {
                    break;
                }
            }
        });
        Self { child, lines }
    }

    /// The next line that satisfies `want`; panics after `WAIT`.
    fn until(&self, what: &str, want: impl Fn(&Value) -> bool) -> Value {
        let mut seen = Vec::new();
        loop {
            match self.lines.recv_timeout(WAIT) {
                Ok(v) if want(&v) => return v,
                Ok(v) => seen.push(v),
                Err(_) => panic!("no line with {what}; saw {seen:?}"),
            }
        }
    }
}

impl Drop for Follow {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn has_class(v: &Value, class: &str) -> bool {
    v["class"]
        .as_array()
        .is_some_and(|c| c.iter().any(|c| c == class))
}

#[test]
fn waybar_prints_one_json_object_with_the_four_fields() {
    let world = World::start(NEXUS);
    go_online(&world);
    let (v, code) = waybar_once(&world);
    assert_eq!(code, Some(0));
    assert_eq!(v["text"], "Home");
    assert_eq!(v["alt"], "online");
    assert_eq!(v["class"], json!(["online", "wifi"]));
    assert_eq!(v["tooltip"], "Online via Home");
}

#[test]
fn waybar_without_a_daemon_says_unavailable_and_exits_3() {
    let world = World::builder(NEXUS).daemon(false).start();
    let (v, code) = waybar_once(&world);
    assert_eq!(code, Some(3));
    assert_eq!(v["alt"], "unavailable");
}

#[test]
fn follow_streams_a_line_per_state_change() {
    let world = World::start(NEXUS);
    go_online(&world);
    let follow = Follow::start(&world);
    follow.until("online", |v| v["alt"] == "online" && v["text"] == "Home");
    world.nm.set_primary(None);
    world.nm.set_state(NmState::Disconnected);
    let off = follow.until("offline", |v| v["alt"] == "offline");
    assert_eq!(off["text"], "Offline");
}

#[test]
fn follow_shows_a_vpn_coming_up_and_going_down() {
    let world = World::start(NEXUS);
    go_online(&world);
    let conf = "[Interface]\nPrivateKey = dwdtCnMYpX08FsFyUbJmRd9ML4frwJkqsXf7pR25LCo=\nAddress = 10.0.0.2/32\n\n[Peer]\nPublicKey = xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=\nAllowedIPs = 0.0.0.0/0\nEndpoint = vpn.example.net:51820\n";
    let added = world
        .wire()
        .call(json!({"op": "vpn_import", "name": "Work", "config": conf}));
    let id = added["ok"]["id"].as_str().expect("imported").to_string();
    let follow = Follow::start(&world);
    follow.until("plain online", |v| v["alt"] == "online");
    let up = world.wire().call(json!({"op": "vpn_up", "vpn": id}));
    assert!(up.get("ok").is_some(), "{up}");
    let line = follow.until("the vpn", |v| has_class(v, "vpn"));
    assert_eq!(line["alt"], "vpn");
    assert!(line["tooltip"].as_str().unwrap().contains("VPN Work: up"));
    world.wire().call(json!({"op": "vpn_down", "vpn": id}));
    follow.until("the vpn gone", |v| !has_class(v, "vpn"));
}

#[test]
fn follow_shows_a_running_hotspot() {
    let world = World::start(NEXUS);
    world.nm.add_wifi_device("wlan0");
    go_online(&world);
    let follow = Follow::start(&world);
    follow.until("no hotspot", |v| v["alt"] == "online");
    let started = world.wire().call(json!({
        "op": "hotspot_start", "ssid": "Garage", "password": "open sesame 42"
    }));
    assert!(started.get("ok").is_some(), "{started}");
    let line = follow.until("the hotspot", |v| has_class(v, "hotspot"));
    assert!(line["tooltip"].as_str().unwrap().contains("Hotspot Garage"));
}

#[test]
fn follow_survives_the_daemon_going_away_and_coming_back() {
    let mut world = World::start(NEXUS);
    go_online(&world);
    let follow = Follow::start(&world);
    follow.until("online", |v| v["alt"] == "online");
    world.stop_daemon();
    follow.until("unavailable", |v| v["alt"] == "unavailable");
    world.start_daemon();
    follow.until("online again", |v| v["alt"] == "online");
}

// --- menu ---

/// Puts a fake launcher called `name` on the PATH. It records its arguments,
/// prints the line of the menu that contains `$TMP/pick` (the list is on
/// stdin), and in hidden-input mode prints the next line of `$TMP/answers`.
fn fake_launcher(world: &World, name: &str) {
    world.install_tool(
        name,
        r#"d="$(dirname "$0")/.."
echo "$(basename "$0") $*" >> "$d/launcher.log"
case " $* " in
*" --password "*|*" -password "*|*" --hide-input=true "*)
  head -n1 "$d/answers"; sed -i 1d "$d/answers"; exit 0;;
esac
cat > "$d/menu.txt"
grep -F -m1 -- "$(cat "$d/pick")" "$d/menu.txt""#,
    );
}

/// Writes the config and waits until the Daemon has it.
fn configure(world: &World, text: &str) {
    let mut events = world.wire();
    events.subscribe();
    world.write_config(text);
    events.next_event("config_reloaded");
}

fn tmp(world: &World) -> std::path::PathBuf {
    world.home().parent().expect("tmp").to_owned()
}

fn pick(world: &World, line: &str, answers: &[&str]) {
    std::fs::write(tmp(world).join("pick"), line).unwrap();
    std::fs::write(tmp(world).join("answers"), answers.join("\n") + "\n").unwrap();
}

fn launcher_log(world: &World) -> String {
    std::fs::read_to_string(tmp(world).join("launcher.log")).unwrap_or_default()
}

fn menu_world() -> (World, nexus_testkit::DeviceId) {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:01", 80));
    world
        .nm
        .add_ap(wlan0, Ap::open("Cafe", "BB:00:00:00:00:01", 60));
    (world, wlan0)
}

#[test]
fn menu_lists_networks_and_vpns_and_joins_an_open_network() {
    let (world, wlan0) = menu_world();
    fake_launcher(&world, "fuzzel");
    pick(&world, "Wi-Fi  Cafe", &[]);
    let out = world.run_nexus(&["menu"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(world.nm.joined_ssid(wlan0).as_deref(), Some("Cafe"));
    let menu = std::fs::read_to_string(tmp(&world).join("menu.txt")).unwrap();
    assert!(menu.contains("Wi-Fi  Home  80%"), "{menu}");
    assert!(menu.contains("Wi-Fi  Cafe  60%"), "{menu}");
    assert!(launcher_log(&world).contains("fuzzel --dmenu"));
}

#[test]
fn menu_asks_for_a_new_networks_password_in_the_launchers_password_mode() {
    let (world, wlan0) = menu_world();
    world.nm.set_psk("Home", "hunter2!!");
    fake_launcher(&world, "fuzzel");
    pick(&world, "Wi-Fi  Home", &["nope-nope", "hunter2!!"]);
    let out = world.run_nexus(&["menu"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(world.nm.joined_ssid(wlan0).as_deref(), Some("Home"));
    let log = launcher_log(&world);
    assert_eq!(
        log.matches("--password").count(),
        2,
        "a wrong password asks again: {log}"
    );
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("hunter2"),
        "the password is never printed"
    );
}

#[test]
fn cancelling_the_password_prompt_joins_nothing() {
    let (world, wlan0) = menu_world();
    fake_launcher(&world, "fuzzel");
    pick(&world, "Wi-Fi  Home", &[""]);
    let out = world.run_nexus(&["menu"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(world.nm.joined_ssid(wlan0), None);
}

#[test]
fn picking_the_joined_network_disconnects_it() {
    let (world, wlan0) = menu_world();
    let joined = world
        .wire()
        .call(json!({"op": "wifi_join", "network": nexus_ipc::network_id(b"Cafe")}));
    assert!(joined.get("ok").is_some(), "{joined}");
    fake_launcher(&world, "fuzzel");
    pick(&world, "Wi-Fi  Cafe  connected", &[]);
    let out = world.run_nexus(&["menu"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(world.nm.joined_ssid(wlan0), None);
}

#[test]
fn menu_toggles_a_vpn() {
    let (world, _) = menu_world();
    let conf = "[Interface]\nPrivateKey = dwdtCnMYpX08FsFyUbJmRd9ML4frwJkqsXf7pR25LCo=\nAddress = 10.0.0.2/32\n\n[Peer]\nPublicKey = xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=\nAllowedIPs = 0.0.0.0/0\nEndpoint = vpn.example.net:51820\n";
    let added = world
        .wire()
        .call(json!({"op": "vpn_import", "name": "Work", "config": conf}));
    let uuid = added["ok"]["id"].as_str().unwrap().to_string();
    fake_launcher(&world, "fuzzel");
    pick(&world, "VPN  Work  down", &[]);
    let out = world.run_nexus(&["menu"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(world.nm.vpn_active_state(&uuid), 2, "the VPN is activated");
    pick(&world, "VPN  Work  up", &[]);
    let out = world.run_nexus(&["menu"]);
    assert!(out.status.success(), "{out:?}");
    let vpns = world.wire().call(json!({"op": "vpn_list"}));
    assert_eq!(vpns["ok"]["vpns"][0]["state"], "down");
}

#[test]
fn the_config_picks_the_launcher_over_detection() {
    let (world, wlan0) = menu_world();
    fake_launcher(&world, "fuzzel");
    fake_launcher(&world, "rofi");
    configure(&world, "[menu]\nlauncher = \"rofi\"\n");
    pick(&world, "Wi-Fi  Cafe", &[]);
    let out = world.run_nexus(&["menu"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(world.nm.joined_ssid(wlan0).as_deref(), Some("Cafe"));
    let log = launcher_log(&world);
    assert!(log.contains("rofi -dmenu"), "{log}");
    assert!(!log.contains("fuzzel"), "{log}");
}

#[test]
fn detection_prefers_fuzzel_then_walker_then_rofi_then_tofi() {
    let (world, _) = menu_world();
    fake_launcher(&world, "tofi");
    fake_launcher(&world, "walker");
    pick(&world, "Wi-Fi  Cafe", &[]);
    let out = world.run_nexus(&["menu"]);
    assert!(out.status.success(), "{out:?}");
    assert!(launcher_log(&world).contains("walker --dmenu"));
}

#[test]
fn no_launcher_exits_49_and_a_strange_one_in_the_config_exits_51() {
    let (world, _) = menu_world();
    let out = world
        .nexus()
        .env("PATH", "/nonexistent")
        .arg("menu")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(49), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("no launcher found"));
    configure(&world, "[menu]\nlauncher = \"dmenu\"\n");
    let out = world.run_nexus(&["menu"]);
    assert_eq!(out.status.code(), Some(51), "{out:?}");
}
