//! Runs `nexus` (the TUI) against a private world: fake NetworkManager, real
//! Daemon, nothing of the user's session touched.
//!
//! ```text
//! cargo build -p nexus
//! cargo run -p nexus-testkit --example demo -- target/debug/nexus [online|offline|portal|busy|wifi|wifi-prompt|vpn]
//! ```
//!
//! Scenarios:
//! - `online`: joined to Home-5G, no traffic.
//! - `offline`: nothing connected.
//! - `portal`: joined to CafeGuest behind a sign-in page.
//! - `wifi`: six Wi-Fi Networks in range (Home-5G joined), two saved ones out
//!   of range. Press `2` for the Wi-Fi tab; `r` rescans, `enter` joins, `x`
//!   disconnects, `f` forgets, `a` toggles autoconnect, `u` undoes. `s` on a saved
//!   row (Home-5G, Parents House, Hotel Lobby) opens the share view; `enter`
//!   reveals the QR code, `p` shows the password, `esc` closes.
//! - `wifi-prompt`: `wifi` with a password prompt for Work-Secure already open.
//! - `vpn`: online, with three VPNs: Work (WireGuard, up), Home Lab
//!   (WireGuard, down) and Office OpenVPN (plugin, down). Press `4` for the
//!   VPN tab; `enter` brings the selected one up or down, `p` shows its public
//!   key, `s` then `enter` reveals its config as a QR code (needs a window of
//!   about 70 by 38 cells), `i` imports a `.conf` (type a path), `c` creates a
//!   tunnel with generated keys, `esc` closes a layer.
//! - `busy`: online with live, wandering traffic on the graphs, and a VPN
//!   going up and down every few seconds so the activity strip moves.
//!
//! Extra arguments after the scenario go to `nexus` (e.g. `--ascii`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nexus_testkit::{ActiveConnection, NmConnectivity, NmState, World};

fn main() {
    let mut args = std::env::args().skip(1);
    let bin = args
        .next()
        .expect("usage: demo <path-to-nexus> [online|offline|portal|busy] [nexus args]");
    let scenario = args.next().unwrap_or_else(|| "online".into());
    let rest: Vec<String> = args.collect();

    let world = World::start(&bin);
    let mut _prompt = None;
    let mut wlan0 = None;
    match scenario.as_str() {
        "offline" => {}
        // Press 2 for the Wi-Fi tab. Six Networks in range (Home-5G joined),
        // two saved ones out of range; r rescans, f forgets, a toggles
        // autoconnect, x disconnects, u undoes.
        "wifi" | "wifi-prompt" => {
            if scenario == "wifi" {
                nexus_testkit::scenarios::wifi_busy(&world);
            } else {
                // Same, plus a password prompt for Work-Secure, already open.
                _prompt = Some(nexus_testkit::scenarios::wifi_with_open_prompt(&world));
            }
            world.nm.set_connectivity(NmConnectivity::Full);
            world.nm.set_state(NmState::ConnectedGlobal);
        }
        "vpn" => nexus_testkit::scenarios::vpn_busy(&world),
        "portal" => {
            world
                .nm
                .set_primary(Some(ActiveConnection::wifi("CafeGuest")));
            world.nm.set_connectivity(NmConnectivity::Portal);
            world.nm.set_state(NmState::ConnectedGlobal);
        }
        other => {
            if other == "busy" {
                wlan0 = Some(world.nm.add_wifi_device("wlan0"));
            }
            world
                .nm
                .set_primary(Some(ActiveConnection::wifi("Home-5G")));
            world.nm.set_connectivity(NmConnectivity::Full);
            world.nm.set_state(NmState::ConnectedGlobal);
        }
    }

    let mut cmd = world.nexus();
    // The harness clears the environment; the TUI needs the terminal's.
    for var in ["TERM", "COLORTERM", "LANG", "LC_ALL", "NO_COLOR"] {
        if let Some(v) = std::env::var_os(var) {
            cmd.env(var, v);
        }
    }
    let done = AtomicBool::new(false);
    let status = std::thread::scope(|scope| {
        if let Some(dev) = wlan0 {
            let (done, nm) = (&done, &world.nm);
            scope.spawn(move || {
                // A tiny deterministic wobble: bursts of download, a trickle of upload.
                let (mut rx, mut tx, mut tick) = (0u64, 0u64, 0u64);
                while !done.load(Ordering::SeqCst) {
                    tick += 1;
                    let wave = (tick % 24) as f64 / 24.0 * std::f64::consts::TAU;
                    rx +=
                        (900_000.0 + 800_000.0 * wave.sin() + (tick % 5) as f64 * 150_000.0) as u64;
                    tx += (90_000.0 + 70_000.0 * (wave * 2.0).cos()) as u64;
                    nm.set_traffic(dev, rx, tx);
                    if tick % 8 == 0 {
                        nm.set_primary(Some(ActiveConnection::vpn("Work")));
                    } else if tick % 8 == 4 {
                        nm.set_primary(Some(ActiveConnection::wifi("Home-5G")));
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            });
        }
        let status = cmd.args(&rest).status().expect("run nexus");
        done.store(true, Ordering::SeqCst);
        status
    });
    drop(world);
    drop(_prompt);
    std::process::exit(status.code().unwrap_or(1));
}
