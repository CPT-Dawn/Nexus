//! Seam S1 for WireGuard and the VPN list: a real `nexus daemon` against a
//! fake NetworkManager. Import, create, up and down, the public key, the
//! reveal, and the alert when a tunnel dies on its own.

use nexus_testkit::World;
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

// RFC 7748 section 6.1, Alice: a private key and the public key it must give.
const ALICE_PRIVATE: &str = "dwdtCnMYpX08FsFyUbJmRd9ML4frwJkqsXf7pR25LCo=";
const ALICE_PUBLIC: &str = "hSDwCYkwp1R0i33ctD73Wg2/Og0mOBr066SpjqqbTmo=";
const PEER: &str = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=";

fn conf() -> String {
    format!(
        "[Interface]\nPrivateKey = {ALICE_PRIVATE}\nAddress = 10.0.0.2/32\nDNS = 1.1.1.1\n\n[Peer]\nPublicKey = {PEER}\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = vpn.example.net:51820\nPersistentKeepalive = 25\n"
    )
}

fn import(world: &World, name: &str) -> Value {
    world
        .wire()
        .call(json!({"op": "vpn_import", "name": name, "config": conf()}))
}

fn list(world: &World) -> Value {
    world.wire().call(json!({"op": "vpn_list"}))["ok"]["vpns"].clone()
}

fn id_of(world: &World, name: &str) -> String {
    world.nm.vpn_uuid(name).expect("the VPN exists")
}

#[test]
fn import_saves_a_wireguard_profile_with_its_key_and_reports_the_public_key() {
    let world = World::start(NEXUS);
    let resp = import(&world, "Work");
    assert_eq!(resp["ok"]["name"], "Work", "{resp}");
    assert_eq!(resp["ok"]["public_key"], ALICE_PUBLIC, "{resp}");
    let uuid = id_of(&world, "Work");
    assert_eq!(resp["ok"]["id"], uuid.as_str());
    assert_eq!(
        world.nm.vpn_private_key(&uuid).as_deref(),
        Some(ALICE_PRIVATE)
    );
    // A native NetworkManager profile, not a plugin one.
    let kind = world.nm.vpn_setting(&uuid, "connection", "type").unwrap();
    assert!(kind.contains("wireguard"), "{kind}");
    assert!(
        !world
            .nm
            .vpn_setting(&uuid, "connection", "autoconnect")
            .unwrap()
            .contains("true"),
        "an imported tunnel does not start by itself"
    );
    assert_eq!(
        list(&world),
        json!([{
            "id": uuid, "name": "Work", "kind": "wireguard", "state": "down",
            "addresses": ["10.0.0.2/32"], "endpoint": "vpn.example.net:51820"
        }])
    );
}

#[test]
fn a_bad_config_is_refused_with_the_line_at_fault_and_saves_nothing() {
    let world = World::start(NEXUS);
    let bad = conf().replace("10.0.0.2/32", "10.0.0.2/99");
    let resp = world
        .wire()
        .call(json!({"op": "vpn_import", "name": "Bad", "config": bad}));
    assert_eq!(resp["err"]["code"], "invalid_config", "{resp}");
    assert!(
        resp["err"]["message"].as_str().unwrap().contains("line 3"),
        "{resp}"
    );
    let cmds = conf().replace("DNS = 1.1.1.1", "PostUp = rm -rf /");
    let resp = world
        .wire()
        .call(json!({"op": "vpn_import", "name": "Bad", "config": cmds}));
    assert_eq!(resp["err"]["code"], "invalid_config", "{resp}");
    assert_eq!(world.nm.profile_count(), 0);
}

#[test]
fn create_generates_a_key_pair_and_the_public_key_matches_the_stored_private_one() {
    let world = World::start(NEXUS);
    let mut wire = world.wire();
    let resp = wire.call(json!({
        "op": "vpn_create", "name": "Home", "addresses": ["10.9.0.2"],
        "dns": ["9.9.9.9"], "peer_key": PEER, "endpoint": "home.example.org:51820"
    }));
    let public = resp["ok"]["public_key"].as_str().unwrap().to_owned();
    let uuid = id_of(&world, "Home");
    let private = world.nm.vpn_private_key(&uuid).unwrap();
    assert_eq!(private.len(), 44, "{private}");
    assert_ne!(private, public);
    assert_eq!(public.len(), 44);
    // The Daemon derives the same public key from what NetworkManager stores.
    let key = wire.call(json!({"op": "vpn_key", "vpn": uuid}));
    assert_eq!(
        key["ok"],
        json!({"name": "Home", "public_key": public}),
        "{key}"
    );
    // A second tunnel gets a different pair.
    let again =
        wire.call(json!({"op": "vpn_create", "name": "Other", "addresses": ["10.9.0.3/32"]}));
    assert_ne!(again["ok"]["public_key"], json!(public));
    // Defaults: everything goes through the tunnel; /32 added to a bare address.
    assert_eq!(list(&world)[0]["addresses"], json!(["10.9.0.2/32"]));
    assert_eq!(list(&world)[0]["endpoint"], "home.example.org:51820");
}

#[test]
fn create_checks_its_input() {
    let world = World::start(NEXUS);
    let mut wire = world.wire();
    for op in [
        json!({"op": "vpn_create", "name": "", "addresses": ["10.0.0.2/32"]}),
        json!({"op": "vpn_create", "name": "X", "addresses": []}),
        json!({"op": "vpn_create", "name": "X", "addresses": ["nope"]}),
        json!({"op": "vpn_create", "name": "X", "addresses": ["10.0.0.2/32"], "peer_key": "short"}),
        json!({"op": "vpn_create", "name": "X", "addresses": ["10.0.0.2/32"], "peer_key": PEER, "endpoint": "nowhere"}),
    ] {
        let resp = wire.call(op.clone());
        assert_eq!(resp["err"]["code"], "invalid_config", "{op}: {resp}");
    }
    assert_eq!(world.nm.profile_count(), 0);
}

#[test]
fn up_and_down_move_the_state_and_push_events() {
    let world = World::start(NEXUS);
    import(&world, "Work");
    let uuid = id_of(&world, "Work");
    let mut events = world.wire();
    events.subscribe();
    let mut wire = world.wire();

    let up = wire.call(json!({"op": "vpn_up", "vpn": uuid}));
    assert_eq!(
        up["ok"],
        json!({"id": uuid, "name": "Work", "state": "up"}),
        "{up}"
    );
    assert_eq!(world.nm.vpn_active_state(&uuid), 2);
    assert_eq!(list(&world)[0]["state"], "up");
    let ev = events.next_event("vpns_changed");
    assert!(ev["vpns"]["vpns"].is_array(), "{ev}");

    let down = wire.call(json!({"op": "vpn_down", "vpn": uuid}));
    assert_eq!(down["ok"]["state"], "down", "{down}");
    assert_eq!(list(&world)[0]["state"], "down");
    let again = wire.call(json!({"op": "vpn_down", "vpn": uuid}));
    assert_eq!(again["err"]["code"], "not_connected", "{again}");
    let missing = wire.call(json!({"op": "vpn_up", "vpn": "no-such-id"}));
    assert_eq!(missing["err"]["code"], "unknown_vpn", "{missing}");
}

#[test]
fn a_tunnel_that_fails_to_start_says_so() {
    let world = World::start(NEXUS);
    import(&world, "Work");
    let uuid = id_of(&world, "Work");
    world.nm.script_vpn_failure(&uuid, true);
    let resp = world.wire().call(json!({"op": "vpn_up", "vpn": uuid}));
    assert_eq!(resp["err"]["code"], "vpn_failed", "{resp}");
    assert!(
        resp["err"]["message"]
            .as_str()
            .unwrap()
            .starts_with("Work did not come up"),
        "{resp}"
    );
    assert_eq!(list(&world)[0]["state"], "down");
}

#[test]
fn plugin_vpns_are_listed() {
    let world = World::start(NEXUS);
    world.nm.add_plugin_vpn("uuid-ovpn", "Office OpenVPN");
    let vpns = list(&world);
    assert_eq!(vpns[0]["name"], "Office OpenVPN");
    assert_eq!(vpns[0]["kind"], "plugin");
}

#[test]
fn no_secret_is_read_until_someone_asks_for_the_key_or_the_reveal() {
    let world = World::start(NEXUS);
    import(&world, "Work");
    let uuid = id_of(&world, "Work");
    list(&world);
    world.wire().call(json!({"op": "vpn_up", "vpn": uuid}));
    world.wire().call(json!({"op": "vpn_down", "vpn": uuid}));
    assert_eq!(world.nm.secret_reads(), 0);

    let resp = world.wire().call(json!({"op": "vpn_reveal", "vpn": uuid}));
    assert_eq!(world.nm.secret_reads(), 1);
    assert_eq!(resp["ok"]["public_key"], ALICE_PUBLIC);
    let text = resp["ok"]["config"].as_str().unwrap();
    assert!(
        text.contains(&format!("PrivateKey = {ALICE_PRIVATE}")),
        "{text}"
    );
    assert!(text.contains("Endpoint = vpn.example.net:51820"), "{text}");
    assert!(text.contains("AllowedIPs = 0.0.0.0/0, ::/0"), "{text}");
}

#[test]
fn a_denied_reveal_is_reveal_denied_and_shows_nothing() {
    let world = World::start(NEXUS);
    import(&world, "Work");
    let uuid = id_of(&world, "Work");
    world.nm.deny_secret_reads(true);
    let resp = world.wire().call(json!({"op": "vpn_reveal", "vpn": uuid}));
    assert_eq!(resp["err"]["code"], "reveal_denied", "{resp}");
    let resp = world.wire().call(json!({"op": "vpn_key", "vpn": uuid}));
    assert_eq!(resp["err"]["code"], "reveal_denied", "{resp}");
}

#[test]
fn a_preshared_key_survives_import_and_reveal() {
    let world = World::start(NEXUS);
    let psk = "pHqPoBBV3BEZ1u1FLKn7OFh+3jbQuLZAK3kA6xr4Zis=";
    let config = conf().replace("AllowedIPs", &format!("PresharedKey = {psk}\nAllowedIPs"));
    world
        .wire()
        .call(json!({"op": "vpn_import", "name": "Psk", "config": config}));
    let uuid = id_of(&world, "Psk");
    let resp = world.wire().call(json!({"op": "vpn_reveal", "vpn": uuid}));
    assert!(
        resp["ok"]["config"]
            .as_str()
            .unwrap()
            .contains(&format!("PresharedKey = {psk}")),
        "{resp}"
    );
}

mod cli {
    use super::*;

    fn out(o: &std::process::Output) -> String {
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    #[test]
    fn import_list_up_down_key_share() {
        let world = World::start(NEXUS);
        let file = world.dir().join("office.conf");
        std::fs::write(&file, conf()).unwrap();
        let o = world.run_nexus(&["vpn", "import", file.to_str().unwrap()]);
        assert!(o.status.success(), "{o:?}");
        assert!(out(&o).contains("Saved office"), "{}", out(&o));
        assert!(out(&o).contains(ALICE_PUBLIC));

        let o = world.run_nexus(&["vpn", "list"]);
        assert!(
            out(&o).contains("office") && out(&o).contains("WireGuard") && out(&o).contains("down"),
            "{}",
            out(&o)
        );

        let o = world.run_nexus(&["vpn", "up", "OFFICE"]);
        assert!(o.status.success(), "{o:?}");
        assert!(out(&o).contains("office is up"), "{}", out(&o));
        assert!(out(&world.run_nexus(&["vpn", "list"])).contains(" up "));

        let o = world.run_nexus(&["vpn", "down", "office"]);
        assert!(out(&o).contains("office is down"), "{}", out(&o));

        let o = world.run_nexus(&["vpn", "key", "office"]);
        assert_eq!(out(&o).trim(), ALICE_PUBLIC);

        let o = world.run_nexus(&["vpn", "share", "office"]);
        assert!(
            out(&o).contains("[Interface]") && out(&o).contains(ALICE_PRIVATE),
            "{}",
            out(&o)
        );
        let o = world.run_nexus(&["vpn", "share", "office", "--qr"]);
        assert!(!out(&o).contains(ALICE_PRIVATE));
        assert!(
            out(&o).lines().count() > 20,
            "a QR code is many rows: {}",
            out(&o)
        );
    }

    #[test]
    fn create_prints_the_public_key_and_failures_have_their_own_exit_codes() {
        let world = World::start(NEXUS);
        let o = world.run_nexus(&["vpn", "create", "Home", "--address", "10.1.0.2/32"]);
        assert!(o.status.success(), "{o:?}");
        let public = out(&o).trim().rsplit(' ').next().unwrap().to_owned();
        let k = world.run_nexus(&["vpn", "key", "home"]);
        assert_eq!(out(&k).trim(), public);

        let o = world.run_nexus(&["vpn", "up", "nothing"]);
        assert_eq!(o.status.code(), Some(30));
        let file = world.dir().join("bad.conf");
        std::fs::write(&file, "[Interface]\n").unwrap();
        let o = world.run_nexus(&["vpn", "import", file.to_str().unwrap()]);
        assert_eq!(o.status.code(), Some(31));
        world
            .nm
            .script_vpn_failure(&world.nm.vpn_uuid("Home").unwrap(), true);
        let o = world.run_nexus(&["vpn", "up", "home"]);
        assert_eq!(o.status.code(), Some(32));
    }
}

mod drops {
    use super::*;
    use std::time::{Duration, Instant};

    fn world() -> World {
        let world = World::builder(NEXUS).notifications(true).start();
        import(&world, "Work");
        world
    }

    fn wait_for(what: &str, cond: impl Fn() -> bool) {
        let start = Instant::now();
        while !cond() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "never happened: {what}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_tunnel_that_dies_on_its_own_is_announced_and_logged() {
        let world = world();
        let notes = world.notifications.as_ref().unwrap();
        let uuid = id_of(&world, "Work");
        let mut events = world.wire();
        events.subscribe();
        world.wire().call(json!({"op": "vpn_up", "vpn": uuid}));
        let up = events.next_event("activity");
        assert_eq!(up["entry"]["text"], "VPN Work is up");
        assert_eq!(notes.count(), 0, "coming up is not an alert");

        world.nm.drop_vpn(&uuid);
        let down = events.next_event("activity");
        assert_eq!(down["entry"]["text"], "VPN Work dropped");
        assert_eq!(down["entry"]["kind"], "vpn_down");
        let note = notes.notified(1);
        assert_eq!(note.summary, "VPN dropped");
        assert!(note.body.contains("Work went down"), "{note:?}");
        assert_eq!(note.hints["desktop-entry"], "nexus");
    }

    #[test]
    fn taking_a_tunnel_down_on_purpose_is_quiet() {
        let world = world();
        let notes = world.notifications.as_ref().unwrap();
        let uuid = id_of(&world, "Work");
        let mut events = world.wire();
        events.subscribe();
        world.wire().call(json!({"op": "vpn_up", "vpn": uuid}));
        events.next_event("activity");
        world.wire().call(json!({"op": "vpn_down", "vpn": uuid}));
        let down = events.next_event("activity");
        assert_eq!(down["entry"]["text"], "VPN Work is down");
        // A real drop after it still alerts, proving the quiet was not a bug.
        world.wire().call(json!({"op": "vpn_up", "vpn": uuid}));
        events.next_event("activity");
        world.nm.drop_vpn(&uuid);
        wait_for("the drop alert", || notes.count() == 1);
        assert_eq!(notes.notified(1).summary, "VPN dropped");
    }

    #[test]
    fn the_alert_has_a_switch() {
        let mut world = World::builder(NEXUS)
            .notifications(true)
            .daemon(false)
            .start();
        world.write_config("[notifications]\nvpn_dropped = false\n");
        world.start_daemon();
        import(&world, "Work");
        let uuid = id_of(&world, "Work");
        let notes = world.notifications.as_ref().unwrap();
        let mut events = world.wire();
        events.subscribe();
        world.wire().call(json!({"op": "vpn_up", "vpn": uuid}));
        events.next_event("activity");
        world.nm.drop_vpn(&uuid);
        let down = events.next_event("activity");
        assert_eq!(down["entry"]["text"], "VPN Work dropped");
        assert_eq!(notes.count(), 0);
    }
}
