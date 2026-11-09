//! Runs `nexus` (the TUI) against a private world: fake NetworkManager, real
//! Daemon, nothing of the user's session touched.
//!
//! ```text
//! cargo build -p nexus
//! cargo run -p nexus-testkit --example demo -- target/debug/nexus [online|offline|portal|busy|wifi|wifi-prompt|vpn]
//! cargo run -p nexus-testkit --example demo -- target/debug/nexus [online|offline|portal|busy|wifi|wifi-prompt|tether|tether-off]
//! cargo run -p nexus-testkit --example demo -- target/debug/nexus [online|offline|portal|busy|wifi|wifi-prompt|wifi-enterprise|profile]
//! ```
//!
//! Scenarios:
//! - `online`: joined to Home-5G, no traffic.
//! - `offline`: nothing connected.
//! - `portal`: joined to CafeGuest behind a sign-in page. The Overview hero says
//!   so; press `o` to "open the login page" (a stand-in `xdg-open` only logs
//!   the URL, no browser starts) and two seconds later the fake network lets you
//!   in: the hero turns Online and a toast says "You're online.". Press `4` to
//!   see the banner other tabs get while the Portal is up.
//! - `wifi`: six Wi-Fi Networks in range (Home-5G joined), two saved ones out
//!   of range. Press `2` for the Wi-Fi tab; `r` rescans, `enter` joins, `x`
//!   disconnects, `f` forgets, `a` toggles autoconnect, `u` undoes. `s` on a saved
//!   row (Home-5G, Parents House, Hotel Lobby) opens the share view; `enter`
//!   reveals the QR code, `p` shows the password, `esc` closes.
//! - `wifi-enterprise`: `wifi` plus eduroam (WPA-Enterprise). Press `2`, move
//!   to eduroam (`j`), `enter` opens the sign-in form: type an identity,
//!   `tab` to move, `←`/`→` change the method, `space` toggles the server
//!   check, `ctrl-r` shows passwords, `enter` joins, `esc` cancels. The right
//!   login is `you@uni.example` / `correct-horse`, server name `radius.uni.example`.
//! - `wifi-prompt`: `wifi` with a password prompt for Work-Secure already open.
//! - `vpn`: online, with three VPNs: Work (WireGuard, up), Home Lab
//!   (WireGuard, down) and Office OpenVPN (plugin, down). Press `4` for the
//!   VPN tab; `enter` brings the selected one up or down, `p` shows its public
//!   key, `s` then `enter` reveals its config as a QR code (needs a window of
//!   about 70 by 38 cells), `i` imports a `.conf` (type a path), `c` creates a
//!   tunnel with generated keys, `esc` closes a layer.
//! - `vpn-plugin`: plugin VPNs. Press `4`. `Office OpenVPN` asks for a user
//!   name, a password and a one-time code on `enter` (type, `enter` sends,
//!   `ctrl-r` shows, `esc` declines); any answers work. `Legacy L2TP` has no
//!   plugin installed, so `enter` says which package to install. `i` imports
//!   the sample `.ovpn` whose path the demo prints on stderr.
//! - `tether`: Bluetooth on and three paired phones (Pixel 8 with a saved
//!   Profile, Sam's iPhone, a long-named Galaxy). Press `5` for the Hotspot tab
//!   (Tethering lives there until the Hotspot arrives); `j`/`k` move, `enter`
//!   tethers, `x` drops the tether, `u` undoes. `d` opens details when narrow.
//! - `tether-off`: the same, with Bluetooth switched off, so `enter` shows how
//!   to turn it on.
//! - `profile`: `wifi`, where Parents House (saved, out of range) already has a
//!   static address, custom DNS, metered and a priority. Press `2`, then `e`
//!   on Home-5G (in use: Apply starts the 15 s "Keep these settings?"
//!   countdown, `y` keeps, `n` or waiting reverts) or `G` `k` `e` on Parents
//!   House (saved at once). In the form: `tab` next row, `space` or arrows
//!   change a choice, type into address rows, `enter` apply, `esc` cancel.
//! - `effects`: `wifi`, with Cafe Guest set to fail its join. Press `3`..`6`
//!   and `1`: the body glides in on each tab change. Press `2`, move to Cafe
//!   Guest (`j`) and `enter`: it fails, and the "Problem" toast slides in and
//!   shakes once. Move to Work-Secure, `enter` and type any password: it
//!   joins, blooms and the toast slides in. While it connects the header
//!   status and the Overview hero pulse. Set `NEXUS_REDUCE_MOTION=1` to see
//!   the same events without movement. With nothing happening it draws nothing.
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

    let world = if scenario.starts_with("tether") {
        nexus_testkit::scenarios::tether_world(&bin, scenario == "tether")
    } else {
        World::start(&bin)
    };
    let mut _prompt = None;
    let opened = world.dir().join("opened.log");
    let mut wlan0 = None;
    match scenario.as_str() {
        "offline" | "tether" | "tether-off" => {}
        // Press 2 for the Wi-Fi tab. Six Networks in range (Home-5G joined),
        // two saved ones out of range; r rescans, f forgets, a toggles
        // autoconnect, x disconnects, u undoes.
        "wifi" | "wifi-prompt" | "wifi-enterprise" | "profile" | "effects" => {
            if scenario == "wifi" || scenario == "effects" {
                nexus_testkit::scenarios::wifi_busy(&world);
                if scenario == "effects" {
                    // Activation fails (state 60) with "no secrets" (reason 7).
                    world.nm.script_join_failure("Cafe Guest", 60, 7);
                }
            } else if scenario == "wifi-enterprise" {
                nexus_testkit::scenarios::wifi_enterprise(&world);
            } else if scenario == "profile" {
                nexus_testkit::scenarios::profile_busy(&world);
            } else {
                // Same, plus a password prompt for Work-Secure, already open.
                _prompt = Some(nexus_testkit::scenarios::wifi_with_open_prompt(&world));
            }
            world.nm.set_connectivity(NmConnectivity::Full);
            world.nm.set_state(NmState::ConnectedGlobal);
        }
        "vpn" => nexus_testkit::scenarios::vpn_busy(&world),
        "vpn-plugin" => {
            let file = nexus_testkit::scenarios::vpn_plugins(&world);
            eprintln!("A sample file to import with `i`: {}", file.display());
        }
        "portal" => {
            // No browser: the stand-in records the URL, which also plays "signed in".
            world.install_tool(
                "xdg-open",
                &format!("echo \"$1\" >> '{}'", opened.display()),
            );
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
        if scenario == "portal" {
            let (done, nm, opened) = (&done, &world.nm, &opened);
            scope.spawn(move || {
                while !done.load(Ordering::SeqCst) && !opened.exists() {
                    std::thread::sleep(Duration::from_millis(100));
                }
                std::thread::sleep(Duration::from_secs(2));
                nm.set_connectivity(NmConnectivity::Full);
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
