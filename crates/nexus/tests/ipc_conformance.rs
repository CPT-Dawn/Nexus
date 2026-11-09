//! The conformance suite for `docs/ipc.md` (#35).
//!
//! The page is the contract, so it is also the test input. Every request
//! example (a `sh` block that pipes one line into `socat`, followed by a `json`
//! block with the answer) is sent, in document order, to a real Daemon on a
//! fake NetworkManager, and the answer must have the shape the page shows. Every
//! event example must decode as a known event, and match a live event of its
//! name. Which ops, events and error codes are documented at all is checked
//! next to the types, in `nexus-ipc`'s `contract_tests`.
//!
//! "Shape" means: every key the page shows exists in the answer with the same
//! JSON type (`null` on the page matches anything, an array matches by its
//! first element), and an error's `code` is equal. Values such as ids and
//! times are the daemon's to choose.

use std::collections::BTreeSet;
use std::time::Duration;

use nexus_testkit::{ActiveConnection, NmConnectivity, NmState, Wire, World, scenarios};
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");
const DOC: &str = include_str!("../../../docs/ipc.md");

/// What the page calls the id of the VPN named Work in its examples.
const WORK_VPN: &str = "WORK_VPN_ID";

struct Block {
    info: String,
    text: String,
}

/// The fenced code blocks of the page, in order.
fn blocks() -> Vec<Block> {
    let mut out = Vec::new();
    let mut current: Option<Block> = None;
    for line in DOC.lines() {
        match (&mut current, line.strip_prefix("```")) {
            (None, Some(info)) => {
                current = Some(Block {
                    info: info.to_owned(),
                    text: String::new(),
                });
            }
            (Some(_), Some("")) => out.extend(current.take()),
            (Some(block), _) => {
                block.text.push_str(line);
                block.text.push('\n');
            }
            (None, None) => {}
        }
    }
    out
}

struct Example {
    request: String,
    expected: Value,
}

/// `echo '<request>' | socat -t1 - ...` followed by its answer.
fn examples() -> Vec<Example> {
    let blocks = blocks();
    let mut out = Vec::new();
    for pair in blocks.windows(2) {
        let (sh, answer) = (&pair[0], &pair[1]);
        if sh.info != "sh" || answer.info != "json" || !sh.text.contains("socat -t1") {
            continue;
        }
        let Some(rest) = sh.text.strip_prefix("echo '") else {
            continue;
        };
        let request = &rest[..rest.find("' | socat -t1").expect("closing quote")];
        out.push(Example {
            request: request.to_owned(),
            expected: serde_json::from_str(&answer.text)
                .unwrap_or_else(|e| panic!("{e}: {}", answer.text)),
        });
    }
    out
}

/// Every key of `expected` exists in `actual` with the same JSON type.
fn shape(expected: &Value, actual: &Value, path: &str, problems: &mut Vec<String>) {
    match (expected, actual) {
        (Value::Null, _) => {}
        (Value::Object(e), Value::Object(a)) => {
            for (key, want) in e {
                let here = format!("{path}.{key}");
                match a.get(key) {
                    None => problems.push(format!("{here}: missing")),
                    Some(got) if key == "code" && path.ends_with(".err") => {
                        if got != want {
                            problems.push(format!("{here}: {got} is not {want}"));
                        }
                    }
                    Some(got) => shape(want, got, &here, problems),
                }
            }
        }
        (Value::Array(e), Value::Array(a)) => match (e.first(), a.first()) {
            (None, _) => {}
            (Some(_), None) => problems.push(format!("{path}: empty, the page shows an element")),
            (Some(want), Some(got)) => shape(want, got, &format!("{path}[0]"), problems),
        },
        (Value::Bool(_), Value::Bool(_))
        | (Value::Number(_), Value::Number(_))
        | (Value::String(_), Value::String(_)) => {}
        (e, a) => problems.push(format!("{path}: the page shows {e}, the daemon sent {a}")),
    }
}

fn shape_of(expected: &Value, actual: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    shape(expected, actual, "", &mut problems);
    problems
}

/// The machine the page's examples were written against: Wi-Fi with a joined
/// Network and an Enterprise one, an Ethernet port, three VPNs (Work is up),
/// two Bluetooth phones, a Daemon with its default config.
fn world() -> World {
    let world = World::builder(NEXUS).bluez(true).start();
    let wlan0 = scenarios::wifi_enterprise(&world);
    world.nm.add_phone("AA:BB:CC:DD:EE:01", "Pixel 8");
    world.nm.add_phone("AA:BB:CC:DD:EE:02", "Sam's iPhone");
    world.nm.add_ethernet_device("enp3s0");
    scenarios::vpn_busy(&world);
    world.nm.set_traffic(wlan0, 1_000_000, 100_000);
    world
}

fn work_vpn(world: &World) -> String {
    let list = world.wire().call(json!({"op": "vpn_list"}));
    list["ok"]["vpns"]
        .as_array()
        .expect("vpns")
        .iter()
        .find(|v| v["name"] == "Work")
        .expect("Work VPN")["id"]
        .as_str()
        .expect("id")
        .to_owned()
}

/// Everything the Daemon pushed to a subscriber until it was quiet.
fn drain(events: &mut Wire, into: &mut Vec<Value>) {
    while let Some(line) = events.try_read_line(Duration::from_millis(600)) {
        into.push(line);
    }
}

#[test]
fn the_page_has_examples_for_every_op_but_subscribe() {
    let ops: BTreeSet<String> = examples()
        .iter()
        .filter(|e| e.expected["err"]["code"] != "unknown_op")
        .map(|e| {
            serde_json::from_str::<Value>(&e.request)
                .ok()
                .and_then(|v| v["op"].as_str().map(str::to_owned))
                .unwrap_or_default()
        })
        .filter(|op| !op.is_empty())
        .collect();
    let documented: BTreeSet<String> = DOC
        .match_indices("\n#### `")
        .map(|(i, _)| {
            let rest = &DOC[i + 7..];
            rest[..rest.find('`').unwrap()].to_owned()
        })
        .collect();
    let mut want = documented.clone();
    want.remove("subscribe");
    assert_eq!(
        ops, want,
        "an op without a runnable example, or a stale one"
    );
}

#[test]
fn every_request_example_gets_the_answer_the_page_shows() {
    let world = world();
    let work = work_vpn(&world);
    let mut failures = Vec::new();
    let mut ran = 0;
    for example in examples() {
        let request = example.request.replace(WORK_VPN, &work);
        let actual = world.wire().raw(&request);
        let problems = shape_of(&example.expected, &actual);
        if !problems.is_empty() {
            failures.push(format!(
                "{request}\n  -> {actual}\n  {}",
                problems.join("\n  ")
            ));
        }
        ran += 1;
    }
    assert!(ran > 40, "only {ran} examples ran");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn every_event_example_decodes_and_matches_what_the_daemon_pushes() {
    let mut world = world();
    let work = work_vpn(&world);
    let mut events = world.wire();
    events.subscribe();

    // Play the page, which causes most events...
    for example in examples() {
        let request = example.request.replace(WORK_VPN, &work);
        world.wire().raw(&request);
    }
    // ...and the three it cannot: a phone going away, a Portal, a prompt.
    world.bluez().set_powered(false);
    world
        .nm
        .set_primary(Some(ActiveConnection::wifi("CafeGuest")));
    world.nm.set_connectivity(NmConnectivity::Portal);
    world.nm.set_state(NmState::ConnectedGlobal);
    let mut joiner = world.wire();
    let asking = std::thread::spawn(move || {
        joiner.call(json!({
            "op": "wifi_join", "network": nexus_ipc::network_id(b"Work-Secure")
        }))
    });
    let mut live = Vec::new();
    loop {
        let line = events.read_line();
        assert!(!line.is_null(), "no secret_prompt_opened arrived");
        let opened = line["event"] == "secret_prompt_opened";
        let prompt = line["prompt"]["id"].clone();
        live.push(line);
        if opened {
            let cancelled = world
                .wire()
                .call(json!({"op": "secret_cancel", "prompt": prompt}));
            assert!(cancelled.get("ok").is_some(), "{cancelled}");
            break;
        }
    }
    let _ = asking.join();
    drain(&mut events, &mut live);

    let mut seen = BTreeSet::new();
    let mut failures = Vec::new();
    let mut documented = 0;
    for block in blocks().iter().filter(|b| b.info == "json event") {
        documented += 1;
        let example: Value = serde_json::from_str(&block.text).expect("event example is JSON");
        let name = example["event"].as_str().expect("an event name").to_owned();
        let decoded = serde_json::from_value::<nexus_ipc::EventLine>(example.clone())
            .unwrap_or_else(|e| panic!("{name} does not decode: {e}"));
        assert!(
            !matches!(decoded.event, nexus_ipc::Event::Unknown),
            "{name} decodes as an unknown event"
        );
        let mut best: Option<Vec<String>> = None;
        for line in live.iter().filter(|l| l["event"] == name.as_str()) {
            let problems = shape_of(&example, line);
            if problems.is_empty() {
                best = Some(problems);
                break;
            }
            best.get_or_insert(problems);
        }
        match best {
            None => {}
            Some(problems) if problems.is_empty() => {
                seen.insert(name);
            }
            Some(problems) => failures.push(format!("{name}: {}", problems.join(", "))),
        }
    }
    assert_eq!(documented, 14, "every event has an example");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // The Hotspot event is timing dependent (a quick start and stop merge into
    // one tick); every other event has to have been seen to be proven.
    let mut unseen: BTreeSet<&str> = [
        "status_changed",
        "networks_changed",
        "activity",
        "config_reloaded",
        "secret_prompt_opened",
        "secret_prompt_closed",
        "doctor_findings",
        "vpns_changed",
        "tethers_changed",
        "portal_changed",
        "ethernet_changed",
        "profile_pending",
        "profile_settled",
    ]
    .into();
    for name in &seen {
        unseen.remove(name.as_str());
    }
    assert!(unseen.is_empty(), "never pushed during the run: {unseen:?}");
}
