//! Seam S1 for the Doctor: a real Daemon, a fake NetworkManager and a fake
//! systemd on the private bus. Assertions are on the wire and the CLI.

use nexus_testkit::{ActiveConnection, NmConnectivity, NmState, Unit, World};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

/// A name no resolver will ever answer (RFC 6761).
const NO_SUCH_HOST: &str = "http://nexus-doctor.invalid/";

const LOCAL_CHECK: &str = "[connectivity]\nurl = \"http://localhost/check\"\n";
/// No outside contact and no probe of the host this test runs on.
const NO_CHECK: &str = "[connectivity]\nenabled = false\n[doctor]\ngateway = \"off\"\n";

/// A world whose Daemon started with `config` in place (the config is
/// reloaded lazily, so it has to exist before the Daemon does).
fn world_with(config: &str, nm_running: bool) -> World {
    let mut world = World::builder(NEXUS)
        .nm_running(nm_running)
        .daemon(false)
        .start();
    world.write_config(config);
    world.start_daemon();
    world
}

/// A world that is online over Wi-Fi with a reachable gateway, whose
/// connectivity check host resolves from /etc/hosts.
fn healthy() -> World {
    healthy_with(LOCAL_CHECK)
}

fn healthy_with(config: &str) -> World {
    let world = world_with(config, true);
    world.nm.set_primary(Some(ActiveConnection::wifi("Home")));
    world.nm.set_connectivity(NmConnectivity::Full);
    world.nm.set_state(NmState::ConnectedGlobal);
    // Nothing listens on port 53 of loopback: a refusal still proves the host answers.
    world.nm.set_gateway(Some("127.0.0.1"));
    world.nm.set_ipv6(true);
    world
        .systemd
        .set_unit("NetworkManager.service", Unit::Active);
    world
}

fn findings(world: &World) -> Vec<Value> {
    let resp = world.wire().call(json!({"op": "doctor"}));
    resp["ok"]["findings"]
        .as_array()
        .unwrap_or_else(|| panic!("no findings in {resp}"))
        .clone()
}

fn find<'a>(all: &'a [Value], id: &str) -> &'a Value {
    all.iter()
        .find(|f| f["id"] == id)
        .unwrap_or_else(|| panic!("no `{id}` finding in {all:#?}"))
}

fn has(all: &[Value], id: &str) -> bool {
    all.iter().any(|f| f["id"] == id)
}

fn commands(finding: &Value) -> Vec<String> {
    finding["fix"]["commands"]
        .as_array()
        .map(|c| c.iter().map(|c| c.as_str().unwrap().to_owned()).collect())
        .unwrap_or_default()
}

fn trouble(all: &[Value]) -> Vec<&str> {
    all.iter()
        .filter(|f| f["severity"] == "warning" || f["severity"] == "problem")
        .map(|f| f["id"].as_str().unwrap())
        .collect()
}

#[test]
fn a_healthy_machine_has_no_trouble_and_says_which_networkmanager() {
    let world = healthy();
    let all = findings(&world);
    assert_eq!(trouble(&all), Vec::<&str>::new(), "{all:#?}");
    let nm = find(&all, "networkmanager");
    assert_eq!(nm["severity"], "ok");
    assert!(nm["title"].as_str().unwrap().contains("1.58.1"), "{nm}");
    assert_eq!(find(&all, "dns")["severity"], "ok");
    assert_eq!(find(&all, "gateway")["severity"], "ok");
    assert_eq!(find(&all, "ipv6")["severity"], "ok");
}

#[test]
fn every_finding_explains_itself_in_words() {
    let world = healthy();
    world.nm.set_radio(false, true);
    for f in findings(&world) {
        assert!(!f["title"].as_str().unwrap().is_empty(), "{f}");
        assert!(!f["cause"].as_str().unwrap().is_empty(), "{f}");
    }
}

#[test]
fn networkmanager_not_installed_says_what_to_install() {
    let world = world_with(NO_CHECK, false);
    let all = findings(&world);
    let nm = find(&all, "networkmanager");
    assert_eq!(nm["severity"], "problem");
    assert!(nm["title"].as_str().unwrap().contains("not installed"));
    assert_eq!(
        nm["fix"]["runnable"], false,
        "installing is the user's call"
    );
    assert!(
        nm["fix"]["text"]
            .as_str()
            .unwrap()
            .contains("NetworkManager")
    );
}

#[test]
fn a_stopped_networkmanager_next_to_networkd_gets_one_ordered_plan() {
    let world = world_with(NO_CHECK, false);
    world
        .systemd
        .set_unit("NetworkManager.service", Unit::Inactive);
    world
        .systemd
        .set_unit("systemd-networkd.service", Unit::Active);
    // iwd is installed but not running: not a conflict, not touched.
    world.systemd.set_unit("iwd.service", Unit::Inactive);
    let all = findings(&world);

    let nm = find(&all, "networkmanager");
    assert_eq!(nm["severity"], "problem");
    assert_eq!(nm["fix"]["runnable"], true);
    assert_eq!(nm["fix"]["privileged"], true);
    assert_eq!(
        commands(nm),
        [
            "sudo systemctl disable --now systemd-networkd.service",
            "sudo systemctl enable --now NetworkManager.service",
        ]
    );

    let conflict = find(&all, "conflict-networkd");
    assert_eq!(conflict["severity"], "warning");
    assert!(
        conflict["cause"]
            .as_str()
            .unwrap()
            .contains("systemd-networkd")
    );
    assert!(!has(&all, "conflict-iwd"), "an inactive iwd is no conflict");
}

#[test]
fn a_running_iwd_stays_and_becomes_networkmanagers_wifi_backend() {
    let world = world_with(NO_CHECK, false);
    world
        .systemd
        .set_unit("NetworkManager.service", Unit::Inactive);
    world.systemd.set_unit("iwd.service", Unit::Active);
    let all = findings(&world);
    let plan = commands(find(&all, "networkmanager")).join("\n");
    assert!(plan.contains("wifi.backend=iwd"), "{plan}");
    assert!(
        !plan.contains("disable --now iwd"),
        "iwd is kept as the backend: {plan}"
    );
    assert!(
        plan.contains("enable --now NetworkManager.service"),
        "{plan}"
    );
}

#[test]
fn networkd_next_to_a_running_networkmanager_is_a_conflict_with_its_own_fix() {
    let world = healthy();
    world
        .systemd
        .set_unit("systemd-networkd.service", Unit::Active);
    world.systemd.set_unit("dhcpcd.service", Unit::Active);
    // NetworkManager starts wpa_supplicant itself: not a conflict.
    world
        .systemd
        .set_unit("wpa_supplicant.service", Unit::Active);
    let all = findings(&world);
    assert_eq!(
        commands(find(&all, "conflict-networkd")),
        ["sudo systemctl disable --now systemd-networkd.service"]
    );
    assert_eq!(
        commands(find(&all, "conflict-dhcpcd")),
        ["sudo systemctl disable --now dhcpcd.service"]
    );
    assert!(!has(&all, "conflict-wpa_supplicant"));
    // Found while NetworkManager runs, so the plan is the conflict's own.
    assert_eq!(find(&all, "networkmanager")["severity"], "ok");
}

#[test]
fn a_hardware_or_software_radio_block_is_named() {
    let world = healthy();
    world.nm.set_radio(false, true);
    let all = findings(&world);
    let soft = find(&all, "rfkill");
    assert_eq!(soft["severity"], "problem");
    assert_eq!(commands(soft), ["nmcli radio wifi on"]);
    assert_eq!(soft["fix"]["privileged"], false);

    world.nm.set_radio(false, false);
    let all = findings(&world);
    let hard = find(&all, "rfkill");
    assert!(
        hard["title"].as_str().unwrap().contains("hardware"),
        "{hard}"
    );
    assert!(commands(hard).is_empty(), "only a switch helps: {hard}");
}

#[test]
fn offline_with_networkmanager_up_says_to_join_a_network() {
    let world = World::start(NEXUS);
    world
        .systemd
        .set_unit("NetworkManager.service", Unit::Active);
    let all = findings(&world);
    let link = find(&all, "link");
    assert_eq!(link["severity"], "problem");
    assert!(
        link["fix"]["text"]
            .as_str()
            .unwrap()
            .contains("nexus wifi join")
    );
    // Without a Link there is nothing to resolve or ping.
    assert!(!has(&all, "dns") && !has(&all, "gateway"), "{all:#?}");
}

#[test]
fn a_name_that_does_not_resolve_is_a_dns_problem_and_names_the_host() {
    let world = healthy_with(&format!("[connectivity]\nurl = \"{NO_SUCH_HOST}\"\n"));
    world.nm.set_connectivity(NmConnectivity::Limited);
    let all = findings(&world);
    let dns = find(&all, "dns");
    assert_eq!(dns["severity"], "problem");
    assert!(
        dns["cause"]
            .as_str()
            .unwrap()
            .contains("nexus-doctor.invalid")
    );
    assert_eq!(find(&all, "gateway")["severity"], "ok");
}

#[test]
fn an_unreachable_gateway_is_a_problem() {
    let world = healthy();
    // TEST-NET-1 (RFC 5737) is never routed.
    world.nm.set_gateway(Some("192.0.2.1"));
    world.nm.set_connectivity(NmConnectivity::Limited);
    let all = findings(&world);
    let gw = find(&all, "gateway");
    assert_eq!(gw["severity"], "problem");
    assert!(gw["title"].as_str().unwrap().contains("192.0.2.1"), "{gw}");
}

#[test]
fn connected_with_working_dns_and_gateway_but_no_internet_blames_beyond_the_router() {
    let world = healthy();
    world.nm.set_connectivity(NmConnectivity::Limited);
    let all = findings(&world);
    let net = find(&all, "internet");
    assert_eq!(net["severity"], "problem");
    assert!(net["cause"].as_str().unwrap().contains("router"), "{net}");
}

#[test]
fn a_portal_is_named_and_blamed_instead_of_dns() {
    let world = healthy_with(&format!("[connectivity]\nurl = \"{NO_SUCH_HOST}\"\n"));
    world.nm.set_connectivity(NmConnectivity::Portal);
    let all = findings(&world);
    assert_eq!(find(&all, "portal")["severity"], "warning");
    // A portal answers DNS its own way; resolving would only confuse.
    assert!(!has(&all, "dns"), "{all:#?}");
}

#[test]
fn missing_ipv6_is_information_not_trouble() {
    let world = healthy();
    world.nm.set_ipv6(false);
    let all = findings(&world);
    assert_eq!(find(&all, "ipv6")["severity"], "info");
    assert_eq!(trouble(&all), Vec::<&str>::new());
}

#[test]
fn the_gateway_to_probe_can_be_pinned_or_switched_off_in_config() {
    // Without NetworkManager the Doctor would read the host's route table;
    // config says which gateway to test instead.
    let alive = world_with(
        "[connectivity]\nenabled = false\n[doctor]\ngateway = \"127.0.0.1\"\n",
        false,
    );
    let all = findings(&alive);
    assert_eq!(find(&all, "gateway")["severity"], "ok", "{all:#?}");

    let dead = world_with(
        "[connectivity]\nenabled = false\n[doctor]\ngateway = \"192.0.2.1\"\n",
        false,
    );
    assert_eq!(find(&findings(&dead), "gateway")["severity"], "problem");

    let off = world_with(NO_CHECK, false);
    assert!(!has(&findings(&off), "gateway"));
}

#[test]
fn a_gateway_that_is_not_an_address_is_refused_by_config() {
    let world = World::start(NEXUS);
    let resp = world
        .wire()
        .call(json!({"op": "config_set", "key": "doctor.gateway", "value": "the router"}));
    assert!(resp.get("err").is_some(), "{resp}");
}

#[test]
fn a_switched_off_connectivity_check_is_said_and_nothing_is_resolved() {
    let world = healthy_with(NO_CHECK);
    let all = findings(&world);
    assert_eq!(find(&all, "connectivity-check")["severity"], "info");
    assert!(
        !has(&all, "dns"),
        "no host to resolve when the check is off"
    );
}

#[test]
fn fixes_need_an_explicit_yes() {
    let world = healthy();
    world
        .systemd
        .set_unit("systemd-networkd.service", Unit::Active);
    let resp = world
        .wire()
        .call(json!({"op": "doctor_fix", "finding": "conflict-networkd"}));
    assert_eq!(resp["err"]["code"], "not_confirmed", "{resp}");
}

#[test]
fn a_confirmed_fix_runs_through_pkexec_and_only_its_own_commands() {
    let world = healthy();
    world
        .systemd
        .set_unit("systemd-networkd.service", Unit::Active);
    let log = world.dir().join("pkexec.log");
    world.install_tool(
        "pkexec",
        &format!(
            "printf '%s|' \"$@\" >> {}; echo >> {}",
            log.display(),
            log.display()
        ),
    );
    let resp = world
        .wire()
        .call(json!({"op": "doctor_fix", "finding": "conflict-networkd", "confirm": true}));
    assert_eq!(
        resp["ok"]["ran"],
        json!(["sudo systemctl disable --now systemd-networkd.service"]),
        "{resp}"
    );
    let logged = std::fs::read_to_string(&log).expect("pkexec was called");
    assert_eq!(
        logged.trim(),
        "/bin/sh|-c|systemctl disable --now systemd-networkd.service|"
    );
}

#[test]
fn an_unprivileged_fix_does_not_touch_pkexec() {
    let world = healthy();
    world.nm.set_radio(false, true);
    let log = world.dir().join("tools.log");
    world.install_tool("pkexec", &format!("echo pkexec >> {}", log.display()));
    world.install_tool("nmcli", &format!("echo nmcli \"$@\" >> {}", log.display()));
    let resp = world
        .wire()
        .call(json!({"op": "doctor_fix", "finding": "rfkill", "confirm": true}));
    assert!(resp.get("ok").is_some(), "{resp}");
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().trim(),
        "nmcli radio wifi on"
    );
}

#[test]
fn a_dismissed_authentication_dialog_is_a_failed_fix_not_a_crash() {
    let world = healthy();
    world
        .systemd
        .set_unit("systemd-networkd.service", Unit::Active);
    world.install_tool("pkexec", "exit 126");
    let resp = world
        .wire()
        .call(json!({"op": "doctor_fix", "finding": "conflict-networkd", "confirm": true}));
    assert_eq!(resp["err"]["code"], "fix_failed", "{resp}");
    assert!(
        resp["err"]["message"]
            .as_str()
            .unwrap()
            .contains("not authorized"),
        "{resp}"
    );
}

#[test]
fn a_finding_with_nothing_to_fix_is_unknown() {
    let world = healthy();
    let resp = world
        .wire()
        .call(json!({"op": "doctor_fix", "finding": "dns", "confirm": true}));
    assert_eq!(resp["err"]["code"], "unknown_finding", "{resp}");
}

#[test]
fn findings_are_pushed_when_a_run_finds_something_new() {
    let world = healthy();
    let mut events = world.wire();
    events.subscribe();
    world.nm.set_radio(false, true);
    let _ = findings(&world);
    let ev = events.next_event("doctor_findings");
    assert!(
        ev["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["id"] == "rfkill")
    );
}

#[test]
fn the_cli_prints_words_and_fails_when_something_is_a_problem() {
    let world = healthy();
    let out = world.run_nexus(&["doctor"]);
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("NetworkManager 1.58.1 is running"), "{text}");

    world.nm.set_radio(false, true);
    let out = world.run_nexus(&["doctor"]);
    assert_eq!(out.status.code(), Some(13), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Fix:"), "{text}");
    assert!(text.contains("nmcli radio wifi on"), "{text}");
}

#[test]
fn the_cli_speaks_json() {
    let world = healthy();
    let out = world.run_nexus(&["doctor", "--json"]);
    let report: Value = serde_json::from_slice(&out.stdout).expect("JSON on stdout");
    assert!(
        report["findings"].as_array().unwrap().len() >= 4,
        "{report}"
    );
}
