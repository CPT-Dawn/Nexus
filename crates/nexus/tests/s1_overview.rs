//! Seam S1 for the Overview's data: Link traffic rates and recent activity,
//! from a real Daemon against a fake NetworkManager.

use std::time::{Duration, Instant};

use nexus_testkit::{ActiveConnection, NmConnectivity, NmState, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

fn go_online(world: &World, link: ActiveConnection) {
    world.nm.set_primary(Some(link));
    world.nm.set_connectivity(NmConnectivity::Full);
    world.nm.set_state(NmState::ConnectedGlobal);
}

fn traffic(wire: &mut nexus_testkit::Wire) -> Value {
    let resp = wire.call(json!({"op": "traffic"}));
    assert!(resp.get("ok").is_some(), "traffic failed: {resp}");
    resp["ok"].clone()
}

fn activity(wire: &mut nexus_testkit::Wire) -> Vec<Value> {
    let resp = wire.call(json!({"op": "activity"}));
    resp["ok"]["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("activity failed: {resp}"))
        .clone()
}

fn texts(entries: &[Value]) -> Vec<&str> {
    entries
        .iter()
        .map(|e| e["text"].as_str().unwrap())
        .collect()
}

#[test]
fn traffic_is_zero_without_a_link() {
    let world = World::start(NEXUS);
    let t = traffic(&mut world.wire());
    assert_eq!(t, json!({"rx_bps": 0, "tx_bps": 0}));
}

#[test]
fn traffic_rates_follow_the_primary_links_counters() {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    go_online(&world, ActiveConnection::wifi("Home"));
    let mut wire = world.wire();

    // The first sample is only a baseline.
    assert_eq!(traffic(&mut wire), json!({"rx_bps": 0, "tx_bps": 0}));

    world.nm.set_traffic(wlan0, 1_000_000, 100_000);
    let deadline = Instant::now() + Duration::from_secs(10);
    let rates = loop {
        let t = traffic(&mut wire);
        if t["rx_bps"].as_u64().unwrap() > 0 {
            break t;
        }
        assert!(Instant::now() < deadline, "no rate appeared: {t}");
        std::thread::sleep(Duration::from_millis(100));
    };
    let (rx, tx) = (
        rates["rx_bps"].as_u64().unwrap(),
        rates["tx_bps"].as_u64().unwrap(),
    );
    // 1 MB over at least 0.8 s, and ten times the upload.
    assert!(rx <= 1_250_000, "rx {rx}");
    assert!(tx > 0 && tx * 5 < rx, "rx {rx} tx {tx}");
}

#[test]
fn the_daemon_asks_networkmanager_for_counters_only_when_asked() {
    let world = World::start(NEXUS);
    world.nm.add_wifi_device("wlan0");
    let mut events = world.wire();
    events.subscribe();
    go_online(&world, ActiveConnection::wifi("Home"));
    // The Daemon has seen the Link (its re-read pushed this) and not read counters.
    events.next_event("activity");
    let reads = world.nm.statistics_reads();
    assert_eq!(world.nm.statistics_reads(), reads);
    traffic(&mut world.wire());
    assert!(world.nm.statistics_reads() > reads);
}

#[test]
fn activity_records_joins_and_losses_and_pushes_them() {
    let world = World::start(NEXUS);
    let mut wire = world.wire();
    assert!(activity(&mut wire).is_empty());
    let mut events = world.wire();
    events.subscribe();

    go_online(&world, ActiveConnection::wifi("Home"));
    let ev = events.next_event("activity");
    assert_eq!(ev["entry"]["kind"], "joined");
    assert_eq!(ev["entry"]["text"], "Joined Home (Wi-Fi)");
    assert!(ev["entry"]["at"].as_u64().unwrap() > 1_600_000_000);

    world.nm.set_primary(None);
    world.nm.set_state(NmState::Disconnected);
    let ev = events.next_event("activity");
    assert_eq!(ev["entry"]["kind"], "lost");
    assert_eq!(ev["entry"]["text"], "Lost Home");

    // Newest first, and a new client sees the history.
    let all = activity(&mut world.wire());
    assert_eq!(texts(&all), ["Lost Home", "Joined Home (Wi-Fi)"]);
}

#[test]
fn activity_notes_portals_sign_in_and_vpns() {
    let world = World::start(NEXUS);
    let mut events = world.wire();
    events.subscribe();

    go_online(&world, ActiveConnection::wifi("CafeGuest"));
    events.next_event("activity"); // Joined CafeGuest (Wi-Fi)

    world.nm.set_connectivity(NmConnectivity::Portal);
    let ev = events.next_event("activity");
    assert_eq!(ev["entry"]["kind"], "portal");
    assert_eq!(ev["entry"]["text"], "Sign-in page detected on CafeGuest");

    world.nm.set_connectivity(NmConnectivity::Full);
    let ev = events.next_event("activity");
    assert_eq!(ev["entry"]["kind"], "online");
    assert_eq!(ev["entry"]["text"], "Signed in to CafeGuest, you're online");

    world.nm.set_primary(Some(ActiveConnection::vpn("Work")));
    let ev = events.next_event("activity");
    assert_eq!(ev["entry"]["kind"], "vpn_up");
    assert_eq!(ev["entry"]["text"], "VPN Work is up");
}
