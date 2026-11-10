//! Runs `nexus` (the TUI) against a private world: fake NetworkManager, real
//! Daemon, nothing of the user's session touched.
//!
//! ```text
//! cargo build -p nexus
//! cargo run -p nexus-testkit --example demo -- target/debug/nexus [online|offline|portal|busy|wifi|wifi-prompt|vpn]
//! cargo run -p nexus-testkit --example demo -- target/debug/nexus [online|offline|portal|busy|wifi|wifi-prompt|tether|tether-off|hotspot|hotspot-on|hotspot-nocard]
//! cargo run -p nexus-testkit --example demo -- target/debug/nexus [online|offline|portal|busy|wifi|wifi-prompt|wifi-enterprise|profile|profile-keyring]
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
//! - `radios`: `wifi`, for the radio keys and the list filter. Press `2`, then
//!   `w` turns Wi-Fi off (the list says so and how to turn it on; the
//!   Overview's Wi-Fi card reads Off) and `u` takes it back; `A` is airplane
//!   mode (Wi-Fi and Bluetooth off), `u` undoes it. `/` then a few letters
//!   (try `ho`) filters the list by name, `enter` keeps the filter, `esc`
//!   clears it; `/` works on the VPN tab (`4`) too. `r` rescans: the list keeps
//!   its order while the scan runs and is only re-sorted when it ends, if you
//!   have not moved the cursor.
//! - `radios-blocked`: `radios` with a hardware switch holding Wi-Fi off. The
//!   Wi-Fi tab explains it, and `w` (or `:` turn wifi on) says software cannot
//!   help instead of trying.
//! - `wifi-prompt`: `wifi` with a password prompt for Work-Secure already open.
//! - `wifi-edge`: the awkward cases, two Wi-Fi cards, nothing joined. Press `2`.
//!   `n` opens the hidden-network form: type `Ghost-Net`, `tab`, `←`/`→` pick
//!   the security (WPA2), `tab`, password `boo-boo-boo`, `enter` joins (leave
//!   the password empty and a prompt asks for it). `i` on Mesh-Home lists its
//!   access points (2.4, 5 and 6 GHz); `c` pins joins to wlan0 or wlan1 (wlan1
//!   hears only Mesh-Home and Mixed-Mode); Airport Free is OWE, Mixed-Mode
//!   is WPA2/WPA3, and one name is not valid UTF-8 and shows a replacement mark.
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
//!   `j`/`k` move, `enter`
//!   tethers, `x` drops the tether, `u` undoes. `d` opens details when narrow.
//! - `tether-off`: the same, with Bluetooth switched off, so `enter` shows how
//!   to turn it on.
//! - `settings`: Press `6`. `j`/`k` move, `enter` changes the row (the theme
//!   cycles through the seven built-ins and the custom `sunrise`; switches
//!   flip), `r` resets it. The config file (with its comments) is edited in
//!   place, so `nexus config get` in another shell agrees. `Lost` notifications
//!   start switched off.
//! - `settings-bad`: the same, but the config selects the mistaken theme
//!   `broken`: the TUI falls back to Midnight Bloom and says why.
//! - `hotspot`: the Hotspot tab (press `5`) on a machine whose Wi-Fi card can
//!   host one, above the Tethering phones. `n` starts one with a made-up
//!   password, `c` opens the form for your own name and password, then `s`
//!   `enter` reveals the QR code and password (`p` shows the letters; the QR
//!   needs a window of about 40 by 24 cells), `S` stops it, `esc` closes a layer.
//! - `hotspot-on`: the same with `Garage Wi-Fi` already up and two devices
//!   joined (password `open sesame 42`).
//! - `hotspot-nocard`: a card that cannot be an access point, so the tab
//!   explains why and `n` says no.
//! - `profile`: `wifi`, where Parents House (saved, out of range) already has a
//!   static address, custom DNS, metered and a priority. Press `2`, then `e`
//!   on Home-5G (in use: Apply starts the 15 s "Keep these settings?"
//!   countdown, `y` keeps, `n` or waiting reverts) or `G` `k` `e` on Parents
//!   House (saved at once). In the form: `tab` next row, `space` or arrows
//!   change a choice, type into address rows, `enter` apply, `esc` cancel.
//! - `ethernet`: two Ethernet ports. enp3s0 has a cable and a Profile but sits
//!   behind an 802.1X switch, so it reads as plugged in and misconfigured (no
//!   address); enp4s0 has no cable. Press `3` for the Ethernet tab; `enter` tries
//!   again, `s` opens the sign-in form (login `alice@office.example` /
//!   `hunter2`, server name `radius.office.example`), `x` drops the Link, `e`
//!   edits the Profile (settings of the Link in use run under the 15 s
//!   countdown). Press `1` to see the Overview card.
//! - `profile-keyring`: `profile`, with a stand-in Secret Service (keyring)
//!   running. Press `G` `k` `e` for Parents House, `tab` to the Keyring row
//!   (last), `space` to switch it to "yes, in your keyring", `enter` applies:
//!   the password moves into the keyring and NetworkManager stops holding it.
//!   Open the form again to see it stayed on; `enter` on the row's neighbour
//!   (`space` back to "no, system store") moves it back. On `profile` (no
//!   keyring) the same switch is refused with a message saying why.
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

    let world = if scenario.starts_with("settings") {
        nexus_testkit::scenarios::settings_world(&bin, scenario == "settings-bad")
    } else if let Some(start) = match scenario.as_str() {
        "hotspot" => Some(nexus_testkit::scenarios::HotspotStart::Capable),
        "hotspot-on" => Some(nexus_testkit::scenarios::HotspotStart::Running),
        "hotspot-nocard" => Some(nexus_testkit::scenarios::HotspotStart::NoCard),
        _ => None,
    } {
        nexus_testkit::scenarios::hotspot_world(&bin, start)
    } else if scenario.starts_with("tether") {
        nexus_testkit::scenarios::tether_world(&bin, scenario == "tether")
    } else if scenario == "profile-keyring" {
        World::builder(&bin).keyring(true).start()
    } else {
        World::start(&bin)
    };
    let mut _prompt = None;
    let opened = world.dir().join("opened.log");
    let mut wlan0 = None;
    match scenario.as_str() {
        "offline" | "tether" | "tether-off" | "settings" | "settings-bad" | "hotspot"
        | "hotspot-on" | "hotspot-nocard" => {}
        // Press 2 for the Wi-Fi tab. Six Networks in range (Home-5G joined),
        // two saved ones out of range; r rescans, f forgets, a toggles
        // autoconnect, x disconnects, u undoes.
        "wifi" | "wifi-prompt" | "wifi-enterprise" | "wifi-edge" | "profile"
        | "profile-keyring" | "effects" | "radios" | "radios-blocked" => {
            if scenario == "wifi-edge" {
                nexus_testkit::scenarios::wifi_edge(&world);
            } else if matches!(
                scenario.as_str(),
                "wifi" | "effects" | "radios" | "radios-blocked"
            ) {
                nexus_testkit::scenarios::wifi_busy(&world);
                if scenario == "effects" {
                    // Activation fails (state 60) with "no secrets" (reason 7).
                    world.nm.script_join_failure("Cafe Guest", 60, 7);
                }
            } else if scenario == "wifi-enterprise" {
                nexus_testkit::scenarios::wifi_enterprise(&world);
            } else if scenario == "profile" || scenario == "profile-keyring" {
                nexus_testkit::scenarios::profile_busy(&world);
            } else {
                // Same, plus a password prompt for Work-Secure, already open.
                _prompt = Some(nexus_testkit::scenarios::wifi_with_open_prompt(&world));
            }
            if scenario.starts_with("radios") {
                // Home-5G is joined in this scenario: it is the Link.
                world
                    .nm
                    .set_primary(Some(ActiveConnection::wifi("Home-5G")));
            }
            world.nm.set_connectivity(NmConnectivity::Full);
            world.nm.set_state(NmState::ConnectedGlobal);
            if scenario == "radios-blocked" {
                // After the Link is set up: the switch takes it down again.
                world.nm.set_wifi_hardware_blocked(true);
            }
        }
        "vpn" => nexus_testkit::scenarios::vpn_busy(&world),
        "ethernet" => {
            nexus_testkit::scenarios::ethernet_busy(&world);
        }
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
