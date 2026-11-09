//! The text catalog: every sentence a person may read about a Network failing
//! lives here, so wording can be reviewed (and one day translated) in one place.
//!
//! NetworkManager reports why a Device changed state as a number
//! (`NMDeviceStateReason`, 0..=79 today). [`join_failure`] turns the state a
//! join died in and that number into a [`Failure`]: a class the Front end can
//! act on, and a sentence. Numbers the catalog does not know read as
//! "Unknown reason (N)", because the enum grows with every NetworkManager.

use nexus_ipc::ErrorCode;

/// Why a join (or other Backend action) failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub code: ErrorCode,
    pub message: String,
}

impl From<Failure> for nexus_ipc::Failure {
    fn from(f: Failure) -> Self {
        Self {
            code: f.code,
            message: f.message,
        }
    }
}

fn fail(code: ErrorCode, message: impl Into<String>) -> Failure {
    Failure {
        code,
        message: message.into(),
    }
}

// `NMDeviceState`
const STATE_CONFIG: u32 = 50;
const STATE_NEED_AUTH: u32 = 60;

// `NMDeviceStateReason` values the classifier cares about.
const NO_SECRETS: u32 = 7;
const SUPPLICANT_DISCONNECT: u32 = 8;
const SUPPLICANT_FAILED: u32 = 10;
const SUPPLICANT_TIMEOUT: u32 = 11;
const SSID_NOT_FOUND: u32 = 53;

/// What a Device was doing, in words ("asking for an address").
pub fn device_state(state: u32) -> String {
    match state {
        0 => "in an unknown state".into(),
        10 => "not managed by NetworkManager".into(),
        20 => "unavailable".into(),
        30 => "disconnected".into(),
        40 => "getting ready".into(),
        50 => "connecting to the access point".into(),
        60 => "waiting for the password".into(),
        70 => "asking for an address".into(),
        80 => "checking the address".into(),
        90 => "waiting for dependent connections".into(),
        100 => "connected".into(),
        110 => "disconnecting".into(),
        120 => "failed".into(),
        n => format!("in an unknown state ({n})"),
    }
}

/// The sentence for one `NMDeviceStateReason`, without any Network name.
pub fn device_reason(reason: u32) -> String {
    let text = match reason {
        0 => "NetworkManager gave no reason",
        1 => "Nothing went wrong",
        2 => "The device is now managed by NetworkManager",
        3 => "The device is no longer managed by NetworkManager",
        4 => "The network settings could not be applied",
        5 => "The network never gave this computer an address",
        6 => "The address from the network expired",
        7 => "The network needs a password that was not supplied",
        8 => "The Wi-Fi supplicant was disconnected",
        9 => "The Wi-Fi supplicant could not be configured",
        10 => "The Wi-Fi supplicant failed",
        11 => "The Wi-Fi supplicant timed out",
        12 => "The PPP service failed to start",
        13 => "The PPP service disconnected",
        14 => "The PPP service failed",
        15 => "The DHCP client failed to start",
        16 => "The DHCP client hit an error",
        17 => "The DHCP client failed to get an address",
        18 => "Connection sharing failed to start",
        19 => "Connection sharing failed",
        20 => "The link-local address service failed to start",
        21 => "The link-local address service hit an error",
        22 => "The link-local address service failed",
        23 => "The modem is busy",
        24 => "The modem found no dial tone",
        25 => "The modem found no carrier",
        26 => "The modem timed out while dialling",
        27 => "The modem failed to dial",
        28 => "The modem failed to initialise",
        29 => "The mobile access point name was rejected",
        30 => "The mobile network is not searching for a carrier",
        31 => "The mobile network refused the registration",
        32 => "Registering on the mobile network timed out",
        33 => "Registering on the mobile network failed",
        34 => "The SIM PIN check failed",
        35 => "The Wi-Fi firmware is missing",
        36 => "The device was removed",
        37 => "The computer went to sleep",
        38 => "The saved network was removed",
        39 => "You disconnected it",
        40 => "The cable was unplugged",
        41 => "The connection was already up when NetworkManager started",
        42 => "The Wi-Fi supplicant is available again",
        43 => "The modem was not found",
        44 => "The Bluetooth connection failed or timed out",
        45 => "The SIM card is missing",
        46 => "The SIM card needs a PIN",
        47 => "The SIM card needs a PUK",
        48 => "The wrong SIM card is inserted",
        49 => "The InfiniBand device is in the wrong mode",
        50 => "A connection this one depends on failed",
        51 => "The DSL bridge setup failed",
        52 => "ModemManager is not running",
        53 => "The network was not found",
        54 => "A secondary connection (such as a VPN) failed",
        55 => "The DCB or FCoE setup failed",
        56 => "The team controller failed",
        57 => "The modem failed or is no longer available",
        58 => "The modem is available again",
        59 => "The SIM PIN was incorrect",
        60 => "Another connection was started instead",
        61 => "The parent device changed",
        62 => "The parent device's managed state changed",
        63 => "The Open vSwitch database failed",
        64 => "Another device on the network has the same address",
        65 => "The network settings use an address method this computer cannot do",
        66 => "The SR-IOV setup failed",
        67 => "The peer was not found",
        68 => "The device handler failed",
        69 => "The device is not managed by default",
        70 => "The device is not managed because it is down",
        71 => "The device is not managed because its link is not ready",
        72 => "The device is not managed because NetworkManager is quitting",
        73 => "The device is not managed because networking is disabled",
        74 => "The device is not managed by your NetworkManager configuration",
        75 => "You asked for the device not to be managed",
        76 => "The device is not managed by its settings",
        77 => "The device is not managed by a udev rule",
        78 => "Networking is switched off",
        79 => "The modem reported a problem",
        n => return format!("Unknown reason ({n})"),
    };
    text.into()
}

/// Turns the state a join died in and NetworkManager's reason into a
/// [`Failure`] about `ssid`. `secured` says whether a password was in play, so
/// a supplicant failure reads as "wrong password" only when it can be one.
pub fn join_failure(ssid: &str, died_in: u32, reason: u32, secured: bool) -> Failure {
    let auth_stage = died_in == STATE_NEED_AUTH || died_in == STATE_CONFIG;
    match reason {
        NO_SECRETS => fail(ErrorCode::WrongPassword, wrong_password(ssid)),
        SUPPLICANT_DISCONNECT | SUPPLICANT_FAILED | SUPPLICANT_TIMEOUT if secured && auth_stage => {
            fail(ErrorCode::WrongPassword, wrong_password(ssid))
        }
        // WPA3-SAE reports a wrong password as "SSID not found" while it is
        // still authenticating.
        SSID_NOT_FOUND if secured && auth_stage => {
            fail(ErrorCode::WrongPassword, wrong_password(ssid))
        }
        SSID_NOT_FOUND => fail(ErrorCode::OutOfRange, out_of_range_sentence(ssid)),
        5 | 6 | 15 | 16 | 17 => fail(
            ErrorCode::NoAddress,
            format!(
                "Connected to {ssid}, but it never gave this computer an address \
                 (DHCP timed out). Try again, or move closer to the router."
            ),
        ),
        39 => fail(
            ErrorCode::JoinFailed,
            format!("Joining {ssid} was cancelled."),
        ),
        other => fail(
            ErrorCode::JoinFailed,
            format!(
                "Could not join {ssid} while {}. {}.",
                device_state(died_in),
                device_reason(other)
            ),
        ),
    }
}

pub fn wrong_password(ssid: &str) -> String {
    format!("Wrong password for {ssid}.")
}

fn out_of_range_sentence(ssid: &str) -> String {
    format!("{ssid} is not in range. Move closer, or scan again.")
}

/// A hidden Network was named with a security Nexus cannot join blind.
pub fn hidden_security(ssid: &str, what: &str) -> Failure {
    fail(
        ErrorCode::InvalidSettings,
        format!(
            "Cannot join the hidden network {ssid} as {what}. \
             Choose open, owe, wpa2, wpa3 or wpa2/3."
        ),
    )
}

/// No Wi-Fi Device has that name.
pub fn unknown_device(device: &str) -> Failure {
    fail(
        ErrorCode::UnknownDevice,
        format!("No Wi-Fi device called {device}. `nexus wifi list --json` names them."),
    )
}

/// The chosen Wi-Fi Device is not the one that hears the Network.
pub fn not_heard_by(ssid: &str, device: &str) -> Failure {
    fail(
        ErrorCode::OutOfRange,
        format!("{device} cannot hear {ssid}. Pick another Wi-Fi device, or move closer."),
    )
}

pub fn out_of_range(ssid: &str) -> Failure {
    fail(ErrorCode::OutOfRange, out_of_range_sentence(ssid))
}

/// Nobody answered the password prompt in time. The Network stays unsaved and
/// reads as "password needed".
pub fn prompt_expired(ssid: &str) -> Failure {
    fail(
        ErrorCode::PasswordRequired,
        format!(
            "{ssid} needs a password, and nobody gave one in time. Join it again to be asked again."
        ),
    )
}

/// The password prompt was declined (or no Front end could answer it).
pub fn prompt_cancelled(ssid: &str) -> Failure {
    fail(
        ErrorCode::PasswordRequired,
        format!("{ssid} needs a password. Pass --password, or --password-stdin to type it in."),
    )
}

/// The Backend dropped the join while its prompt was open.
pub fn join_withdrawn(ssid: &str) -> Failure {
    fail(
        ErrorCode::JoinFailed,
        format!("Joining {ssid} was cancelled."),
    )
}

/// The sentence on a password prompt.
pub fn password_prompt(ssid: &str, retry: bool) -> String {
    if retry {
        format!("Wrong password for {ssid}. Try again.")
    } else {
        format!("Password for {ssid}")
    }
}

pub fn unknown_prompt() -> String {
    "That prompt is closed already (answered, cancelled or expired).".into()
}

pub fn bad_passphrase_length(ssid: &str) -> Failure {
    fail(
        ErrorCode::BadRequest,
        format!("The password for {ssid} must be 8 to 63 characters long."),
    )
}

pub fn backend_refused(ssid: &str, why: &str) -> Failure {
    fail(
        ErrorCode::JoinFailed,
        format!("NetworkManager would not let Nexus join {ssid}: {why}"),
    )
}

pub fn unsupported_security(ssid: &str, what: &str) -> Failure {
    fail(
        ErrorCode::BadRequest,
        format!("{ssid} uses {what}, which Nexus cannot join yet."),
    )
}

/// The 802.1X form has problems; says each in one sentence.
pub fn invalid_eap(ssid: &str, errors: &[nexus_ipc::EapError]) -> Failure {
    let list: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
    fail(
        ErrorCode::InvalidSettings,
        format!("Cannot sign in to {ssid} yet. {}", list.join(" ")),
    )
}

/// Enterprise settings were given for a Network that is not enterprise.
pub fn not_enterprise(ssid: &str, what: &str) -> Failure {
    fail(
        ErrorCode::InvalidSettings,
        format!(
            "{ssid} is not an enterprise Network (it uses {what}), so it has no sign-in settings."
        ),
    )
}

/// An enterprise Network joined with the plain `wifi_join`.
pub fn needs_eap(ssid: &str) -> Failure {
    fail(
        ErrorCode::InvalidSettings,
        format!(
            "{ssid} is an enterprise Network and needs a sign-in. \
             Pass --eap peap (or ttls, tls) with --identity, --password-stdin and --domain \
             or --ca-cert."
        ),
    )
}

/// The enterprise server refused the sign-in.
pub fn eap_refused(ssid: &str) -> Failure {
    fail(
        ErrorCode::WrongPassword,
        format!(
            "{ssid} refused the sign-in. Check your identity and password, \
             and that the server name or CA certificate is the right one."
        ),
    )
}

pub fn join_timed_out(ssid: &str) -> Failure {
    fail(
        ErrorCode::JoinFailed,
        format!("Joining {ssid} took too long, so Nexus gave up."),
    )
}

pub fn unknown_network(ssid: &str) -> Failure {
    fail(
        ErrorCode::UnknownNetwork,
        format!("No saved network called {ssid}."),
    )
}

pub fn reveal_denied(ssid: &str) -> Failure {
    fail(
        ErrorCode::RevealDenied,
        format!(
            "NetworkManager did not authorize showing the password of {ssid}. \
             Approve the authorization request and try again."
        ),
    )
}

pub fn cannot_share(ssid: &str) -> Failure {
    fail(
        ErrorCode::JoinFailed,
        format!("{ssid} uses a security type that has no QR code format."),
    )
}

pub fn no_stored_password(ssid: &str) -> Failure {
    fail(
        ErrorCode::UnknownNetwork,
        format!("No password is stored for {ssid}; the system keeps it elsewhere."),
    )
}

pub fn not_connected() -> Failure {
    fail(
        ErrorCode::NotConnected,
        "Not connected to any Wi-Fi network.",
    )
}

pub fn wifi_hard_blocked() -> Failure {
    fail(
        ErrorCode::HardBlocked,
        "Wi-Fi is blocked by a hardware switch (a key, slider or BIOS setting). \
         Flip it off and Nexus will pick it up; software cannot turn it on.",
    )
}

/// A join that NetworkManager's restart cut short.
pub fn join_interrupted(ssid: &str) -> Failure {
    fail(
        ErrorCode::JoinFailed,
        format!("NetworkManager restarted while joining {ssid}. Nothing was saved; try again."),
    )
}

/// `nexus wifi on|off` and `nexus airplane on|off` outcomes.
pub fn radios(r: &nexus_ipc::Radios) -> String {
    r.human().map_or_else(
        || "Wi-Fi is on.".to_owned(),
        |what| {
            let mut s = what.to_owned();
            s.replace_range(..1, &s[..1].to_uppercase());
            format!("{s}.")
        },
    )
}

pub fn bad_network_id() -> Failure {
    fail(ErrorCode::BadRequest, "`network` is not a network id")
}

/// Printed by the CLI after a successful action.
pub fn joined(ssid: &str) -> String {
    format!("Joined {ssid}.")
}

pub fn disconnected(ssid: &str) -> String {
    format!("Disconnected from {ssid}.")
}

pub fn trust_marked(ssid: &str, trust: nexus_ipc::Trust) -> String {
    match trust {
        nexus_ipc::Trust::Untrusted => format!("{ssid} is now untrusted."),
        _ => format!("{ssid} is now trusted."),
    }
}

pub fn forgot(ssid: &str) -> String {
    format!("Forgot {ssid}.")
}

// Profile settings

pub fn invalid_settings(problems: &[String]) -> Failure {
    fail(
        ErrorCode::InvalidSettings,
        format!("Nothing was changed. {}", problems.join(" ")),
    )
}

/// The Profile changed since it was read.
pub fn profile_conflict(ssid: &str) -> Failure {
    fail(
        ErrorCode::Conflict,
        format!(
            "{ssid} was changed elsewhere while you were editing. \
             Nothing was saved; open its settings again."
        ),
    )
}

pub fn change_waiting(ssid: &str) -> Failure {
    fail(
        ErrorCode::Conflict,
        format!("A change to {ssid} is waiting to be kept or reverted. Settle it first."),
    )
}

/// The keyring does not answer; the Profile was left as it was.
pub fn keyring_unavailable(ssid: &str, why: &str) -> Failure {
    fail(
        ErrorCode::KeyringUnavailable,
        format!(
            "Nexus cannot use your keyring ({why}). Start a Secret Service such as \
             gnome-keyring, KWallet or KeePassXC and unlock it. \
             The password of {ssid} was left where it is."
        ),
    )
}

/// "Keep in keyring" for a Network that has no stored password to move.
pub fn no_password_to_keep(ssid: &str) -> Failure {
    fail(
        ErrorCode::InvalidSettings,
        format!(
            "{ssid} has no password Nexus can read, so there is nothing to keep in the keyring."
        ),
    )
}

pub fn nothing_pending() -> Failure {
    fail(
        ErrorCode::NothingPending,
        "No change is waiting for confirmation. It was kept, reverted or timed out already.",
    )
}

pub fn checkpoint_refused(why: &str) -> Failure {
    fail(
        ErrorCode::Internal,
        format!(
            "NetworkManager would not set up the safety net for this change, so nothing was \
             changed: {why}"
        ),
    )
}

pub fn profile_kept(ssid: &str) -> String {
    format!("Keeping the new settings of {ssid}.")
}

pub fn profile_reverted(ssid: &str) -> String {
    format!("Reverted: {ssid} has its old settings again.")
}

pub fn profile_saved(ssid: &str) -> String {
    format!("Saved the settings of {ssid}.")
}

pub fn share_password(ssid: &str, password: &str) -> String {
    format!("{ssid}: {password}")
}

pub fn share_open(ssid: &str) -> String {
    format!("{ssid} is an open network: no password to share.")
}

pub fn share_scan_hint(ssid: &str) -> String {
    format!("Point a phone camera at the code to join {ssid}.")
}

// VPN

pub fn unknown_vpn() -> Failure {
    fail(
        ErrorCode::UnknownVpn,
        "No VPN with that name or id is saved.",
    )
}

pub fn vpn_not_up(name: &str) -> Failure {
    fail(ErrorCode::NotConnected, format!("{name} is not up."))
}

pub fn vpn_failed(name: &str, why: &str) -> Failure {
    fail(
        ErrorCode::VpnFailed,
        format!("{name} did not come up: {why}."),
    )
}

pub fn vpn_timed_out(name: &str) -> Failure {
    vpn_failed(name, "it took too long")
}

pub fn vpn_reveal_denied(name: &str) -> Failure {
    fail(
        ErrorCode::RevealDenied,
        format!(
            "NetworkManager did not authorize showing the keys of {name}. \
             Approve the authorization request and try again."
        ),
    )
}

pub fn invalid_config(why: &str) -> Failure {
    fail(
        ErrorCode::InvalidConfig,
        format!("That is not a WireGuard config I can use: {why}."),
    )
}

pub fn missing_plugin(plugin: &str) -> Failure {
    fail(
        ErrorCode::MissingPlugin,
        nexus_tui::plugin::install_hint(plugin),
    )
}

pub fn invalid_provider_file(plugin: &str, why: &str) -> Failure {
    let label = nexus_tui::plugin::by_name(plugin).map_or(plugin, |p| p.label);
    fail(
        ErrorCode::InvalidConfig,
        format!("That is not a {label} file I can use: {why}."),
    )
}

pub fn unknown_plugin(plugin: &str) -> Failure {
    fail(
        ErrorCode::InvalidConfig,
        format!("`{plugin}` is not a VPN type I can import."),
    )
}

/// The sentence of a plugin VPN's Secret prompt.
pub fn vpn_prompt(vpn: &str, field: nexus_ipc::PromptField, retry: bool) -> String {
    use nexus_ipc::PromptField;
    match (field, retry) {
        (PromptField::Otp, false) => format!("One-time code for {vpn}"),
        (PromptField::Otp, true) => format!("That code did not work for {vpn}. Enter a new one."),
        (PromptField::Username, _) => format!("User name for {vpn}"),
        (_, false) => format!("Password for {vpn}"),
        (_, true) => format!("Wrong password for {vpn}. Try again."),
    }
}

pub fn not_wireguard(name: &str) -> Failure {
    fail(
        ErrorCode::InvalidConfig,
        format!("{name} is not a WireGuard VPN, so it has no keys to show."),
    )
}

pub fn no_stored_key(name: &str) -> Failure {
    fail(
        ErrorCode::InvalidConfig,
        format!("{name} has no private key stored."),
    )
}

pub fn vpn_dropped(name: &str) -> String {
    format!("{name} went down. Your traffic is no longer tunnelled.")
}

pub fn vpn_scan_hint(name: &str) -> String {
    format!("Scan with the WireGuard app to copy {name} to a phone.")
}

// Bluetooth Tethering.

pub fn bluetooth_off() -> Failure {
    fail(
        ErrorCode::BluetoothOff,
        "Bluetooth is off. Turn it on (`bluetoothctl power on`, or `rfkill unblock bluetooth`) and try again.",
    )
}

pub fn bluetooth_absent() -> Failure {
    fail(
        ErrorCode::BluetoothOff,
        "This machine has no Bluetooth adapter, or the Bluetooth service is not running \
         (`systemctl start bluetooth`).",
    )
}

pub fn no_phone() -> Failure {
    fail(
        ErrorCode::NoPhone,
        "No paired phone offers network access. Pair your phone (`bluetoothctl`), switch on \
         Bluetooth tethering in its settings, and try again.",
    )
}

pub fn unknown_phone(who: &str, paired: &[String]) -> Failure {
    fail(
        ErrorCode::UnknownPhone,
        format!(
            "No paired phone is called {who}. Paired: {}.",
            paired.join(", ")
        ),
    )
}

pub fn which_phone(paired: &[String]) -> Failure {
    fail(
        ErrorCode::UnknownPhone,
        format!(
            "Several phones are paired ({}). Say which one to tether through.",
            paired.join(", ")
        ),
    )
}

/// A Tethering Link that died on the way up.
pub fn tether_failure(phone: &str, died_in: u32, reason: u32) -> Failure {
    fail(
        ErrorCode::JoinFailed,
        format!(
            "Could not tether through {phone} (stopped while {}: {}). Check that Bluetooth \
             tethering is switched on in the phone's settings.",
            device_state(died_in),
            device_reason(reason).to_lowercase()
        ),
    )
}

pub fn tether_refused(phone: &str, why: &str) -> Failure {
    fail(
        ErrorCode::JoinFailed,
        format!("NetworkManager would not tether through {phone}: {why}"),
    )
}

pub fn tether_timed_out(phone: &str) -> Failure {
    fail(
        ErrorCode::JoinFailed,
        format!("Tethering through {phone} took too long, so Nexus gave up."),
    )
}

pub fn tether_interrupted(phone: &str) -> Failure {
    fail(
        ErrorCode::JoinFailed,
        format!("NetworkManager restarted while tethering through {phone}. Try again."),
    )
}

pub fn not_tethered() -> Failure {
    fail(ErrorCode::NotConnected, "Not tethered to any phone.")
}

pub fn tethered(phone: &str) -> String {
    format!("Tethered through {phone}.")
}

pub fn untethered(phone: &str) -> String {
    format!("Dropped the tether to {phone}.")
}

/// The keyring switch is for Wi-Fi passwords.
pub fn keyring_not_for_wired() -> Failure {
    fail(
        ErrorCode::InvalidSettings,
        "The keyring keeps Wi-Fi passwords; a wired Profile has no password of its own to move.",
    )
}

// ---- Ethernet ---------------------------------------------------------------

/// No saved wired Profile has the uuid a profile op named.
pub fn unknown_profile() -> Failure {
    fail(
        ErrorCode::UnknownNetwork,
        "No saved Ethernet Profile has that id.",
    )
}

/// The cable is out: nothing can be joined until it is back.
pub fn unplugged(port: &str) -> Failure {
    fail(ErrorCode::Unplugged, unplugged_sentence(port))
}

/// The sentence for a port with no cable.
pub fn unplugged_sentence(port: &str) -> String {
    nexus_ipc::unplugged_message(port)
}

/// Which port the person meant, when they did not say or said wrong.
pub fn unknown_port(asked: Option<&str>, ports: &[String]) -> Failure {
    let message = match (asked, ports) {
        (_, []) => "This computer has no Ethernet port that NetworkManager knows of.".to_owned(),
        (Some(asked), _) => format!(
            "No Ethernet port is called {asked} (ports: {}).",
            ports.join(", ")
        ),
        (None, _) => format!(
            "There are several Ethernet ports; name one ({}).",
            ports.join(", ")
        ),
    };
    fail(ErrorCode::UnknownPort, message)
}

pub fn ethernet_not_connected(port: &str) -> Failure {
    fail(ErrorCode::NotConnected, format!("{port} is not connected."))
}

/// What is wrong with a port that has a cable in it, by NetworkManager's
/// reason number. `enterprise` says the Profile already signs in with 802.1X.
pub fn trouble(port: &str, reason: u32, enterprise: bool) -> nexus_ipc::EthernetTrouble {
    use nexus_ipc::{EthernetProblem, EthernetTrouble};
    match reason {
        5 | 6 | 15 | 16 | 17 => EthernetTrouble {
            kind: EthernetProblem::NoAddress,
            message: format!(
                "The cable is plugged in, but the network gave {port} no address (DHCP got \
                 no answer). Check the router or switch, or set an address by hand in the \
                 Profile settings. An office or campus port may also need a sign-in (802.1X)."
            ),
        },
        NO_SECRETS | SUPPLICANT_DISCONNECT | 9 | SUPPLICANT_FAILED | SUPPLICANT_TIMEOUT => {
            EthernetTrouble {
                kind: EthernetProblem::SignIn,
                message: if enterprise {
                    format!(
                        "{port} refused the 802.1X sign-in. Check your identity, password \
                         and certificates, and that the server name is the right one."
                    )
                } else {
                    format!(
                        "{port} wants an 802.1X sign-in. Sign in with your identity and password."
                    )
                },
            }
        }
        other => EthernetTrouble {
            kind: EthernetProblem::Other,
            message: format!(
                "The cable is plugged in, but {port} would not come up. {}.",
                device_reason(other)
            ),
        },
    }
}

/// A join that died on the way up, as a [`Failure`].
pub fn ethernet_failure(port: &str, reason: u32, enterprise: bool) -> Failure {
    use nexus_ipc::EthernetProblem;
    let t = trouble(port, reason, enterprise);
    let code = match t.kind {
        EthernetProblem::NoAddress => ErrorCode::NoAddress,
        EthernetProblem::SignIn if enterprise => ErrorCode::WrongPassword,
        _ => ErrorCode::JoinFailed,
    };
    fail(code, t.message)
}

pub fn ethernet_refused(port: &str, why: &str) -> Failure {
    fail(
        ErrorCode::JoinFailed,
        format!("NetworkManager would not bring {port} up: {why}"),
    )
}

pub fn ethernet_timed_out(port: &str) -> Failure {
    fail(
        ErrorCode::JoinFailed,
        format!("Bringing {port} up took too long, so Nexus gave up."),
    )
}

pub fn ethernet_interrupted(port: &str) -> Failure {
    fail(
        ErrorCode::JoinFailed,
        format!("NetworkManager restarted while bringing {port} up. Try again."),
    )
}

pub fn ethernet_joined(port: &str) -> String {
    format!("{port} is up.")
}

pub fn ethernet_dropped(port: &str) -> String {
    format!("Dropped {port}.")
}

// Hotspot.

pub fn no_wifi_for_hotspot() -> Failure {
    fail(
        ErrorCode::HotspotUnsupported,
        "This machine has no Wi-Fi card, so it cannot host a Hotspot.",
    )
}

pub fn wifi_off_for_hotspot() -> Failure {
    fail(
        ErrorCode::HotspotUnsupported,
        "Wi-Fi is switched off, so a Hotspot cannot start. Turn it on with `nexus wifi on`.",
    )
}

pub fn card_cannot_host() -> Failure {
    fail(
        ErrorCode::HotspotUnsupported,
        "This Wi-Fi card cannot host a Hotspot: its driver has no access point mode (a USB \
         adapter that has one would do; see `iw list`).",
    )
}

pub fn hotspot_running(ssid: &str) -> Failure {
    fail(
        ErrorCode::JoinFailed,
        format!("The Hotspot {ssid} is already running. Stop it first."),
    )
}

pub fn hotspot_failed(ssid: &str, why: &str) -> Failure {
    fail(
        ErrorCode::JoinFailed,
        format!("The Hotspot {ssid} did not start: {why}."),
    )
}

pub fn hotspot_timed_out(ssid: &str) -> Failure {
    hotspot_failed(ssid, "it took too long")
}

pub fn invalid_hotspot(why: &str) -> Failure {
    fail(ErrorCode::InvalidSettings, why)
}

pub fn hotspot_not_running() -> Failure {
    fail(ErrorCode::NotConnected, "No Hotspot is running.")
}

pub fn hotspot_started(ssid: &str) -> String {
    format!("Hotspot {ssid} is on.")
}

pub fn hotspot_stopped(ssid: &str) -> String {
    format!("Hotspot {ssid} stopped.")
}

/// One line for `nexus hotspot status`.
pub fn hotspot_line(h: &nexus_ipc::Hotspot) -> String {
    match (&h.ssid, h.on) {
        (Some(ssid), true) if h.starting => format!("Hotspot {ssid} is starting."),
        (Some(ssid), true) => format!("Hotspot {ssid} is on, {}.", hotspot_devices(h.devices)),
        _ if h.available => "No Hotspot is running. Start one with `nexus hotspot start`.".into(),
        _ => h
            .why
            .clone()
            .unwrap_or_else(|| "This machine cannot host a Hotspot.".into()),
    }
}

pub fn hotspot_devices(n: Option<u32>) -> String {
    match n {
        Some(1) => "1 device joined".into(),
        Some(n) => format!("{n} devices joined"),
        None => "device count unknown (install `iw` to count them)".into(),
    }
}

// Portal, autoconnect and the activity log on the command line.

pub fn autoconnect_set(ssid: &str, enabled: bool) -> String {
    if enabled {
        format!("{ssid} will be joined automatically.")
    } else {
        format!("{ssid} will only be joined when you ask.")
    }
}

pub fn portal_line(portal: Option<&nexus_ipc::Portal>) -> String {
    match portal {
        Some(p) => format!(
            "{} wants a login. Run `nexus portal open` to open {}.",
            p.network, p.url
        ),
        None => "No login is being asked for.".into(),
    }
}

pub fn portal_opened(url: &str) -> String {
    format!("Opened {url} in your browser.")
}

/// `90` is `1 min ago`, `7300` is `2 h ago`.
pub fn ago(secs: u64) -> String {
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_reason_has_its_own_sentence() {
        for n in 0..=79 {
            assert!(!device_reason(n).starts_with("Unknown"), "reason {n}");
        }
    }

    #[test]
    fn numbers_beyond_the_catalog_fall_back() {
        assert_eq!(device_reason(80), "Unknown reason (80)");
        assert_eq!(device_state(130), "in an unknown state (130)");
    }

    #[test]
    fn every_device_state_reads_as_words() {
        for n in [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120] {
            assert!(!device_state(n).contains("unknown state ("), "state {n}");
        }
    }

    #[test]
    fn supplicant_trouble_is_a_wrong_password_only_when_a_password_was_in_play() {
        let f = join_failure("Home", STATE_CONFIG, SUPPLICANT_TIMEOUT, true);
        assert_eq!(f.code, ErrorCode::WrongPassword);
        let f = join_failure("Cafe", STATE_CONFIG, SUPPLICANT_TIMEOUT, false);
        assert_eq!(f.code, ErrorCode::JoinFailed);
        assert!(f.message.contains("supplicant timed out"), "{}", f.message);
    }

    #[test]
    fn the_wrong_password_sentence_names_the_network() {
        let f = join_failure("Home-5G", STATE_NEED_AUTH, NO_SECRETS, true);
        assert_eq!(f.message, "Wrong password for Home-5G.");
    }
}

// ---- Doctor fixes, config and notifications ---------------------------------

pub fn fix_not_confirmed() -> Failure {
    fail(
        ErrorCode::NotConfirmed,
        "A fix runs only after an explicit yes: send `confirm: true`.",
    )
}

pub fn nothing_to_fix(id: &str) -> Failure {
    fail(
        ErrorCode::UnknownFinding,
        format!("There is nothing to fix for `{id}` right now."),
    )
}

pub fn fix_not_started(command: &str, why: &std::io::Error) -> Failure {
    fail(
        ErrorCode::FixFailed,
        format!("Could not start `{command}`: {why}. Run it yourself in a terminal."),
    )
}

/// A fix command that ran and did not work; `detail` says how.
pub fn fix_failed(detail: &str) -> Failure {
    fail(ErrorCode::FixFailed, detail)
}

pub fn config_invalid(why: &str) -> Failure {
    fail(ErrorCode::BadRequest, why)
}

pub fn config_unwritable(path: &std::path::Path, why: &std::io::Error) -> Failure {
    fail(
        ErrorCode::Internal,
        format!("Could not write {}: {why}.", path.display()),
    )
}

pub fn unknown_config_key(key: &str) -> Failure {
    fail(
        ErrorCode::BadRequest,
        format!("There is no config key `{key}`."),
    )
}

pub fn nmcli_failed(why: &std::io::Error) -> Failure {
    fail(ErrorCode::Internal, format!("Could not run nmcli: {why}."))
}

/// The title and body of the Portal notification.
pub fn login_required(network: &str) -> (String, &'static str) {
    (
        format!("Login required for {network}"),
        "This Network wants you to sign in before it gives internet access.",
    )
}
