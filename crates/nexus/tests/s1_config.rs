//! Seam S1: the optional config file, seen through the socket and the CLI.

use nexus_testkit::World;
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

fn get(world: &World) -> Value {
    world.wire().call(json!({"op": "config_get"}))
}

#[test]
fn no_config_file_means_defaults() {
    let world = World::start(NEXUS);
    assert!(!world.config_path().exists());
    let resp = get(&world);
    assert_eq!(resp["ok"]["value"]["theme"], "midnight-bloom");
    assert_eq!(resp["ok"]["value"]["connectivity"]["enabled"], true);
    assert!(resp["ok"].get("warning").is_none(), "{resp}");
}
