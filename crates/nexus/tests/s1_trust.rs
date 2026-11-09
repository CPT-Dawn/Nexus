//! Seam S1 for Trust and the policies it drives: a real Daemon, a fake
//! NetworkManager, a fake notification server. Joining an Untrusted Network
//! brings up the chosen VPN (after any Portal login) and can mark the
//! Network metered; a Trusted one is left alone.

use std::time::{Duration, Instant};

use nexus_testkit::{ActiveConnection, Ap, DeviceId, NmConnectivity, NmState, Wire, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");
const ACTIVATED: u32 = 2;
const CONF: &str = "[Interface]\nPrivateKey = dwdtCnMYpX08FsFyUbJmRd9ML4frwJkqsXf7pR25LCo=\nAddress = 10.0.0.2/32\n\n[Peer]\nPublicKey = xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=\nAllowedIPs = 0.0.0.0/0\nEndpoint = vpn.example.net:51820\n";
const QUIET: &str = "[notifications]\njoined = false\nlost = false\n";

fn id(ssid: &str) -> String {
    nexus_ipc::network_id(ssid.as_bytes())
}

fn world(config: &str) -> (World, DeviceId) {
    let mut world = World::builder(NEXUS)
        .notifications(true)
        .daemon(false)
        .start();
    world.write_config(config);
    world.start_daemon();
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:00:01", 80));
    world
        .nm
        .add_ap(wlan0, Ap::open("Cafe", "BB:00:00:00:00:01", 60));
    let added = world
        .wire()
        .call(json!({"op": "vpn_import", "name": "Work", "config": CONF}));
    assert!(added["ok"].is_object(), "{added}");
    (world, wlan0)
}

fn join(world: &World, ssid: &str, password: Option<&str>) -> Value {
    let mut op = json!({"op": "wifi_join", "network": id(ssid)});
    if let Some(p) = password {
        op["password"] = json!(p);
    }
    world.wire().call(op)
}

/// What NetworkManager does once the join is up: the Network becomes the
/// primary connection and its connectivity check answers.
fn go_online(world: &World, ssid: &str, connectivity: NmConnectivity) {
    world.nm.set_primary(Some(ActiveConnection::wifi(ssid)));
    world.nm.set_connectivity(connectivity);
    world.nm.set_state(NmState::ConnectedGlobal);
}

fn network(world: &World, ssid: &str) -> Value {
    let list = world.wire().call(json!({"op": "wifi_list"}));
    list["ok"]["networks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["ssid"] == ssid)
        .cloned()
        .unwrap_or_else(|| panic!("no {ssid} in {list}"))
}

fn vpn_state(world: &World) -> u32 {
    world
        .nm
        .vpn_active_state(&world.nm.vpn_uuid("Work").unwrap())
}

fn wait_vpn_up(world: &World) {
    let start = Instant::now();
    while vpn_state(world) != ACTIVATED {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the VPN never came up"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A Front end that sees the Daemon's events, subscribed before anything happens.
fn front_end(world: &World) -> Wire {
    let mut fe = world.wire();
    fe.subscribe();
    fe
}

/// Proof that the Daemon has re-read and applied everything NetworkManager did
/// before this call: it handles changes in order, so once a change made now
/// (Home's signal, to `marker`) is pushed, the Trust policy has already decided
/// about the earlier ones. Use a new `marker` for every call on one world.
fn sentinel(world: &World, fe: &mut Wire, marker: u8) {
    world.nm.set_strength_of("Home", marker);
    loop {
        let ev = fe.next_event("networks_changed");
        let seen = ev["wifi"]["networks"].as_array().is_some_and(|nets| {
            nets.iter()
                .any(|n| n["ssid"] == "Home" && n["signal"] == marker)
        });
        if seen {
            return;
        }
    }
}

#[test]
fn unknown_open_networks_start_untrusted_and_secured_ones_trusted() {
    let (world, _) = world(QUIET);
    assert_eq!(network(&world, "Cafe")["trust"], "untrusted");
    assert_eq!(network(&world, "Home")["trust"], "trusted");
}

#[test]
fn trust_set_overrides_the_default_and_is_remembered() {
    let (mut world, _) = world(QUIET);
    let set = world
        .wire()
        .call(json!({"op": "trust_set", "network": id("Cafe"), "trust": "trusted"}));
    assert_eq!(
        set["ok"],
        json!({"ssid": "Cafe", "trust": "trusted"}),
        "{set}"
    );
    assert_eq!(network(&world, "Cafe")["trust"], "trusted");
    let set = world
        .wire()
        .call(json!({"op": "trust_set", "network": id("Home"), "trust": "untrusted"}));
    assert!(set["ok"].is_object(), "{set}");
    assert_eq!(network(&world, "Home")["trust"], "untrusted");

    // The marks outlive the Daemon.
    world.stop_daemon();
    world.start_daemon();
    assert_eq!(network(&world, "Cafe")["trust"], "trusted");
    assert_eq!(network(&world, "Home")["trust"], "untrusted");
}

#[test]
fn trust_set_refuses_a_network_out_of_the_list_and_tells_subscribers() {
    let (world, _) = world(QUIET);
    let no = world
        .wire()
        .call(json!({"op": "trust_set", "network": id("Nowhere"), "trust": "trusted"}));
    assert_eq!(no["err"]["code"], "unknown_network", "{no}");

    let mut events = world.wire();
    events.subscribe();
    world
        .wire()
        .call(json!({"op": "trust_set", "network": id("Cafe"), "trust": "trusted"}));
    let changed = events.next_event("networks_changed");
    let cafe = changed["wifi"]["networks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["ssid"] == "Cafe")
        .unwrap();
    assert_eq!(cafe["trust"], "trusted");
}

#[test]
fn joining_an_untrusted_network_brings_up_the_chosen_vpn_and_says_so() {
    let (world, _) = world(&format!("{QUIET}[trust]\nauto_vpn = \"Work\"\n"));
    let notes = world.notifications.as_ref().unwrap();
    assert_eq!(join(&world, "Cafe", None)["ok"]["ssid"], "Cafe");
    go_online(&world, "Cafe", NmConnectivity::Full);
    wait_vpn_up(&world);
    let note = notes.notified(1);
    assert_eq!(note.summary, "Cafe is untrusted");
    assert_eq!(note.body, "Brought up the VPN Work.");
}

#[test]
fn the_vpn_waits_for_the_portal_login() {
    let (world, _) = world(&format!("{QUIET}[trust]\nauto_vpn = \"Work\"\n"));
    let mut fe = front_end(&world);
    join(&world, "Cafe", None);
    go_online(&world, "Cafe", NmConnectivity::Portal);
    sentinel(&world, &mut fe, 41);
    assert_ne!(vpn_state(&world), ACTIVATED, "still behind the Portal");
    world.nm.set_connectivity(NmConnectivity::Full);
    wait_vpn_up(&world);
}

#[test]
fn a_trusted_network_is_left_untouched() {
    let (world, _) = world(&format!(
        "{QUIET}[trust]\nauto_vpn = \"Work\"\nmetered = true\n"
    ));
    let mut fe = front_end(&world);
    join(&world, "Home", Some("hunter22hunter22"));
    go_online(&world, "Home", NmConnectivity::Full);
    sentinel(&world, &mut fe, 41);
    assert_ne!(vpn_state(&world), ACTIVATED);
    let notes = world.notifications.as_ref().unwrap();
    assert_eq!(notes.count(), 0, "{:?}", notes.all());
    let uuid = network(&world, "Home")["profile"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(world.nm.profile_int(&uuid, "connection", "metered"), None);

    // Marked Trusted by hand, an open Network is left alone too.
    world
        .wire()
        .call(json!({"op": "trust_set", "network": id("Cafe"), "trust": "trusted"}));
    join(&world, "Cafe", None);
    go_online(&world, "Cafe", NmConnectivity::Full);
    sentinel(&world, &mut fe, 42);
    assert_ne!(vpn_state(&world), ACTIVATED);
    assert_eq!(notes.count(), 0, "{:?}", notes.all());
}

#[test]
fn an_untrusted_network_can_be_marked_metered() {
    let (world, _) = world(&format!("{QUIET}[trust]\nmetered = true\n"));
    let notes = world.notifications.as_ref().unwrap();
    join(&world, "Cafe", None);
    go_online(&world, "Cafe", NmConnectivity::Full);
    let note = notes.notified(1);
    assert_eq!(note.body, "Marked the network metered.");
    let uuid = network(&world, "Cafe")["profile"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        world.nm.profile_int(&uuid, "connection", "metered"),
        Some(1)
    );
    assert_ne!(vpn_state(&world), ACTIVATED, "no VPN was asked for");
}

#[test]
fn policy_notifications_can_be_switched_off_and_the_activity_log_keeps_the_line() {
    let (world, _) = world(
        "[notifications]\njoined = false\nlost = false\npolicy = false\n[trust]\nauto_vpn = \"Work\"\n",
    );
    let mut fe = front_end(&world);
    join(&world, "Cafe", None);
    go_online(&world, "Cafe", NmConnectivity::Full);
    wait_vpn_up(&world);
    let notes = world.notifications.as_ref().unwrap();
    // The policy's activity line comes right before its (switched off) toast.
    loop {
        let ev = fe.next_event("activity");
        if ev["entry"]["text"]
            .as_str()
            .is_some_and(|t| t.contains("brought up the VPN"))
        {
            break;
        }
    }
    sentinel(&world, &mut fe, 41);
    assert_eq!(notes.count(), 0, "{:?}", notes.all());
    let log = world.wire().call(json!({"op": "activity"}));
    let lines: Vec<&str> = log["ok"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["text"].as_str())
        .collect();
    assert!(
        lines.contains(&"Cafe is untrusted: brought up the VPN Work."),
        "{lines:?}"
    );
}

#[test]
fn the_cli_lists_and_sets_trust() {
    let (world, _) = world(QUIET);
    let list = world.run_nexus(&["trust", "list"]);
    let text = String::from_utf8_lossy(&list.stdout).into_owned();
    assert!(list.status.success(), "{list:?}");
    assert!(
        text.contains("Cafe") && text.contains("Untrusted"),
        "{text}"
    );
    assert!(text.contains("Home") && text.contains("Trusted"), "{text}");

    let set = world.run_nexus(&["trust", "set", "Cafe", "trusted"]);
    assert!(set.status.success(), "{set:?}");
    assert_eq!(
        String::from_utf8_lossy(&set.stdout).trim(),
        "Cafe is now trusted."
    );
    assert_eq!(network(&world, "Cafe")["trust"], "trusted");

    let gone = world.run_nexus(&["trust", "set", "Nowhere", "untrusted"]);
    assert_eq!(gone.status.code(), Some(9), "{gone:?}");
}
