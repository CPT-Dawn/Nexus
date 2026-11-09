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

/// One event line, pushed to subscribers (no op sends them yet).
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
    let op = obj.get("op").and_then(Value::as_str).unwrap_or("?").to_owned();
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
        let path = socket_path().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set")
        })?;
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
