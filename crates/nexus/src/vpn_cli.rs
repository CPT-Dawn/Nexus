//! `nexus vpn ...`: the VPN commands. VPNs are named by their Profile name
//! (any case) or by id; the CLI looks the id up with `vpn_list`.

use std::process::ExitCode;

use nexus_ipc::{Connection, NewTunnel, Op, Secret, Vpn, VpnKind, VpnList, VpnState};
use serde_json::Value;

use crate::cli::{EXIT_DAEMON_ERROR, answering_prompts, exit_code, report, unreachable, wifi_call};

fn state_word(state: VpnState) -> &'static str {
    match state {
        VpnState::Up => "up",
        VpnState::Activating => "starting",
        VpnState::Down => "down",
    }
}

/// A Profile name is untrusted text: control characters would drive the terminal.
fn printable(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

/// The table `nexus vpn list` prints.
pub fn table(list: &VpnList) -> String {
    use std::fmt::Write as _;
    if list.vpns.is_empty() {
        return "No VPNs saved. Import one with `nexus vpn import <file.conf>`.\n".into();
    }
    let width = list
        .vpns
        .iter()
        .map(|v| printable(&v.name).chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<width$}  {:<9}  {:<6}  ADDRESS",
        "NAME", "KIND", "STATE"
    );
    for v in &list.vpns {
        let kind = match v.kind {
            VpnKind::Wireguard => "WireGuard",
            VpnKind::Plugin => "plugin",
        };
        let _ = writeln!(
            out,
            "{:<width$}  {:<9}  {:<6}  {}",
            printable(&v.name),
            kind,
            state_word(v.state),
            v.addresses.join(", ")
        );
    }
    out
}

pub fn list(json: bool) -> ExitCode {
    wifi_call(Op::VpnList, |value| {
        if json {
            println!("{value}");
        } else if let Ok(list) = serde_json::from_value::<VpnList>(value) {
            print!("{}", table(&list));
        }
    })
}

/// Finds a VPN by name or id. `Err` is the exit code, already reported.
fn resolve(conn: &mut Connection, name: &str) -> Result<Vpn, ExitCode> {
    let list = match conn.call(Op::VpnList) {
        Err(e) => return Err(unreachable(&e)),
        Ok(Err(f)) => {
            eprintln!("nexus: {}", f.message);
            return Err(ExitCode::from(exit_code(f.code)));
        }
        Ok(Ok(v)) => serde_json::from_value::<VpnList>(v).unwrap_or_default(),
    };
    let lower = name.to_lowercase();
    let mut hits: Vec<Vpn> = list
        .vpns
        .into_iter()
        .filter(|v| v.id == name || v.name.to_lowercase() == lower)
        .collect();
    match hits.len() {
        1 => Ok(hits.remove(0)),
        0 => {
            eprintln!("nexus: {}", crate::text::unknown_vpn().message);
            Err(ExitCode::from(exit_code(nexus_ipc::ErrorCode::UnknownVpn)))
        }
        _ => {
            eprintln!(
                "nexus: more than one VPN is called {name}; use its id (`nexus vpn list --json`)."
            );
            Err(ExitCode::from(EXIT_DAEMON_ERROR))
        }
    }
}

/// `nexus vpn up <name>` and `nexus vpn down <name>`.
pub fn toggle(name: &str, up: bool, json: bool) -> ExitCode {
    let mut conn = match Connection::connect() {
        Ok(c) => c,
        Err(e) => return unreachable(&e),
    };
    let vpn = match resolve(&mut conn, name) {
        Ok(v) => v,
        Err(code) => return code,
    };
    let op = if up {
        Op::VpnUp {
            vpn: vpn.id.clone(),
        }
    } else {
        Op::VpnDown {
            vpn: vpn.id.clone(),
        }
    };
    let show = |value: Value| {
        if json {
            println!("{value}");
        } else if let Ok(t) = serde_json::from_value::<nexus_ipc::VpnToggled>(value) {
            println!("{} is {}.", printable(&t.name), state_word(t.state));
        }
    };
    if !up {
        return report(conn.call(op), show);
    }
    // A plugin VPN may ask for a user name, password or one-time code: this
    // terminal answers.
    drop(conn);
    answering_prompts(op, &vpn.id, show)
}

/// `nexus vpn import <file> [--name N]`.
pub fn import(path: &str, name: Option<String>, kind: Option<String>, json: bool) -> ExitCode {
    // `.ovpn` and `.pcf` files belong to a plugin; `--type` names one outright.
    let plugin = kind.or_else(|| nexus_tui::plugin::for_path(path).map(|p| p.name.to_owned()));
    let text = match std::fs::read_to_string(path) {
        Ok(t) => zeroize::Zeroizing::new(t),
        Err(e) => {
            eprintln!("nexus: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    // `wg0.conf` becomes a VPN called `wg0`, as `wg-quick` would call it.
    let name = name.or_else(|| {
        std::path::Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
    });
    wifi_call(
        Op::VpnImport {
            name,
            config: Secret::new(text.as_str()),
            plugin,
        },
        |value| added(value, json),
    )
}

fn added(value: Value, json: bool) {
    if json {
        println!("{value}");
    } else if let Ok(a) = serde_json::from_value::<nexus_ipc::VpnAdded>(value) {
        if a.public_key.is_empty() {
            println!("Saved {}.", printable(&a.name));
            return;
        }
        println!(
            "Saved {}. Its public key is {}",
            printable(&a.name),
            a.public_key
        );
    }
}

/// What `nexus vpn create` was given.
pub struct Create {
    pub name: String,
    pub addresses: Vec<String>,
    pub dns: Vec<String>,
    pub peer_key: Option<String>,
    pub endpoint: Option<String>,
    pub allowed_ips: Vec<String>,
    pub json: bool,
}

pub fn create(c: Create) -> ExitCode {
    let mut tunnel = NewTunnel {
        name: c.name,
        addresses: c.addresses,
        dns: c.dns,
        peer_key: c.peer_key,
        endpoint: c.endpoint,
        allowed_ips: c.allowed_ips,
    };
    if tunnel.allowed_ips.is_empty() {
        tunnel.allowed_ips = vec!["0.0.0.0/0".into(), "::/0".into()];
    }
    wifi_call(Op::VpnCreate(tunnel), |value| added(value, c.json))
}

/// `nexus vpn key <name>`: the public key, safe to hand out.
pub fn key(name: &str, json: bool) -> ExitCode {
    let mut conn = match Connection::connect() {
        Ok(c) => c,
        Err(e) => return unreachable(&e),
    };
    let vpn = match resolve(&mut conn, name) {
        Ok(v) => v,
        Err(code) => return code,
    };
    report(conn.call(Op::VpnKey { vpn: vpn.id }), |value| {
        if json {
            println!("{value}");
        } else if let Ok(k) = serde_json::from_value::<nexus_ipc::VpnKey>(value) {
            println!("{}", k.public_key);
        }
    })
}

/// `nexus vpn share <name> [--qr] [--json]`: asking is the explicit reveal.
/// Prints the whole config (private key included), or its QR code with `--qr`.
pub fn share(name: &str, qr: bool, json: bool) -> ExitCode {
    let mut conn = match Connection::connect() {
        Ok(c) => c,
        Err(e) => return unreachable(&e),
    };
    let vpn = match resolve(&mut conn, name) {
        Ok(v) => v,
        Err(code) => return code,
    };
    report(conn.call(Op::VpnReveal { vpn: vpn.id }), |value| {
        if json {
            println!("{value}");
            return;
        }
        let Ok(share) = serde_json::from_value::<nexus_ipc::VpnShare>(value) else {
            eprintln!("nexus: undecodable share result");
            return;
        };
        if !qr {
            print!("{}", share.config.expose());
            return;
        }
        let Some(d) = nexus_tui::qr::draw(share.config.expose()) else {
            eprintln!("nexus: the config is too long for a QR code.");
            return;
        };
        let tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
        for line in &d.lines {
            if tty {
                println!("\x1b[30;47m{line}\x1b[0m");
            } else {
                println!("{line}");
            }
        }
        println!("{}", crate::text::vpn_scan_hint(&printable(&share.name)));
    })
}
