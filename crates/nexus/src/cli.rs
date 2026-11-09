//! Front end commands. They talk to the Daemon only through `nexus-ipc`.

use std::io::ErrorKind;
use std::process::ExitCode;

use nexus_ipc::{Connection, Op};
use serde_json::Value;

/// Exit code when the Daemon cannot be reached.
pub const EXIT_UNREACHABLE: u8 = 3;
/// Exit code when the Daemon answered with an error.
pub const EXIT_DAEMON_ERROR: u8 = 4;

pub fn status(json: bool) -> ExitCode {
    let mut conn = match Connection::connect() {
        Ok(c) => c,
        Err(e) => return unreachable(&e),
    };
    match conn.status() {
        Err(e) => unreachable(&e),
        Ok(Err(failure)) => {
            eprintln!("nexus: the daemon refused: {}", failure.message);
            ExitCode::from(EXIT_DAEMON_ERROR)
        }
        Ok(Ok(status)) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&status).expect("status serializes")
                );
            } else {
                println!("{}", status.human());
            }
            ExitCode::SUCCESS
        }
    }
}

/// `nexus config get [key]`.
pub fn config_get(key: Option<String>) -> ExitCode {
    config_call(Op::ConfigGet { key })
}

/// `nexus config set <key> <value>`. The value is read as JSON (`true`, `3`,
/// `"text"`) and falls back to plain text, so `nexus config set theme nord` works.
pub fn config_set(key: String, value: &str) -> ExitCode {
    let value = serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.into()));
    config_call(Op::ConfigSet { key, value })
}

fn config_call(op: Op) -> ExitCode {
    let mut conn = match Connection::connect() {
        Ok(c) => c,
        Err(e) => return unreachable(&e),
    };
    match conn.call(op) {
        Err(e) => unreachable(&e),
        Ok(Err(failure)) => {
            eprintln!("nexus: the daemon refused: {}", failure.message);
            ExitCode::from(EXIT_DAEMON_ERROR)
        }
        Ok(Ok(result)) => {
            if let Some(warning) = result.get("warning").and_then(Value::as_str) {
                eprintln!("nexus: warning: {warning}");
            }
            match &result["value"] {
                Value::String(s) => println!("{s}"),
                v @ Value::Object(_) => {
                    println!("{}", serde_json::to_string_pretty(v).expect("serializes"));
                }
                v => println!("{v}"),
            }
            ExitCode::SUCCESS
        }
    }
}

fn unreachable(e: &std::io::Error) -> ExitCode {
    let path = nexus_ipc::socket_path().map_or_else(
        || "(XDG_RUNTIME_DIR is not set)".into(),
        |p| p.display().to_string(),
    );
    let hint = match e.kind() {
        ErrorKind::NotFound | ErrorKind::ConnectionRefused => {
            "Start it with `systemctl --user start nexus.socket`, or run `nexus daemon`."
        }
        _ => "Is it running as your user?",
    };
    eprintln!("nexus: cannot reach the daemon at {path}: {e}\n{hint}");
    ExitCode::from(EXIT_UNREACHABLE)
}
