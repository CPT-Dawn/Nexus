//! `nexus menu`: Networks and VPNs in the user's dmenu-style launcher.
//! One list, one pick, one action; a password is asked through the
//! launcher's hidden-input mode and goes straight into the join.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

use nexus_ipc::{
    Connection, Failure, Op, Secret, Security, Vpn, VpnList, VpnState, WifiList, WifiNetwork,
};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::cli::{EXIT_DAEMON_ERROR, exit_code, unreachable};
use crate::text;

/// No launcher on the PATH, and none named in the config.
pub const EXIT_NO_LAUNCHER: u8 = 29;
/// The launcher was found but would not run.
pub const EXIT_LAUNCHER_FAILED: u8 = 30;
/// `[menu] launcher` names something Nexus does not know how to drive.
pub const EXIT_UNKNOWN_LAUNCHER: u8 = 31;

/// The launchers Nexus can drive, in the order it looks for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launcher {
    Fuzzel,
    Walker,
    Rofi,
    Tofi,
}

const ALL: [Launcher; 4] = [
    Launcher::Fuzzel,
    Launcher::Walker,
    Launcher::Rofi,
    Launcher::Tofi,
];

impl Launcher {
    pub fn name(self) -> &'static str {
        match self {
            Self::Fuzzel => "fuzzel",
            Self::Walker => "walker",
            Self::Rofi => "rofi",
            Self::Tofi => "tofi",
        }
    }

    /// The launcher a config value names (a bare name or a path to one).
    pub fn named(name: &str) -> Option<Self> {
        let base = name.rsplit('/').next().unwrap_or(name);
        ALL.into_iter().find(|l| l.name() == base)
    }

    /// The first launcher found in `path` (a `PATH` value).
    pub fn detect(path: &std::ffi::OsStr) -> Option<Self> {
        ALL.into_iter().find(|l| on_path(l.name(), path).is_some())
    }

    /// The arguments that make it read lines from stdin, print the pick on
    /// stdout, show `prompt`, and (for `hidden`) not echo what is typed.
    pub fn args(self, prompt: &str, hidden: bool) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        let mut push = |parts: &[&str]| args.extend(parts.iter().map(|s| (*s).to_string()));
        match self {
            Self::Fuzzel => {
                push(&["--dmenu", "--prompt", prompt]);
                if hidden {
                    push(&["--password"]);
                }
            }
            Self::Walker => {
                push(&["--dmenu", "--placeholder", prompt]);
                if hidden {
                    push(&["--password"]);
                }
            }
            Self::Rofi => {
                push(&["-dmenu", "-p", prompt]);
                if hidden {
                    push(&["-password"]);
                }
            }
            Self::Tofi => {
                push(&["--prompt-text", prompt]);
                if hidden {
                    push(&["--hide-input=true"]);
                }
            }
        }
        args
    }
}

fn on_path(program: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path)
        .map(|dir| dir.join(program))
        .find(|p| {
            p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

/// What picking a line does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    Join(WifiNetwork),
    Leave(WifiNetwork),
    VpnUp(Vpn),
    VpnDown(Vpn),
}

/// The lines to show and what each one does. Networks first (the strongest
/// first, as the Daemon lists them), then VPNs.
pub fn entries(wifi: &WifiList, vpns: &VpnList) -> Vec<(String, Pick)> {
    let mut out = Vec::new();
    for n in &wifi.networks {
        let name = one_line(&n.ssid);
        let line = if n.joined {
            format!("Wi-Fi  {name}  connected")
        } else {
            format!("Wi-Fi  {name}  {}%  {}", n.signal, n.security.label())
        };
        let pick = if n.joined {
            Pick::Leave(n.clone())
        } else {
            Pick::Join(n.clone())
        };
        out.push((line, pick));
    }
    for v in &vpns.vpns {
        let name = one_line(&v.name);
        let (state, pick) = match v.state {
            VpnState::Up => ("up", Pick::VpnDown(v.clone())),
            VpnState::Activating => ("connecting", Pick::VpnDown(v.clone())),
            VpnState::Down => ("down", Pick::VpnUp(v.clone())),
        };
        out.push((format!("VPN  {name}  {state}"), pick));
    }
    out
}

/// Names are whatever their owner typed: a newline would split a line.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Whether joining `n` needs a password asked for up front.
fn needs_password(n: &WifiNetwork) -> bool {
    n.profile.is_none()
        && !matches!(
            n.security,
            Security::Open | Security::Owe | Security::Enterprise
        )
}

/// Runs the launcher with `lines` on stdin. `None`: the person cancelled.
fn ask(
    launcher: Launcher,
    prompt: &str,
    hidden: bool,
    lines: &[String],
) -> Result<Option<Zeroizing<String>>, ExitCode> {
    let mut child = Command::new(launcher.name())
        .args(launcher.args(prompt, hidden))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| {
            eprintln!("nexus: cannot run {}: {e}", launcher.name());
            ExitCode::from(EXIT_LAUNCHER_FAILED)
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        // A launcher that quits early closes the pipe: not an error.
        let _ = stdin.write_all(lines.join("\n").as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| {
        eprintln!("nexus: {} stopped: {e}", launcher.name());
        ExitCode::from(EXIT_LAUNCHER_FAILED)
    })?;
    let text = Zeroizing::new(String::from_utf8_lossy(&out.stdout).into_owned());
    let picked = text.trim_end_matches(['\n', '\r']);
    if picked.is_empty() {
        return Ok(None);
    }
    Ok(Some(Zeroizing::new(picked.to_string())))
}

fn refused(failure: &Failure) -> ExitCode {
    eprintln!("nexus: {}", failure.message);
    ExitCode::from(exit_code(failure.code))
}

fn decode<T: serde::de::DeserializeOwned>(conn: &mut Connection, op: Op) -> Result<T, ExitCode> {
    match conn.call(op) {
        Err(e) => Err(unreachable(&e)),
        Ok(Err(f)) => Err(refused(&f)),
        Ok(Ok(v)) => serde_json::from_value(v).map_err(|e| {
            eprintln!("nexus: undecodable answer: {e}");
            ExitCode::from(EXIT_DAEMON_ERROR)
        }),
    }
}

/// `nexus menu`.
pub fn run() -> ExitCode {
    match menu() {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn menu() -> Result<(), ExitCode> {
    let mut conn = Connection::connect().map_err(|e| unreachable(&e))?;
    let configured: Value = decode(
        &mut conn,
        Op::ConfigGet {
            key: Some("menu.launcher".into()),
        },
    )?;
    let launcher = match configured["value"].as_str() {
        Some(name) => Launcher::named(name).ok_or_else(|| {
            eprintln!(
                "nexus: [menu] launcher = \"{name}\" is not one Nexus can drive. \
                 Use fuzzel, walker, rofi or tofi."
            );
            ExitCode::from(EXIT_UNKNOWN_LAUNCHER)
        })?,
        None => {
            Launcher::detect(&std::env::var_os("PATH").unwrap_or_default()).ok_or_else(|| {
                eprintln!(
                    "nexus: no launcher found. Install fuzzel, walker, rofi or tofi, \
                     or name one under [menu] launcher in config.toml."
                );
                ExitCode::from(EXIT_NO_LAUNCHER)
            })?
        }
    };

    let wifi: WifiList = decode(&mut conn, Op::WifiList)?;
    let vpns: VpnList = decode(&mut conn, Op::VpnList)?;
    let entries = entries(&wifi, &vpns);
    if entries.is_empty() {
        eprintln!("nexus: no Networks or VPNs to show.");
        return Ok(());
    }
    let lines: Vec<String> = entries.iter().map(|(l, _)| l.clone()).collect();
    let Some(chosen) = ask(launcher, "Network", false, &lines)? else {
        return Ok(());
    };
    let Some((_, pick)) = entries.into_iter().find(|(l, _)| l == chosen.as_str()) else {
        eprintln!("nexus: \"{}\" is not in the list.", chosen.as_str());
        return Err(ExitCode::from(EXIT_DAEMON_ERROR));
    };
    act(&mut conn, launcher, pick)
}

fn act(conn: &mut Connection, launcher: Launcher, pick: Pick) -> Result<(), ExitCode> {
    match pick {
        Pick::Leave(n) => {
            call(conn, Op::WifiDisconnect)?;
            println!("{}", text::disconnected(&n.ssid));
        }
        Pick::VpnUp(v) => {
            call(conn, Op::VpnUp { vpn: v.id })?;
            println!("{} is up.", v.name);
        }
        Pick::VpnDown(v) => {
            call(conn, Op::VpnDown { vpn: v.id })?;
            println!("{} is down.", v.name);
        }
        Pick::Join(n) => join(conn, launcher, &n)?,
    }
    Ok(())
}

fn call(conn: &mut Connection, op: Op) -> Result<Value, ExitCode> {
    match conn.call(op) {
        Err(e) => Err(unreachable(&e)),
        Ok(Err(f)) => Err(refused(&f)),
        Ok(Ok(v)) => Ok(v),
    }
}

/// Joins `n`, asking for its password first when it has no Profile, and
/// again when the Daemon says the password was wrong.
fn join(conn: &mut Connection, launcher: Launcher, n: &WifiNetwork) -> Result<(), ExitCode> {
    let mut password = None;
    let mut prompt = format!("Password for {}", one_line(&n.ssid));
    loop {
        if password.is_none() && needs_password(n) {
            match ask(launcher, &prompt, true, &[])? {
                Some(p) => password = Some(p),
                None => return Ok(()),
            }
        }
        let op = Op::WifiJoin {
            network: n.id.clone(),
            password: password.as_ref().map(|p| Secret::new(p.as_str())),
            device: None,
            hidden: None,
        };
        match conn.call(op) {
            Err(e) => return Err(unreachable(&e)),
            Ok(Ok(_)) => {
                println!("{}", text::joined(&n.ssid));
                return Ok(());
            }
            Ok(Err(f)) if f.code == nexus_ipc::ErrorCode::WrongPassword && password.is_some() => {
                password = None;
                prompt = format!("Wrong password for {}. Try again", one_line(&n.ssid));
            }
            Ok(Err(f)) => return Err(refused(&f)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_ipc::{Band, VpnKind};

    fn net(ssid: &str, joined: bool, profile: Option<&str>, security: Security) -> WifiNetwork {
        WifiNetwork {
            id: nexus_ipc::network_id(ssid.as_bytes()),
            ssid: ssid.into(),
            signal: 70,
            security,
            band: Band::Ghz5,
            access_points: 1,
            joined,
            device: "wlan0".into(),
            profile: profile.map(Into::into),
            autoconnect: None,
            trust: nexus_ipc::Trust::default(),
        }
    }

    fn vpn(name: &str, state: VpnState) -> Vpn {
        Vpn {
            id: format!("id-{name}"),
            name: name.into(),
            kind: VpnKind::Wireguard,
            state,
            addresses: vec![],
            endpoint: None,
        }
    }

    #[test]
    fn lines_say_what_a_pick_will_do() {
        let wifi = WifiList {
            devices: vec![],
            networks: vec![
                net("Home", true, Some("u"), Security::Wpa2),
                net("Cafe", false, None, Security::Open),
            ],
            saved: vec![],
        };
        let vpns = VpnList {
            vpns: vec![vpn("Work", VpnState::Up), vpn("Spare", VpnState::Down)],
        };
        let lines: Vec<_> = entries(&wifi, &vpns)
            .into_iter()
            .map(|(l, p)| (l, std::mem::discriminant(&p)))
            .collect();
        assert_eq!(lines[0].0, "Wi-Fi  Home  connected");
        assert!(lines[1].0.starts_with("Wi-Fi  Cafe  70%"));
        assert_eq!(lines[2].0, "VPN  Work  up");
        assert_eq!(lines[3].0, "VPN  Spare  down");
        assert_eq!(
            lines[0].1,
            std::mem::discriminant(&Pick::Leave(wifi.networks[0].clone()))
        );
    }

    #[test]
    fn a_newline_in_a_name_cannot_split_a_line() {
        assert_eq!(one_line("a\nb"), "a b");
    }

    #[test]
    fn only_new_secured_networks_ask_for_a_password() {
        assert!(needs_password(&net("A", false, None, Security::Wpa2)));
        assert!(!needs_password(&net("A", false, Some("u"), Security::Wpa2)));
        assert!(!needs_password(&net("A", false, None, Security::Open)));
    }

    #[test]
    fn launchers_get_their_own_dialect() {
        assert_eq!(
            Launcher::Rofi.args("Network", true),
            ["-dmenu", "-p", "Network", "-password"]
        );
        assert_eq!(
            Launcher::Fuzzel.args("Network", false),
            ["--dmenu", "--prompt", "Network"]
        );
        assert!(
            Launcher::Tofi
                .args("x", true)
                .contains(&"--hide-input=true".to_string())
        );
        assert_eq!(Launcher::named("/usr/bin/rofi"), Some(Launcher::Rofi));
        assert_eq!(Launcher::named("dmenu"), None);
    }
}
