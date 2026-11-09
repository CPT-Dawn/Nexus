//! Front end commands. They talk to the Daemon only through `nexus-ipc`.

use std::io::ErrorKind;
use std::process::ExitCode;

use nexus_ipc::{Connection, ErrorCode, Event, Op, Prompt, Secret};
use serde_json::Value;

/// Exit code when the Daemon cannot be reached.
pub const EXIT_UNREACHABLE: u8 = 3;
/// Exit code when the Daemon answered with an error.
pub const EXIT_DAEMON_ERROR: u8 = 4;
/// Exit code of `nexus doctor` when it found a problem (a warning alone is 0).
pub const EXIT_PROBLEM: u8 = 13;

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
        ErrorCode::HardBlocked => 12,
        // 13 is `nexus doctor` finding a problem.
        ErrorCode::RevealDenied => 14,
        ErrorCode::InvalidSettings => 15,
        ErrorCode::Conflict => 16,
        ErrorCode::NothingPending => 17,
        ErrorCode::BluetoothOff => 18,
        ErrorCode::NoPhone => 19,
        ErrorCode::UnknownPhone => 20,
        ErrorCode::UnknownVpn => 21,
        ErrorCode::InvalidConfig => 22,
        ErrorCode::VpnFailed => 23,
        ErrorCode::MissingPlugin => 24,
        ErrorCode::KeyringUnavailable => 25,
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

/// `nexus doctor [--json] [--fix]`.
pub fn doctor(json: bool, fix: bool) -> ExitCode {
    let mut conn = match Connection::connect() {
        Ok(c) => c,
        Err(e) => return unreachable(&e),
    };
    let report = match conn.doctor() {
        Err(e) => return unreachable(&e),
        Ok(Err(failure)) => {
            eprintln!("nexus: the daemon refused: {}", failure.message);
            return ExitCode::from(EXIT_DAEMON_ERROR);
        }
        Ok(Ok(report)) => report,
    };
    if json {
        println!(
            "{}",
            serde_json::to_string(&report).expect("report serializes")
        );
    } else {
        print!("{}", crate::doctor::render(&report));
    }
    if fix && !json {
        for finding in report.findings.iter().filter(|f| f.is_trouble()) {
            let Some(fix) = finding.fix.as_ref().filter(|f| f.runnable) else {
                continue;
            };
            if !confirm_fix(&finding.title, fix) {
                continue;
            }
            let op = Op::DoctorFix {
                finding: finding.id.clone(),
                confirm: true,
            };
            match conn.call(op) {
                Err(e) => return unreachable(&e),
                Ok(Err(failure)) => {
                    eprintln!("nexus: {}", failure.message);
                    return ExitCode::from(exit_code(failure.code));
                }
                Ok(Ok(_)) => println!("Done. Run `nexus doctor` again to check."),
            }
        }
    }
    if report.worst() >= nexus_ipc::Severity::Problem {
        ExitCode::from(EXIT_PROBLEM)
    } else {
        ExitCode::SUCCESS
    }
}

/// Shows what would run and asks. Anything but a typed yes on a terminal is a no.
fn confirm_fix(title: &str, fix: &nexus_ipc::Fix) -> bool {
    use std::io::{BufRead, IsTerminal, Write};
    println!("\nFix for \"{title}\":");
    for command in &fix.commands {
        println!("    $ {command}");
    }
    if !std::io::stdin().is_terminal() {
        println!("(not a terminal: nothing was run; run the commands yourself)");
        return false;
    }
    let who = if fix.privileged {
        " They run as administrator: polkit will ask for your password."
    } else {
        ""
    };
    print!("Run these now?{who} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer).is_ok()
        && matches!(answer.trim(), "y" | "Y" | "yes")
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

/// `nexus wifi join <ssid> --eap ...`: an enterprise (802.1X) Network. The
/// settings are checked here first, so a bad form never costs a round trip.
pub fn wifi_join_eap(
    ssid: &str,
    mut eap: nexus_ipc::Eap,
    source: PasswordSource,
    json: bool,
) -> ExitCode {
    eap.password = match source {
        PasswordSource::Flag(p) => Some(Secret::new(p)),
        PasswordSource::Stdin => {
            let mut line = zeroize::Zeroizing::new(String::new());
            if let Err(e) = std::io::stdin().read_line(&mut line) {
                eprintln!("nexus: cannot read the password from stdin: {e}");
                return ExitCode::FAILURE;
            }
            Some(Secret::new(line.trim_end_matches(['\n', '\r'])))
        }
        PasswordSource::None
            if eap.method != nexus_ipc::EapMethod::Tls
                && std::io::IsTerminal::is_terminal(&std::io::stdin()) =>
        {
            read_masked("Password")
        }
        PasswordSource::None => None,
    };
    let errors = eap.validate();
    if !errors.is_empty() {
        let failure = crate::text::invalid_eap(ssid, &errors);
        eprintln!("nexus: {}", failure.message);
        return ExitCode::from(exit_code(failure.code));
    }
    if eap.skips_validation() {
        eprintln!(
            "nexus: warning: not checking {ssid}'s certificate. Anyone nearby can pose as it \
             and read your password."
        );
    }
    let op = Op::WifiJoinEap {
        network: nexus_ipc::network_id(ssid.as_bytes()),
        eap,
    };
    wifi_call(op, |value| {
        if json {
            println!("{value}");
        } else if let Ok(j) = serde_json::from_value::<nexus_ipc::Joined>(value) {
            println!("{}", crate::text::joined(&j.ssid));
        }
    })
}

/// `nexus wifi join <ssid>`. The SSID's bytes are its Network id, so no
/// lookup is needed.
pub fn wifi_join(ssid: &str, source: PasswordSource, json: bool) -> ExitCode {
    let password = match source {
        PasswordSource::None => None,
        PasswordSource::Flag(p) => Some(Secret::new(p)),
        PasswordSource::Stdin => {
            let mut line = zeroize::Zeroizing::new(String::new());
            if let Err(e) = std::io::stdin().read_line(&mut line) {
                eprintln!("nexus: cannot read the password from stdin: {e}");
                return ExitCode::FAILURE;
            }
            Some(Secret::new(line.trim_end_matches(['\n', '\r'])))
        }
    };
    let network = nexus_ipc::network_id(ssid.as_bytes());
    let op = Op::WifiJoin {
        network: network.clone(),
        password: password.clone(),
    };
    let show = |value: Value| {
        if json {
            println!("{value}");
        } else if let Ok(j) = serde_json::from_value::<nexus_ipc::Joined>(value) {
            println!("{}", crate::text::joined(&j.ssid));
        }
    };
    if password.is_some() {
        return wifi_call(op, show);
    }
    // No password given: the Daemon opens a Secret prompt if the Network needs
    // one, and this terminal answers it.
    answering_prompts(op, &network, show)
}

/// Runs `op` while this terminal answers the Secret prompts it opens for
/// `network` (a Network id, or a VPN Profile id).
pub fn answering_prompts(op: Op, network: &str, show: impl FnOnce(Value)) -> ExitCode {
    let (mut call, mut events) = match (Connection::connect(), Connection::connect()) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return unreachable(&e),
    };
    if let Err(e) = events.call(Op::Subscribe) {
        return unreachable(&e);
    }
    let finished = std::sync::atomic::AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        let joiner = scope.spawn(|| {
            let result = call.call(op);
            finished.store(true, std::sync::atomic::Ordering::SeqCst);
            result
        });
        let mut replies: Option<Connection> = None;
        while !finished.load(std::sync::atomic::Ordering::SeqCst) {
            let Ok(Some(Event::SecretPromptOpened { prompt })) =
                events.next_event(std::time::Duration::from_millis(100))
            else {
                continue;
            };
            if prompt.network_id != network {
                continue;
            }
            // Answers go over their own connection: a retry prompt can arrive
            // before this one's reply, and `call` would swallow it.
            if replies.is_none() {
                replies = Connection::connect().ok();
            }
            if let Some(replies) = replies.as_mut() {
                answer_prompt(replies, &prompt);
            }
        }
        joiner.join().expect("the join thread does not panic")
    });
    report(result, show)
}

/// Asks the person at the terminal, masked, and sends the answer. With no
/// terminal to ask on (or an empty line) the prompt is cancelled.
fn answer_prompt(replies: &mut Connection, prompt: &Prompt) {
    let typed = if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        read_masked(&prompt.message)
    } else {
        None
    };
    let op = match typed {
        Some(secret) => Op::SecretAnswer {
            prompt: prompt.id,
            secret,
        },
        None => Op::SecretCancel { prompt: prompt.id },
    };
    // A refusal means the prompt closed meanwhile (expired, answered elsewhere).
    let _ = replies.call(op);
}

/// Reads one line from the terminal with echo off. `None` for an empty line
/// or when the terminal cannot be read.
fn read_masked(message: &str) -> Option<Secret> {
    use rustix::termios::{LocalModes, OptionalActions, tcgetattr, tcsetattr};
    let stdin = std::io::stdin();
    let original = tcgetattr(&stdin).ok()?;
    let mut quiet = original.clone();
    quiet.local_modes.remove(LocalModes::ECHO);
    tcsetattr(&stdin, OptionalActions::Flush, &quiet).ok()?;
    eprint!("{message}: ");
    let mut line = zeroize::Zeroizing::new(String::new());
    let read = stdin.read_line(&mut line);
    let _ = tcsetattr(&stdin, OptionalActions::Now, &original);
    eprintln!();
    read.ok()?;
    let text = line.trim_end_matches(['\n', '\r']);
    (!text.is_empty()).then(|| Secret::new(text))
}

/// `nexus wifi on|off` and `nexus airplane on|off`.
pub fn radio(op: Op, json: bool) -> ExitCode {
    wifi_call(op, |value| {
        if json {
            println!("{value}");
        } else if let Ok(r) = serde_json::from_value::<nexus_ipc::Radios>(value) {
            println!("{}", crate::text::radios(&r));
        }
    })
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

/// `nexus wifi share <ssid> [--qr] [--json]`: asking is the explicit reveal.
/// Prints the password, or its QR code with `--qr` (and then not the password).
pub fn wifi_share(ssid: &str, qr: bool, json: bool) -> ExitCode {
    wifi_call(
        Op::WifiReveal {
            network: nexus_ipc::network_id(ssid.as_bytes()),
        },
        |value| {
            if json {
                println!("{value}");
                return;
            }
            let Ok(share) = serde_json::from_value::<nexus_ipc::WifiShare>(value) else {
                eprintln!("nexus: undecodable share result");
                return;
            };
            if qr {
                match nexus_tui::qr::draw(&share.payload()) {
                    Some(d) => print_qr(&d, &share.ssid),
                    None => eprintln!("nexus: the join details are too long for a QR code."),
                }
            } else if let Some(p) = &share.password {
                println!("{}", crate::text::share_password(&share.ssid, p.expose()));
            } else {
                println!("{}", crate::text::share_open(&share.ssid));
            }
        },
    )
}

/// Prints a QR drawing black on white when stdout is a terminal, so the code
/// scans whatever the theme; plain glyphs when piped.
fn print_qr(d: &nexus_tui::qr::Drawing, ssid: &str) {
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    for line in &d.lines {
        if tty {
            println!("\x1b[30;47m{line}\x1b[0m");
        } else {
            println!("{line}");
        }
    }
    println!("{}", crate::text::share_scan_hint(ssid));
}

/// `nexus tether list [--json]`.
pub fn tether_list(json: bool) -> ExitCode {
    wifi_call(Op::Tethers, |value| {
        if json {
            println!("{value}");
            return;
        }
        match serde_json::from_value::<nexus_ipc::Tethers>(value) {
            Ok(t) => print!("{}", crate::tether_view::table(&t)),
            Err(e) => eprintln!("nexus: undecodable list: {e}"),
        }
    })
}

/// `nexus tether connect [phone]`.
pub fn tether_connect(phone: Option<String>, json: bool) -> ExitCode {
    wifi_call(Op::TetherConnect { phone }, |value| {
        if json {
            println!("{value}");
        } else if let Ok(t) = serde_json::from_value::<nexus_ipc::Tethered>(value) {
            println!("{}", crate::text::tethered(&t.phone));
        }
    })
}

/// `nexus tether disconnect`.
pub fn tether_disconnect(json: bool) -> ExitCode {
    wifi_call(Op::TetherDisconnect, |value| {
        if json {
            println!("{value}");
        } else if let Ok(t) = serde_json::from_value::<nexus_ipc::Untethered>(value) {
            println!("{}", crate::text::untethered(&t.phone));
        }
    })
}

pub(crate) fn wifi_call(op: Op, show: impl FnOnce(Value)) -> ExitCode {
    let mut conn = match Connection::connect() {
        Ok(c) => c,
        Err(e) => return unreachable(&e),
    };
    report(conn.call(op), show)
}

pub(crate) fn report(
    result: std::io::Result<Result<Value, nexus_ipc::Failure>>,
    show: impl FnOnce(Value),
) -> ExitCode {
    match result {
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

pub(crate) fn unreachable(e: &std::io::Error) -> ExitCode {
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
            ErrorCode::HardBlocked,
            ErrorCode::UnknownVpn,
            ErrorCode::InvalidConfig,
            ErrorCode::VpnFailed,
            ErrorCode::MissingPlugin,
            ErrorCode::BluetoothOff,
            ErrorCode::NoPhone,
            ErrorCode::UnknownPhone,
            ErrorCode::InvalidSettings,
            ErrorCode::Conflict,
            ErrorCode::NothingPending,
            ErrorCode::KeyringUnavailable,
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
