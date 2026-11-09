//! Runs `nexus` (the TUI) against a private world: fake NetworkManager, real
//! Daemon, nothing of the user's session touched.
//!
//! ```text
//! cargo build -p nexus
//! cargo run -p nexus-testkit --example demo -- target/debug/nexus [online|offline|portal]
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
    match scenario.as_str() {
        "offline" => {}
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
    std::process::exit(status.code().unwrap_or(1));
}
