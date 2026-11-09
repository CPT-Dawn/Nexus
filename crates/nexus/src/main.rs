mod backend;
mod cli;
mod config;
mod daemon;
mod doctor;
mod secrets;
mod text;
mod vpn_cli;
mod wifi_view;
mod wireguard;

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
    /// VPNs: list, bring up and down, import and create WireGuard tunnels.
    Vpn {
        #[command(subcommand)]
        action: VpnAction,
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

#[derive(Subcommand)]
enum VpnAction {
    /// List the VPN Profiles and whether each is up.
    List {
        /// Print the list as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Bring a VPN up, waiting until it works.
    Up {
        /// The VPN's name.
        name: String,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Bring a VPN down.
    Down {
        /// The VPN's name.
        name: String,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Save a WireGuard `.conf` file as a VPN.
    Import {
        /// The file to read.
        file: String,
        /// What to call the VPN (default: the file name without `.conf`).
        #[arg(long)]
        name: Option<String>,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Create a WireGuard VPN with a freshly generated key pair, and print
    /// the public key to give to the other side.
    Create {
        /// What to call the VPN.
        name: String,
        /// This end's address, e.g. 10.0.0.2/32 (repeat for IPv6).
        #[arg(long = "address", required = true)]
        addresses: Vec<String>,
        /// A DNS server to use while the VPN is up (repeatable).
        #[arg(long)]
        dns: Vec<String>,
        /// The peer's public key. Without it the tunnel cannot carry traffic yet.
        #[arg(long)]
        peer_key: Option<String>,
        /// The peer's address, host:port.
        #[arg(long, requires = "peer_key")]
        endpoint: Option<String>,
        /// What to send through the tunnel (default: everything).
        #[arg(long = "allowed-ips")]
        allowed_ips: Vec<String>,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print a WireGuard VPN's public key.
    Key {
        /// The VPN's name.
        name: String,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Reveal a WireGuard VPN's whole config (private key included), or show
    /// it as a QR code for a phone. NetworkManager may ask you to authorize it.
    Share {
        /// The VPN's name.
        name: String,
        /// Draw the config as a QR code instead of printing it.
        #[arg(long)]
        qr: bool,
        /// Print the result, private key included, as JSON.
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
            let source = match (a.password, a.password_stdin) {
                (Some(p), _) => cli::PasswordSource::Flag(p),
                (None, true) => cli::PasswordSource::Stdin,
                (None, false) => cli::PasswordSource::None,
            };
            cli::wifi_join(&a.ssid, source, a.json)
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
        Command::Vpn { action } => match action {
            VpnAction::List { json } => vpn_cli::list(json),
            VpnAction::Up { name, json } => vpn_cli::toggle(&name, true, json),
            VpnAction::Down { name, json } => vpn_cli::toggle(&name, false, json),
            VpnAction::Import { file, name, json } => vpn_cli::import(&file, name, json),
            VpnAction::Create {
                name,
                addresses,
                dns,
                peer_key,
                endpoint,
                allowed_ips,
                json,
            } => vpn_cli::create(vpn_cli::Create {
                name,
                addresses,
                dns,
                peer_key,
                endpoint,
                allowed_ips,
                json,
            }),
            VpnAction::Key { name, json } => vpn_cli::key(&name, json),
            VpnAction::Share { name, qr, json } => vpn_cli::share(&name, qr, json),
        },
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
