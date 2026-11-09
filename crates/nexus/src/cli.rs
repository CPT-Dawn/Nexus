//! Front end commands. They talk to the Daemon only through `nexus-ipc`.

use std::io::ErrorKind;
use std::process::ExitCode;

use nexus_ipc::{Connection, ErrorCode, Op, Secret};
use serde_json::Value;

/// Exit code when the Daemon cannot be reached.
pub const EXIT_UNREACHABLE: u8 = 3;
/// Exit code when the Daemon answered with an error.
pub const EXIT_DAEMON_ERROR: u8 = 4;

/// The exit code for a refusal, one per failure class, so scripts can tell
/// a typo'd password (5) from a Network out of range (6) and so on.
/// 3 and 4 are taken by "cannot reach the Daemon" and "other Daemon error".
pub fn exit_code(code: ErrorCode) -> u8 {
    match code {
        ErrorCode::WrongPassword => 5,
        ErrorCode::OutOfRange => 6,
        ErrorCode::NoAddress => 7,
        ErrorCode::JoinFailed => 8,
        ErrorCode::UnknownNetwork => 9,
        ErrorCode::PasswordRequired => 10,
        ErrorCode::NotConnected => 11,
        _ => EXIT_DAEMON_ERROR,
    }
}

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

/// `nexus wifi list [--json]`.
pub fn wifi_list(json: bool) -> ExitCode {
    wifi_call(Op::WifiList, |value| {
        if json {
            println!("{value}");
            return;
        }
        match serde_json::from_value::<nexus_ipc::WifiList>(value) {
            Ok(list) => print!("{}", crate::wifi_view::table(&list)),
            Err(e) => eprintln!("nexus: undecodable list: {e}"),
        }
    })
}

/// `nexus wifi scan [--json]`.
pub fn wifi_scan(json: bool) -> ExitCode {
    wifi_call(Op::WifiScan, |value| {
        if json {
            println!("{value}");
            return;
        }
        match serde_json::from_value::<nexus_ipc::ScanOutcome>(value) {
            Ok(outcome) => println!("{}", crate::wifi_view::scan_line(&outcome)),
            Err(e) => eprintln!("nexus: undecodable scan result: {e}"),
        }
    })
}

/// Where `nexus wifi join` gets the password from.
pub enum PasswordSource {
    None,
    Flag(String),
    Stdin,
}

/// `nexus wifi join <ssid>`. The SSID's bytes are its Network id, so no
/// lookup is needed.
pub fn wifi_join(ssid: &str, source: PasswordSource, json: bool) -> ExitCode {
    let password = match source {
        PasswordSource::None => None,
        PasswordSource::Flag(p) => Some(Secret::new(p)),
        PasswordSource::Stdin => {
            let mut line = String::new();
            if let Err(e) = std::io::stdin().read_line(&mut line) {
                eprintln!("nexus: cannot read the password from stdin: {e}");
                return ExitCode::FAILURE;
            }
            Some(Secret::new(line.trim_end_matches(['\n', '\r'])))
        }
    };
    wifi_call(
        Op::WifiJoin {
            network: nexus_ipc::network_id(ssid.as_bytes()),
            password,
        },
        |value| {
            if json {
                println!("{value}");
            } else if let Ok(j) = serde_json::from_value::<nexus_ipc::Joined>(value) {
                println!("{}", crate::text::joined(&j.ssid));
            }
        },
    )
}

/// `nexus wifi disconnect`.
pub fn wifi_disconnect(json: bool) -> ExitCode {
    wifi_call(Op::WifiDisconnect, |value| {
        if json {
            println!("{value}");
        } else if let Ok(d) = serde_json::from_value::<nexus_ipc::Disconnected>(value) {
            println!("{}", crate::text::disconnected(&d.ssid));
        }
    })
}

/// `nexus wifi forget <ssid>`.
pub fn wifi_forget(ssid: &str, json: bool) -> ExitCode {
    wifi_call(
        Op::WifiForget {
            network: nexus_ipc::network_id(ssid.as_bytes()),
        },
        |value| {
            if json {
                println!("{value}");
            } else if let Ok(f) = serde_json::from_value::<nexus_ipc::Forgotten>(value) {
                println!("{}", crate::text::forgot(&f.ssid));
            }
        },
    )
}

fn wifi_call(op: Op, show: impl FnOnce(Value)) -> ExitCode {
    let mut conn = match Connection::connect() {
        Ok(c) => c,
        Err(e) => return unreachable(&e),
    };
    match conn.call(op) {
        Err(e) => unreachable(&e),
        Ok(Err(failure)) => {
            eprintln!("nexus: {}", failure.message);
            ExitCode::from(exit_code(failure.code))
        }
        Ok(Ok(value)) => {
            show(value);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_failure_class_has_its_own_exit_code() {
        let classes = [
            ErrorCode::WrongPassword,
            ErrorCode::OutOfRange,
            ErrorCode::NoAddress,
            ErrorCode::JoinFailed,
            ErrorCode::UnknownNetwork,
            ErrorCode::PasswordRequired,
            ErrorCode::NotConnected,
        ];
        let mut codes: Vec<u8> = classes.iter().map(|c| exit_code(*c)).collect();
        codes.push(EXIT_UNREACHABLE);
        codes.push(EXIT_DAEMON_ERROR);
        let n = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), n, "exit codes must not collide");
    }
}
