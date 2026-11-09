//! Runs `nexus` (the TUI) against a private world: fake NetworkManager, real
//! Daemon, nothing of the user's session touched.
//!
//! ```text
//! cargo build -p nexus
//! cargo run -p nexus-testkit --example demo -- target/debug/nexus [online|offline|portal|wifi|wifi-prompt]
//! ```
//!
//! Extra arguments after the scenario go to `nexus` (e.g. `--ascii`).

use nexus_testkit::{ActiveConnection, NmConnectivity, NmState, World};

fn main() {
    let mut args = std::env::args().skip(1);
    let bin = args
        .next()
        .expect("usage: demo <path-to-nexus> [online|offline|portal] [nexus args]");
    let scenario = args.next().unwrap_or_else(|| "online".into());
    let rest: Vec<String> = args.collect();

    let world = World::start(&bin);
    let mut _prompt = None;
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
        "portal" => {
            world
                .nm
                .set_primary(Some(ActiveConnection::wifi("CafeGuest")));
            world.nm.set_connectivity(NmConnectivity::Portal);
            world.nm.set_state(NmState::ConnectedGlobal);
        }
        _ => {
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
    let status = cmd.args(&rest).status().expect("run nexus");
    drop(world);
    drop(_prompt);
    std::process::exit(status.code().unwrap_or(1));
}
