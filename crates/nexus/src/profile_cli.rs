//! `nexus profile show|set`: the CLI twin of the TUI's Profile editor.

use std::io::BufRead as _;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clap::ValueEnum;

use crate::cli::refused;
use nexus_ipc::{
    Connection, ErrorCode, Event, Ipv4Method, Ipv6Method, MacMode, Op, PendingChange,
    ProfileApplied, ProfileSettings, ProfileView, Settled, profile,
};

use crate::cli::{exit_code, unreachable};
use crate::text;

#[derive(Clone, Copy, ValueEnum)]
enum Method4 {
    Auto,
    Manual,
}

#[derive(Clone, Copy, ValueEnum)]
enum Method6 {
    Auto,
    Manual,
    Disabled,
}

#[derive(Clone, Copy, ValueEnum)]
enum Mac {
    PerNetwork,
    EveryTime,
    Permanent,
}

#[derive(Clone, Copy, ValueEnum)]
enum YesNo {
    Yes,
    No,
}

#[derive(clap::Args)]
pub struct SetArgs {
    /// The Network's name (SSID).
    ssid: String,
    /// How to get an IPv4 address.
    #[arg(long, value_enum)]
    ipv4: Option<Method4>,
    /// A static IPv4 address with its prefix, like 192.168.1.20/24 (repeat for more).
    /// Implies `--ipv4 manual`.
    #[arg(long)]
    ipv4_address: Vec<String>,
    /// The IPv4 gateway, or `none`.
    #[arg(long)]
    ipv4_gateway: Option<String>,
    /// How to get an IPv6 address.
    #[arg(long, value_enum)]
    ipv6: Option<Method6>,
    /// A static IPv6 address with its prefix, like fd00::20/64. Implies `--ipv6 manual`.
    #[arg(long)]
    ipv6_address: Vec<String>,
    /// The IPv6 gateway, or `none`.
    #[arg(long)]
    ipv6_gateway: Option<String>,
    /// DNS servers, comma separated; an empty value takes the Network's own.
    #[arg(long)]
    dns: Option<String>,
    /// Which MAC address to show.
    #[arg(long, value_enum)]
    mac: Option<Mac>,
    /// Ask apps to save data on this Network.
    #[arg(long, value_enum)]
    metered: Option<YesNo>,
    /// Autoconnect priority, -999 to 999: higher joins first.
    #[arg(long, allow_hyphen_values = true)]
    priority: Option<i32>,
    /// Keep this Profile's password in your Secret Service keyring instead of
    /// NetworkManager's system store (the default).
    #[arg(long, value_enum)]
    keyring: Option<YesNo>,
    /// For a Profile in use: seconds to wait for confirmation before the
    /// change rolls back (default 15).
    #[arg(long)]
    rollback_secs: Option<u32>,
    /// Keep the change at once instead of asking (for scripts).
    #[arg(long)]
    keep: bool,
    /// Print results as JSON.
    #[arg(long)]
    json: bool,
}

fn network(ssid: &str) -> String {
    nexus_ipc::network_id(ssid.as_bytes())
}

fn bad(message: &str) -> ExitCode {
    eprintln!("nexus: {message}");
    ExitCode::from(exit_code(ErrorCode::InvalidSettings))
}

fn read(conn: &mut Connection, ssid: &str) -> Result<ProfileView, ExitCode> {
    let value = match conn.call(Op::ProfileGet {
        network: network(ssid),
        profile: None,
    }) {
        Err(e) => return Err(unreachable(&e)),
        Ok(Err(f)) => return Err(refused(&f)),
        Ok(Ok(v)) => v,
    };
    serde_json::from_value(value).map_err(|e| {
        eprintln!("nexus: the daemon sent a Profile this nexus cannot read: {e}");
        ExitCode::from(crate::cli::EXIT_DAEMON_ERROR)
    })
}

/// `nexus profile list [--json]`.
pub fn list(json: bool) -> ExitCode {
    crate::cli::wifi_call(Op::ProfileList, |value| {
        if json {
            println!("{value}");
            return;
        }
        match serde_json::from_value::<nexus_ipc::ProfileList>(value) {
            Ok(list) if list.profiles.is_empty() => println!("No saved Profiles."),
            Ok(list) => {
                for p in list.profiles {
                    let kind = match p.kind {
                        nexus_ipc::ProfileKind::Wifi => "wifi",
                        nexus_ipc::ProfileKind::Ethernet => "ethernet",
                        _ => "other",
                    };
                    let auto = match p.autoconnect {
                        Some(true) => "  autoconnect",
                        _ => "",
                    };
                    println!(
                        "{kind:<9} {}{auto}",
                        nexus_ipc::display_ssid(p.name.as_bytes())
                    );
                }
            }
            Err(e) => eprintln!("nexus: undecodable answer: {e}"),
        }
    })
}

/// `nexus profile show <ssid>`.
pub fn show(ssid: &str, json: bool) -> ExitCode {
    let mut conn = match Connection::connect() {
        Ok(c) => c,
        Err(e) => return unreachable(&e),
    };
    let view = match read(&mut conn, ssid) {
        Ok(v) => v,
        Err(code) => return code,
    };
    if json {
        println!("{}", serde_json::to_string(&view).expect("serializes"));
        return ExitCode::SUCCESS;
    }
    let s = &view.settings;
    let in_use = if view.active { " (in use)" } else { "" };
    println!("{}{in_use}", view.ssid);
    println!("  IPv4      {}", describe4(s));
    println!("  IPv6      {}", describe6(s));
    println!(
        "  DNS       {}",
        if s.dns.is_empty() {
            "from the network".to_owned()
        } else {
            s.dns.join(", ")
        }
    );
    println!("  MAC       {}", mac_words(s.mac));
    println!("  Metered   {}", if s.metered { "yes" } else { "no" });
    println!("  Priority  {}", s.priority);
    println!("  Keyring   {}", if s.keyring { "yes" } else { "no" });
    ExitCode::SUCCESS
}

fn describe4(s: &ProfileSettings) -> String {
    match s.ipv4.method {
        Ipv4Method::Manual => format!(
            "static {}{}",
            profile::addresses_text(&s.ipv4.addresses),
            s.ipv4
                .gateway
                .as_ref()
                .map_or(String::new(), |g| format!(" via {g}"))
        ),
        Ipv4Method::Auto => "automatic (DHCP)".into(),
        _ => "set up outside Nexus".into(),
    }
}

fn describe6(s: &ProfileSettings) -> String {
    match s.ipv6.method {
        Ipv6Method::Manual => format!(
            "static {}{}",
            profile::addresses_text(&s.ipv6.addresses),
            s.ipv6
                .gateway
                .as_ref()
                .map_or(String::new(), |g| format!(" via {g}"))
        ),
        Ipv6Method::Auto => "automatic".into(),
        Ipv6Method::Disabled => "disabled".into(),
        _ => "set up outside Nexus".into(),
    }
}

fn mac_words(mac: MacMode) -> &'static str {
    match mac {
        MacMode::PerNetwork => "random, the same for each Network",
        MacMode::EveryTime => "random, new at every join",
        MacMode::Permanent => "the hardware address",
        _ => "set up outside Nexus",
    }
}

fn gateway(text: &str) -> Option<String> {
    (text != "none" && !text.is_empty()).then(|| text.to_owned())
}

/// Folds the flags into `s`. Parse errors are the person's typos.
fn edit(s: &mut ProfileSettings, a: &SetArgs) -> Result<(), String> {
    if !a.ipv4_address.is_empty() {
        s.ipv4.addresses = profile::parse_addresses(&a.ipv4_address.join(","), false)?;
        s.ipv4.method = Ipv4Method::Manual;
    }
    match a.ipv4 {
        Some(Method4::Auto) => s.ipv4.method = Ipv4Method::Auto,
        Some(Method4::Manual) => s.ipv4.method = Ipv4Method::Manual,
        None => {}
    }
    if let Some(g) = &a.ipv4_gateway {
        s.ipv4.gateway = gateway(g);
    }
    if !a.ipv6_address.is_empty() {
        s.ipv6.addresses = profile::parse_addresses(&a.ipv6_address.join(","), true)?;
        s.ipv6.method = Ipv6Method::Manual;
    }
    match a.ipv6 {
        Some(Method6::Auto) => s.ipv6.method = Ipv6Method::Auto,
        Some(Method6::Manual) => s.ipv6.method = Ipv6Method::Manual,
        Some(Method6::Disabled) => s.ipv6.method = Ipv6Method::Disabled,
        None => {}
    }
    if let Some(g) = &a.ipv6_gateway {
        s.ipv6.gateway = gateway(g);
    }
    if let Some(dns) = &a.dns {
        s.dns = profile::parse_dns(dns)?;
    }
    match a.mac {
        Some(Mac::PerNetwork) => s.mac = MacMode::PerNetwork,
        Some(Mac::EveryTime) => s.mac = MacMode::EveryTime,
        Some(Mac::Permanent) => s.mac = MacMode::Permanent,
        None => {}
    }
    match a.metered {
        Some(YesNo::Yes) => s.metered = true,
        Some(YesNo::No) => s.metered = false,
        None => {}
    }
    if let Some(p) = a.priority {
        s.priority = p;
    }
    match a.keyring {
        Some(YesNo::Yes) => s.keyring = true,
        Some(YesNo::No) => s.keyring = false,
        None => {}
    }
    Ok(())
}

/// `nexus profile set <ssid> ...`.
pub fn set(args: SetArgs) -> ExitCode {
    let mut conn = match Connection::connect() {
        Ok(c) => c,
        Err(e) => return unreachable(&e),
    };
    let view = match read(&mut conn, &args.ssid) {
        Ok(v) => v,
        Err(code) => return code,
    };
    let mut settings = view.settings.clone();
    if let Err(message) = edit(&mut settings, &args) {
        return bad(&message);
    }
    // Hear how a pending change ends on a second link, subscribed before the
    // change so nothing is missed.
    let mut events = match Connection::connect() {
        Ok(c) => c,
        Err(e) => return unreachable(&e),
    };
    if let Err(e) = events.call(Op::Subscribe) {
        return unreachable(&e);
    }
    let value = match conn.call(Op::ProfileUpdate {
        network: network(&args.ssid),
        profile: None,
        version: view.version,
        settings,
        rollback_secs: args.rollback_secs,
    }) {
        Err(e) => return unreachable(&e),
        Ok(Err(f)) => return refused(&f),
        Ok(Ok(v)) => v,
    };
    if args.json {
        println!("{value}");
    }
    let Ok(applied) = serde_json::from_value::<ProfileApplied>(value) else {
        return ExitCode::SUCCESS;
    };
    let Some(pending) = applied.pending else {
        if !args.json {
            println!("{}", text::profile_saved(&applied.ssid));
        }
        return ExitCode::SUCCESS;
    };
    settle(&mut conn, &mut events, &pending, &args)
}

/// A change to the Profile in use: keep it on `--keep` or a `y` on stdin,
/// otherwise watch it roll back.
fn settle(
    conn: &mut Connection,
    events: &mut Connection,
    pending: &PendingChange,
    args: &SetArgs,
) -> ExitCode {
    let (tx, answers) = mpsc::channel::<String>();
    if !args.keep {
        println!(
            "Keep these settings? Reverting in {}s. Type y and Enter to keep them, n to revert now.",
            pending.rollback_secs
        );
        std::thread::spawn(move || {
            let mut line = String::new();
            if std::io::stdin()
                .lock()
                .read_line(&mut line)
                .is_ok_and(|n| n > 0)
            {
                let _ = tx.send(line);
            }
        });
    }
    let mut keep = args.keep;
    let mut revert = false;
    let deadline = Instant::now() + Duration::from_secs(u64::from(pending.rollback_secs) + 10);
    let mut result = ExitCode::SUCCESS;
    let outcome = loop {
        if !keep && let Ok(line) = answers.try_recv() {
            keep = matches!(line.trim().to_lowercase().as_str(), "y" | "yes");
            revert = !keep;
        }
        if revert {
            revert = false; // ask once
            match conn.call(Op::ProfileRevert {
                pending: pending.id,
            }) {
                Ok(Err(f)) if f.code == ErrorCode::NothingPending => {
                    eprintln!("nexus: {}", f.message);
                    result = ExitCode::from(exit_code(f.code));
                }
                Ok(Err(f)) => result = refused(&f),
                Err(e) => return unreachable(&e),
                Ok(Ok(_)) => {}
            }
        }
        if keep {
            keep = false; // ask once
            match conn.call(Op::ProfileConfirm {
                pending: pending.id,
            }) {
                Ok(Err(f)) if f.code == ErrorCode::NothingPending => {
                    eprintln!("nexus: {}", f.message);
                    result = ExitCode::from(exit_code(f.code));
                }
                Ok(Err(f)) => result = refused(&f),
                Err(e) => return unreachable(&e),
                Ok(Ok(_)) => {}
            }
        }
        match events.next_event(Duration::from_millis(100)) {
            Ok(Some(Event::ProfileSettled { id, outcome, .. })) if id == pending.id => {
                break outcome;
            }
            Ok(_) => {}
            Err(e) => return unreachable(&e),
        }
        if Instant::now() > deadline {
            break Settled::RolledBack;
        }
    };
    let words = if outcome == Settled::Confirmed {
        text::profile_kept(&pending.ssid)
    } else {
        text::profile_reverted(&pending.ssid)
    };
    if args.json {
        let name = if outcome == Settled::Confirmed {
            "confirmed"
        } else {
            "rolled_back"
        };
        println!("{}", serde_json::json!({ "outcome": name }));
    } else {
        println!("{words}");
    }
    result
}
