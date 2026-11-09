//! Request, response and event types for the Nexus daemon's JSON-lines
//! socket API (ADR-0002), plus a small blocking client.
//!
//! One JSON object per `\n`-terminated line. Unknown fields are ignored and
//! every enum that can grow ends in an `Unknown` variant, so additive changes
//! keep `v: 1`.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// Join a Wi-Fi Network, waiting until it is up or has failed. `network`
    /// is a `WifiNetwork.id`. A Profile is saved only if the join succeeds.
    /// Result: [`Joined`]. Failures carry a human sentence and one of the
    /// codes `wrong_password`, `out_of_range`, `no_address`, `join_failed`,
    /// `password_required`.
    WifiJoin {
        network: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<Secret>,
    },
    /// Drop the current Wi-Fi Link, keeping its Profile. Result: [`Disconnected`];
    /// `not_connected` when there is nothing to drop.
    WifiDisconnect,
    /// Forget (delete the Profile of) a Wi-Fi Network by `WifiNetwork.id`.
    /// Result: [`Forgotten`]; `unknown_network` when nothing is saved for it.
    WifiForget { network: String },
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
/// needs it) but never shows in `Debug` output, so logs cannot leak it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

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
    BadRequest,
    UnsupportedVersion,
    UnknownOp,
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
    /// There is no Wi-Fi Link to disconnect.
    NotConnected,
    #[serde(other)]
    Unknown,
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
    StatusChanged {
        status: Status,
    },
    /// The Wi-Fi list changed. Carries the whole new list; a burst of
    /// Backend changes yields one event per Daemon tick.
    NetworksChanged {
        wifi: WifiList,
    },
    /// `config.toml` changed (or was fixed, or broke). `config` is what is now
    /// in force; `warning` says why the file is being ignored, if it is.
    ConfigReloaded {
        config: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        warning: Option<String>,
    },
    /// An event from a newer daemon; its payload is discarded.
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
    serde_json::from_value::<Request>(value).map_err(|e| {
        let text = e.to_string();
        if text.contains("unknown variant `") {
            Response::err(id, ErrorCode::UnknownOp, format!("unknown op `{op}`"))
        } else {
            bad(id, &text)
        }
    })
}

/// Blocking client for Front ends that do not need async.
#[derive(Debug)]
pub struct Connection {
    reader: BufReader<UnixStream>,
    next_id: u64,
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
                continue; // an event line, or noise
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

    /// Convenience for the `wifi_list` op.
    pub fn wifi_list(&mut self) -> io::Result<Result<WifiList, Failure>> {
        Ok(self.call(Op::WifiList)?.and_then(decode))
    }

    /// Convenience for the `wifi_scan` op.
    pub fn wifi_scan(&mut self) -> io::Result<Result<ScanOutcome, Failure>> {
        Ok(self.call(Op::WifiScan)?.and_then(decode))
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
