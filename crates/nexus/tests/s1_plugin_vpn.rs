//! Seam S1 for plugin VPNs (OpenVPN and friends): a real `nexus daemon`
//! against a fake NetworkManager and a stand-in `nmcli` on the `PATH`.
//! Import, the missing-plugin message, and the user name / password /
//! one-time-code prompts a VPN asks for while it comes up.

use std::thread::JoinHandle;

use nexus_testkit::{Wire, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

const OVPN: &str = "client\ndev tun\nremote vpn.example.net 1194\nauth-user-pass\n\
<key>\n-----BEGIN PRIVATE KEY-----\nS3CR3T-KEY-MATERIAL\n-----END PRIVATE KEY-----\n</key>\n";

fn world() -> World {
    let world = World::start(NEXUS);
    world.install_vpn_plugin("openvpn");
    world.install_fake_nmcli();
    world
}

fn import(world: &World, name: &str, config: &str) -> Value {
    world
        .wire()
        .call(json!({"op": "vpn_import", "name": name, "config": config, "plugin": "openvpn"}))
}

fn list(world: &World) -> Value {
    world.wire().call(json!({"op": "vpn_list"}))["ok"]["vpns"].clone()
}

fn front_end(world: &World) -> Wire {
    let mut wire = world.wire();
    wire.subscribe();
    wire
}

fn up_in_background(world: &World, uuid: &str) -> JoinHandle<Value> {
    let mut wire = world.wire();
    let uuid = uuid.to_owned();
    std::thread::spawn(move || wire.call(json!({"op": "vpn_up", "vpn": uuid})))
}

fn answer(fe: &mut Wire, prompt: &Value, secret: &str) -> Value {
    fe.call(json!({"op": "secret_answer", "prompt": prompt["prompt"]["id"], "secret": secret}))
}

#[test]
fn importing_an_ovpn_file_goes_through_nmcli_and_appears_as_a_plugin_profile() {
    let world = world();
    let resp = import(&world, "Office", OVPN);
    assert_eq!(resp["ok"]["name"], "Office", "{resp}");
    assert_eq!(
        resp["ok"]["public_key"], "",
        "a plugin VPN has no key: {resp}"
    );
    let uuid = world.nm.vpn_uuid("Office").expect("the Profile exists");
    assert_eq!(resp["ok"]["id"], uuid.as_str());
    assert_eq!(
        list(&world),
        json!([{
            "id": uuid, "name": "Office", "kind": "plugin", "state": "down", "addresses": []
        }])
    );

    // nmcli got the right type, and the file was private while it existed.
    let log = world.nmcli_log();
    assert!(log.contains("type openvpn file"), "{log}");
    assert!(log.contains("Office.ovpn"), "{log}");
    assert!(log.contains("file-mode: 600 dir-mode: 700"), "{log}");
    // ... and is gone now.
    let leftovers: Vec<_> = std::fs::read_dir(world.runtime_dir())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("nexus-import-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    // The key material went to nmcli only.
    assert!(!world.daemon_log().contains("S3CR3T"));
}

#[test]
fn a_file_nmcli_refuses_is_invalid_config_and_saves_nothing() {
    let world = world();
    let resp = import(&world, "Broken", "client\ndev tun\n");
    assert_eq!(resp["err"]["code"], "invalid_config", "{resp}");
    let message = resp["err"]["message"].as_str().unwrap();
    assert!(message.contains("OpenVPN"), "{message}");
    assert!(message.contains("missing remote"), "{message}");
    assert!(
        !message.contains(".ovpn"),
        "no scratch path leaks: {message}"
    );
    assert_eq!(list(&world), json!([]));
}

#[test]
fn an_unknown_plugin_is_refused_before_anything_runs() {
    let world = world();
    let resp = world
        .wire()
        .call(json!({"op": "vpn_import", "name": "x", "config": OVPN, "plugin": "../../bin/sh"}));
    assert_eq!(resp["err"]["code"], "invalid_config", "{resp}");
    assert_eq!(world.nmcli_log(), "");
}

#[test]
fn importing_without_the_plugin_names_the_package_to_install() {
    let world = World::start(NEXUS);
    world.install_fake_nmcli();
    let resp = import(&world, "Office", OVPN);
    assert_eq!(resp["err"]["code"], "missing_plugin", "{resp}");
    let message = resp["err"]["message"].as_str().unwrap();
    assert!(message.contains("networkmanager-openvpn"), "{message}");
    assert_eq!(world.nmcli_log(), "", "nmcli never ran");
}

#[test]
fn bringing_up_a_vpn_whose_plugin_is_missing_names_the_package() {
    let world = World::start(NEXUS);
    world
        .nm
        .add_plugin_vpn_of("uuid-ovpn", "Office", Some("openvpn"));
    let resp = world
        .wire()
        .call(json!({"op": "vpn_up", "vpn": "uuid-ovpn"}));
    assert_eq!(resp["err"]["code"], "missing_plugin", "{resp}");
    assert!(
        resp["err"]["message"]
            .as_str()
            .unwrap()
            .contains("networkmanager-openvpn"),
        "{resp}"
    );
    assert_eq!(world.nm.vpn_active_state("uuid-ovpn"), 0);

    // With the package in place the same VPN comes up.
    world.install_vpn_plugin("openvpn");
    let resp = world
        .wire()
        .call(json!({"op": "vpn_up", "vpn": "uuid-ovpn"}));
    assert_eq!(resp["ok"]["state"], "up", "{resp}");
}

#[test]
fn a_vpn_that_wants_a_password_opens_a_prompt_and_the_answer_reaches_the_plugin() {
    let world = world();
    world
        .nm
        .add_plugin_vpn_of("uuid-ovpn", "Office", Some("openvpn"));
    world.nm.script_vpn_asks("uuid-ovpn", &["password"]);
    let mut fe = front_end(&world);
    let up = up_in_background(&world, "uuid-ovpn");

    let opened = fe.next_event("secret_prompt_opened");
    assert_eq!(opened["prompt"]["kind"], "vpn_secret", "{opened}");
    assert_eq!(opened["prompt"]["field"], "password");
    assert_eq!(opened["prompt"]["network"], "Office");
    assert_eq!(opened["prompt"]["network_id"], "uuid-ovpn");
    assert_eq!(opened["prompt"]["message"], "Password for Office");
    let resp = answer(&mut fe, &opened, "hunter2-vpn");
    assert!(resp.get("ok").is_some(), "{resp}");
    assert_eq!(fe.next_event("secret_prompt_closed")["reason"], "answered");

    let done = up.join().unwrap();
    assert_eq!(done["ok"]["state"], "up", "{done}");
    assert_eq!(
        world.nm.vpn_answers("uuid-ovpn"),
        [("password".to_owned(), "hunter2-vpn".to_owned())]
    );
    assert!(!world.daemon_log().contains("hunter2-vpn"));
}

#[test]
fn user_name_password_and_one_time_code_are_asked_one_after_another() {
    let world = world();
    world
        .nm
        .add_plugin_vpn_of("uuid-ovpn", "Office", Some("openvpn"));
    world
        .nm
        .script_vpn_asks("uuid-ovpn", &["username", "password", "otp"]);
    let mut fe = front_end(&world);
    let up = up_in_background(&world, "uuid-ovpn");

    for (field, message, reply) in [
        ("username", "User name for Office", "ada"),
        ("password", "Password for Office", "s3cret"),
        ("otp", "One-time code for Office", "123456"),
    ] {
        let opened = fe.next_event("secret_prompt_opened");
        assert_eq!(opened["prompt"]["field"], field, "{opened}");
        assert_eq!(opened["prompt"]["message"], message, "{opened}");
        answer(&mut fe, &opened, reply);
        fe.next_event("secret_prompt_closed");
    }
    assert_eq!(up.join().unwrap()["ok"]["state"], "up");
    assert_eq!(
        world.nm.vpn_answers("uuid-ovpn"),
        [
            ("otp".to_owned(), "123456".to_owned()),
            ("password".to_owned(), "s3cret".to_owned()),
            ("username".to_owned(), "ada".to_owned()),
        ]
    );
}

#[test]
fn declining_the_prompt_leaves_the_vpn_down_with_a_clear_error() {
    let world = world();
    world
        .nm
        .add_plugin_vpn_of("uuid-ovpn", "Office", Some("openvpn"));
    world.nm.script_vpn_asks("uuid-ovpn", &["password"]);
    let mut fe = front_end(&world);
    let up = up_in_background(&world, "uuid-ovpn");
    let opened = fe.next_event("secret_prompt_opened");
    let resp = fe.call(json!({"op": "secret_cancel", "prompt": opened["prompt"]["id"]}));
    assert!(resp.get("ok").is_some(), "{resp}");
    let done = up.join().unwrap();
    assert_eq!(done["err"]["code"], "vpn_failed", "{done}");
    assert_eq!(list(&world)[0]["state"], "down");
}

#[test]
fn a_hint_from_the_plugin_becomes_the_prompt_sentence() {
    let world = world();
    world
        .nm
        .add_plugin_vpn_of("uuid-ovpn", "Office", Some("openvpn"));
    world.nm.script_vpn_asks(
        "uuid-ovpn",
        &[
            "x-vpn-message:Enter the code from your authenticator",
            "otp",
        ],
    );
    let mut fe = front_end(&world);
    let up = up_in_background(&world, "uuid-ovpn");
    let opened = fe.next_event("secret_prompt_opened");
    assert_eq!(opened["prompt"]["field"], "otp");
    assert_eq!(
        opened["prompt"]["message"],
        "Enter the code from your authenticator"
    );
    answer(&mut fe, &opened, "654321");
    assert_eq!(up.join().unwrap()["ok"]["state"], "up");
}

#[test]
fn the_cli_picks_the_plugin_from_the_extension_and_missing_plugins_exit_24() {
    let world = world();
    let file = world.dir().join("Branch.ovpn");
    std::fs::write(&file, OVPN).unwrap();
    let out = world.run_nexus(&["vpn", "import", file.to_str().unwrap()]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "Saved Branch.");
    assert_eq!(list(&world)[0]["name"], "Branch");

    let bare = World::start(NEXUS);
    bare.install_fake_nmcli();
    let file = bare.dir().join("Branch.ovpn");
    std::fs::write(&file, OVPN).unwrap();
    let out = bare.run_nexus(&["vpn", "import", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(24), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("networkmanager-openvpn"),
        "{out:?}"
    );
}
