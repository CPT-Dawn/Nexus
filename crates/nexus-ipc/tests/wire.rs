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
