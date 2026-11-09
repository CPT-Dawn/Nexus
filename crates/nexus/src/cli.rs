//! Front end commands. They talk to the Daemon only through `nexus-ipc`.

use std::io::ErrorKind;
use std::process::ExitCode;

use nexus_ipc::{Connection, ErrorCode, Event, Op, Prompt, Secret};
use serde_json::Value;

/// Exit code when the Daemon cannot be reached.
pub const EXIT_UNREACHABLE: u8 = 3;
/// Exit code when the Daemon answered with an error that has no code of its own.
pub const EXIT_DAEMON_ERROR: u8 = 4;
/// Exit code of `nexus doctor` when it found a problem (a warning alone is 0).
pub const EXIT_PROBLEM: u8 = 13;
/// `nexus menu`: no launcher on the PATH, and none named in the config.
pub const EXIT_NO_LAUNCHER: u8 = 29;
/// `nexus menu`: the launcher was found but would not run.
pub const EXIT_LAUNCHER_FAILED: u8 = 30;
/// `nexus menu`: `[menu] launcher` names something Nexus cannot drive.
pub const EXIT_UNKNOWN_LAUNCHER: u8 = 31;

/// Every exit code `nexus` returns, sorted: the code, the Daemon error code it
/// stands for (none for codes the Front end makes up), and what it means.
/// This is the one list: [`exit_code`] reads it, the man page and
/// `docs/ipc.md` publish it, and a test keeps them equal.
pub const EXIT_CODES: &[(u8, Option<ErrorCode>, &str)] = &[
    (0, None, "Success."),
    (1, None, "Another failure (the message says which)."),
    (
        2,
        None,
        "Usage error: a flag or argument was not understood.",
    ),
    (EXIT_UNREACHABLE, None, "The Daemon cannot be reached."),
    (
        EXIT_DAEMON_ERROR,
        None,
        "The Daemon refused for a reason with no code of its own (backend_unavailable, bad_request, internal, ...).",
    ),
    (
        5,
        Some(ErrorCode::WrongPassword),
        "wrong_password: the password was rejected.",
    ),
    (
        6,
        Some(ErrorCode::OutOfRange),
        "out_of_range: the Network is not in range.",
    ),
    (
        7,
        Some(ErrorCode::NoAddress),
        "no_address: joined, but no address was handed out.",
    ),
    (
        8,
        Some(ErrorCode::JoinFailed),
        "join_failed: the join failed; the message says why.",
    ),
    (
        9,
        Some(ErrorCode::UnknownNetwork),
        "unknown_network: nothing is saved or nearby by that name.",
    ),
    (
        10,
        Some(ErrorCode::PasswordRequired),
        "password_required: the Network needs a password and none was given.",
    ),
    (
        11,
        Some(ErrorCode::NotConnected),
        "not_connected: there is nothing to disconnect or stop.",
    ),
    (
        12,
        Some(ErrorCode::HardBlocked),
        "hard_blocked: a hardware switch holds the radio off.",
    ),
    (EXIT_PROBLEM, None, "`nexus doctor` found a problem."),
    (
        14,
        Some(ErrorCode::RevealDenied),
        "reveal_denied: authorization to read a stored secret was refused.",
    ),
    (
        15,
        Some(ErrorCode::InvalidSettings),
        "invalid_settings: the settings are not valid.",
    ),
    (
        16,
        Some(ErrorCode::Conflict),
        "conflict: the Profile changed since it was read.",
    ),
    (
        17,
        Some(ErrorCode::NothingPending),
        "nothing_pending: no change is waiting for confirmation.",
    ),
    (
        18,
        Some(ErrorCode::BluetoothOff),
        "bluetooth_off: Bluetooth is off or missing.",
    ),
    (
        19,
        Some(ErrorCode::NoPhone),
        "no_phone: no paired phone offers network access.",
    ),
    (
        20,
        Some(ErrorCode::UnknownPhone),
        "unknown_phone: no paired phone matches, or several do.",
    ),
    (
        21,
        Some(ErrorCode::UnknownVpn),
        "unknown_vpn: no VPN has that name.",
    ),
    (
        22,
        Some(ErrorCode::InvalidConfig),
        "invalid_config: a VPN config could not be read.",
    ),
    (
        23,
        Some(ErrorCode::VpnFailed),
        "vpn_failed: the VPN did not come up.",
    ),
    (
        24,
        Some(ErrorCode::MissingPlugin),
        "missing_plugin: the VPN's NetworkManager plugin is not installed.",
    ),
    (
        25,
        Some(ErrorCode::KeyringUnavailable),
        "keyring_unavailable: no Secret Service answers, or it is locked.",
    ),
    (
        26,
        Some(ErrorCode::HotspotUnsupported),
        "hotspot_unsupported: this Wi-Fi card cannot host a Hotspot.",
    ),
    (
        27,
        Some(ErrorCode::Unplugged),
        "unplugged: no cable in the Ethernet port.",
    ),
    (
        28,
        Some(ErrorCode::UnknownPort),
        "unknown_port: no Ethernet port by that name.",
    ),
    (EXIT_NO_LAUNCHER, None, "`nexus menu`: no launcher found."),
    (
        EXIT_LAUNCHER_FAILED,
        None,
        "`nexus menu`: the launcher would not run.",
    ),
    (
        EXIT_UNKNOWN_LAUNCHER,
        None,
        "`nexus menu`: the launcher named in the config is not one Nexus knows.",
    ),
    (
        32,
        Some(ErrorCode::UnknownDevice),
        "unknown_device: no Wi-Fi device by that name.",
    ),
    (
        33,
        Some(ErrorCode::NoPortal),
        "no_portal: no Network is asking for a login.",
    ),
    (
        34,
        Some(ErrorCode::OpenFailed),
        "open_failed: the browser could not be started.",
    ),
];

/// The exit code for a refusal, one per failure class, so scripts can tell
/// a typo'd password (5) from a Network out of range (6) and so on. A code
/// with no row of its own (and any from a newer daemon) exits
/// [`EXIT_DAEMON_ERROR`].
pub fn exit_code(code: ErrorCode) -> u8 {
    EXIT_CODES
        .iter()
        .find(|(_, own, _)| *own == Some(code))
        .map_or(EXIT_DAEMON_ERROR, |(exit, _, _)| *exit)
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

/// `nexus trust list [--json]`.
pub fn trust_list(json: bool) -> ExitCode {
    wifi_call(Op::WifiList, |value| {
        if json {
            println!("{value}");
            return;
        }
        match serde_json::from_value::<nexus_ipc::WifiList>(value) {
            Ok(list) => print!("{}", crate::wifi_view::trust_table(&list)),
            Err(e) => eprintln!("nexus: undecodable list: {e}"),
        }
    })
}

/// `nexus trust set <ssid> trusted|untrusted [--json]`.
pub fn trust_set(ssid: &str, trust: nexus_ipc::Trust, json: bool) -> ExitCode {
    wifi_call(
        Op::TrustSet {
            network: nexus_ipc::network_id(ssid.as_bytes()),
            trust,
        },
        |value| {
            if json {
                println!("{value}");
            } else if let Ok(t) = serde_json::from_value::<nexus_ipc::TrustSet>(value) {
                println!("{}", crate::text::trust_marked(&t.ssid, t.trust));
            }
        },
    )
}

/// `nexus wifi aps <ssid> [--json]`: the access points behind a Network.
pub fn wifi_aps(ssid: &str, hex: bool, json: bool) -> ExitCode {
    let network = match network_of(ssid, hex) {
        Ok(n) => n,
        Err(code) => return code,
    };
    wifi_call(Op::WifiAps { network }, |value| {
        if json {
            println!("{value}");
            return;
        }
        match serde_json::from_value::<nexus_ipc::ApList>(value) {
            Ok(list) => print!("{}", crate::wifi_view::aps_table(&list)),
            Err(e) => eprintln!("nexus: undecodable access point list: {e}"),
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
    hex: bool,
) -> ExitCode {
    if let Err(code) = prepare_eap(ssid, &mut eap, source) {
        return code;
    }
    let network = match network_of(ssid, hex) {
        Ok(n) => n,
        Err(code) => return code,
    };
    let op = Op::WifiJoinEap { network, eap };
    wifi_call(op, |value| {
        if json {
            println!("{value}");
        } else if let Ok(j) = serde_json::from_value::<nexus_ipc::Joined>(value) {
            println!("{}", crate::text::joined(&j.ssid));
        }
    })
}

/// Fills in the password of `eap` from `source` and checks the settings, so a
/// bad form never costs a round trip. `name` is what the messages call the
/// Network or port.
fn prepare_eap(
    name: &str,
    eap: &mut nexus_ipc::Eap,
    source: PasswordSource,
) -> Result<(), ExitCode> {
    eap.password = match source {
        PasswordSource::Flag(p) => Some(Secret::new(p)),
        PasswordSource::Stdin => {
            let mut line = zeroize::Zeroizing::new(String::new());
            if let Err(e) = std::io::stdin().read_line(&mut line) {
                eprintln!("nexus: cannot read the password from stdin: {e}");
                return Err(ExitCode::FAILURE);
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
        let failure = crate::text::invalid_eap(name, &errors);
        eprintln!("nexus: {}", failure.message);
        return Err(ExitCode::from(exit_code(failure.code)));
    }
    if eap.skips_validation() {
        eprintln!(
            "nexus: warning: not checking {name}'s certificate. Anyone nearby can pose as it \
             and read your password."
        );
    }
    Ok(())
}

/// The Network id of what was typed: the SSID's own bytes, or with `hex` the
/// id as `wifi list --json` prints it (how SSIDs that are not UTF-8 are named).
pub fn network_of(ssid: &str, hex: bool) -> Result<String, ExitCode> {
    if !hex {
        return Ok(nexus_ipc::network_id(ssid.as_bytes()));
    }
    match nexus_ipc::parse_network_id(ssid) {
        Some(bytes) if !bytes.is_empty() => Ok(nexus_ipc::network_id(&bytes)),
        _ => {
            eprintln!("nexus: `{ssid}` is not an SSID in hex (two digits per byte).");
            Err(ExitCode::from(2))
        }
    }
}

/// Where a join happens, beyond the Network's name.
#[derive(Default)]
pub struct JoinWhere {
    /// `ssid` is the hex id, not the name.
    pub hex: bool,
    /// The Wi-Fi Device to join through (interface name or id).
    pub device: Option<String>,
    /// The Network is hidden; authenticate with this security.
    pub hidden: Option<nexus_ipc::Security>,
}

/// `nexus wifi join <ssid>`. The SSID's bytes are its Network id, so no
/// lookup is needed.
pub fn wifi_join(ssid: &str, source: PasswordSource, json: bool, at: JoinWhere) -> ExitCode {
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
    let network = match network_of(ssid, at.hex) {
        Ok(n) => n,
        Err(code) => return code,
    };
    let op = Op::WifiJoin {
        network: network.clone(),
        password: password.clone(),
        device: at.device,
        hidden: at.hidden,
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

/// `nexus wifi autoconnect <ssid> on|off`.
pub fn wifi_autoconnect(ssid: &str, enabled: bool, hex: bool, json: bool) -> ExitCode {
    let network = match network_of(ssid, hex) {
        Ok(n) => n,
        Err(code) => return code,
    };
    wifi_call(Op::WifiAutoconnect { network, enabled }, |value| {
        if json {
            println!("{value}");
        } else if let Ok(a) = serde_json::from_value::<nexus_ipc::AutoconnectSet>(value) {
            println!("{}", crate::text::autoconnect_set(&a.ssid, a.enabled));
        }
    })
}

/// `nexus portal [status] [--json]`.
pub fn portal_status(json: bool) -> ExitCode {
    wifi_call(Op::Portal, |value| {
        if json {
            println!("{value}");
        } else if let Ok(p) = serde_json::from_value::<nexus_ipc::PortalState>(value) {
            println!("{}", crate::text::portal_line(p.portal.as_ref()));
        }
    })
}

/// `nexus portal open [--json]`.
pub fn portal_open(json: bool) -> ExitCode {
    wifi_call(Op::PortalOpen, |value| {
        if json {
            println!("{value}");
        } else if let Ok(o) = serde_json::from_value::<nexus_ipc::PortalOpened>(value) {
            println!("{}", crate::text::portal_opened(&o.url));
        }
    })
}

/// `nexus activity [--json]`.
pub fn activity(json: bool) -> ExitCode {
    wifi_call(Op::Activity, |value| {
        if json {
            println!("{value}");
            return;
        }
        match serde_json::from_value::<nexus_ipc::ActivityLog>(value) {
            Ok(log) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                for entry in &log.entries {
                    println!(
                        "{:>8}  {}",
                        crate::text::ago(now.saturating_sub(entry.at)),
                        entry.text
                    );
                }
                if log.entries.is_empty() {
                    println!("Nothing has happened yet.");
                }
            }
            Err(e) => eprintln!("nexus: undecodable activity log: {e}"),
        }
    })
}

/// `nexus wifi forget <ssid>`.
pub fn wifi_forget(ssid: &str, hex: bool, json: bool) -> ExitCode {
    let network = match network_of(ssid, hex) {
        Ok(n) => n,
        Err(code) => return code,
    };
    wifi_call(Op::WifiForget { network }, |value| {
        if json {
            println!("{value}");
        } else if let Ok(f) = serde_json::from_value::<nexus_ipc::Forgotten>(value) {
            println!("{}", crate::text::forgot(&f.ssid));
        }
    })
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

/// `nexus ethernet list [--json]`.
pub fn ethernet_list(json: bool) -> ExitCode {
    wifi_call(Op::EthernetList, |value| {
        if json {
            println!("{value}");
            return;
        }
        match serde_json::from_value::<nexus_ipc::EthernetList>(value) {
            Ok(list) => print!("{}", crate::ethernet_view::table(&list)),
            Err(e) => eprintln!("nexus: undecodable list: {e}"),
        }
    })
}

/// `nexus ethernet connect [port]`.
pub fn ethernet_connect(port: Option<String>, json: bool) -> ExitCode {
    wifi_call(Op::EthernetConnect { port }, |value| {
        if json {
            println!("{value}");
        } else if let Ok(j) = serde_json::from_value::<nexus_ipc::EthernetJoined>(value) {
            println!("{}", crate::text::ethernet_joined(&j.port));
        }
    })
}

/// `nexus ethernet connect [port] --eap ...`: sign in to an 802.1X port.
pub fn ethernet_join_eap(
    port: Option<String>,
    mut eap: nexus_ipc::Eap,
    source: PasswordSource,
    json: bool,
) -> ExitCode {
    let name = port.as_deref().unwrap_or("the port");
    if let Err(code) = prepare_eap(name, &mut eap, source) {
        return code;
    }
    wifi_call(Op::EthernetJoinEap { port, eap }, |value| {
        if json {
            println!("{value}");
        } else if let Ok(j) = serde_json::from_value::<nexus_ipc::EthernetJoined>(value) {
            println!("{}", crate::text::ethernet_joined(&j.port));
        }
    })
}

/// `nexus ethernet disconnect [port]`.
pub fn ethernet_disconnect(port: Option<String>, json: bool) -> ExitCode {
    wifi_call(Op::EthernetDisconnect { port }, |value| {
        if json {
            println!("{value}");
        } else if let Ok(d) = serde_json::from_value::<nexus_ipc::EthernetDropped>(value) {
            println!("{}", crate::text::ethernet_dropped(&d.port));
        }
    })
}

/// `nexus hotspot status [--json]`.
pub fn hotspot_status(json: bool) -> ExitCode {
    wifi_call(Op::Hotspot, |value| {
        if json {
            println!("{value}");
            return;
        }
        match serde_json::from_value::<nexus_ipc::Hotspot>(value) {
            Ok(h) => println!("{}", crate::text::hotspot_line(&h)),
            Err(e) => eprintln!("nexus: undecodable hotspot: {e}"),
        }
    })
}

/// `nexus hotspot start`.
pub fn hotspot_start(
    ssid: Option<String>,
    source: PasswordSource,
    qr: bool,
    json: bool,
) -> ExitCode {
    let password = match source {
        PasswordSource::Flag(p) => Some(Secret::new(p)),
        PasswordSource::Stdin => {
            let mut line = zeroize::Zeroizing::new(String::new());
            if let Err(e) = std::io::stdin().read_line(&mut line) {
                eprintln!("nexus: cannot read the password from stdin: {e}");
                return ExitCode::FAILURE;
            }
            Some(Secret::new(line.trim_end_matches(['\n', '\r'])))
        }
        PasswordSource::None => None,
    };
    wifi_call(Op::HotspotStart { ssid, password }, |value| {
        if json {
            println!("{value}");
            return;
        }
        let Ok(h) = serde_json::from_value::<nexus_ipc::HotspotStarted>(value) else {
            eprintln!("nexus: undecodable hotspot result");
            return;
        };
        println!("{}", crate::text::hotspot_started(&h.ssid));
        println!(
            "{}",
            crate::text::share_password(&h.ssid, h.password.expose())
        );
        if qr {
            let share = nexus_ipc::WifiShare {
                ssid: h.ssid.clone(),
                auth: nexus_ipc::ShareAuth::Wpa,
                password: Some(h.password.clone()),
            };
            match nexus_tui::qr::draw(&share.payload()) {
                Some(d) => print_qr(&d, &h.ssid),
                None => eprintln!("nexus: the join details are too long for a QR code."),
            }
        }
    })
}

/// `nexus hotspot stop`.
pub fn hotspot_stop(json: bool) -> ExitCode {
    wifi_call(Op::HotspotStop, |value| {
        if json {
            println!("{value}");
        } else if let Ok(h) = serde_json::from_value::<nexus_ipc::HotspotStopped>(value) {
            println!("{}", crate::text::hotspot_stopped(&h.ssid));
        }
    })
}

/// `nexus hotspot qr [--json]`: the running Hotspot's join code. Asks the
/// Backend for the password, so it needs the same authorization as
/// `nexus wifi share`.
pub fn hotspot_qr(json: bool) -> ExitCode {
    wifi_call(Op::HotspotReveal, |value| {
        if json {
            println!("{value}");
            return;
        }
        let Ok(share) = serde_json::from_value::<nexus_ipc::WifiShare>(value) else {
            eprintln!("nexus: undecodable share result");
            return;
        };
        match nexus_tui::qr::draw(&share.payload()) {
            Some(d) => print_qr(&d, &share.ssid),
            None => eprintln!("nexus: the join details are too long for a QR code."),
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

/// Prints a refusal and returns the exit code for its class.
pub(crate) fn refused(failure: &nexus_ipc::Failure) -> ExitCode {
    eprintln!("nexus: {}", failure.message);
    ExitCode::from(exit_code(failure.code))
}

/// One call, with every failure already reported: `Err` is the exit code.
pub(crate) fn call_value(conn: &mut Connection, op: Op) -> Result<Value, ExitCode> {
    match conn.call(op) {
        Err(e) => Err(unreachable(&e)),
        Ok(Err(f)) => Err(refused(&f)),
        Ok(Ok(v)) => Ok(v),
    }
}

/// One call decoded into `T`. An answer that will not decode is reported,
/// never read as empty.
pub(crate) fn decode_call<T: serde::de::DeserializeOwned>(
    conn: &mut Connection,
    op: Op,
) -> Result<T, ExitCode> {
    serde_json::from_value(call_value(conn, op)?).map_err(|e| {
        eprintln!("nexus: undecodable answer: {e}");
        ExitCode::from(EXIT_DAEMON_ERROR)
    })
}

pub(crate) fn report(
    result: std::io::Result<Result<Value, nexus_ipc::Failure>>,
    show: impl FnOnce(Value),
) -> ExitCode {
    match result {
        Err(e) => unreachable(&e),
        Ok(Err(failure)) => refused(&failure),
        Ok(Ok(value)) => {
            show(value);
            ExitCode::SUCCESS
        }
    }
}

/// `nexus config get [key]`.
pub fn config_get(key: Option<String>, json: bool) -> ExitCode {
    config_call(Op::ConfigGet { key }, json)
}

/// `nexus config set <key> <value>`. The value is read as JSON (`true`, `3`,
/// `"text"`) and falls back to plain text, so `nexus config set theme nord` works.
pub fn config_set(key: String, value: &str) -> ExitCode {
    let value = serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.into()));
    config_call(Op::ConfigSet { key, value }, false)
}

fn config_call(op: Op, json: bool) -> ExitCode {
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
                v if json => println!("{v}"),
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
    fn exit_codes_do_not_collide() {
        for (i, (_, own, _)) in EXIT_CODES.iter().enumerate() {
            if let Some(own) = own {
                assert!(
                    !EXIT_CODES[..i].iter().any(|r| r.1 == Some(*own)),
                    "{} has two rows",
                    own.name()
                );
            }
        }
    }

    #[test]
    fn the_table_is_sorted_and_without_gaps() {
        for (i, (code, _, _)) in EXIT_CODES.iter().enumerate() {
            assert_eq!(usize::from(*code), i, "codes run 0, 1, 2, ... in order");
        }
    }

    #[test]
    fn every_error_code_exits_with_a_code_the_table_explains() {
        for code in ErrorCode::ALL {
            let exit = exit_code(*code);
            let row = &EXIT_CODES[usize::from(exit)];
            assert_eq!(row.0, exit);
            let own = exit != EXIT_DAEMON_ERROR;
            assert_eq!(row.1 == Some(*code), own, "{} exits {exit}", code.name());
            assert_eq!(
                row.2.starts_with(&format!("{}:", code.name())),
                own,
                "{} exits {exit}: {}",
                code.name(),
                row.2
            );
        }
        assert_eq!(exit_code(ErrorCode::Unknown), EXIT_DAEMON_ERROR);
    }

    #[test]
    fn docs_publish_the_same_exit_codes() {
        const DOC: &str = include_str!("../../../docs/ipc.md");
        for (code, _, meaning) in EXIT_CODES {
            let row = format!("| {code} | {meaning} |");
            assert!(DOC.contains(&row), "docs/ipc.md lacks `{row}`");
        }
        for code in ErrorCode::ALL {
            let row = format!("| `{}` | {} |", code.name(), exit_code(*code));
            assert!(DOC.contains(&row), "docs/ipc.md lacks `{row}`");
        }
    }

    #[test]
    fn the_other_constants_are_in_the_table() {
        let says = |code: u8| EXIT_CODES[usize::from(code)].2;
        assert!(says(EXIT_UNREACHABLE).contains("cannot be reached"));
        assert!(says(EXIT_DAEMON_ERROR).contains("no code of its own"));
        assert!(says(EXIT_PROBLEM).contains("doctor"));
        assert!(says(EXIT_NO_LAUNCHER).contains("no launcher"));
        assert!(says(EXIT_LAUNCHER_FAILED).contains("would not run"));
        assert!(says(EXIT_UNKNOWN_LAUNCHER).contains("not one Nexus knows"));
    }
}
