//! The VPN tab's state: the list, the forms (import, create), the public key
//! view and the config QR. Like the Wi-Fi tab, commands run on threads with
//! their own Daemon connection and come back as [`Msg::Vpn`]. The config QR
//! holds the private key, so nothing is read until the person presses the
//! reveal key, and closing the view drops (and wipes) it.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use nexus_ipc::{
    Connection, NewTunnel, Op, Secret, Vpn, VpnAdded, VpnKey, VpnList, VpnShare, VpnState,
    VpnToggled,
};

use super::app::Msg;
use crate::qr::{self, Drawing};

/// What the person asked of the Daemon.
#[derive(Debug, Clone)]
pub enum Cmd {
    Toggle { id: String, name: String, up: bool },
    Import { path: String, name: String },
    Create(NewTunnel),
    Key { id: String, name: String },
    Reveal { id: String, name: String },
}

/// The Daemon's answer to a [`Cmd`], decoded.
#[derive(Debug, Clone)]
pub enum Reply {
    Toggled(VpnToggled),
    Added(VpnAdded),
    Key(VpnKey),
    Share(VpnShare),
}

/// A finished command.
#[derive(Debug, Clone)]
pub struct Done {
    pub cmd: Cmd,
    /// `Err` is the sentence to show.
    pub result: Result<Reply, String>,
}

/// Runs commands on threads of their own: a tunnel can take seconds to come
/// up, and NetworkManager may wait on polkit for a reveal.
#[derive(Debug)]
pub struct Runner {
    socket: PathBuf,
    tx: Sender<Msg>,
}

impl Runner {
    pub fn new(socket: PathBuf, tx: Sender<Msg>) -> Runner {
        Runner { socket, tx }
    }

    pub fn run(&self, cmd: Cmd) {
        let (socket, tx) = (self.socket.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let result = call(&socket, &cmd);
            let _ = tx.send(Msg::Vpn(Box::new(Done { cmd, result })));
        });
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| format!("The daemon sent something unreadable: {e}"))
}

/// `~/x` means `$HOME/x`.
fn expand(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

fn call(socket: &std::path::Path, cmd: &Cmd) -> Result<Reply, String> {
    let op = match cmd {
        Cmd::Toggle { id, up: true, .. } => Op::VpnUp { vpn: id.clone() },
        Cmd::Toggle { id, up: false, .. } => Op::VpnDown { vpn: id.clone() },
        Cmd::Import { path, name } => {
            let file = expand(path);
            let text = std::fs::read_to_string(&file)
                .map(zeroize::Zeroizing::new)
                .map_err(|e| format!("Cannot read {}: {e}.", file.display()))?;
            let name = if name.trim().is_empty() {
                file.file_stem().map(|s| s.to_string_lossy().into_owned())
            } else {
                Some(name.trim().to_owned())
            };
            Op::VpnImport {
                name,
                config: Secret::new(text.as_str()),
                plugin: crate::plugin::for_path(path).map(|p| p.name.to_owned()),
            }
        }
        Cmd::Create(tunnel) => Op::VpnCreate(tunnel.clone()),
        Cmd::Key { id, .. } => Op::VpnKey { vpn: id.clone() },
        Cmd::Reveal { id, .. } => Op::VpnReveal { vpn: id.clone() },
    };
    let mut conn =
        Connection::connect_to(socket).map_err(|e| format!("Could not reach the daemon: {e}"))?;
    let value = conn
        .call(op)
        .map_err(|e| format!("Could not reach the daemon: {e}"))?
        .map_err(|f| f.message)?;
    match cmd {
        Cmd::Toggle { .. } => decode(value).map(Reply::Toggled),
        Cmd::Import { .. } | Cmd::Create(_) => decode(value).map(Reply::Added),
        Cmd::Key { .. } => decode(value).map(Reply::Key),
        Cmd::Reveal { .. } => decode(value).map(Reply::Share),
    }
}

/// One line of a form.
#[derive(Debug, Clone)]
pub struct Field {
    pub label: &'static str,
    pub hint: &'static str,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormKind {
    Import,
    Create,
}

/// A small form: type into the focused line, Tab or Enter moves on, Enter on
/// the last line sends it.
#[derive(Debug, Clone)]
pub struct Form {
    pub kind: FormKind,
    pub fields: Vec<Field>,
    pub focus: usize,
    /// Sent, waiting for the Daemon.
    pub busy: bool,
    /// Why the Daemon (or the form itself) turned it down.
    pub error: Option<String>,
}

impl Form {
    pub fn import() -> Form {
        Form {
            kind: FormKind::Import,
            fields: vec![
                Field {
                    label: "File",
                    hint: "path to a .conf, .ovpn or .pcf",
                    value: String::new(),
                },
                Field {
                    label: "Name",
                    hint: "optional, default is the file name",
                    value: String::new(),
                },
            ],
            focus: 0,
            busy: false,
            error: None,
        }
    }

    pub fn create() -> Form {
        let field = |label, hint| Field {
            label,
            hint,
            value: String::new(),
        };
        Form {
            kind: FormKind::Create,
            fields: vec![
                field("Name", "what to call this VPN"),
                field("Address", "this end, e.g. 10.0.0.2/32"),
                field("DNS", "optional, comma separated"),
                field("Peer key", "optional: the other side's public key"),
                field("Endpoint", "optional: host:port of the other side"),
            ],
            focus: 0,
            busy: false,
            error: None,
        }
    }

    pub fn title(&self) -> &'static str {
        match self.kind {
            FormKind::Import => "Import a VPN config",
            FormKind::Create => "Create a WireGuard VPN",
        }
    }

    fn value(&self, i: usize) -> &str {
        self.fields[i].value.trim()
    }

    /// The command this form sends, or why it cannot yet.
    pub fn command(&self) -> Result<Cmd, String> {
        let list = |s: &str| -> Vec<String> {
            s.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_owned)
                .collect()
        };
        match self.kind {
            FormKind::Import => {
                if self.value(0).is_empty() {
                    return Err("Type the path of the .conf file.".into());
                }
                Ok(Cmd::Import {
                    path: self.value(0).to_owned(),
                    name: self.value(1).to_owned(),
                })
            }
            FormKind::Create => {
                if self.value(0).is_empty() {
                    return Err("Give the VPN a name.".into());
                }
                if self.value(1).is_empty() {
                    return Err("Give this end an address, e.g. 10.0.0.2/32.".into());
                }
                let some = |s: &str| (!s.is_empty()).then(|| s.to_owned());
                Ok(Cmd::Create(NewTunnel {
                    name: self.value(0).to_owned(),
                    addresses: list(self.value(1)),
                    dns: list(self.value(2)),
                    peer_key: some(self.value(3)),
                    endpoint: some(self.value(4)),
                    allowed_ips: vec!["0.0.0.0/0".into(), "::/0".into()],
                }))
            }
        }
    }
}

/// Where the config QR view is.
#[derive(Debug)]
pub enum Stage {
    /// Asking whether to show the QR; nothing has been read.
    Ask,
    /// The Daemon is reading the keys (NetworkManager may be waiting for the
    /// person to authorize it).
    Reading,
    Shown {
        share: VpnShare,
        /// `None` when the config is too long for a QR code.
        drawing: Option<Drawing>,
    },
    Failed(String),
}

#[derive(Debug)]
pub struct ShareView {
    pub id: String,
    pub name: String,
    pub stage: Stage,
}

impl ShareView {
    pub fn arrived(&mut self, result: Result<VpnShare, String>) {
        self.stage = match result {
            Ok(share) => Stage::Shown {
                drawing: qr::draw(share.config.expose()),
                share,
            },
            Err(message) => Stage::Failed(message),
        };
    }
}

/// The layer over the VPN list, if any. It has the keys while open.
#[derive(Debug)]
pub enum Layer {
    Form(Form),
    /// `(name, public key)`, with the sentence that introduced it.
    Key {
        name: String,
        key: String,
        saved: bool,
    },
    Share(ShareView),
}

#[derive(Debug, Default)]
pub struct VpnTab {
    vpns: Vec<Vpn>,
    cursor: usize,
    loaded: bool,
    /// The VPN a toggle is in flight for.
    pub busy: Option<String>,
    pub layer: Option<Layer>,
}

impl VpnTab {
    pub fn len(&self) -> usize {
        self.vpns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vpns.is_empty()
    }

    pub fn has_list(&self) -> bool {
        self.loaded
    }

    pub fn rows(&self) -> &[Vpn] {
        &self.vpns
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn selected(&self) -> Option<&Vpn> {
        self.vpns.get(self.cursor)
    }

    pub fn move_to(&mut self, row: usize) -> bool {
        let row = row.min(self.vpns.len().saturating_sub(1));
        let changed = row != self.cursor;
        self.cursor = row;
        changed
    }

    /// Takes a fresh list. The cursor follows the VPN it was on. Returns
    /// whether anything changed.
    pub fn refresh(&mut self, list: VpnList) -> bool {
        let id = self.selected().map(|v| v.id.clone());
        let changed = !self.loaded || self.vpns != list.vpns;
        self.loaded = true;
        self.vpns = list.vpns;
        self.cursor = id
            .and_then(|id| self.vpns.iter().position(|v| v.id == id))
            .unwrap_or(self.cursor)
            .min(self.vpns.len().saturating_sub(1));
        if self
            .busy
            .as_ref()
            .is_some_and(|b| self.vpns.iter().all(|v| &v.id != b))
        {
            self.busy = None;
        }
        changed
    }

    /// Whether the selected VPN is up or coming up.
    pub fn selected_is_on(&self) -> bool {
        self.selected().is_some_and(|v| v.state != VpnState::Down)
    }

    /// Drops anything holding secrets (the link to the Daemon went away).
    pub fn drop_secrets(&mut self) -> bool {
        if matches!(self.layer, Some(Layer::Share(_))) {
            self.layer = None;
            return true;
        }
        false
    }
}
