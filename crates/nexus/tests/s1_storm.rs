//! Seam S1 guard: a storm of NetworkManager signals while the Daemon re-reads
//! the world must never stall its answers. The Backend's signal streams are
//! only polled between re-reads; if their queues filled up, the bus
//! connection would stop receiving (including replies) until a call timed out.

use std::time::{Duration, Instant};

use nexus_testkit::{Ap, World};
use serde_json::json;

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

#[test]
fn a_signal_storm_does_not_stall_the_daemon() {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    for n in 0..30 {
        world.nm.add_ap(
            wlan0,
            Ap::wpa2(&format!("Net-{n}"), &format!("AA:00:00:00:01:{n:02}"), 50),
        );
    }
    let mut wire = world.wire();
    assert!(wire.call(json!({"op": "status"})).get("ok").is_some());

    let nm = &world.nm;
    let started = Instant::now();
    let mut worst = Duration::ZERO;
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for round in 0..40u8 {
                for n in 0..30 {
                    nm.set_strength_of(&format!("Net-{n}"), 20 + (round + n) % 60);
                }
            }
        });
        while started.elapsed() < Duration::from_secs(8) {
            let t = Instant::now();
            let resp = wire.call(json!({"op": "wifi_list"}));
            assert!(resp.get("ok").is_some(), "{resp}");
            worst = worst.max(t.elapsed());
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    assert!(
        worst < Duration::from_secs(3),
        "an answer took {worst:?} during a signal storm"
    );
}

/// While a Hotspot runs the Daemon also wakes on its own count timer. That
/// timer winning the race against a Backend signal must not eat the signal.
#[test]
fn a_hotspot_poll_never_swallows_a_lone_change() {
    let mut world = World::builder(NEXUS).daemon(false).start();
    world.write_config("[hotspot]\npoll_secs = 1\n");
    world.start_daemon();
    let wlan0 = world.nm.add_wifi_device("wlan0");
    world
        .nm
        .add_ap(wlan0, Ap::wpa2("Home", "AA:00:00:00:02:01", 50));
    let mut ev = world.wire();
    ev.subscribe();
    let resp = world
        .wire()
        .call(json!({"op": "hotspot_start", "ssid": "Garage"}));
    assert!(resp.get("ok").is_some(), "{resp}");
    ev.next_event("hotspot_changed");
    // One signal at a time, just before, then just after, the 1 s timer that restarts with every wake.
    for round in 0..24u8 {
        world.nm.set_strength_of("Home", 20 + round * 3);
        ev.next_event("networks_changed");
        std::thread::sleep(Duration::from_millis(900 + 10 * u64::from(round)));
    }
}
