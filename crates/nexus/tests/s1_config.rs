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

#[test]
fn editing_the_file_applies_live_and_announces_it() {
    let world = World::start(NEXUS);
    let mut events = world.wire();
    events.subscribe();

    world.write_config("theme = \"nord\"\n[connectivity]\nenabled = false\n");

    let event = events.next_event("config_reloaded");
    assert_eq!(event["config"]["theme"], "nord");
    assert!(event.get("warning").is_none(), "{event}");
    let resp = get(&world);
    assert_eq!(resp["ok"]["value"]["theme"], "nord");
    assert_eq!(resp["ok"]["value"]["connectivity"]["enabled"], false);
    // Untouched options keep their defaults.
    assert_eq!(
        resp["ok"]["value"]["connectivity"]["url"],
        "https://ping.archlinux.org/nm-check.txt"
    );
}

#[test]
fn a_half_written_file_is_not_reported_as_broken() {
    let world = World::start(NEXUS);
    let mut events = world.wire();
    events.subscribe();

    // The first half ends mid-value, which on its own would not parse.
    world.write_config_in_pieces(
        "theme = \"nord\"\n[connectivity]\nenabled = fal",
        "se\n",
        std::time::Duration::from_millis(50),
    );

    let event = events.next_event("config_reloaded");
    assert!(event.get("warning").is_none(), "{event}");
    assert_eq!(event["config"]["theme"], "nord");
    assert_eq!(event["config"]["connectivity"]["enabled"], false);
}

#[test]
fn a_single_key_can_be_read() {
    let world = World::start(NEXUS);
    let mut events = world.wire();
    events.subscribe();
    world.write_config("theme = \"nord\"\n");
    events.next_event("config_reloaded");

    let mut wire = world.wire();
    let resp = wire.call(json!({"op": "config_get", "key": "theme"}));
    assert_eq!(resp["ok"]["value"], "nord");
    let unknown = wire.call(json!({"op": "config_get", "key": "nope.nothing"}));
    assert_eq!(unknown["err"]["code"], "bad_request");
}

#[test]
fn an_invalid_file_falls_back_to_defaults_with_a_warning() {
    let world = World::start(NEXUS);
    let mut events = world.wire();
    events.subscribe();

    world.write_config("theme = \"nord\"\n");
    events.next_event("config_reloaded");
    world.write_config("theme = \"nord\"\nthemee = 3\n");

    let event = events.next_event("config_reloaded");
    let warning = event["warning"].as_str().expect("a warning");
    assert!(warning.contains("themee"), "{warning}");
    assert!(warning.contains("line 2"), "{warning}");
    assert_eq!(event["config"]["theme"], "midnight-bloom");

    let resp = get(&world);
    assert_eq!(resp["ok"]["value"]["theme"], "midnight-bloom");
    assert_eq!(resp["ok"]["warning"].as_str(), Some(warning));

    // Fixing the file clears the warning.
    world.write_config("theme = \"gruvbox\"\n");
    let fixed = events.next_event("config_reloaded");
    assert!(fixed.get("warning").is_none(), "{fixed}");
    assert_eq!(get(&world)["ok"]["value"]["theme"], "gruvbox");
}

#[test]
fn a_file_that_is_broken_at_startup_never_stops_the_daemon() {
    let mut world = World::builder(NEXUS).daemon(false).start();
    world.write_config("this is = = not toml\n");
    world.start_daemon();

    let resp = get(&world);
    assert_eq!(resp["ok"]["value"]["theme"], "midnight-bloom");
    assert!(
        resp["ok"]["warning"]
            .as_str()
            .is_some_and(|w| w.contains("line 1")),
        "{resp}"
    );
    // The rest of the product still answers.
    let status = world.wire().call(json!({"op": "status"}));
    assert_eq!(status["ok"]["state"], "offline");
}

#[test]
fn a_bad_value_is_named_in_the_warning() {
    let world = World::start(NEXUS);
    let mut events = world.wire();
    events.subscribe();
    world.write_config("[connectivity]\nurl = \"ftp://x\"\n");
    let event = events.next_event("config_reloaded");
    assert!(
        event["warning"]
            .as_str()
            .is_some_and(|w| w.contains("connectivity.url")),
        "{event}"
    );
}

#[test]
fn deleting_the_file_returns_to_defaults() {
    let world = World::start(NEXUS);
    let mut events = world.wire();
    events.subscribe();
    world.write_config("theme = \"nord\"\n");
    events.next_event("config_reloaded");
    std::fs::remove_file(world.config_path()).unwrap();
    let event = events.next_event("config_reloaded");
    assert_eq!(event["config"]["theme"], "midnight-bloom");
}

#[test]
fn the_cli_prints_the_config_and_warns_about_a_broken_file() {
    let world = World::start(NEXUS);
    let mut events = world.wire();
    events.subscribe();
    world.write_config("theme = 3\n");
    events.next_event("config_reloaded");

    let out = world.run_nexus(&["config", "get"]);
    assert!(out.status.success(), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("being ignored"), "stderr was: {err}");
    let shown = String::from_utf8_lossy(&out.stdout);
    assert!(shown.contains("midnight-bloom"), "stdout was: {shown}");
}

#[test]
fn set_edits_in_place_and_keeps_comments_and_order() {
    let world = World::start(NEXUS);
    let original = "\
# My Nexus config
theme = \"nord\" # I like it cold

[menu]
launcher = \"rofi\"

# the one outside contact
[connectivity]
enabled = true
";
    let mut events = world.wire();
    events.subscribe();
    world.write_config(original);
    events.next_event("config_reloaded");

    let resp = world
        .wire()
        .call(json!({"op": "config_set", "key": "connectivity.enabled", "value": false}));
    assert_eq!(resp["ok"]["value"], false, "{resp}");
    events.next_event("config_reloaded");

    let after = std::fs::read_to_string(world.config_path()).unwrap();
    assert_eq!(after, original.replace("enabled = true", "enabled = false"));
    assert_eq!(get(&world)["ok"]["value"]["connectivity"]["enabled"], false);

    // A key absent from the file is appended, the rest untouched.
    let resp = world.wire().call(
        json!({"op": "config_set", "key": "connectivity.url", "value": "http://example.org/x"}),
    );
    assert!(resp.get("ok").is_some(), "{resp}");
    let after2 = std::fs::read_to_string(world.config_path()).unwrap();
    assert!(after2.starts_with(&after[..after.len() - 1]), "{after2}");
    assert!(
        after2.contains("url = \"http://example.org/x\""),
        "{after2}"
    );
}

#[test]
fn set_creates_the_file_and_null_resets_a_key() {
    let world = World::start(NEXUS);
    let mut wire = world.wire();
    let resp = wire.call(json!({"op": "config_set", "key": "theme", "value": "nord"}));
    assert_eq!(resp["ok"]["value"], "nord", "{resp}");
    assert_eq!(
        std::fs::read_to_string(world.config_path()).unwrap(),
        "theme = \"nord\"\n"
    );
    let resp = wire.call(json!({"op": "config_set", "key": "theme", "value": null}));
    assert_eq!(resp["ok"]["value"], "midnight-bloom", "{resp}");
}

#[test]
fn set_refuses_bad_input_and_leaves_the_file_alone() {
    let world = World::start(NEXUS);
    let mut events = world.wire();
    events.subscribe();
    world.write_config("theme = \"nord\"\n");
    events.next_event("config_reloaded");
    let mut wire = world.wire();

    for (key, value) in [
        ("nope", json!("x")),
        ("theme", json!(3)),
        ("connectivity.url", json!("ftp://x")),
        ("theme", json!(["a"])),
    ] {
        let resp = wire.call(json!({"op": "config_set", "key": key, "value": value}));
        assert_eq!(resp["err"]["code"], "bad_request", "{key}: {resp}");
    }
    assert_eq!(
        std::fs::read_to_string(world.config_path()).unwrap(),
        "theme = \"nord\"\n"
    );
}

#[test]
fn set_will_not_overwrite_a_file_the_user_has_broken() {
    let world = World::start(NEXUS);
    let mut events = world.wire();
    events.subscribe();
    world.write_config("theme = = oops\n");
    events.next_event("config_reloaded");
    let resp = world
        .wire()
        .call(json!({"op": "config_set", "key": "theme", "value": "nord"}));
    assert_eq!(resp["err"]["code"], "bad_request", "{resp}");
    assert_eq!(
        std::fs::read_to_string(world.config_path()).unwrap(),
        "theme = = oops\n"
    );
}

#[test]
fn the_cli_sets_and_gets_a_key() {
    let world = World::start(NEXUS);
    let out = world.run_nexus(&["config", "set", "theme", "nord"]);
    assert!(out.status.success(), "{out:?}");
    let out = world.run_nexus(&["config", "get", "theme"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "nord\n");
    // --json prints the value as JSON: a string keeps its quotes.
    let out = world.run_nexus(&["config", "get", "theme", "--json"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "\"nord\"\n");
    // Values that look like booleans or numbers are typed.
    let out = world.run_nexus(&["config", "set", "connectivity.enabled", "false"]);
    assert!(out.status.success(), "{out:?}");
    let out = world.run_nexus(&["config", "get", "connectivity.enabled"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "false\n");
    // A refused value exits 4 with the Daemon's words.
    let out = world.run_nexus(&["config", "set", "nope", "1"]);
    assert_eq!(out.status.code(), Some(4), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown"));
}

#[test]
fn the_prompt_timeout_is_a_setting_with_sane_bounds() {
    let world = World::start(NEXUS);
    let mut wire = world.wire();
    let resp = wire.call(json!({"op": "config_get", "key": "prompts.timeout_secs"}));
    assert_eq!(resp["ok"]["value"], 120, "{resp}");

    let resp = wire.call(json!({"op": "config_set", "key": "prompts.timeout_secs", "value": 0}));
    assert_eq!(resp["err"]["code"], "bad_request", "{resp}");
    assert!(
        resp["err"]["message"]
            .as_str()
            .unwrap()
            .contains("between 1 and 3600")
    );
    let resp = wire.call(json!({"op": "config_set", "key": "prompts.timeout_secs", "value": 30}));
    assert_eq!(resp["ok"]["value"], 30, "{resp}");
}

#[test]
fn ui_switches_and_custom_themes_are_ordinary_keys() {
    let world = World::start(NEXUS);
    let resp = get(&world);
    assert_eq!(resp["ok"]["value"]["ui"]["nerd_font"], false, "{resp}");
    assert_eq!(resp["ok"]["value"]["ui"]["reduce_motion"], false, "{resp}");
    assert_eq!(resp["ok"]["value"]["themes"], json!({}), "{resp}");
    let mut wire = world.wire();
    for (key, value) in [
        ("ui.reduce_motion", json!(true)),
        ("themes.mine.accent", json!("#ff8800")),
        ("themes.mine.base", json!("nord")),
    ] {
        let resp = wire.call(json!({"op": "config_set", "key": key, "value": value}));
        assert_eq!(resp["ok"]["value"], value, "{resp}");
    }
    let text = std::fs::read_to_string(world.config_path()).unwrap();
    assert!(text.contains("[themes.mine]") && text.contains("reduce_motion = true"));
    // A mistake inside a custom theme is the TUI's to report: the Daemon
    // keeps the rest of the file in force.
    let resp = wire.call(json!({"op": "config_set", "key": "themes.mine.accent", "value": "teal"}));
    assert!(resp.get("ok").is_some(), "{resp}");
    assert!(get(&world)["ok"].get("warning").is_none());
}

/// Polls for `want` on NetworkManager's `ConnectivityCheckEnabled`.
fn wait_for_check(world: &World, want: bool) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while world.nm.connectivity_check() != want {
        assert!(
            std::time::Instant::now() < until,
            "ConnectivityCheckEnabled never became {want}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn connectivity_enabled_switches_networkmanagers_check_both_ways() {
    let world = World::start(NEXUS);
    assert!(world.nm.connectivity_check());
    let off = world
        .wire()
        .call(json!({"op": "config_set", "key": "connectivity.enabled", "value": false}));
    assert!(off.get("ok").is_some(), "{off}");
    wait_for_check(&world, false);
    world
        .wire()
        .call(json!({"op": "config_set", "key": "connectivity.enabled", "value": true}));
    wait_for_check(&world, true);
}

#[test]
fn a_check_switched_off_in_the_config_is_off_in_networkmanager_at_start() {
    let mut world = World::builder(NEXUS).daemon(false).start();
    world.write_config("[connectivity]\nenabled = false\n");
    world.start_daemon();
    wait_for_check(&world, false);
}

#[test]
fn a_check_switched_off_in_networkmanager_is_not_switched_on_by_a_default_config() {
    let world = World::start(NEXUS);
    world.nm.set_connectivity_check(false);
    let resp = get(&world);
    assert!(resp.get("ok").is_some(), "{resp}");
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!world.nm.connectivity_check());
}

#[test]
fn outside_systemd_the_daemon_logs_to_a_file_in_the_state_dir() {
    let mut world = World::builder(NEXUS).daemon(false).start();
    let mut cmd = world.nexus();
    cmd.env_remove("JOURNAL_STREAM")
        .env("XDG_STATE_HOME", world.dir().join("state"))
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let child = cmd.spawn().expect("spawn nexus daemon");
    world.adopt_daemon(child);
    let log = world.dir().join("state/nexus/daemon.log");
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if text.contains("listening") {
            assert!(!text.contains('\u{1b}'), "colour codes in a log file");
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "no log line in {}",
            log.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn config_get_reports_the_address_networkmanager_checks() {
    let world = World::start(NEXUS);
    world.nm.set_connectivity_uri("http://check.example/nm.txt");
    let resp = get(&world);
    assert_eq!(
        resp["ok"]["connectivity_uri"], "http://check.example/nm.txt",
        "{resp}"
    );
}
