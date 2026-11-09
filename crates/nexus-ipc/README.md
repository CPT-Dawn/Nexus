# nexus-ipc

Request, response and event types for the Nexus Daemon's JSON-lines socket API, plus a small blocking client. It is the one crate every Nexus Front end (the TUI, the CLI, `nexus menu`, the Waybar module, and the Zenith `nexus` module) depends on. The protocol itself, with a runnable example for every op, event and error, is in [`docs/ipc.md`](../../docs/ipc.md); this page covers the crate.

The wire is one JSON object per `\n`-terminated line on `$XDG_RUNTIME_DIR/nexus/nexus.sock`. You can speak it with `socat` and `jq`, or with this crate.

## Version 1 and compatibility

The crate speaks protocol **version 1** ([`PROTOCOL`]) and is frozen for the 2.0 line. The rules are the same for the wire and for the types:

- **Additive changes keep `v: 1` and are released as a minor version.** A new op, event, error code, optional field or enum value can appear in any 2.x release. So the enums that can grow are `#[non_exhaustive]` and end in an `Unknown` variant (`ErrorCode::Unknown`, `Event::Unknown`, and so on): an old client reads a new value as `Unknown` instead of failing. Always keep a wildcard arm.
- **Unknown fields are ignored** when decoding, so a result may gain fields. Do not use `deny_unknown_fields` on your own copies of these types.
- **Breaking changes bump `v` and the major version.** Removing or renaming an op, event, field or code, changing a type, or making an optional field required. The Daemon lists the versions it accepts in [`SUPPORTED`]; a request for another gets `unsupported_version`.
- Match on `ErrorCode`, never on the `message` sentence, which is for people and may change.
- `Secret` never prints in `Debug` output and wipes its memory on drop; keep secrets in it as long as you can.

Depend on it by git tag, not crates.io:

```toml
nexus-ipc = { git = "https://github.com/CPT-Dawn/Nexus", tag = "<a release tag>" }
```

## Writing a request and reading the answer

Everything a Front end sends is a [`Request`]; [`encode`] turns it into the line to write and [`parse_request`] is the Daemon's side of that.

```rust
use nexus_ipc::{encode, parse_request, Op, Request, PROTOCOL};

let line = encode(&Request { v: PROTOCOL, id: 7, op: Op::WifiList });
assert_eq!(line, "{\"v\":1,\"id\":7,\"op\":\"wifi_list\"}\n");

// The Daemon's side: the same line comes back as the same op.
let request = parse_request(&line).expect("a valid request");
assert_eq!(request.id, 7);
assert!(matches!(request.op, Op::WifiList));
```

An answer is a [`Response`] with either `ok` or `err`. A code from a newer Daemon reads as `Unknown`, so handle it as a plain failure:

```rust
use nexus_ipc::{ErrorCode, Outcome, Response};

let line = r#"{"v":1,"id":7,"err":{"code":"wrong_password","message":"The password was rejected."}}"#;
let response: Response = serde_json::from_str(line).unwrap();
match response.outcome {
    Outcome::Ok(_) => unreachable!(),
    Outcome::Err(failure) => match failure.code {
        ErrorCode::WrongPassword => println!("try another password"),
        _ => println!("failed: {}", failure.message), // includes ErrorCode::Unknown
    },
}

// A code this crate has never heard of is not a decode error.
let newer = r#"{"v":1,"id":8,"err":{"code":"from_the_future","message":"?"}}"#;
let response: Response = serde_json::from_str(newer).unwrap();
assert!(matches!(response.outcome, Outcome::Err(f) if f.code == ErrorCode::Unknown));
```

## The blocking client

[`Connection`] connects to the socket, sends a request and waits for its answer. The outer `io::Result` is the transport, the inner `Result` is the Daemon's refusal.

```no_run
use nexus_ipc::{Connection, Event, Op};
use std::time::Duration;

let mut conn = Connection::connect()?;
match conn.status()? {
    Ok(status) => println!("{}", status.human()),
    Err(failure) => eprintln!("refused: {}", failure.message),
}

// Events: subscribe once, then wait for them.
let _ = conn.call(Op::Subscribe)?;
while let Some(event) = conn.next_event(Duration::from_secs(60))? {
    if let Event::StatusChanged { status } = event {
        println!("now: {}", status.human());
    }
}
# Ok::<(), std::io::Error>(())
```

Long calls (a join waits until the Network is up) hold their connection, so give them one of their own.
