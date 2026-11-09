mod backend;
mod cli;
mod config;
mod daemon;
mod doctor;
mod secrets;
mod text;
mod wifi_view;

use std::process::ExitCode;

use clap::{Args as ClapArgs, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name = "nexus", version = VERSION, about = "A network manager for Wayland desktops.")]
struct Args {
    /// Draw with plain ASCII only (the TUI).
    #[arg(long, global = true)]
    ascii: bool,
    /// Use Nerd Font icons (the TUI). Needs a patched font.
    #[arg(long, global = true)]
    nerd_font: bool,
    /// Without a subcommand, `nexus` opens the TUI.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Show the overall network state and the primary Link.
    Status {
        /// Print the status object as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Nearby Wi-Fi Networks and scanning.
    Wifi {
        #[command(subcommand)]
        action: WifiAction,
    },
    /// Explain why the machine is or is not online, and how to fix it.
    Doctor {
        /// Print the findings as JSON.
        #[arg(long)]
        json: bool,
        /// Offer to run the fixes (asks first; privileged ones go through pkexec).
        #[arg(long)]
        fix: bool,
    },
    /// Airplane mode: every radio off (`on`) or back on (`off`).
    Airplane {
        /// `on` switches the radios off; `off` brings them back.
        #[arg(value_enum)]
        state: Switch,
        /// Print the radio state as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Read or change the config file.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Run the Daemon in the foreground (systemd socket activation also uses this).
    Daemon,
    /// Print shell completions to stdout.
    Completions {
        /// Shell to generate for.
        shell: Shell,
    },
    /// Print the man page (roff) to stdout.
    Man,
}

#[derive(Subcommand)]
enum WifiAction {
    /// List nearby Networks (one per SSID) and saved ones out of range.
    List {
        /// Print the list as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Ask for a fresh scan (rate limited by NetworkManager; recent results are reused).
    Scan {
        /// Print the outcome as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Join a Network. The Profile is saved only if the join works.
    Join(JoinArgs),
    /// Switch the Wi-Fi radio on.
    On {
        /// Print the radio state as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Switch the Wi-Fi radio off.
    Off {
        /// Print the radio state as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Drop the current Wi-Fi Link; the Profile stays.
    Disconnect {
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Forget a Network: delete its saved Profile.
    Forget {
        /// The Network's name (SSID).
        ssid: String,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Reveal a saved Network's password, or show it as a QR code a phone can scan.
    /// NetworkManager may ask you to authorize reading the password.
    Share {
        /// The Network's name (SSID).
        ssid: String,
        /// Draw the join details as a QR code instead of printing the password.
        #[arg(long)]
        qr: bool,
        /// Print the result, password included, as JSON.
        #[arg(long, conflicts_with = "qr")]
        json: bool,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Switch {
    On,
    Off,
}

#[derive(ClapArgs)]
struct JoinArgs {
    /// The Network's name (SSID).
    ssid: String,
    /// The password. Visible in the process list: prefer --password-stdin.
    #[arg(long, conflicts_with = "password_stdin")]
    password: Option<String>,
    /// Read the password from the first line of stdin.
    #[arg(long)]
    password_stdin: bool,
    /// Print the result as JSON.
    #[arg(long)]
    json: bool,
    /// Join a WPA-Enterprise Network (eduroam, office Wi-Fi) with this outer method.
    #[arg(long, value_enum)]
    eap: Option<EapKind>,
    /// Inner method for PEAP and TTLS (default: mschapv2).
    #[arg(long, value_enum, requires = "eap")]
    phase2: Option<InnerKind>,
    /// Your identity, usually your email or username.
    #[arg(long, requires = "eap")]
    identity: Option<String>,
    /// The outer identity sent in the clear (eduroam: anonymous@your.domain).
    #[arg(long, requires = "eap")]
    anonymous_identity: Option<String>,
    /// The server name (or its tail) the certificate must match, e.g. radius.uni.example.
    #[arg(long, requires = "eap")]
    domain: Option<String>,
    /// CA certificate file. Without it the system's CAs are used (needs --domain).
    #[arg(long, requires = "eap")]
    ca_cert: Option<String>,
    /// Do not check the server's certificate at all. Anyone can pose as the network.
    #[arg(long, requires = "eap")]
    no_ca_validation: bool,
    /// Client certificate file (TLS).
    #[arg(long, requires = "eap")]
    client_cert: Option<String>,
    /// Private key file (TLS).
    #[arg(long, requires = "eap")]
    private_key: Option<String>,
    /// Private key password. Visible in the process list.
    #[arg(long, requires = "eap")]
    private_key_password: Option<String>,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum EapKind {
    Peap,
    Ttls,
    Tls,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum InnerKind {
    Mschapv2,
    Mschap,
    Pap,
    Chap,
    Gtc,
    Md5,
}

impl JoinArgs {
    /// The 802.1X settings the flags describe, if `--eap` was given.
    fn eap(&self) -> Option<nexus_ipc::Eap> {
        use nexus_ipc::{Eap, EapMethod, Phase2};
        let method = match self.eap? {
            EapKind::Peap => EapMethod::Peap,
            EapKind::Ttls => EapMethod::Ttls,
            EapKind::Tls => EapMethod::Tls,
        };
        let mut eap = Eap::new(method);
        if let Some(p) = self.phase2 {
            eap.phase2 = Some(match p {
                InnerKind::Mschapv2 => Phase2::Mschapv2,
                InnerKind::Mschap => Phase2::Mschap,
                InnerKind::Pap => Phase2::Pap,
                InnerKind::Chap => Phase2::Chap,
                InnerKind::Gtc => Phase2::Gtc,
                InnerKind::Md5 => Phase2::Md5,
            });
        }
        eap.identity = self.identity.clone().unwrap_or_default();
        eap.anonymous_identity.clone_from(&self.anonymous_identity);
        eap.domain_suffix.clone_from(&self.domain);
        eap.ca_cert.clone_from(&self.ca_cert);
        eap.no_ca_validation = self.no_ca_validation;
        eap.client_cert.clone_from(&self.client_cert);
        eap.private_key.clone_from(&self.private_key);
        eap.private_key_password = self
            .private_key_password
            .clone()
            .map(nexus_ipc::Secret::new);
        Some(eap)
    }
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Print the effective config, or one value by dotted key (e.g. `connectivity.url`).
    Get { key: Option<String> },
    /// Set a value in config.toml, keeping your comments. `null` resets it.
    Set { key: String, value: String },
}

fn main() -> ExitCode {
    let args = Args::parse();
    let Some(command) = args.command else {
        return match nexus_tui::tui::run(args.ascii, args.nerd_font) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nexus: the TUI stopped: {e}");
                ExitCode::FAILURE
            }
        };
    };
    match command {
        Command::Status { json } => cli::status(json),
        Command::Wifi {
            action: WifiAction::List { json },
        } => cli::wifi_list(json),
        Command::Wifi {
            action: WifiAction::Scan { json },
        } => cli::wifi_scan(json),
        Command::Wifi {
            action: WifiAction::Join(a),
        } => {
            let eap = a.eap();
            let source = match (a.password, a.password_stdin) {
                (Some(p), _) => cli::PasswordSource::Flag(p),
                (None, true) => cli::PasswordSource::Stdin,
                (None, false) => cli::PasswordSource::None,
            };
            match eap {
                Some(eap) => cli::wifi_join_eap(&a.ssid, eap, source, a.json),
                None => cli::wifi_join(&a.ssid, source, a.json),
            }
        }
        Command::Wifi {
            action: WifiAction::On { json },
        } => cli::radio(nexus_ipc::Op::WifiPower { on: true }, json),
        Command::Wifi {
            action: WifiAction::Off { json },
        } => cli::radio(nexus_ipc::Op::WifiPower { on: false }, json),
        Command::Airplane { state, json } => cli::radio(
            nexus_ipc::Op::Airplane {
                on: matches!(state, Switch::On),
            },
            json,
        ),
        Command::Wifi {
            action: WifiAction::Disconnect { json },
        } => cli::wifi_disconnect(json),
        Command::Wifi {
            action: WifiAction::Forget { ssid, json },
        } => cli::wifi_forget(&ssid, json),
        Command::Wifi {
            action: WifiAction::Share { ssid, qr, json },
        } => cli::wifi_share(&ssid, qr, json),
        Command::Doctor { json, fix } => cli::doctor(json, fix),
        Command::Config {
            action: ConfigAction::Get { key },
        } => cli::config_get(key),
        Command::Config {
            action: ConfigAction::Set { key, value },
        } => cli::config_set(key, &value),
        Command::Daemon => daemon::run(),
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Args::command(), "nexus", &mut std::io::stdout());
            ExitCode::SUCCESS
        }
        Command::Man => match clap_mangen::Man::new(Args::command()).render(&mut std::io::stdout())
        {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nexus: cannot write the man page: {e}");
                ExitCode::FAILURE
            }
        },
    }
}
