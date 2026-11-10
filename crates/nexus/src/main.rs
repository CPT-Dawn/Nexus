mod backend;
mod bar_cli;
mod cli;
mod config;
mod daemon;
mod doctor;
mod ethernet_view;
mod keyring;
mod manpages;
mod menu;
mod profile_cli;
mod secrets;
mod sysutil;
mod tether_view;
mod text;
mod vpn_cli;
mod waybar;
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
        #[arg(long, conflicts_with = "waybar")]
        json: bool,
        /// Print Waybar custom-module JSON (text, tooltip, class, alt).
        #[arg(long)]
        waybar: bool,
        /// With --waybar: keep running and print a line whenever it changes.
        #[arg(long, requires = "waybar")]
        follow: bool,
    },
    /// Pick a Network or VPN in your launcher (fuzzel, walker, rofi, tofi).
    Menu,
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
    /// Ethernet ports: Link details, plug state, bring up and down.
    Ethernet {
        #[command(subcommand)]
        action: EthernetAction,
    },
    /// Get online through a paired phone over Bluetooth.
    Tether {
        #[command(subcommand)]
        action: TetherAction,
    },
    /// Share this machine's connection as a Wi-Fi Hotspot.
    Hotspot {
        #[command(subcommand)]
        action: HotspotAction,
    },
    /// Show or change the settings of a saved Network's Profile.
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },
    /// Mark Networks Trusted or Untrusted (joining an Untrusted one can bring up a VPN).
    Trust {
        #[command(subcommand)]
        action: TrustAction,
    },
    /// The Network's login page (captive Portal): whether one is asked for, and open it.
    Portal {
        #[command(subcommand)]
        action: Option<PortalAction>,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// What the Daemon did lately: joins, drops, VPNs, Portals (newest first).
    Activity {
        /// Print the log as JSON.
        #[arg(long)]
        json: bool,
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
    /// Print the man page (roff) to stdout, or write one page per command into DIR.
    Man {
        /// Write nexus.1 and nexus-<command>.1 for every command into this directory.
        #[arg(long, value_name = "DIR")]
        dir: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum PortalAction {
    /// Say whether the Network is asking for a login (the default).
    Status,
    /// Open the login page in your browser.
    Open,
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
    Join(Box<JoinArgs>),
    /// Show the access points behind a Network: address, channel, frequency,
    /// band and link rate.
    Aps {
        /// The Network's name (SSID).
        ssid: String,
        /// `ssid` is the hex id from `wifi list --json` (for names that are not UTF-8).
        #[arg(long)]
        hex: bool,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
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
        /// `ssid` is the hex id from `wifi list --json` (for names that are not UTF-8).
        #[arg(long)]
        hex: bool,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Turn automatic joining of a saved Network on or off.
    Autoconnect {
        /// The Network's name (SSID).
        ssid: String,
        /// `on` joins it whenever it is in range; `off` only when you ask.
        #[arg(value_enum)]
        state: Switch,
        /// `ssid` is the hex id from `wifi list --json` (for names that are not UTF-8).
        #[arg(long)]
        hex: bool,
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
    /// Save a WireGuard `.conf`, an OpenVPN `.ovpn` or another provider file
    /// as a VPN.
    Import {
        /// The file to read.
        file: String,
        /// What to call the VPN (default: the file name without its extension).
        #[arg(long)]
        name: Option<String>,
        /// The plugin that reads the file (openvpn, vpnc, ...). Default: by
        /// extension (`.ovpn`, `.pcf`); anything else is read as WireGuard.
        #[arg(long = "type", value_name = "PLUGIN")]
        kind: Option<String>,
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

#[derive(Subcommand)]
enum TrustAction {
    /// List nearby Networks and whether each is Trusted or Untrusted.
    List {
        /// Print the list as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Mark a Network Trusted or Untrusted.
    Set {
        /// The Network's name (SSID).
        ssid: String,
        /// How much to trust it. Unmarked open Networks count as untrusted.
        #[arg(value_enum)]
        trust: TrustLevel,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum TrustLevel {
    Trusted,
    Untrusted,
}

#[derive(Subcommand)]
enum EthernetAction {
    /// List the Ethernet ports: whether a cable is in, the speed, addresses, and what is wrong.
    List {
        /// Print the list as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Bring a port up with its Profile (a plain DHCP one is made if it has none).
    /// With --eap, sign in to an 802.1X port.
    Connect {
        /// The port, e.g. enp3s0 (not needed when there is only one).
        port: Option<String>,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
        /// The password for --eap. Visible in the process list: prefer --password-stdin.
        #[arg(long, requires = "eap", conflicts_with = "password_stdin")]
        password: Option<String>,
        /// Read the --eap password from the first line of stdin.
        #[arg(long, requires = "eap")]
        password_stdin: bool,
        #[command(flatten)]
        eap: Box<EapArgs>,
    },
    /// Drop a port's Link; its Profile stays.
    Disconnect {
        /// The port, e.g. enp3s0 (not needed when there is only one).
        port: Option<String>,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum TetherAction {
    /// List paired phones that offer network access, and whether Bluetooth is on.
    List {
        /// Print the list as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Tether through a phone (the only paired one if you name none).
    Connect {
        /// The phone's name or Bluetooth address.
        phone: Option<String>,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Drop the tether; the phone stays paired and its Profile stays saved.
    Disconnect {
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum HotspotAction {
    /// Whether this machine can host a Hotspot, and the one that runs.
    Status {
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Start a Hotspot. Without a name and password, Nexus makes both up.
    Start {
        /// The Hotspot's name (default: Nexus-XXXX).
        #[arg(long)]
        ssid: Option<String>,
        /// The password, 8 to 63 characters. Visible in the process list:
        /// prefer --password-stdin.
        #[arg(long, conflicts_with = "password_stdin")]
        password: Option<String>,
        /// Read the password from the first line of stdin.
        #[arg(long)]
        password_stdin: bool,
        /// Also draw the join code as a QR code.
        #[arg(long)]
        qr: bool,
        /// Print the result, password included, as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Stop the Hotspot.
    Stop {
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Draw the running Hotspot's join code as a QR code.
    Qr {
        /// Print the join details, password included, as JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ProfileAction {
    /// List every saved Profile, Wi-Fi and Ethernet.
    List {
        /// Print the list as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show a Profile's IP, DNS, MAC, metered and priority settings.
    Show {
        /// The Network's name (SSID).
        ssid: String,
        /// Print the Profile as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Change some of a Profile's settings; what you leave out stays.
    Set(profile_cli::SetArgs),
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
    /// `ssid` is the hex id from `wifi list --json` (for names that are not UTF-8).
    #[arg(long)]
    hex: bool,
    /// The Network is hidden (it does not broadcast its name): join it by name
    /// with this security.
    #[arg(long, value_enum, conflicts_with = "eap")]
    hidden: Option<HiddenSecurity>,
    /// Join through this Wi-Fi device (interface name, e.g. wlan1) instead of
    /// the one that hears the Network best.
    #[arg(long, conflicts_with = "eap")]
    device: Option<String>,
    /// The password. Visible in the process list: prefer --password-stdin.
    #[arg(long, conflicts_with = "password_stdin")]
    password: Option<String>,
    /// Read the password from the first line of stdin.
    #[arg(long)]
    password_stdin: bool,
    /// Print the result as JSON.
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    eap: EapArgs,
}

/// The 802.1X flags shared by `wifi join` and `ethernet connect`.
#[derive(ClapArgs)]
struct EapArgs {
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
    /// Private key password. Visible in the process list: prefer
    /// --private-key-password-stdin.
    #[arg(
        long,
        requires = "eap",
        conflicts_with_all = ["private_key_password_stdin", "password_stdin"]
    )]
    private_key_password: Option<String>,
    /// Read the private key password from the first line of stdin (TLS has no
    /// other password, so this cannot clash with --password-stdin).
    #[arg(long, requires = "eap", conflicts_with = "password_stdin")]
    private_key_password_stdin: bool,
}

/// The first line of stdin, without its newline. Exits 1 when stdin fails.
fn stdin_secret() -> nexus_ipc::Secret {
    let mut line = zeroize::Zeroizing::new(String::new());
    if let Err(e) = std::io::stdin().read_line(&mut line) {
        eprintln!("nexus: cannot read the private key password from stdin: {e}");
        std::process::exit(1);
    }
    nexus_ipc::Secret::new(line.trim_end_matches(['\n', '\r']).to_owned())
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum HiddenSecurity {
    Open,
    /// Enhanced Open (OWE).
    Owe,
    Wpa2,
    Wpa3,
    /// Accepts both WPA2 and WPA3 clients.
    #[value(name = "wpa2-3")]
    Wpa23,
}

impl HiddenSecurity {
    fn security(self) -> nexus_ipc::Security {
        use nexus_ipc::Security;
        match self {
            HiddenSecurity::Open => Security::Open,
            HiddenSecurity::Owe => Security::Owe,
            HiddenSecurity::Wpa2 => Security::Wpa2,
            HiddenSecurity::Wpa3 => Security::Wpa3,
            HiddenSecurity::Wpa23 => Security::Wpa3Transition,
        }
    }
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

impl EapArgs {
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
        eap.private_key_password = if self.private_key_password_stdin {
            Some(stdin_secret())
        } else {
            self.private_key_password
                .clone()
                .map(nexus_ipc::Secret::new)
        };
        Some(eap)
    }
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Print the effective config, or one value by dotted key (e.g. `connectivity.url`).
    Get {
        key: Option<String>,
        /// Print the value as JSON (strings quoted).
        #[arg(long)]
        json: bool,
    },
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
        Command::Status {
            waybar: true,
            follow: true,
            ..
        } => bar_cli::follow(),
        Command::Status { waybar: true, .. } => bar_cli::once(),
        Command::Status { json, .. } => cli::status(json),
        Command::Menu => menu::run(),
        Command::Wifi {
            action: WifiAction::List { json },
        } => cli::wifi_list(json),
        Command::Wifi {
            action: WifiAction::Scan { json },
        } => cli::wifi_scan(json),
        Command::Wifi {
            action: WifiAction::Join(a),
        } => {
            let a = *a;
            let eap = a.eap.eap();
            let source = match (a.password, a.password_stdin) {
                (Some(p), _) => cli::PasswordSource::Flag(p),
                (None, true) => cli::PasswordSource::Stdin,
                (None, false) => cli::PasswordSource::None,
            };
            match eap {
                Some(eap) => cli::wifi_join_eap(&a.ssid, eap, source, a.json, a.hex),
                None => {
                    let at = cli::JoinWhere {
                        hex: a.hex,
                        device: a.device,
                        hidden: a.hidden.map(HiddenSecurity::security),
                    };
                    cli::wifi_join(&a.ssid, source, a.json, at)
                }
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
            action: WifiAction::Forget { ssid, hex, json },
        } => cli::wifi_forget(&ssid, hex, json),
        Command::Wifi {
            action: WifiAction::Aps { ssid, hex, json },
        } => cli::wifi_aps(&ssid, hex, json),
        Command::Profile {
            action: ProfileAction::List { json },
        } => profile_cli::list(json),
        Command::Profile {
            action: ProfileAction::Show { ssid, json },
        } => profile_cli::show(&ssid, json),
        Command::Profile {
            action: ProfileAction::Set(args),
        } => profile_cli::set(args),
        Command::Wifi {
            action: WifiAction::Share { ssid, qr, json },
        } => cli::wifi_share(&ssid, qr, json),
        Command::Vpn { action } => match action {
            VpnAction::List { json } => vpn_cli::list(json),
            VpnAction::Up { name, json } => vpn_cli::toggle(&name, true, json),
            VpnAction::Down { name, json } => vpn_cli::toggle(&name, false, json),
            VpnAction::Import {
                file,
                name,
                kind,
                json,
            } => vpn_cli::import(&file, name, kind, json),
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
        Command::Ethernet { action } => match action {
            EthernetAction::List { json } => cli::ethernet_list(json),
            EthernetAction::Connect {
                port,
                json,
                password,
                password_stdin,
                eap,
            } => match eap.eap() {
                Some(eap) => {
                    let source = match (password, password_stdin) {
                        (Some(p), _) => cli::PasswordSource::Flag(p),
                        (None, true) => cli::PasswordSource::Stdin,
                        (None, false) => cli::PasswordSource::None,
                    };
                    cli::ethernet_join_eap(port, eap, source, json)
                }
                None => cli::ethernet_connect(port, json),
            },
            EthernetAction::Disconnect { port, json } => cli::ethernet_disconnect(port, json),
        },
        Command::Hotspot {
            action: HotspotAction::Status { json },
        } => cli::hotspot_status(json),
        Command::Hotspot {
            action:
                HotspotAction::Start {
                    ssid,
                    password,
                    password_stdin,
                    qr,
                    json,
                },
        } => {
            let source = match (password, password_stdin) {
                (Some(p), _) => cli::PasswordSource::Flag(p),
                (None, true) => cli::PasswordSource::Stdin,
                (None, false) => cli::PasswordSource::None,
            };
            cli::hotspot_start(ssid, source, qr, json)
        }
        Command::Hotspot {
            action: HotspotAction::Stop { json },
        } => cli::hotspot_stop(json),
        Command::Hotspot {
            action: HotspotAction::Qr { json },
        } => cli::hotspot_qr(json),
        Command::Trust {
            action: TrustAction::List { json },
        } => cli::trust_list(json),
        Command::Trust {
            action: TrustAction::Set { ssid, trust, json },
        } => cli::trust_set(
            &ssid,
            match trust {
                TrustLevel::Trusted => nexus_ipc::Trust::Trusted,
                TrustLevel::Untrusted => nexus_ipc::Trust::Untrusted,
            },
            json,
        ),
        Command::Tether {
            action: TetherAction::List { json },
        } => cli::tether_list(json),
        Command::Tether {
            action: TetherAction::Connect { phone, json },
        } => cli::tether_connect(phone, json),
        Command::Tether {
            action: TetherAction::Disconnect { json },
        } => cli::tether_disconnect(json),
        Command::Portal { action, json } => match action.unwrap_or(PortalAction::Status) {
            PortalAction::Status => cli::portal_status(json),
            PortalAction::Open => cli::portal_open(json),
        },
        Command::Activity { json } => cli::activity(json),
        Command::Wifi {
            action:
                WifiAction::Autoconnect {
                    ssid,
                    state,
                    hex,
                    json,
                },
        } => cli::wifi_autoconnect(&ssid, matches!(state, Switch::On), hex, json),
        Command::Doctor { json, fix } => cli::doctor(json, fix),
        Command::Config {
            action: ConfigAction::Get { key, json },
        } => cli::config_get(key, json),
        Command::Config {
            action: ConfigAction::Set { key, value },
        } => cli::config_set(key, &value),
        Command::Daemon => daemon::run(),
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Args::command(), "nexus", &mut std::io::stdout());
            ExitCode::SUCCESS
        }
        Command::Man { dir } => match manpages::write(&Args::command(), dir.as_deref()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nexus: cannot write the man page: {e}");
                ExitCode::FAILURE
            }
        },
    }
}
