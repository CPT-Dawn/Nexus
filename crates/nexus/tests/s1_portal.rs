//! Seam S1 for Portal detection and login: a real Daemon, a fake
//! NetworkManager reporting `portal` connectivity, and a fake notification
//! server. The browser is a stand-in `xdg-open` first on the Daemon's PATH
//! that records its argument, so no real browser (or request) ever happens.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use nexus_testkit::{ActiveConnection, NmConnectivity, NmState, Wire, World};
use serde_json::json;

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");
/// Joined/lost toasts would come first and muddy the count.
const QUIET: &str = "[notifications]\njoined = false\nlost = false\n";

fn world_with(config: &str) -> (World, PathBuf) {
    let mut world = World::builder(NEXUS)
        .notifications(true)
        .daemon(false)
        .start();
    world.write_config(config);
    world.start_daemon();
    let log = world.dir().join("opened.log");
    world.install_tool("xdg-open", &format!("echo \"$1\" >> '{}'", log.display()));
    (world, log)
}

fn behind_portal(world: &World, ssid: &str) {
    world.nm.set_primary(Some(ActiveConnection::wifi(ssid)));
    world.nm.set_connectivity(NmConnectivity::Portal);
    world.nm.set_state(NmState::ConnectedGlobal);
}

fn sign_in(world: &World) {
    world.nm.set_connectivity(NmConnectivity::Full);
}

fn front_end(world: &World) -> Wire {
    let mut wire = world.wire();
    wire.subscribe();
    wire
}

fn opened(log: &PathBuf) -> Vec<String> {
    std::fs::read_to_string(log)
        .map(|t| t.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

fn wait_opened(log: &PathBuf, n: usize) -> Vec<String> {
    let start = Instant::now();
    loop {
        let got = opened(log);
        if got.len() >= n {
            return got;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "never opened");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_portal_is_announced_to_front_ends_and_readable_on_demand() {
    let (world, _) = world_with(QUIET);
    let mut events = front_end(&world);

    behind_portal(&world, "CafeGuest");
    let event = events.next_event("portal_changed");
    assert_eq!(event["portal"]["network"], "CafeGuest");
    assert_eq!(
        event["portal"]["url"],
        "http://ping.archlinux.org/nm-check.txt"
    );

    let now = world.wire().call(json!({"op": "portal"}));
    assert_eq!(now["ok"]["portal"]["network"], "CafeGuest", "{now}");

    sign_in(&world);
    let gone = events.next_event("portal_changed");
    assert!(gone["portal"].is_null(), "{gone}");
    let now = world.wire().call(json!({"op": "portal"}));
    assert!(now["ok"]["portal"].is_null(), "{now}");
}

#[test]
fn the_notification_offers_the_login_page_and_its_action_opens_it() {
    let (world, log) = world_with(QUIET);
    let notes = world.notifications.as_ref().unwrap();

    behind_portal(&world, "CafeGuest");
    let note = notes.notified(1);
    assert_eq!(note.summary, "Login required for CafeGuest");
    assert_eq!(note.actions, ["open-login", "Open login page"]);
    assert!(opened(&log).is_empty(), "nothing opens by itself");

    notes.invoke(note.id, "open-login");
    assert_eq!(
        wait_opened(&log, 1),
        ["http://ping.archlinux.org/nm-check.txt"]
    );
}

#[test]
fn signing_in_clears_the_offer_and_says_you_are_online() {
    let (world, _) = world_with(QUIET);
    let notes = world.notifications.as_ref().unwrap();

    behind_portal(&world, "CafeGuest");
    let offer = notes.notified(1);
    sign_in(&world);
    notes.wait_closed(offer.id);
    let online = notes.notified(2);
    assert_eq!(online.summary, "You're online");
    assert_eq!(online.body, "Signed in to CafeGuest, you're online");
}

#[test]
fn portal_notifications_can_be_switched_off() {
    let (world, _) = world_with("[notifications]\njoined = false\nportal = false\n");
    let notes = world.notifications.as_ref().unwrap();
    let mut events = front_end(&world);

    behind_portal(&world, "CafeGuest");
    events.next_event("portal_changed");
    sign_in(&world);
    events.next_event("portal_changed");
    // The loss is on, and queues behind anything the portal would have said.
    world.nm.set_primary(None);
    world.nm.set_state(NmState::Disconnected);
    assert_eq!(notes.notified(1).summary, "Lost CafeGuest");
    assert_eq!(notes.count(), 1, "{:?}", notes.all());
}

#[test]
fn portal_open_runs_the_browser_and_refuses_when_there_is_no_portal() {
    let (world, log) = world_with(QUIET);
    let refused = world.wire().call(json!({"op": "portal_open"}));
    assert_eq!(refused["err"]["code"], "no_portal", "{refused}");
    assert!(opened(&log).is_empty());

    let mut events = front_end(&world);
    behind_portal(&world, "CafeGuest");
    events.next_event("portal_changed");
    let ok = world.wire().call(json!({"op": "portal_open"}));
    assert_eq!(
        ok["ok"]["url"], "http://ping.archlinux.org/nm-check.txt",
        "{ok}"
    );
    assert_eq!(
        wait_opened(&log, 1),
        ["http://ping.archlinux.org/nm-check.txt"]
    );
}

#[test]
fn the_login_page_follows_the_configured_check_url() {
    let (world, _) = world_with(
        "[connectivity]\nurl = \"https://check.example.test/ok\"\n[notifications]\njoined = false\n",
    );
    let mut events = front_end(&world);
    behind_portal(&world, "CafeGuest");
    let event = events.next_event("portal_changed");
    assert_eq!(event["portal"]["url"], "http://check.example.test/ok");
}

#[test]
fn a_switched_off_check_means_no_portal_is_announced() {
    let (world, _) =
        world_with("[connectivity]\nenabled = false\n[notifications]\njoined = false\n");
    let notes = world.notifications.as_ref().unwrap();
    let mut events = front_end(&world);

    behind_portal(&world, "CafeGuest");
    events.next_event("activity");
    let now = world.wire().call(json!({"op": "portal"}));
    assert!(now["ok"]["portal"].is_null(), "{now}");
    let refused = world.wire().call(json!({"op": "portal_open"}));
    assert_eq!(refused["err"]["code"], "no_portal", "{refused}");
    world.nm.set_primary(None);
    world.nm.set_state(NmState::Disconnected);
    assert_eq!(notes.notified(1).summary, "Lost CafeGuest");
    assert_eq!(notes.count(), 1, "{:?}", notes.all());
}
