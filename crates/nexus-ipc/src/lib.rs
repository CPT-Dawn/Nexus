#![doc = include_str!("../README.md")]

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

mod doctor;
mod eap;
mod ethernet;
mod inventory;
pub use doctor::{DoctorFixed, DoctorReport, Finding, Fix, Severity};
pub use eap::{Eap, EapError, EapField, EapMethod, Phase2};
pub use ethernet::{
    EthernetDropped, EthernetJoined, EthernetList, EthernetPort, EthernetProblem, EthernetProfile,
    EthernetState, EthernetTrouble, speed_text, unplugged_message,
};

pub use inventory::{DeviceEntry, DeviceKind, DeviceList, ProfileEntry, ProfileKind, ProfileList};
pub mod profile;
pub use profile::{
    AddressEntry, Field, FieldError, Ipv4, Ipv4Method, Ipv6, Ipv6Method, MacMode, PendingChange,
    ProfileApplied, ProfileKey, ProfileSettings, ProfileView, Settled,
};

/// The protocol version this crate speaks.
pub const PROTOCOL: u32 = 1;
/// Versions a daemon built from this crate accepts.
pub const SUPPORTED: &[u32] = &[1];

/// `$XDG_RUNTIME_DIR/nexus/nexus.sock`, or `None` when the runtime dir is unset.
pub fn socket_path() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty())?;
    Some(PathBuf::from(dir).join("nexus").join("nexus.sock"))
}

/// One request line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub v: u32,
    pub id: u64,
    #[serde(flatten)]
    pub op: Op,
}

/// What a request asks for.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Op {
    /// Overall network state and the primary Link.
    Status,
    /// Nearby Wi-Fi Networks, one per SSID, plus saved out-of-range ones.
    WifiList,
    /// Ask the Backend to refresh the Wi-Fi list. Results arrive as
    /// `networks_changed` events; the response only says what was done.
    WifiScan,
    /// Download and upload rates of the primary Link, in bytes per second.
    /// Zero without a Link; the first call only sets a baseline.
    Traffic,
    /// The recent activity log, newest first.
    Activity,
    /// Join a Wi-Fi Network, waiting until it is up or has failed. `network`
    /// is a `WifiNetwork.id`. A Profile is saved only if the join succeeds.
    /// Result: [`Joined`]. Failures carry a human sentence and one of the
    /// codes `wrong_password`, `out_of_range`, `no_address`, `join_failed`,
    /// `password_required`.
    WifiJoin {
        network: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<Secret>,
        /// Join through this Wi-Fi Device (`WifiDevice.id` or its interface
        /// name) instead of the one that hears the Network best.
        /// `unknown_device` when there is none by that name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
        /// Join a hidden Network: one that does not broadcast its name, so it
        /// is not in the list. `network` is still its SSID bytes in hex; the
        /// security says how to authenticate (`open`, `owe`, `wpa`, `wpa2`,
        /// `wpa3`, `wpa3_transition`). `invalid_settings` for anything else.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hidden: Option<Security>,
    },
    /// The access points behind a Network (`WifiNetwork.id`): address,
    /// channel, frequency, band and link rate of each. Result: [`ApList`];
    /// `out_of_range` when none is heard.
    WifiAps { network: String },
    /// Join a WPA-Enterprise (802.1X) Network such as eduroam, with the
    /// settings in `eap`. Like `wifi_join` otherwise (result [`Joined`]);
    /// `invalid_settings` when `eap` fails its validation, `wrong_password`
    /// when the server refuses the sign-in.
    WifiJoinEap { network: String, eap: Eap },
    /// Drop the current Wi-Fi Link, keeping its Profile. Result: [`Disconnected`];
    /// `not_connected` when there is nothing to drop.
    WifiDisconnect,
    /// Forget (delete the Profile of) a Wi-Fi Network by `WifiNetwork.id`.
    /// Result: [`Forgotten`]; `unknown_network` when nothing is saved for it.
    WifiForget { network: String },
    /// Turn automatic joining of a saved Network on or off, by
    /// `WifiNetwork.id`. Result: [`AutoconnectSet`]; `unknown_network` when
    /// nothing is saved for it.
    WifiAutoconnect { network: String, enabled: bool },
    /// Marks a nearby or joined Wi-Fi Network (`WifiNetwork.id`) Trusted or
    /// Untrusted. Result: [`TrustSet`]; `unknown_network` when it is not in
    /// the Wi-Fi list.
    TrustSet { network: String, trust: Trust },
    /// Reveal what a phone needs to join a saved Network, by `WifiNetwork.id`:
    /// reads the stored password, which needs the Backend's authorization
    /// (NetworkManager asks polkit). Result: [`WifiShare`]; `unknown_network`
    /// when nothing is saved for it, `reveal_denied` when authorization was
    /// refused. Front ends call it only after the person pressed a reveal key.
    WifiReveal { network: String },
    /// Paired phones that offer network access over Bluetooth, and whether
    /// Bluetooth is on. Result: [`Tethers`].
    Tethers,
    /// Tether through a phone, waiting until the Link is up or has failed.
    /// `phone` is its Bluetooth address or name; it may be left out when only
    /// one phone is paired. Result: [`Tethered`]; `bluetooth_off`, `no_phone`,
    /// `unknown_phone` or `join_failed` on failure.
    TetherConnect {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phone: Option<String>,
    },
    /// Drop the Tethering Link, keeping the phone paired. Result:
    /// [`Untethered`]; `not_connected` when nothing is tethered.
    TetherDisconnect,
    /// Whether this machine can host a Hotspot, and the Hotspot that runs.
    /// Result: [`Hotspot`].
    Hotspot,
    /// Start a Hotspot sharing the internet connection, waiting until it
    /// is up or has failed. Without `ssid` and `password` Nexus makes both
    /// up (a strong password). Result: [`HotspotStarted`]; `hotspot_unsupported`
    /// (the card or the radio cannot host one), `join_failed`,
    /// `invalid_settings` (the name or password is not usable).
    HotspotStart {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ssid: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<Secret>,
    },
    /// Stop the Hotspot and forget its Profile. Result: [`HotspotStopped`];
    /// `not_connected` when none runs.
    HotspotStop,
    /// The join details of the running Hotspot, for a QR code. Result:
    /// [`WifiShare`]; `not_connected` when none runs, `reveal_denied` when
    /// authorization was refused. Front ends call it only after the person
    /// pressed a reveal key.
    HotspotReveal,
    /// Answer an open Secret prompt (`prompt` is the id from its `secret_prompt_opened`
    /// event). The only request besides `wifi_join` that carries a secret.
    /// Result `{}`; `unknown_prompt` when it is closed already.
    SecretAnswer { prompt: u64, secret: Secret },
    /// Decline an open Secret prompt. The join waiting on it fails with
    /// `password_required`. Result `{}`; `unknown_prompt` when closed already.
    SecretCancel { prompt: u64 },
    /// Run the Doctor's checks. Result: [`DoctorReport`], in check order
    /// (sort by `severity` to lead with the worst). Contacts nothing outside
    /// beyond the configured connectivity check's host name lookup.
    Doctor,
    /// Run the fix of one finding (`finding` is a `Finding.id`) through
    /// `pkexec` when it needs privileges. Refused with `not_confirmed` unless
    /// `confirm` is true: a Front end sets it only after the person said yes.
    /// Result: [`DoctorFixed`]; `unknown_finding` when the check has nothing to
    /// fix any more; `fix_failed` when a command failed (the message says which).
    DoctorFix {
        finding: String,
        #[serde(default)]
        confirm: bool,
    },
    /// Read the settings of a saved Profile: a Wi-Fi Network's (`network` is a
    /// `WifiNetwork.id`) or, with `profile` (a Profile uuid, e.g.
    /// `EthernetProfile.uuid`), any Ethernet Profile. `profile` wins when both
    /// are given. Result: [`ProfileView`]; `unknown_network` when nothing is
    /// saved for it.
    ProfileGet {
        #[serde(default)]
        network: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile: Option<String>,
    },
    /// Change the settings of a saved Profile. `version` is the one
    /// `profile_get` returned: a Profile that changed since fails with
    /// `conflict`. Invalid settings fail with `invalid_settings` before
    /// anything is written. When the Profile carries the current Link the
    /// change is checkpointed: the result has `pending`, and unless
    /// `profile_confirm` follows within `rollback_secs` (default 15) the
    /// Backend puts the old settings back. Result: [`ProfileApplied`].
    ProfileUpdate {
        #[serde(default)]
        network: String,
        /// A Profile uuid instead of a Wi-Fi `network` (see `profile_get`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile: Option<String>,
        version: u64,
        settings: ProfileSettings,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rollback_secs: Option<u32>,
    },
    /// Every saved Profile, Wi-Fi (nearby or not) and Ethernet, in one list.
    /// Result: [`ProfileList`].
    ProfileList,
    /// Every Device: the Wi-Fi cards, then the Ethernet ports.
    /// Result: [`DeviceList`].
    DeviceList,
    /// Keep a pending change (`pending` is `PendingChange.id`). Result `{}`;
    /// `nothing_pending` when it was confirmed, reverted or timed out already.
    ProfileConfirm { pending: u64 },
    /// Revert a pending change now instead of waiting. Result `{}`;
    /// `nothing_pending` as for `profile_confirm`.
    ProfileRevert { pending: u64 },
    /// The Ethernet ports, their Link details and why a plugged-in one does
    /// not work. Result: [`EthernetList`].
    EthernetList,
    /// Bring an Ethernet port up with its saved Profile (a plain DHCP Profile
    /// is made and saved when it has none), waiting until it is up or has
    /// failed. `port` may be left out when the machine has one port. Result:
    /// [`EthernetJoined`]; `unplugged`, `unknown_port`, `wrong_password` (the
    /// port refused the 802.1X sign-in), `no_address`, `join_failed`.
    EthernetConnect {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<String>,
    },
    /// Drop an Ethernet Link, keeping its Profile. Result: [`EthernetDropped`];
    /// `not_connected`, `unknown_port`.
    EthernetDisconnect {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<String>,
    },
    /// Sign in to an 802.1X (wired Enterprise Network) port with the settings
    /// in `eap`: the port's Profile gets them (one is made when there is none)
    /// and is brought up. Like `ethernet_connect` otherwise;
    /// `invalid_settings` when `eap` fails its validation.
    EthernetJoinEap {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<String>,
        eap: Eap,
    },
    /// Switch Wi-Fi on or off. Result: [`Radios`]. `hard_blocked` while a
    /// hardware switch holds the radio off.
    WifiPower { on: bool },
    /// Airplane mode: every radio the Backend controls off (`on`) or back on.
    /// Result: [`Radios`]. `hard_blocked` when a hardware switch prevents it.
    Airplane { on: bool },
    /// The VPN Profiles and their state. Result: [`VpnList`].
    VpnList,
    /// Bring a VPN up, waiting until it is up or has failed (`vpn` is a
    /// `Vpn.id`). Result: [`VpnToggled`]; `unknown_vpn`, `vpn_failed`.
    VpnUp { vpn: String },
    /// Bring a VPN down. Result: [`VpnToggled`]; `unknown_vpn`, `not_connected`
    /// when it is not up.
    VpnDown { vpn: String },
    /// Import a WireGuard `.conf` (its text, private key included) as a VPN
    /// Profile. Result: [`VpnAdded`]; `invalid_config` with the line at fault.
    VpnImport {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        config: Secret,
        /// A provider file instead of WireGuard: the NetworkManager plugin
        /// that reads it (`openvpn`, `vpnc`, ...). Imported by `nmcli`;
        /// `missing_plugin` when its package is not installed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plugin: Option<String>,
    },
    /// Create a WireGuard VPN Profile from scratch with a fresh key pair.
    /// Result: [`VpnAdded`] (carries the public key); `invalid_config`.
    VpnCreate(NewTunnel),
    /// The public key of a WireGuard VPN Profile. Result: [`VpnKey`].
    VpnKey { vpn: String },
    /// Reveal a WireGuard Profile's whole config, private key included, so it
    /// can be shown as a QR code. Needs the Backend's authorization. Result:
    /// [`VpnShare`]; `reveal_denied` when refused. Front ends call it only after
    /// the person pressed a reveal key.
    VpnReveal { vpn: String },
    /// The Portal the Link is behind, if any. Result: [`PortalState`].
    Portal,
    /// Open the Portal's login page in the user's browser (the Daemon runs
    /// `xdg-open`). Result: [`PortalOpened`]; `no_portal` when no Network is
    /// asking for a login right now; `open_failed` when no browser started.
    PortalOpen,
    /// Start receiving event lines on this connection. Answered with
    /// `{"ok": {}}`; events follow, interleaved with later responses.
    Subscribe,
    /// Read the effective config, or one value by dotted key (`theme`,
    /// `connectivity.url`). The result is `{"value": ..., "warning"?: ...}`;
    /// `warning` is present while the file on disk is being ignored.
    ConfigGet {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
    },
    /// Set one value by dotted key, editing `config.toml` in place so the
    /// user's comments and ordering survive. The result is as for `config_get`.
    ConfigSet { key: String, value: Value },
}

/// A password or other secret. It serializes as the plain string (the wire
/// needs it), never shows in `Debug` output, and wipes its memory on drop.
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(zeroize::Zeroizing<String>);

impl Secret {
    pub fn new(text: impl Into<String>) -> Self {
        Self(zeroize::Zeroizing::new(text.into()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        self.expose() == other.expose()
    }
}

impl Eq for Secret {}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// The `WifiNetwork.id` of an SSID: its bytes in lowercase hex.
pub fn network_id(ssid: &[u8]) -> String {
    use std::fmt::Write as _;
    ssid.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// An SSID as text a terminal can safely show. SSIDs are up to 32 arbitrary
/// bytes: invalid UTF-8 and every control or direction-override character is
/// replaced with U+FFFD, so a hostile name can neither drive the terminal nor
/// reorder the text around it. The bytes stay joinable by `WifiNetwork.id`.
pub fn display_ssid(ssid: &[u8]) -> String {
    String::from_utf8_lossy(ssid)
        .chars()
        .map(|c| {
            let bidi = matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
            if c.is_control() || bidi { '\u{fffd}' } else { c }
        })
        .collect()
}

/// The SSID bytes behind a `WifiNetwork.id`, if it is one.
pub fn parse_network_id(id: &str) -> Option<Vec<u8>> {
    if !id.len().is_multiple_of(2) || !id.is_ascii() {
        return None;
    }
    (0..id.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&id[i..i + 2], 16).ok())
        .collect()
}

/// One response line: `ok` or `err`, never both.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub v: u32,
    /// Absent when the request was too broken to carry an id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok(Value),
    Err(Failure),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Failure {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCode {
    /// The line is not JSON, or not a request this daemon can read (a missing
    /// or wrongly typed field). The message says what.
    BadRequest,
    /// The request's `v` is not one the daemon speaks; the message lists them.
    UnsupportedVersion,
    /// No op has that name.
    UnknownOp,
    /// The daemon failed in a way that is not the request's fault (a
    /// NetworkManager read that errored, say); the message says what.
    Internal,
    /// The Backend (NetworkManager) is not running or not reachable.
    BackendUnavailable,
    /// The password was rejected.
    WrongPassword,
    /// The Network is not (or no longer) in range.
    OutOfRange,
    /// Joined the Network but never got an address.
    NoAddress,
    /// The join failed for another reason; the message says which.
    JoinFailed,
    /// The Network needs a password and none was given.
    PasswordRequired,
    /// No Network by that name is saved or nearby.
    UnknownNetwork,
    /// There is nothing to disconnect or stop: no Wi-Fi, Ethernet or tethering Link,
    /// no running Hotspot, or a VPN that is not up.
    NotConnected,
    /// A hardware switch (not software) holds the radio off.
    HardBlocked,
    /// No Secret prompt with that id is open (it was answered, cancelled or expired).
    UnknownPrompt,
    /// A fix needs `confirm: true` and did not get it.
    NotConfirmed,
    /// No finding with that id has a fix to run.
    UnknownFinding,
    /// A fix command failed or was not authorized.
    FixFailed,
    /// Bluetooth is switched off, or this machine has none.
    BluetoothOff,
    /// No paired phone offers network access.
    NoPhone,
    /// No paired phone matches the name given, or several do and none was named.
    UnknownPhone,
    /// No Network is asking for a login right now.
    NoPortal,
    /// The browser could not be started.
    OpenFailed,
    /// Settings are not valid (Profile settings, 802.1X sign-in); the message says which and why.
    InvalidSettings,
    /// The Profile changed since it was read (or a change still waits for
    /// confirmation); read it again.
    Conflict,
    /// No change is waiting for confirmation under that id.
    NothingPending,
    /// The Backend refused to hand over a stored secret (authorization denied).
    RevealDenied,
    /// No VPN Profile has that id.
    UnknownVpn,
    /// A WireGuard config could not be read; the message names the line.
    InvalidConfig,
    /// The VPN did not come up; the message says why.
    VpnFailed,
    /// No cable is plugged into the Ethernet port.
    Unplugged,
    /// No Ethernet port has that name (or several exist and none was named).
    UnknownPort,
    /// A plugin VPN's NetworkManager plugin is not installed; the message
    /// names the package.
    MissingPlugin,
    /// No Secret Service (keyring) answers, or it is locked or refused: a
    /// Profile's secrets cannot be moved into or out of it.
    KeyringUnavailable,
    /// No Wi-Fi Device has that name or id.
    UnknownDevice,
    /// This machine's Wi-Fi cannot host a Hotspot; the message says why.
    HotspotUnsupported,
    #[serde(other)]
    Unknown,
}

impl ErrorCode {
    /// Every code this version of the crate knows, in documentation order
    /// (`Unknown` is what an unrecognised code from a newer daemon reads as,
    /// so it is not listed).
    pub const ALL: &'static [ErrorCode] = &[
        ErrorCode::BadRequest,
        ErrorCode::UnsupportedVersion,
        ErrorCode::UnknownOp,
        ErrorCode::Internal,
        ErrorCode::BackendUnavailable,
        ErrorCode::WrongPassword,
        ErrorCode::OutOfRange,
        ErrorCode::NoAddress,
        ErrorCode::JoinFailed,
        ErrorCode::PasswordRequired,
        ErrorCode::UnknownNetwork,
        ErrorCode::NotConnected,
        ErrorCode::HardBlocked,
        ErrorCode::UnknownPrompt,
        ErrorCode::NotConfirmed,
        ErrorCode::UnknownFinding,
        ErrorCode::FixFailed,
        ErrorCode::BluetoothOff,
        ErrorCode::NoPhone,
        ErrorCode::UnknownPhone,
        ErrorCode::NoPortal,
        ErrorCode::OpenFailed,
        ErrorCode::InvalidSettings,
        ErrorCode::Conflict,
        ErrorCode::NothingPending,
        ErrorCode::RevealDenied,
        ErrorCode::UnknownVpn,
        ErrorCode::InvalidConfig,
        ErrorCode::VpnFailed,
        ErrorCode::Unplugged,
        ErrorCode::UnknownPort,
        ErrorCode::MissingPlugin,
        ErrorCode::KeyringUnavailable,
        ErrorCode::UnknownDevice,
        ErrorCode::HotspotUnsupported,
    ];

    /// The code as it appears on the wire (`wrong_password`).
    pub fn name(self) -> &'static str {
        match self {
            ErrorCode::BadRequest => "bad_request",
            ErrorCode::UnsupportedVersion => "unsupported_version",
            ErrorCode::UnknownOp => "unknown_op",
            ErrorCode::Internal => "internal",
            ErrorCode::BackendUnavailable => "backend_unavailable",
            ErrorCode::WrongPassword => "wrong_password",
            ErrorCode::OutOfRange => "out_of_range",
            ErrorCode::NoAddress => "no_address",
            ErrorCode::JoinFailed => "join_failed",
            ErrorCode::PasswordRequired => "password_required",
            ErrorCode::UnknownNetwork => "unknown_network",
            ErrorCode::NotConnected => "not_connected",
            ErrorCode::HardBlocked => "hard_blocked",
            ErrorCode::UnknownPrompt => "unknown_prompt",
            ErrorCode::NotConfirmed => "not_confirmed",
            ErrorCode::UnknownFinding => "unknown_finding",
            ErrorCode::FixFailed => "fix_failed",
            ErrorCode::BluetoothOff => "bluetooth_off",
            ErrorCode::NoPhone => "no_phone",
            ErrorCode::UnknownPhone => "unknown_phone",
            ErrorCode::NoPortal => "no_portal",
            ErrorCode::OpenFailed => "open_failed",
            ErrorCode::InvalidSettings => "invalid_settings",
            ErrorCode::Conflict => "conflict",
            ErrorCode::NothingPending => "nothing_pending",
            ErrorCode::RevealDenied => "reveal_denied",
            ErrorCode::UnknownVpn => "unknown_vpn",
            ErrorCode::InvalidConfig => "invalid_config",
            ErrorCode::VpnFailed => "vpn_failed",
            ErrorCode::Unplugged => "unplugged",
            ErrorCode::UnknownPort => "unknown_port",
            ErrorCode::MissingPlugin => "missing_plugin",
            ErrorCode::KeyringUnavailable => "keyring_unavailable",
            ErrorCode::UnknownDevice => "unknown_device",
            ErrorCode::HotspotUnsupported => "hotspot_unsupported",
            ErrorCode::Unknown => "unknown",
        }
    }
}

impl Response {
    pub fn ok(id: u64, value: Value) -> Self {
        Self {
            v: PROTOCOL,
            id: Some(id),
            outcome: Outcome::Ok(value),
        }
    }

    pub fn err(id: Option<u64>, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL,
            id,
            outcome: Outcome::Err(Failure {
                code,
                message: message.into(),
            }),
        }
    }
}

/// One event line, pushed to subscribers after `subscribe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventLine {
    pub v: u32,
    #[serde(flatten)]
    pub event: Event,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Event {
    /// The overall state or the primary Link changed. Carries the whole new
    /// status (what `status` would answer).
    StatusChanged { status: Status },
    /// The Wi-Fi list changed. Carries the whole new list; a burst of
    /// Backend changes yields one event per Daemon tick.
    NetworksChanged { wifi: WifiList },
    /// Something worth a line in the activity strip just happened.
    Activity { entry: ActivityEntry },
    /// `config.toml` changed (or was fixed, or broke). `config` is what is now
    /// in force; `warning` says why the file is being ignored, if it is.
    ConfigReloaded {
        config: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        warning: Option<String>,
    },
    /// The Backend needs a secret from the user. Prompts still open when a
    /// Front end subscribes are replayed to it, so one that was not running
    /// when the prompt opened still sees it.
    SecretPromptOpened { prompt: Prompt },
    /// A Secret prompt is gone: answer it no more.
    SecretPromptClosed { id: u64, reason: PromptClosed },
    /// The Doctor's findings changed since it last ran (any Front end asked it
    /// to run). Carries all of them.
    DoctorFindings { findings: Vec<Finding> },
    /// The VPN list changed (a Profile added, a VPN up or down).
    VpnsChanged { vpns: VpnList },
    /// The Tethering picture changed (Bluetooth switched, a phone paired,
    /// tethered or dropped). Carries the whole new picture.
    TethersChanged { tethers: Tethers },
    /// The Hotspot started, stopped or its device count changed.
    HotspotChanged { hotspot: Hotspot },
    /// The Network is now behind a Portal (`Some`), or no longer is (`None`:
    /// signed in, or the Link went away).
    PortalChanged { portal: Option<Portal> },
    /// The Ethernet picture changed (a cable plugged or pulled, a port joined,
    /// dropped or failed). Carries the whole new picture.
    EthernetChanged { ethernet: EthernetList },
    /// A change to the Profile carrying the Link waits for confirmation:
    /// it rolls back in `rollback_secs` unless somebody confirms it.
    ProfilePending { change: PendingChange },
    /// A pending change was confirmed or rolled back.
    ProfileSettled {
        id: u64,
        ssid: String,
        outcome: Settled,
    },
    /// An event from a newer daemon; its payload is discarded.
    #[serde(other)]
    Unknown,
}

/// A Network that wants a login before it gives real internet access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Portal {
    /// The Network's name, as the user knows it.
    pub network: String,
    /// The plain `http://` page to open: the connectivity check URL, which
    /// the Portal redirects to its login page.
    pub url: String,
}

/// The `portal` result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalState {
    pub portal: Option<Portal>,
}

/// The `portal_open` result: what was opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalOpened {
    pub url: String,
}

/// The `traffic` result: the primary Link's rates in bytes per second.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Traffic {
    pub rx_bps: u64,
    pub tx_bps: u64,
}

/// What kind of thing an [`ActivityEntry`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActivityKind {
    Joined,
    Lost,
    VpnUp,
    VpnDown,
    /// A Trust policy did something on joining an Untrusted Network.
    Policy,
    /// A Portal was detected on the Link.
    Portal,
    /// Internet is reachable again (after a Portal login).
    Online,
    #[serde(other)]
    Other,
}

/// One line of the activity log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEntry {
    /// Unix seconds.
    pub at: u64,
    pub kind: ActivityKind,
    /// Plain English, ready to show.
    pub text: String,
}

/// The `activity` result, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityLog {
    pub entries: Vec<ActivityEntry>,
}

/// What a Secret prompt asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PromptKind {
    /// The password of a Wi-Fi Network.
    WifiPassword,
    /// A secret a plugin VPN asks for; [`Prompt::field`] says which.
    VpnSecret,
    #[serde(other)]
    Unknown,
}

/// What a [`PromptKind::VpnSecret`] prompt wants typed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PromptField {
    #[default]
    Password,
    /// A user name: not secret, so a Front end may show it as typed.
    Username,
    /// A one-time code.
    Otp,
    #[serde(other)]
    Unknown,
}

/// An open Secret prompt. Never holds a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prompt {
    pub id: u64,
    pub kind: PromptKind,
    /// What is asked for (always `password` for Wi-Fi).
    #[serde(default)]
    pub field: PromptField,
    /// The Network's name, or the VPN's.
    pub network: String,
    /// `WifiNetwork.id` of the Network, or the VPN's Profile id.
    pub network_id: String,
    /// The password was tried and rejected: this asks again.
    pub retry: bool,
    /// A sentence for the person, e.g. "Wrong password for Home. Try again."
    pub message: String,
    /// Seconds until the prompt expires.
    pub expires_in: u64,
}

/// Why a Secret prompt closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PromptClosed {
    Answered,
    Cancelled,
    /// Nobody answered in time.
    Expired,
    /// The Backend no longer needs it (the join was abandoned).
    Withdrawn,
    #[serde(other)]
    Unknown,
}

/// The overall network state, as the user would phrase it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum State {
    /// The Backend is not running or not reachable.
    Unavailable,
    /// Networking is switched off.
    Asleep,
    Offline,
    Disconnecting,
    Connecting,
    /// Connected, but only to the local network.
    Local,
    /// Connected to a site network, no confirmed internet.
    Site,
    Online,
    #[serde(other)]
    Unknown,
}

/// The Backend's last internet reachability probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Connectivity {
    None,
    Portal,
    Limited,
    Full,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LinkKind {
    Wifi,
    Ethernet,
    Vpn,
    Wireguard,
    #[serde(other)]
    Other,
}

/// The primary Link: the joined Network the machine routes through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub name: String,
    pub kind: LinkKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// The `status` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub state: State,
    pub connectivity: Connectivity,
    /// The primary Link, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Link>,
    pub backend: BackendInfo,
    /// Radio switches. Absent from older daemons: reads as "all on".
    #[serde(default)]
    pub radios: Radios,
}

/// The state of one radio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Radio {
    /// On, and not blocked by anything.
    pub enabled: bool,
    /// A hardware switch (not software) holds it off: asking to turn it on cannot help.
    pub hardware_blocked: bool,
}

/// All radio switches, the `wifi_power` and `airplane` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Radios {
    pub wifi: Radio,
    /// Every radio the Backend controls is off.
    pub airplane: bool,
}

impl Default for Radios {
    fn default() -> Self {
        Self {
            wifi: Radio {
                enabled: true,
                hardware_blocked: false,
            },
            airplane: false,
        }
    }
}

impl Radios {
    /// A phrase about a radio that is off, or `None` when all is well.
    pub fn human(&self) -> Option<&'static str> {
        if self.wifi.hardware_blocked {
            Some("Wi-Fi is blocked by a hardware switch")
        } else if self.airplane {
            Some("airplane mode is on")
        } else if !self.wifi.enabled {
            Some("Wi-Fi is off")
        } else {
            None
        }
    }
}

impl Status {
    /// Status when the Backend cannot be reached.
    pub fn unavailable() -> Self {
        Self {
            state: State::Unavailable,
            connectivity: Connectivity::Unknown,
            link: None,
            backend: BackendInfo {
                name: "networkmanager".into(),
                version: None,
            },
            radios: Radios::default(),
        }
    }

    /// One human line, as `nexus status` prints it.
    pub fn human(&self) -> String {
        let how = match self.connectivity {
            Connectivity::Portal => " (sign-in page required)",
            Connectivity::Limited => " (no internet)",
            _ => "",
        };
        let link = |prefix: &str| match &self.link {
            Some(l) => format!("{prefix}{}", l.name),
            None => String::new(),
        };
        let base = self.human_state(link, how);
        match self.radios.human() {
            Some(radio) if self.state != State::Unavailable => format!("{base}; {radio}"),
            _ => base,
        }
    }

    fn human_state(&self, link: impl Fn(&str) -> String, how: &str) -> String {
        match self.state {
            State::Unavailable => "NetworkManager is not running".into(),
            State::Asleep => "networking is switched off".into(),
            State::Offline => "offline".into(),
            State::Disconnecting => "disconnecting".into(),
            State::Connecting => format!("connecting{}", link(" to ")),
            State::Local | State::Site => format!("connected{}, no internet{how}", link(" to ")),
            State::Online => format!("online{}{how}", link(" via ")),
            State::Unknown => "state unknown".into(),
        }
    }
}

/// How well a Network is protected, as the user would phrase it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Security {
    Open,
    /// Enhanced Open: encrypted without a password.
    Owe,
    Wep,
    Wpa,
    Wpa2,
    Wpa3,
    /// Accepts both WPA2 and WPA3 clients.
    Wpa3Transition,
    Enterprise,
    #[serde(other)]
    Unknown,
}

impl Security {
    /// Short label for tables.
    pub fn label(self) -> &'static str {
        match self {
            Security::Open => "open",
            Security::Owe => "owe",
            Security::Wep => "wep",
            Security::Wpa => "wpa",
            Security::Wpa2 => "wpa2",
            Security::Wpa3 => "wpa3",
            Security::Wpa3Transition => "wpa2/3",
            Security::Enterprise => "enterprise",
            Security::Unknown => "?",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Band {
    #[serde(rename = "2.4")]
    Ghz2_4,
    #[serde(rename = "5")]
    Ghz5,
    #[serde(rename = "6")]
    Ghz6,
    #[serde(other)]
    Unknown,
}

impl Band {
    pub fn label(self) -> &'static str {
        match self {
            Band::Ghz2_4 => "2.4 GHz",
            Band::Ghz5 => "5 GHz",
            Band::Ghz6 => "6 GHz",
            Band::Unknown => "?",
        }
    }
}

/// A Wi-Fi Device (radio).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiDevice {
    /// The Backend's stable key for the Device (its object path on NetworkManager).
    pub id: String,
    /// The interface name, e.g. `wlan0`.
    pub name: String,
}

/// One nearby Wi-Fi Network: every access point that broadcasts the same
/// SSID bytes, folded into the strongest one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiNetwork {
    /// SSID bytes in lowercase hex. This is the key: SSIDs need not be UTF-8.
    pub id: String,
    /// The SSID for display, with invalid UTF-8 replaced.
    pub ssid: String,
    /// 0 to 100, from the strongest access point.
    pub signal: u8,
    pub security: Security,
    pub band: Band,
    /// How many access points broadcast it.
    pub access_points: u32,
    /// True while a Device is associated with it.
    #[serde(default)]
    pub joined: bool,
    /// The Device that hears the strongest access point.
    pub device: String,
    /// UUID of the saved Profile for it, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Whether the saved Profile joins by itself; `None` without a Profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoconnect: Option<bool>,
    /// The user's judgement of it. The Backend leaves this at the default;
    /// the Daemon fills it in from its marks.
    #[serde(default)]
    pub trust: Trust,
}

/// The user's judgement of a Network. A Network nobody marked is Untrusted if
/// it is open, Trusted otherwise.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Trust {
    #[default]
    Trusted,
    Untrusted,
    /// A judgement from a newer daemon. Never sent; a Daemon rejects it in a
    /// request.
    #[serde(other)]
    Unknown,
}

impl Trust {
    pub fn label(self) -> &'static str {
        match self {
            Trust::Trusted => "Trusted",
            Trust::Untrusted => "Untrusted",
            Trust::Unknown => "Unknown",
        }
    }
}

/// The `trust_set` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustSet {
    pub ssid: String,
    pub trust: Trust,
}

/// One access point behind a Network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApInfo {
    /// Its hardware address, e.g. `AA:BB:CC:00:11:22`.
    pub bssid: String,
    /// The channel number; 0 when the frequency is outside the known bands.
    pub channel: u32,
    pub frequency_mhz: u32,
    pub band: Band,
    /// 0 to 100.
    pub signal: u8,
    /// The best link rate it offers, in Mbit/s; 0 when it does not say.
    pub rate_mbps: u32,
    /// The interface name of the Device that hears it.
    pub device: String,
    /// True while a Device is associated with it.
    #[serde(default)]
    pub joined: bool,
}

/// The `wifi_aps` result: strongest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApList {
    pub ssid: String,
    pub aps: Vec<ApInfo>,
}

/// The channel of a frequency in MHz (IEEE 802.11 numbering); 0 when unknown.
pub fn channel_of(mhz: u32) -> u32 {
    match mhz {
        2484 => 14,
        2412..=2472 => (mhz - 2407) / 5,
        5160..=5885 => (mhz - 5000) / 5,
        // 6 GHz channel 2 is the odd one out at 5935; the rest count from 5955.
        5935 => 2,
        5955..=7115 => (mhz - 5950) / 5,
        _ => 0,
    }
}

/// A saved Wi-Fi Profile whose Network is not in range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWifi {
    pub uuid: String,
    pub id: String,
    pub ssid: String,
    pub autoconnect: bool,
}

/// The `wifi_list` result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiList {
    pub devices: Vec<WifiDevice>,
    /// Strongest first; ties by SSID.
    pub networks: Vec<WifiNetwork>,
    /// Saved Profiles that match no nearby Network.
    pub saved: Vec<SavedWifi>,
}

/// The `wifi_scan` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanOutcome {
    /// How many Devices were asked to scan.
    pub requested: u32,
    /// How many were skipped because they scanned too recently.
    pub skipped: u32,
    /// Age of the freshest scan in milliseconds, if any Device ever scanned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_scan_ms_ago: Option<u64>,
}

/// The `wifi_join` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Joined {
    pub ssid: String,
    /// UUID of the Profile now saved for it.
    pub profile: String,
}

/// The `wifi_disconnect` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disconnected {
    pub ssid: String,
}

/// The `wifi_forget` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forgotten {
    pub ssid: String,
}

/// The `wifi_autoconnect` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoconnectSet {
    pub ssid: String,
    pub enabled: bool,
}

/// How a Network authenticates, in the words of the Wi-Fi QR code format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ShareAuth {
    /// WPA or WPA2 with a passphrase.
    Wpa,
    /// WPA3 with a passphrase.
    Sae,
    Wep,
    /// No password (open, or OWE).
    Open,
    /// An authentication from a newer daemon.
    #[serde(other)]
    Unknown,
}

/// The `wifi_reveal` result: the join details of a saved Network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiShare {
    pub ssid: String,
    pub auth: ShareAuth,
    /// Absent for an open Network.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<Secret>,
}

impl WifiShare {
    /// The text a phone camera reads from the QR code:
    /// `WIFI:T:WPA;S:name;P:pass;;`. Wiped from memory when dropped.
    pub fn payload(&self) -> zeroize::Zeroizing<String> {
        // Backslash, semicolon, comma, colon and quote are escaped. Digit-only
        // values stay unquoted, as Android writes them, so phones read them back.
        fn field(value: &str) -> String {
            let mut out = String::with_capacity(value.len() + 2);
            for c in value.chars() {
                if matches!(c, '\\' | ';' | ',' | ':' | '"') {
                    out.push('\\');
                }
                out.push(c);
            }
            out
        }
        let t = match self.auth {
            ShareAuth::Wpa => "WPA",
            ShareAuth::Sae => "SAE",
            ShareAuth::Wep => "WEP",
            ShareAuth::Open => "nopass",
            // A newer daemon's scheme: a password means the common WPA form.
            ShareAuth::Unknown if self.password.is_some() => "WPA",
            ShareAuth::Unknown => "nopass",
        };
        let mut out = zeroize::Zeroizing::new(format!("WIFI:T:{t};S:{};", field(&self.ssid)));
        if let Some(p) = &self.password {
            out.push_str("P:");
            out.push_str(&field(p.expose()));
            out.push(';');
        }
        out.push(';');
        out
    }
}

/// What kind of VPN a Profile is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum VpnKind {
    Wireguard,
    /// A plugin VPN such as OpenVPN.
    Plugin,
    /// A kind from a newer daemon.
    #[serde(other)]
    Unknown,
}

/// Whether a VPN is carrying traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum VpnState {
    Down,
    Activating,
    Up,
    /// A state from a newer daemon (`deactivating`, say).
    #[serde(other)]
    Unknown,
}

/// One VPN Profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vpn {
    /// The Profile's UUID.
    pub id: String,
    pub name: String,
    pub kind: VpnKind,
    pub state: VpnState,
    /// Addresses of the tunnel's own end, as `10.0.0.2/32`.
    #[serde(default)]
    pub addresses: Vec<String>,
    /// The first peer's endpoint, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

/// The `vpn_list` result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VpnList {
    pub vpns: Vec<Vpn>,
}

/// The `vpn_up` / `vpn_down` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VpnToggled {
    pub id: String,
    pub name: String,
    pub state: VpnState,
}

/// The `vpn_import` / `vpn_create` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VpnAdded {
    pub id: String,
    pub name: String,
    /// This end's public key, to give to the other side. Empty for a plugin VPN.
    #[serde(default)]
    pub public_key: String,
}

/// The `vpn_key` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VpnKey {
    pub name: String,
    pub public_key: String,
}

/// The `vpn_reveal` result: a whole WireGuard config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VpnShare {
    pub name: String,
    pub public_key: String,
    /// The `.conf` text. Wiped from memory when dropped.
    pub config: Secret,
}

fn all_traffic() -> Vec<String> {
    vec!["0.0.0.0/0".into(), "::/0".into()]
}

/// What `vpn_create` needs: this end's addresses and, if known yet, the peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewTunnel {
    pub name: String,
    /// Addresses of this end, e.g. `10.0.0.2/32`.
    pub addresses: Vec<String>,
    #[serde(default)]
    pub dns: Vec<String>,
    /// The peer's public key; a tunnel without one cannot carry traffic yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_key: Option<String>,
    /// `host:port`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// What to send through the tunnel; everything by default.
    #[serde(default = "all_traffic")]
    pub allowed_ips: Vec<String>,
}

/// Whether Bluetooth can carry a Tethering Link right now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Bluetooth {
    /// An adapter is powered.
    On,
    /// Every adapter is switched off (software or rfkill).
    Off,
    /// No adapter, or the Bluetooth service is not running.
    #[default]
    Absent,
    /// A state from a newer daemon; read it as "can't tell".
    #[serde(other)]
    Unknown,
}

/// A paired phone that offers network access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phone {
    /// Its Bluetooth address, upper case (`AA:BB:CC:DD:EE:FF`).
    pub id: String,
    pub name: String,
    /// The Tethering Link is up.
    pub connected: bool,
    /// The Link is being brought up.
    #[serde(default)]
    pub connecting: bool,
    /// A Profile is saved for it, so the next connect needs no setup.
    #[serde(default)]
    pub saved: bool,
}

/// The `tethers` result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tethers {
    pub bluetooth: Bluetooth,
    /// Paired phones that offer network access, by name.
    pub phones: Vec<Phone>,
}

/// The `tether_connect` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tethered {
    pub phone: String,
    pub id: String,
}

/// The `tether_disconnect` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Untethered {
    pub phone: String,
}

/// The `hotspot` result and `hotspot_changed` payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotspot {
    /// This machine can host a Hotspot.
    pub available: bool,
    /// Why not, in a sentence, when `available` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// A Hotspot is up or coming up.
    pub on: bool,
    /// It is coming up and not carrying devices yet.
    #[serde(default)]
    pub starting: bool,
    /// The running Hotspot's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssid: Option<String>,
    /// How many devices are joined; `None` when it cannot be counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devices: Option<u32>,
}

/// The `hotspot_start` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotspotStarted {
    pub ssid: String,
    /// The password in use, generated or given.
    pub password: Secret,
    /// Nexus made the password up.
    pub generated: bool,
}

/// The `hotspot_stop` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotspotStopped {
    pub ssid: String,
}

/// Encodes a value as one line, newline included.
pub fn encode<T: Serialize>(value: &T) -> String {
    let mut line = serde_json::to_string(value).expect("wire types always serialize");
    line.push('\n');
    line
}

/// Server-side validation of one request line. The error is the response to
/// send back. Order: JSON, `v` present, `v` supported, `id`, `op`, then the op.
pub fn parse_request(line: &str) -> Result<Request, Response> {
    let bad = |id, msg: &str| Response::err(id, ErrorCode::BadRequest, msg);
    let value: Value = serde_json::from_str(line).map_err(|e| bad(None, &e.to_string()))?;
    let Some(obj) = value.as_object() else {
        return Err(bad(None, "request must be a JSON object"));
    };
    let id = obj.get("id").and_then(Value::as_u64);
    let Some(v) = obj.get("v").and_then(Value::as_u64) else {
        return Err(bad(None, "missing protocol version `v`"));
    };
    if !SUPPORTED.iter().any(|s| u64::from(*s) == v) {
        return Err(Response::err(
            id,
            ErrorCode::UnsupportedVersion,
            format!("protocol version {v} is not supported (supported: {SUPPORTED:?})"),
        ));
    }
    if id.is_none() {
        return Err(bad(None, "missing request `id`"));
    }
    if !obj.contains_key("op") {
        return Err(bad(id, "missing `op`"));
    }
    let op = obj
        .get("op")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_owned();
    let request = serde_json::from_value::<Request>(value).map_err(|e| {
        let text = e.to_string();
        // Only the op itself can be an unknown op; an unknown value deeper in
        // the request (an `eap.method` of "leap") is a bad request.
        if text.starts_with(&format!("unknown variant `{op}`, expected")) {
            Response::err(id, ErrorCode::UnknownOp, format!("unknown op `{op}`"))
        } else {
            bad(id, &text)
        }
    })?;
    match unknown_value(&request.op) {
        Some(what) => Err(bad(
            id,
            &format!("`{what}` is not a value this daemon knows"),
        )),
        None => Ok(request),
    }
}

/// The wire enums decode a value they do not know as `Unknown`, so that a
/// newer peer's answer still reads. In a *request* that would turn a typo
/// (an `eap.method` of "leap") into a silent choice, so name it here.
fn unknown_value(op: &Op) -> Option<&'static str> {
    let eap = |eap: &Eap| {
        if eap.method == EapMethod::Unknown {
            Some("eap.method")
        } else if eap.phase2 == Some(Phase2::Unknown) {
            Some("eap.phase2")
        } else {
            None
        }
    };
    match op {
        Op::TrustSet {
            trust: Trust::Unknown,
            ..
        } => Some("trust"),
        Op::WifiJoinEap { eap: e, .. } | Op::EthernetJoinEap { eap: e, .. } => eap(e),
        _ => None,
    }
}

/// Blocking client for Front ends that do not need async.
#[derive(Debug)]
pub struct Connection {
    reader: BufReader<UnixStream>,
    next_id: u64,
    /// Half a line read before a timeout.
    partial: String,
    /// Events that arrived while `call` waited for its response.
    parked: std::collections::VecDeque<Event>,
}

impl std::os::fd::AsFd for Connection {
    /// The socket, for waiting on it together with something else.
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        std::os::fd::AsFd::as_fd(self.reader.get_ref())
    }
}

impl Connection {
    /// Connects to the default socket path.
    pub fn connect() -> io::Result<Self> {
        let path = socket_path()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
        Self::connect_to(&path)
    }

    pub fn connect_to(path: &Path) -> io::Result<Self> {
        Ok(Self {
            reader: BufReader::new(UnixStream::connect(path)?),
            next_id: 1,
            partial: String::new(),
            parked: std::collections::VecDeque::new(),
        })
    }

    /// Sends `op` and waits for its response. The outer error is transport,
    /// the inner one is the Daemon's refusal.
    pub fn call(&mut self, op: Op) -> io::Result<Result<Value, Failure>> {
        let id = self.next_id;
        self.next_id += 1;
        let line = encode(&Request {
            v: PROTOCOL,
            id,
            op,
        });
        self.reader.get_mut().write_all(line.as_bytes())?;
        loop {
            let mut buf = String::new();
            if self.reader.read_line(&mut buf)? == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            let Ok(resp) = serde_json::from_str::<Response>(&buf) else {
                // An event line (kept for `next_event`), or noise.
                if let Ok(line) = serde_json::from_str::<EventLine>(&buf) {
                    self.parked.push_back(line.event);
                }
                continue;
            };
            if resp.id != Some(id) {
                continue;
            }
            return Ok(match resp.outcome {
                Outcome::Ok(v) => Ok(v),
                Outcome::Err(f) => Err(f),
            });
        }
    }

    /// Whether `next_event` has something without touching the socket (an
    /// event parked by `call`, or bytes already buffered). A caller that
    /// waits on the socket with `poll` must check this first.
    pub fn has_buffered(&self) -> bool {
        !self.parked.is_empty() || !self.partial.is_empty() || !self.reader.buffer().is_empty()
    }

    /// Waits up to `wait` for the next event line, skipping responses.
    /// `None` means nothing arrived in time. Call `Op::Subscribe` first or
    /// none will come.
    pub fn next_event(&mut self, wait: Duration) -> io::Result<Option<Event>> {
        if let Some(event) = self.parked.pop_front() {
            return Ok(Some(event));
        }
        self.reader.get_ref().set_read_timeout(Some(wait))?;
        let result = loop {
            match self.reader.read_line(&mut self.partial) {
                Ok(0) => break Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(_) if !self.partial.ends_with('\n') => continue,
                Ok(_) => {
                    let line = std::mem::take(&mut self.partial);
                    // A line that is not an event at all is a protocol fault: say
                    // so rather than skipping it (an unknown event or enum value
                    // from a newer daemon decodes fine, as `Unknown`).
                    break match serde_json::from_str::<EventLine>(&line) {
                        Ok(line) => Ok(Some(line.event)),
                        Err(e) => Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("undecodable event line: {e}"),
                        )),
                    };
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    break Ok(None);
                }
                Err(e) => break Err(e),
            }
        };
        self.reader.get_ref().set_read_timeout(None)?;
        result
    }

    /// Convenience for the `wifi_list` op.
    pub fn wifi_list(&mut self) -> io::Result<Result<WifiList, Failure>> {
        Ok(self.call(Op::WifiList)?.and_then(decode))
    }

    /// Convenience for the `wifi_scan` op.
    pub fn wifi_scan(&mut self) -> io::Result<Result<ScanOutcome, Failure>> {
        Ok(self.call(Op::WifiScan)?.and_then(decode))
    }

    /// Convenience for the `tethers` op.
    pub fn tethers(&mut self) -> io::Result<Result<Tethers, Failure>> {
        Ok(self.call(Op::Tethers)?.and_then(decode))
    }

    /// Convenience for the `traffic` op.
    pub fn traffic(&mut self) -> io::Result<Result<Traffic, Failure>> {
        Ok(self.call(Op::Traffic)?.and_then(decode))
    }

    /// Convenience for the `activity` op.
    pub fn activity(&mut self) -> io::Result<Result<ActivityLog, Failure>> {
        Ok(self.call(Op::Activity)?.and_then(decode))
    }

    /// Convenience for the `doctor` op.
    pub fn doctor(&mut self) -> io::Result<Result<DoctorReport, Failure>> {
        Ok(self.call(Op::Doctor)?.and_then(decode))
    }

    /// Convenience for the `status` op.
    pub fn status(&mut self) -> io::Result<Result<Status, Failure>> {
        Ok(self.call(Op::Status)?.and_then(|v| {
            serde_json::from_value(v).map_err(|e| Failure {
                code: ErrorCode::Internal,
                message: format!("undecodable status: {e}"),
            })
        }))
    }
}

fn decode<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, Failure> {
    serde_json::from_value(v).map_err(|e| Failure {
        code: ErrorCode::Internal,
        message: format!("undecodable result: {e}"),
    })
}

#[cfg(test)]
mod wifi_edge_tests {
    use super::*;

    #[test]
    fn hostile_ssids_display_safely() {
        assert_eq!(display_ssid(b"Home"), "Home");
        assert_eq!(display_ssid(b"a\x1b[31mb"), "a\u{fffd}[31mb");
        assert_eq!(display_ssid(&[b'x', 0xff, 0xfe]), "x\u{fffd}\u{fffd}");
        assert_eq!(display_ssid("a\u{202e}b".as_bytes()), "a\u{fffd}b");
        assert_eq!(
            display_ssid("Caf\u{e9} \u{1f4f6}".as_bytes()),
            "Caf\u{e9} \u{1f4f6}"
        );
    }

    #[test]
    fn non_utf8_ssids_round_trip_through_their_id() {
        let ssid = [0xc0, 0x80, b'!', 0xff];
        assert_eq!(
            parse_network_id(&network_id(&ssid)).as_deref(),
            Some(&ssid[..])
        );
    }

    #[test]
    fn channels_follow_ieee_numbering() {
        assert_eq!(channel_of(2412), 1);
        assert_eq!(channel_of(2462), 11);
        assert_eq!(channel_of(2484), 14);
        assert_eq!(channel_of(5180), 36);
        assert_eq!(channel_of(5745), 149);
        assert_eq!(channel_of(5935), 2);
        assert_eq!(channel_of(5955), 1);
        assert_eq!(channel_of(6135), 37);
        assert_eq!(channel_of(100), 0);
    }
}

/// The documented contract (`docs/ipc.md`) names exactly what this crate
/// knows. The examples there are run against a real Daemon by
/// `crates/nexus/tests/ipc_conformance.rs`.
#[cfg(test)]
mod contract_tests {
    use std::collections::BTreeSet;

    use super::*;

    const DOC: &str = include_str!("../../../docs/ipc.md");

    /// Names that head a section: `#### \`name\`` (ops) or `### \`name\``
    /// under `## Events`.
    fn headings(level: &str, within: &str) -> BTreeSet<String> {
        let start = DOC.find(within).unwrap_or_else(|| panic!("no {within}"));
        let section = &DOC[start + within.len()..];
        let end = section.find("\n## ").unwrap_or(section.len());
        let marker = format!("\n{level} `");
        section[..end]
            .match_indices(&marker)
            .map(|(i, _)| {
                let rest = &section[i + marker.len()..];
                rest[..rest.find('`').unwrap()].to_owned()
            })
            .collect()
    }

    /// Every op the crate knows, from the error serde gives for a name that
    /// is none of them (it lists all variants).
    fn op_names() -> BTreeSet<String> {
        let err = serde_json::from_str::<Op>(r#"{"op":"__"}"#)
            .unwrap_err()
            .to_string();
        let list = &err[err.find("expected").expect("serde lists the variants")..];
        list.split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect()
    }

    /// Adding an `Event` variant breaks this match: add it here and to the
    /// list below, then document it.
    fn event_name(event: &Event) -> &'static str {
        match event {
            Event::StatusChanged { .. } => "status_changed",
            Event::NetworksChanged { .. } => "networks_changed",
            Event::Activity { .. } => "activity",
            Event::ConfigReloaded { .. } => "config_reloaded",
            Event::SecretPromptOpened { .. } => "secret_prompt_opened",
            Event::SecretPromptClosed { .. } => "secret_prompt_closed",
            Event::DoctorFindings { .. } => "doctor_findings",
            Event::VpnsChanged { .. } => "vpns_changed",
            Event::TethersChanged { .. } => "tethers_changed",
            Event::HotspotChanged { .. } => "hotspot_changed",
            Event::PortalChanged { .. } => "portal_changed",
            Event::EthernetChanged { .. } => "ethernet_changed",
            Event::ProfilePending { .. } => "profile_pending",
            Event::ProfileSettled { .. } => "profile_settled",
            Event::Unknown => "unknown",
        }
    }

    const EVENTS: &[&str] = &[
        "status_changed",
        "networks_changed",
        "activity",
        "config_reloaded",
        "secret_prompt_opened",
        "secret_prompt_closed",
        "doctor_findings",
        "vpns_changed",
        "tethers_changed",
        "hotspot_changed",
        "portal_changed",
        "ethernet_changed",
        "profile_pending",
        "profile_settled",
    ];

    #[test]
    fn every_op_is_documented_and_nothing_else() {
        let ops = op_names();
        assert!(ops.len() > 40, "{ops:?}");
        assert_eq!(headings("####", "\n## Requests"), ops);
    }

    #[test]
    fn every_event_is_documented_and_nothing_else() {
        let known: BTreeSet<String> = EVENTS.iter().map(|e| (*e).to_owned()).collect();
        assert_eq!(headings("###", "\n## Events"), known);
        // An event line of each name decodes to the variant of that name.
        for name in EVENTS {
            let line = format!(r#"{{"v":1,"event":"{name}"}}"#);
            let decoded = serde_json::from_str::<EventLine>(&line);
            // Payload-less lines may fail to decode; they must not decode
            // as another variant.
            if let Ok(line) = decoded {
                assert_ne!(event_name(&line.event), "unknown", "{name}");
            }
        }
        let newer = serde_json::from_str::<EventLine>(r#"{"v":1,"event":"from_the_future"}"#)
            .expect("an unknown event decodes");
        assert_eq!(event_name(&newer.event), "unknown");
    }

    #[test]
    fn every_error_code_is_documented_and_round_trips() {
        let mut documented = BTreeSet::new();
        for line in DOC.lines() {
            if let Some(rest) = line.strip_prefix("| `")
                && let Some((name, _)) = rest.split_once("` | ")
            {
                documented.insert(name.to_owned());
            }
        }
        let known: BTreeSet<String> = ErrorCode::ALL.iter().map(|c| c.name().to_owned()).collect();
        assert_eq!(documented, known);
        for code in ErrorCode::ALL {
            let wire = serde_json::to_value(code).unwrap();
            assert_eq!(wire, serde_json::json!(code.name()));
            assert_eq!(serde_json::from_value::<ErrorCode>(wire).unwrap(), *code);
        }
        assert_eq!(
            serde_json::from_str::<ErrorCode>("\"from_the_future\"").unwrap(),
            ErrorCode::Unknown
        );
    }
}
