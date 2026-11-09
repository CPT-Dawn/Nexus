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

pub fn forgot(ssid: &str) -> String {
    format!("Forgot {ssid}.")
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
