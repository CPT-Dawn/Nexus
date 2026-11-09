//! Seam S1 for Ethernet: a real `nexus daemon` against a fake NetworkManager
//! with wired Devices, over the socket and the CLI.

use nexus_testkit::{DeviceId, Lease, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

fn world() -> (World, DeviceId) {
    let world = World::start(NEXUS);
    let eth = world.nm.add_ethernet_device("enp3s0");
    (world, eth)
}

fn ports(world: &World) -> Vec<Value> {
    let resp = world.wire().call(json!({"op": "ethernet_list"}));
    resp["ok"]["ports"].as_array().expect("ports").clone()
}

fn port(world: &World) -> Value {
    ports(world).remove(0)
}

fn connect(world: &World) -> Value {
    world.wire().call(json!({"op": "ethernet_connect"}))
}

fn peap(identity: &str, password: &str) -> Value {
    json!({
        "method": "peap",
        "phase2": "mschapv2",
        "identity": identity,
        "password": password,
        "domain_suffix": "radius.office.example",
    })
}

fn join_eap(world: &World, eap: Value) -> Value {
    world
        .wire()
        .call(json!({"op": "ethernet_join_eap", "eap": eap}))
}

#[test]
fn a_machine_without_ethernet_lists_no_ports() {
    let world = World::start(NEXUS);
    world.nm.add_wifi_device("wlan0");
    assert!(ports(&world).is_empty());
}

#[test]
fn a_plugged_port_with_nothing_joined_is_disconnected() {
    let (world, _) = world();
    let p = port(&world);
    assert_eq!(p["id"], "enp3s0");
    assert_eq!(p["state"], "disconnected", "{p}");
    assert_eq!(p["speed_mbps"], 1000);
    assert!(p["mac"].as_str().unwrap().starts_with("52:54:00"), "{p}");
    assert!(p.get("trouble").is_none());
}

#[test]
fn connecting_brings_the_link_up_and_shows_its_details() {
    let (world, eth) = world();
    world.nm.set_lease(
        eth,
        Lease {
            address: "10.0.0.42".into(),
            prefix: 24,
            gateway: "10.0.0.1".into(),
            dns: vec!["10.0.0.2".into(), "1.1.1.1".into()],
        },
    );
    world.nm.set_link_speed(eth, 2500);
    let resp = connect(&world);
    assert_eq!(resp["ok"]["port"], "enp3s0", "{resp}");
    assert!(world.nm.ethernet_up(eth));
    let p = port(&world);
    assert_eq!(p["state"], "connected", "{p}");
    assert_eq!(p["speed_mbps"], 2500);
    assert_eq!(
        p["addresses"],
        json!([{"address": "10.0.0.42", "prefix": 24}])
    );
    assert_eq!(p["gateway"], "10.0.0.1");
    assert_eq!(p["dns"], json!(["10.0.0.2", "1.1.1.1"]));
    assert_eq!(p["profile"]["name"], "Wired enp3s0");
    assert_eq!(p["profile"]["enterprise"], false);
    assert_eq!(world.nm.ethernet_profiles().len(), 1);
}

#[test]
fn connecting_again_reuses_the_profile_and_disconnecting_keeps_it() {
    let (world, eth) = world();
    assert!(connect(&world).get("ok").is_some());
    let down = world.wire().call(json!({"op": "ethernet_disconnect"}));
    assert_eq!(down["ok"]["port"], "enp3s0", "{down}");
    assert!(!world.nm.ethernet_up(eth));
    assert_eq!(port(&world)["state"], "disconnected");
    let again = world.wire().call(json!({"op": "ethernet_disconnect"}));
    assert_eq!(again["err"]["code"], "not_connected", "{again}");
    assert!(connect(&world).get("ok").is_some());
    assert!(world.nm.ethernet_up(eth));
    assert_eq!(world.nm.ethernet_profiles().len(), 1, "no second Profile");
}

#[test]
fn an_unplugged_cable_and_a_misconfigured_port_are_told_apart() {
    let (world, eth) = world();

    // Cable out.
    world.nm.set_carrier(eth, false);
    let unplugged = port(&world);
    assert_eq!(unplugged["state"], "unplugged", "{unplugged}");
    assert!(unplugged.get("trouble").is_none());
    assert!(unplugged.get("speed_mbps").is_none());
    let refused = connect(&world);
    assert_eq!(refused["err"]["code"], "unplugged", "{refused}");
    assert!(
        refused["err"]["message"]
            .as_str()
            .unwrap()
            .contains("No cable detected")
    );
    assert_eq!(world.nm.ethernet_profiles().len(), 0, "nothing was made");

    // Cable in, but the network does not answer.
    world.nm.set_carrier(eth, true);
    world.nm.set_dhcp(eth, false);
    let failed = connect(&world);
    assert_eq!(failed["err"]["code"], "no_address", "{failed}");
    let stuck = port(&world);
    assert_eq!(stuck["state"], "misconfigured", "{stuck}");
    assert_eq!(stuck["trouble"]["kind"], "no_address");
    let words = stuck["trouble"]["message"].as_str().unwrap();
    assert!(words.contains("plugged in"), "{words}");
    assert!(!words.contains("No cable"), "{words}");
    assert_eq!(
        world.nm.ethernet_profiles().len(),
        0,
        "a failed join leaves no Profile"
    );

    // Pulling the cable turns the failure into a plain unplugged port.
    world.nm.set_carrier(eth, false);
    assert_eq!(port(&world)["state"], "unplugged");
}

#[test]
fn several_ports_need_a_name() {
    let (world, _) = world();
    world.nm.add_ethernet_device("enp4s0");
    let resp = connect(&world);
    assert_eq!(resp["err"]["code"], "unknown_port", "{resp}");
    assert!(resp["err"]["message"].as_str().unwrap().contains("enp4s0"));
    let named = world
        .wire()
        .call(json!({"op": "ethernet_connect", "port": "ENP4S0"}));
    assert_eq!(named["ok"]["port"], "enp4s0", "{named}");
    let wrong = world
        .wire()
        .call(json!({"op": "ethernet_connect", "port": "eth9"}));
    assert_eq!(wrong["err"]["code"], "unknown_port", "{wrong}");
}

#[test]
fn a_cable_pulled_or_plugged_is_pushed_to_subscribers() {
    let (world, eth) = world();
    let mut ev = world.wire();
    ev.subscribe();
    world.nm.set_carrier(eth, false);
    let e = ev.next_event("ethernet_changed");
    assert_eq!(e["ethernet"]["ports"][0]["state"], "unplugged", "{e}");
    world.nm.set_carrier(eth, true);
    loop {
        let e = ev.next_event("ethernet_changed");
        if e["ethernet"]["ports"][0]["state"] == "disconnected" {
            break;
        }
    }
}

// ---- wired 802.1X -----------------------------------------------------------

#[test]
fn a_plain_profile_on_an_802_1x_port_is_misconfigured_and_wants_a_sign_in() {
    let (world, eth) = world();
    world
        .nm
        .set_port_eap(eth, "alice@office.example", "hunter2");
    let resp = connect(&world);
    assert_eq!(resp["err"]["code"], "no_address", "{resp}");
    let p = port(&world);
    assert_eq!(p["state"], "misconfigured", "{p}");
    assert!(
        p["trouble"]["message"].as_str().unwrap().contains("802.1X"),
        "{p}"
    );
}

#[test]
fn signing_in_makes_a_profile_with_the_whole_802_1x_setting() {
    let (world, eth) = world();
    world
        .nm
        .set_port_eap(eth, "alice@office.example", "hunter2");
    let resp = join_eap(&world, peap("alice@office.example", "hunter2"));
    assert_eq!(resp["ok"]["port"], "enp3s0", "{resp}");
    let uuid = resp["ok"]["profile"].as_str().unwrap();
    assert!(world.nm.ethernet_up(eth));
    let text = |k: &str| world.nm.profile_text(uuid, "802-1x", k);
    assert_eq!(text("identity").as_deref(), Some("alice@office.example"));
    assert_eq!(text("phase2-auth").as_deref(), Some("mschapv2"));
    assert_eq!(
        text("domain-suffix-match").as_deref(),
        Some("radius.office.example")
    );
    let p = port(&world);
    assert_eq!(p["state"], "connected", "{p}");
    assert_eq!(p["profile"]["enterprise"], true);
}

#[test]
fn signing_in_gives_an_existing_plain_profile_its_802_1x_setting() {
    let (world, eth) = world();
    world
        .nm
        .set_port_eap(eth, "alice@office.example", "hunter2");
    world
        .nm
        .add_ethernet_profile("11111111-1111-4111-8111-111111111111", "Office", "enp3s0");
    let resp = join_eap(&world, peap("alice@office.example", "hunter2"));
    assert_eq!(
        resp["ok"]["profile"], "11111111-1111-4111-8111-111111111111",
        "{resp}"
    );
    assert_eq!(world.nm.ethernet_profiles().len(), 1, "no second Profile");
    assert!(world.nm.ethernet_up(eth));
    assert_eq!(
        world
            .nm
            .profile_text("11111111-1111-4111-8111-111111111111", "802-1x", "identity")
            .as_deref(),
        Some("alice@office.example")
    );
}

#[test]
fn a_refused_sign_in_is_a_wrong_password_and_leaves_things_as_they_were() {
    let (world, eth) = world();
    world
        .nm
        .set_port_eap(eth, "alice@office.example", "hunter2");
    let resp = join_eap(&world, peap("alice@office.example", "nope"));
    assert_eq!(resp["err"]["code"], "wrong_password", "{resp}");
    assert!(
        resp["err"]["message"]
            .as_str()
            .unwrap()
            .contains("refused the 802.1X sign-in"),
        "{resp}"
    );
    assert_eq!(
        world.nm.ethernet_profiles().len(),
        0,
        "no Profile for a failed sign-in"
    );

    // With a Profile already saved, the failed sign-in does not change it.
    world
        .nm
        .add_ethernet_profile("22222222-2222-4222-8222-222222222222", "Office", "enp3s0");
    let resp = join_eap(&world, peap("alice@office.example", "nope"));
    assert_eq!(resp["err"]["code"], "wrong_password", "{resp}");
    assert_eq!(
        world
            .nm
            .profile_text("22222222-2222-4222-8222-222222222222", "802-1x", "identity"),
        None
    );
    let p = port(&world);
    assert_eq!(p["state"], "misconfigured", "{p}");
    assert_eq!(p["trouble"]["kind"], "sign_in");
}

#[test]
fn a_sign_in_that_fails_validation_never_reaches_networkmanager() {
    let (world, _) = world();
    let mut eap = peap("alice@office.example", "hunter2");
    eap["password"] = json!("");
    let resp = join_eap(&world, eap);
    assert_eq!(resp["err"]["code"], "invalid_settings", "{resp}");
    assert_eq!(world.nm.ethernet_profiles().len(), 0);
}

#[test]
fn a_sign_in_on_an_unplugged_port_is_unplugged() {
    let (world, eth) = world();
    world.nm.set_carrier(eth, false);
    let resp = join_eap(&world, peap("alice@office.example", "hunter2"));
    assert_eq!(resp["err"]["code"], "unplugged", "{resp}");
}

// ---- Profile settings of a wired Profile ------------------------------------

const OFFICE: &str = "33333333-3333-4333-8333-333333333333";

fn get(world: &World) -> Value {
    world
        .wire()
        .call(json!({"op": "profile_get", "profile": OFFICE}))
}

#[test]
fn a_wired_profile_is_read_and_edited_by_uuid() {
    let (world, _) = world();
    world.nm.add_ethernet_profile(OFFICE, "Office", "enp3s0");
    let view = get(&world);
    assert_eq!(view["ok"]["uuid"], OFFICE, "{view}");
    assert_eq!(view["ok"]["ssid"], "Office");
    assert_eq!(view["ok"]["active"], false);
    assert_eq!(view["ok"]["settings"]["ipv4"]["method"], "auto");
    let version = view["ok"]["version"].as_u64().unwrap();

    let mut settings = view["ok"]["settings"].clone();
    settings["ipv4"] = json!({
        "method": "manual",
        "addresses": [{"address": "192.168.7.20", "prefix": 24}],
        "gateway": "192.168.7.1",
    });
    settings["dns"] = json!(["9.9.9.9"]);
    settings["mac"] = json!("every_time");
    settings["metered"] = json!(true);
    let applied = world.wire().call(json!({
        "op": "profile_update", "profile": OFFICE, "version": version, "settings": settings,
    }));
    // Not active: written at once, no countdown.
    assert!(applied["ok"].get("pending").is_none(), "{applied}");
    assert_eq!(
        world.nm.profile_text(OFFICE, "ipv4", "method").as_deref(),
        Some("manual")
    );
    assert_eq!(
        world
            .nm
            .profile_text(OFFICE, "802-3-ethernet", "assigned-mac-address")
            .as_deref(),
        Some("random")
    );
    assert_eq!(
        world.nm.profile_int(OFFICE, "connection", "metered"),
        Some(1)
    );
    let again = get(&world);
    assert_eq!(
        again["ok"]["settings"]["ipv4"]["gateway"], "192.168.7.1",
        "{again}"
    );
    assert_eq!(again["ok"]["settings"]["mac"], "every_time");
}

#[test]
fn a_per_network_mac_on_ethernet_is_the_stable_option() {
    let (world, _) = world();
    world.nm.add_ethernet_profile(OFFICE, "Office", "enp3s0");
    let view = get(&world);
    let version = view["ok"]["version"].as_u64().unwrap();
    let mut settings = view["ok"]["settings"].clone();
    settings["mac"] = json!("per_network");
    let applied = world.wire().call(json!({
        "op": "profile_update", "profile": OFFICE, "version": version, "settings": settings,
    }));
    assert!(applied.get("ok").is_some(), "{applied}");
    assert_eq!(
        world
            .nm
            .profile_text(OFFICE, "802-3-ethernet", "assigned-mac-address")
            .as_deref(),
        Some("stable")
    );
}

#[test]
fn editing_the_profile_that_carries_the_link_runs_under_a_checkpoint() {
    let (world, eth) = world();
    world.nm.add_ethernet_profile(OFFICE, "Office", "enp3s0");
    assert!(connect(&world).get("ok").is_some());
    assert!(world.nm.ethernet_up(eth));
    let view = get(&world);
    assert_eq!(view["ok"]["active"], true, "{view}");
    let version = view["ok"]["version"].as_u64().unwrap();
    let mut settings = view["ok"]["settings"].clone();
    settings["dns"] = json!(["9.9.9.9"]);
    let applied = world.wire().call(json!({
        "op": "profile_update", "profile": OFFICE, "version": version,
        "settings": settings, "rollback_secs": 30,
    }));
    let pending = applied["ok"]["pending"]["id"]
        .as_u64()
        .unwrap_or_else(|| panic!("{applied}"));
    assert_eq!(world.nm.checkpoints_open(), 1);
    // Reverting puts the old settings back.
    let reverted = world
        .wire()
        .call(json!({"op": "profile_revert", "pending": pending}));
    assert!(reverted.get("ok").is_some(), "{reverted}");
    assert_eq!(world.nm.checkpoint_rollbacks(), 1);
    assert_eq!(world.nm.profile_text(OFFICE, "ipv4", "dns-data"), None);
}

#[test]
fn a_stale_wired_edit_is_a_conflict() {
    let (world, _) = world();
    world.nm.add_ethernet_profile(OFFICE, "Office", "enp3s0");
    let view = get(&world);
    let version = view["ok"]["version"].as_u64().unwrap();
    world.nm.edit_profile_externally(OFFICE);
    let applied = world.wire().call(json!({
        "op": "profile_update", "profile": OFFICE, "version": version,
        "settings": view["ok"]["settings"],
    }));
    assert_eq!(applied["err"]["code"], "conflict", "{applied}");
}

#[test]
fn an_unknown_profile_uuid_is_refused() {
    let (world, _) = world();
    let resp = world
        .wire()
        .call(json!({"op": "profile_get", "profile": "nope"}));
    assert_eq!(resp["err"]["code"], "unknown_network", "{resp}");
}

// ---- CLI --------------------------------------------------------------------

#[test]
fn the_cli_lists_ports_and_tells_unplugged_from_misconfigured_by_exit_code() {
    let (world, eth) = world();
    let out = world.run_nexus(&["ethernet", "list"]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("enp3s0") && text.contains("disconnected"),
        "{text}"
    );

    world.nm.set_carrier(eth, false);
    let out = world.run_nexus(&["ethernet", "list"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("unplugged") && text.contains("No cable detected"),
        "{text}"
    );
    let out = world.run_nexus(&["ethernet", "connect"]);
    assert_eq!(out.status.code(), Some(27), "unplugged");

    world.nm.set_carrier(eth, true);
    world.nm.set_dhcp(eth, false);
    let out = world.run_nexus(&["ethernet", "connect"]);
    assert_eq!(out.status.code(), Some(7), "no address, like Wi-Fi");
    let out = world.run_nexus(&["ethernet", "list", "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ports"][0]["state"], "misconfigured", "{v}");

    let out = world.run_nexus(&["ethernet", "connect", "eth9"]);
    assert_eq!(out.status.code(), Some(28), "unknown port");
}

#[test]
fn the_cli_connects_and_disconnects() {
    let (world, eth) = world();
    let out = world.run_nexus(&["ethernet", "connect"]);
    assert!(out.status.success(), "{out:?}");
    assert!(world.nm.ethernet_up(eth));
    let out = world.run_nexus(&["ethernet", "disconnect"]);
    assert!(out.status.success(), "{out:?}");
    assert!(!world.nm.ethernet_up(eth));
    let out = world.run_nexus(&["ethernet", "disconnect"]);
    assert_eq!(out.status.code(), Some(11), "not connected");
}

#[test]
fn the_keyring_switch_is_refused_cleanly_on_a_wired_profile() {
    let (world, _) = world();
    world.nm.add_ethernet_profile(OFFICE, "Office", "enp3s0");
    let view = get(&world);
    assert_eq!(view["ok"]["settings"]["keyring"], false, "{view}");
    let mut settings = view["ok"]["settings"].clone();
    settings["keyring"] = json!(true);
    let resp = world.wire().call(json!({
        "op": "profile_update", "profile": OFFICE,
        "version": view["ok"]["version"], "settings": settings,
    }));
    assert_eq!(resp["err"]["code"], "invalid_settings", "{resp}");
}
