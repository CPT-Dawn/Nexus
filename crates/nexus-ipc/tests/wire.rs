//! Pins the documented wire lines of `nexus-ipc` v1.

use nexus_ipc::{
    Connectivity, ErrorCode, Event, EventLine, Op, Outcome, Response, State, Status, encode,
    parse_request,
};
use serde_json::{Value, json};

fn parse(line: &str) -> Result<nexus_ipc::Request, Response> {
    parse_request(line)
}

#[test]
fn status_request_line() {
    let req = parse(r#"{"v":1,"id":7,"op":"status"}"#).unwrap();
    assert_eq!(req.id, 7);
    assert!(matches!(req.op, Op::Status));
}

#[test]
fn request_encodes_with_trailing_newline() {
    let line = encode(&nexus_ipc::Request {
        v: 1,
        id: 3,
        op: Op::Status,
    });
    assert_eq!(line, "{\"v\":1,\"id\":3,\"op\":\"status\"}\n");
}

#[test]
fn unknown_request_fields_are_ignored() {
    let req = parse(r#"{"v":1,"id":1,"op":"status","future":[1,2]}"#).unwrap();
    assert!(matches!(req.op, Op::Status));
}

fn err_of(line: &str) -> (Option<u64>, ErrorCode) {
    let resp = parse(line).unwrap_err();
    match resp.outcome {
        Outcome::Err(f) => (resp.id, f.code),
        Outcome::Ok(_) => panic!("expected an error"),
    }
}

#[test]
fn malformed_requests_get_coded_errors() {
    assert_eq!(err_of("not json"), (None, ErrorCode::BadRequest));
    assert_eq!(
        err_of(r#"{"id":1,"op":"status"}"#),
        (None, ErrorCode::BadRequest)
    );
    assert_eq!(
        err_of(r#"{"v":9,"id":4,"op":"status"}"#),
        (Some(4), ErrorCode::UnsupportedVersion)
    );
    assert_eq!(
        err_of(r#"{"v":1,"op":"status"}"#),
        (None, ErrorCode::BadRequest)
    );
    assert_eq!(
        err_of(r#"{"v":1,"id":5}"#),
        (Some(5), ErrorCode::BadRequest)
    );
    assert_eq!(
        err_of(r#"{"v":1,"id":6,"op":"teleport"}"#),
        (Some(6), ErrorCode::UnknownOp)
    );
}

#[test]
fn ok_and_err_response_lines() {
    let ok = Response::ok(2, json!({"x": 1}));
    assert_eq!(encode(&ok), "{\"v\":1,\"id\":2,\"ok\":{\"x\":1}}\n");
    let err = Response::err(Some(2), ErrorCode::BadRequest, "nope");
    assert_eq!(
        encode(&err),
        "{\"v\":1,\"id\":2,\"err\":{\"code\":\"bad_request\",\"message\":\"nope\"}}\n"
    );
}

#[test]
fn unknown_enum_values_decode_as_unknown() {
    let s: Status = serde_json::from_value(json!({
        "state": "hovering", "connectivity": "psychic",
        "backend": {"name": "networkmanager"}, "extra": true
    }))
    .unwrap();
    assert_eq!(s.state, State::Unknown);
    assert_eq!(s.connectivity, Connectivity::Unknown);
    assert!(s.link.is_none());

    let code: ErrorCode = serde_json::from_value(json!("brand_new")).unwrap();
    assert_eq!(code, ErrorCode::Unknown);
}

#[test]
fn unknown_events_decode_as_unknown() {
    let line: EventLine = serde_json::from_str(r#"{"v":1,"event":"meteor","size":3}"#).unwrap();
    assert!(matches!(line.event, Event::Unknown));
}

#[test]
fn wifi_ops_and_unknown_wifi_enums() {
    assert!(matches!(
        parse(r#"{"v":1,"id":1,"op":"wifi_list"}"#).unwrap().op,
        Op::WifiList
    ));
    assert!(matches!(
        parse(r#"{"v":1,"id":1,"op":"wifi_scan"}"#).unwrap().op,
        Op::WifiScan
    ));
    let n: nexus_ipc::WifiNetwork = serde_json::from_value(json!({
        "id": "6162", "ssid": "ab", "signal": 40, "security": "wpa9", "band": "60",
        "access_points": 1, "device": "/d/0", "future": true
    }))
    .unwrap();
    assert_eq!(n.security, nexus_ipc::Security::Unknown);
    assert_eq!(n.band, nexus_ipc::Band::Unknown);
    assert!(n.profile.is_none());
}

#[test]
fn join_ops_round_trip_and_the_password_stays_out_of_debug() {
    let line =
        r#"{"v":1,"id":3,"op":"wifi_join","network":"486f6d65","password":"hunter2hunter2"}"#;
    let req = parse(line).unwrap();
    assert!(!format!("{req:?}").contains("hunter2"));
    match req.op {
        Op::WifiJoin {
            network,
            password,
            device,
            hidden,
        } => {
            assert_eq!(network, "486f6d65");
            assert_eq!(password.unwrap().expose(), "hunter2hunter2");
            assert_eq!((device, hidden), (None, None));
        }
        other => panic!("{other:?}"),
    }
    // A hidden join on a chosen card.
    let line = r#"{"v":1,"id":4,"op":"wifi_join","network":"486f6d65","device":"wlan1","hidden":"wpa3_transition"}"#;
    match parse(line).unwrap().op {
        Op::WifiJoin { device, hidden, .. } => {
            assert_eq!(device.as_deref(), Some("wlan1"));
            assert_eq!(hidden, Some(nexus_ipc::Security::Wpa3Transition));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        parse(r#"{"v":1,"id":1,"op":"wifi_aps","network":"486f6d65"}"#)
            .unwrap()
            .op,
        Op::WifiAps { .. }
    ));
    assert!(matches!(
        parse(r#"{"v":1,"id":1,"op":"wifi_disconnect"}"#)
            .unwrap()
            .op,
        Op::WifiDisconnect
    ));
}

#[test]
fn network_ids_are_hex_ssid_bytes() {
    assert_eq!(nexus_ipc::network_id(b"Home"), "486f6d65");
    assert_eq!(nexus_ipc::parse_network_id("486f6d65").unwrap(), b"Home");
    assert_eq!(nexus_ipc::parse_network_id("abc"), None);
    assert_eq!(nexus_ipc::parse_network_id("zz"), None);
}

#[test]
fn status_event_line() {
    let line = EventLine {
        v: 1,
        event: Event::StatusChanged {
            status: Status::unavailable(),
        },
    };
    let v: Value = serde_json::from_str(&encode(&line)).unwrap();
    assert_eq!(v["event"], "status_changed");
    assert_eq!(v["status"]["state"], "unavailable");
}

#[test]
fn a_secret_answer_parses_but_never_shows_its_secret_in_debug() {
    let line = r#"{"v":1,"id":3,"op":"secret_answer","prompt":7,"secret":"hunter2hunter2"}"#;
    let req = nexus_ipc::parse_request(line).expect("parses");
    assert!(
        !format!("{req:?}").contains("hunter2"),
        "Debug must redact: {req:?}"
    );
    let Op::SecretAnswer { prompt, secret } = req.op else {
        panic!("wrong op");
    };
    assert_eq!((prompt, secret.expose()), (7, "hunter2hunter2"));
}

#[test]
fn prompt_events_carry_no_secret_field() {
    let line = EventLine {
        v: 1,
        event: Event::SecretPromptOpened {
            prompt: nexus_ipc::Prompt {
                id: 4,
                kind: nexus_ipc::PromptKind::WifiPassword,
                field: nexus_ipc::PromptField::Password,
                network: "Home".into(),
                network_id: nexus_ipc::network_id(b"Home"),
                retry: true,
                message: "Wrong password for Home. Try again.".into(),
                expires_in: 120,
            },
        },
    };
    let v: Value = serde_json::from_str(&encode(&line)).unwrap();
    assert_eq!(v["event"], "secret_prompt_opened");
    assert_eq!(v["prompt"]["kind"], "wifi_password");
    let keys: Vec<&str> = v["prompt"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert!(
        !keys
            .iter()
            .any(|k| k.contains("secret") || k.contains("password")),
        "{keys:?}"
    );
}

#[test]
fn the_wifi_qr_payload_escapes_the_characters_the_format_reserves() {
    use nexus_ipc::{Secret, ShareAuth, WifiShare};
    let share = WifiShare {
        ssid: "Caf\u{e9}; Bar:2".into(),
        auth: ShareAuth::Wpa,
        password: Some(Secret::new(r#"a\b;c,d:e"f"#)),
    };
    assert_eq!(
        share.payload().as_str(),
        "WIFI:T:WPA;S:Caf\u{e9}\\; Bar\\:2;P:a\\\\b\\;c\\,d\\:e\\\"f;;"
    );
    let open = WifiShare {
        ssid: "Cafe".into(),
        auth: ShareAuth::Open,
        password: None,
    };
    assert_eq!(open.payload().as_str(), "WIFI:T:nopass;S:Cafe;;");
    // Digit-only passwords stay bare, as Android writes them.
    let digits = WifiShare {
        ssid: "N".into(),
        auth: ShareAuth::Sae,
        password: Some(Secret::new("12345678")),
    };
    assert_eq!(digits.payload().as_str(), "WIFI:T:SAE;S:N;P:12345678;;");
}

#[test]
fn wifi_reveal_is_an_op_and_reveal_denied_an_error_code() {
    let req = parse(r#"{"v":1,"id":1,"op":"wifi_reveal","network":"486f6d65"}"#).unwrap();
    assert!(matches!(req.op, Op::WifiReveal { .. }));
    let line = encode(&Response::err(Some(1), ErrorCode::RevealDenied, "no"));
    assert!(line.contains(r#""code":"reveal_denied""#), "{line}");
}

#[test]
fn unknown_values_of_every_wire_enum_decode_as_unknown() {
    use nexus_ipc::{Bluetooth, EapMethod, Phase2, ShareAuth, Trust, VpnKind, VpnState};
    fn one<T: serde::de::DeserializeOwned>() -> T {
        serde_json::from_value(json!("from_the_future")).unwrap()
    }
    assert_eq!(one::<Trust>(), Trust::Unknown);
    assert_eq!(one::<ShareAuth>(), ShareAuth::Unknown);
    assert_eq!(one::<VpnKind>(), VpnKind::Unknown);
    assert_eq!(one::<VpnState>(), VpnState::Unknown);
    assert_eq!(one::<Bluetooth>(), Bluetooth::Unknown);
    assert_eq!(one::<EapMethod>(), EapMethod::Unknown);
    assert_eq!(one::<Phase2>(), Phase2::Unknown);
}

#[test]
fn a_vpn_that_is_deactivating_still_decodes() {
    let event: EventLine = serde_json::from_str(
        r#"{"v":1,"event":"vpns_changed","vpns":{"vpns":[
            {"id":"u","name":"Work","kind":"wireguard","state":"deactivating"}]}}"#,
    )
    .unwrap();
    match event.event {
        Event::VpnsChanged { vpns } => {
            assert_eq!(vpns.vpns[0].state, nexus_ipc::VpnState::Unknown);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_unknown_value_inside_a_request_is_a_bad_request_not_an_unknown_op() {
    for line in [
        r#"{"v":1,"id":8,"op":"wifi_join_eap","network":"6162","eap":{"method":"leap","identity":"a"}}"#,
        r#"{"v":1,"id":8,"op":"wifi_join_eap","network":"6162","eap":{"method":"peap","phase2":"x","identity":"a"}}"#,
        r#"{"v":1,"id":8,"op":"ethernet_join_eap","eap":{"method":"leap","identity":"a"}}"#,
        r#"{"v":1,"id":8,"op":"trust_set","network":"6162","trust":"maybe"}"#,
    ] {
        assert_eq!(err_of(line), (Some(8), ErrorCode::BadRequest), "{line}");
    }
    assert_eq!(
        err_of(r#"{"v":1,"id":9,"op":"wifi_join_eap","network":"6162","eap":{"method":5}}"#),
        (Some(9), ErrorCode::BadRequest)
    );
}

#[test]
fn an_unknown_eap_method_never_validates() {
    use nexus_ipc::{Eap, EapField, EapMethod};
    let mut eap = Eap::new(EapMethod::Peap);
    eap.method = EapMethod::Unknown;
    eap.identity = "a".into();
    let problems = eap.validate();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].field, EapField::Method);
}

#[test]
fn inventory_ops_parse() {
    assert!(matches!(
        parse(r#"{"v":1,"id":1,"op":"profile_list"}"#).unwrap().op,
        Op::ProfileList
    ));
    assert!(matches!(
        parse(r#"{"v":1,"id":1,"op":"device_list"}"#).unwrap().op,
        Op::DeviceList
    ));
}

#[test]
fn a_line_that_is_not_an_event_is_reported_not_skipped() {
    use std::io::Write as _;
    use std::os::unix::net::UnixListener;
    let dir = std::env::temp_dir().join(format!("nexus-wire-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let server = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.write_all(b"{\"v\":1,\"event\":\"vpns_changed\",\"vpns\":5}\n")
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
    });
    let mut conn = nexus_ipc::Connection::connect_to(&path).unwrap();
    let err = conn
        .next_event(std::time::Duration::from_secs(2))
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    server.join().unwrap();
}
