//! State of the Wi-Fi tab: the list in a stable order, the scan sweep, the
//! inline password field and the commands the tab sends to the Daemon.
//! No I/O here; [`Runner`] is the only part that talks to the socket.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use nexus_ipc::{
    Connection, Eap, Failure, Op, Prompt, SavedWifi, ScanOutcome, Secret, WifiList, WifiNetwork,
};
use zeroize::Zeroizing;

use super::app::Msg;

/// How long a scan sweep may run when the list never changes.
const SCAN_PATIENCE: Duration = Duration::from_secs(8);
/// The sweep ends after this many quiet frames once the scan was answered.
const QUIET_FRAMES: u8 = 8;
/// Frame time of the scan sweep.
pub const SWEEP_FRAME: Duration = Duration::from_millis(120);

/// One row of the list: a nearby Network, or a saved one out of range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Net(WifiNetwork),
    Saved(SavedWifi),
}

impl Entry {
    /// Identity that survives a refresh, so the cursor follows the row.
    fn key(&self) -> String {
        match self {
            Entry::Net(n) => format!("n:{}", n.id),
            Entry::Saved(s) => format!("s:{}", s.uuid),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Entry::Net(n) => &n.ssid,
            Entry::Saved(s) => &s.ssid,
        }
    }

    /// The `WifiNetwork.id` the Daemon's ops take.
    pub fn network_id(&self) -> String {
        match self {
            Entry::Net(n) => n.id.clone(),
            Entry::Saved(s) => nexus_ipc::network_id(s.ssid.as_bytes()),
        }
    }

    /// Whether it signs in with a username and certificate (802.1X).
    pub fn enterprise(&self) -> bool {
        matches!(self, Entry::Net(n) if n.security == nexus_ipc::Security::Enterprise)
    }

    pub fn joined(&self) -> bool {
        matches!(self, Entry::Net(n) if n.joined)
    }

    /// Whether a Profile is saved for it.
    pub fn saved(&self) -> bool {
        match self {
            Entry::Net(n) => n.profile.is_some(),
            Entry::Saved(_) => true,
        }
    }

    /// The user's judgement of it; a saved Network out of range has none
    /// to give (its Trust is read once it is nearby).
    pub fn trust(&self) -> Option<nexus_ipc::Trust> {
        match self {
            Entry::Net(n) => Some(n.trust),
            Entry::Saved(_) => None,
        }
    }

    /// Autoconnect of the saved Profile, if there is one.
    pub fn autoconnect(&self) -> Option<bool> {
        match self {
            Entry::Net(n) => n.autoconnect,
            Entry::Saved(s) => Some(s.autoconnect),
        }
    }
}

/// What the tab asks the Daemon to do. Each runs on its own connection, so a
/// join that waits on a Secret prompt never blocks the others.
#[derive(Debug, Clone)]
pub enum Cmd {
    Scan,
    Join {
        network: String,
        ssid: String,
    },
    /// Join an enterprise Network with the form's settings.
    JoinEap {
        network: String,
        ssid: String,
        eap: Box<Eap>,
    },
    Disconnect {
        network: String,
        ssid: String,
    },
    Forget {
        network: String,
        ssid: String,
    },
    Autoconnect {
        network: String,
        ssid: String,
        enabled: bool,
    },
    /// Mark a Network Trusted or Untrusted.
    Trust {
        network: String,
        ssid: String,
        trust: nexus_ipc::Trust,
    },
    Answer {
        prompt: u64,
        secret: Secret,
    },
    Cancel {
        prompt: u64,
    },
    /// Tether through a paired phone (`phone` is its address).
    Tether {
        phone: String,
        name: String,
    },
    /// Drop the tether to `name`.
    Untether {
        phone: String,
        name: String,
    },
    /// Open the Portal's login page in the browser.
    OpenPortal,
}

impl Cmd {
    fn op(&self) -> Op {
        match self {
            Cmd::Scan => Op::WifiScan,
            Cmd::Join { network, .. } => Op::WifiJoin {
                network: network.clone(),
                password: None,
            },
            Cmd::JoinEap { network, eap, .. } => Op::WifiJoinEap {
                network: network.clone(),
                eap: (**eap).clone(),
            },
            Cmd::Disconnect { .. } => Op::WifiDisconnect,
            Cmd::Forget { network, .. } => Op::WifiForget {
                network: network.clone(),
            },
            Cmd::Autoconnect {
                network, enabled, ..
            } => Op::WifiAutoconnect {
                network: network.clone(),
                enabled: *enabled,
            },
            Cmd::Trust { network, trust, .. } => Op::TrustSet {
                network: network.clone(),
                trust: *trust,
            },
            Cmd::Answer { prompt, secret } => Op::SecretAnswer {
                prompt: *prompt,
                secret: secret.clone(),
            },
            Cmd::Cancel { prompt } => Op::SecretCancel { prompt: *prompt },
            Cmd::Tether { phone, .. } => Op::TetherConnect {
                phone: Some(phone.clone()),
            },
            Cmd::Untether { .. } => Op::TetherDisconnect,
            Cmd::OpenPortal => Op::PortalOpen,
        }
    }

    /// Whether the person is told how it went.
    fn reports(&self) -> bool {
        !matches!(self, Cmd::Answer { .. } | Cmd::Cancel { .. })
    }
}

/// How a [`Cmd`] ended.
#[derive(Debug, Clone)]
pub struct Done {
    pub cmd: Cmd,
    pub result: Result<Option<ScanOutcome>, String>,
}

/// Runs [`Cmd`]s on threads and reports back as [`Msg::Done`].
#[derive(Debug, Clone)]
pub struct Runner {
    socket: PathBuf,
    tx: Sender<Msg>,
}

impl Runner {
    pub fn new(socket: PathBuf, tx: Sender<Msg>) -> Runner {
        Runner { socket, tx }
    }

    /// Reads a saved Network's join details for the share view; the answer
    /// arrives as [`Msg::Shared`]. NetworkManager may wait for the person to
    /// authorize it, so it has its own connection and thread.
    pub fn reveal(&self, network: String) {
        let (socket, tx) = (self.socket.clone(), self.tx.clone());
        let spawned = thread::Builder::new()
            .name("nexus-reveal".into())
            .spawn(move || {
                let _ = tx.send(Msg::Shared(fetch_share(&socket, network)));
            });
        if let Err(e) = spawned {
            tracing::warn!("cannot spawn a reveal thread: {e}");
        }
    }

    /// Like [`Runner::reveal`], for the running Hotspot.
    pub fn reveal_hotspot(&self) {
        let (socket, tx) = (self.socket.clone(), self.tx.clone());
        let spawned = thread::Builder::new()
            .name("nexus-reveal".into())
            .spawn(move || {
                let _ = tx.send(Msg::Shared(fetch_hotspot_share(&socket)));
            });
        if let Err(e) = spawned {
            tracing::warn!("cannot spawn a reveal thread: {e}");
        }
    }

    pub fn run(&self, cmd: Cmd) {
        let (socket, tx) = (self.socket.clone(), self.tx.clone());
        let spawned = thread::Builder::new()
            .name("nexus-command".into())
            .spawn(move || {
                let result = execute(&socket, &cmd);
                if cmd.reports() {
                    let _ = tx.send(Msg::Done(Done { cmd, result }));
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("cannot spawn a command thread: {e}");
        }
    }
}

impl Runner {
    /// Runs one op on its own connection and reports as the message `wrap`
    /// makes of the result (the Daemon's words on a refusal).
    pub fn call(
        &self,
        op: Op,
        wrap: impl FnOnce(Result<serde_json::Value, String>) -> Msg + Send + 'static,
    ) {
        let (socket, tx) = (self.socket.clone(), self.tx.clone());
        let spawned = thread::Builder::new()
            .name("nexus-command".into())
            .spawn(move || {
                let result = Connection::connect_to(&socket)
                    .map_err(|_| "Can't reach the Nexus daemon.".to_string())
                    .and_then(|mut c| {
                        c.call(op)
                            .map_err(|_| "Lost the Nexus daemon.".to_string())?
                            .map_err(|f: Failure| f.message)
                    });
                let _ = tx.send(wrap(result));
            });
        if let Err(e) = spawned {
            tracing::warn!("cannot spawn a command thread: {e}");
        }
    }
}

fn fetch_share(socket: &std::path::Path, network: String) -> Result<nexus_ipc::WifiShare, String> {
    fetch(socket, Op::WifiReveal { network })
}

fn fetch_hotspot_share(socket: &std::path::Path) -> Result<nexus_ipc::WifiShare, String> {
    fetch(socket, Op::HotspotReveal)
}

fn fetch(socket: &std::path::Path, op: Op) -> Result<nexus_ipc::WifiShare, String> {
    let mut conn =
        Connection::connect_to(socket).map_err(|_| "Can't reach the Nexus daemon.".to_string())?;
    let value = conn
        .call(op)
        .map_err(|_| "Lost the Nexus daemon.".to_string())?
        .map_err(|f: Failure| f.message)?;
    serde_json::from_value(value)
        .map_err(|_| "The daemon sent details Nexus cannot read.".to_string())
}

fn execute(socket: &std::path::Path, cmd: &Cmd) -> Result<Option<ScanOutcome>, String> {
    let mut conn =
        Connection::connect_to(socket).map_err(|_| "Can't reach the Nexus daemon.".to_string())?;
    let value = conn
        .call(cmd.op())
        .map_err(|_| "Lost the Nexus daemon.".to_string())?
        .map_err(|f: Failure| f.message)?;
    Ok(match cmd {
        Cmd::Scan => serde_json::from_value(value).ok(),
        _ => None,
    })
}

/// A short message at the corner of the screen.
#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub bad: bool,
    pub undo: Option<Cmd>,
    pub until: Instant,
}

/// The inline masked password field of a Secret prompt.
pub struct PasswordField {
    pub prompt: Prompt,
    text: Zeroizing<String>,
    pub reveal: bool,
}

impl PasswordField {
    pub fn new(prompt: Prompt) -> Self {
        // A user name is not secret: show it as it is typed.
        let reveal = prompt.field == nexus_ipc::PromptField::Username;
        Self {
            prompt,
            text: Zeroizing::new(String::new()),
            reveal,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn push(&mut self, c: char) {
        self.text.push(c);
    }

    pub fn pop(&mut self) {
        self.text.pop();
    }

    pub fn clear(&mut self) {
        self.text.clear();
    }
}

impl fmt::Debug for PasswordField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PasswordField")
            .field("prompt", &self.prompt)
            .field("text", &"<redacted>")
            .field("reveal", &self.reveal)
            .finish()
    }
}

/// A Forget waiting for a yes.
#[derive(Debug, Clone)]
pub struct Confirm {
    pub network: String,
    pub ssid: String,
}

#[derive(Debug, Default)]
pub struct WifiTab {
    list: Option<WifiList>,
    rows: Vec<Entry>,
    cursor: usize,
    /// Take the Daemon's order (strongest first) while a rescan runs.
    resort: bool,
    scanning: bool,
    /// The scan request was answered; the sweep ends once lists stop coming.
    scan_acked: bool,
    /// Sweep frames since the last list arrived (counted once acked).
    quiet: u8,
    scan_deadline: Option<Instant>,
    phase: usize,
    /// The SSID being joined.
    pub busy: Option<String>,
}

impl WifiTab {
    pub fn has_list(&self) -> bool {
        self.list.is_some()
    }

    pub fn rows(&self) -> &[Entry] {
        &self.rows
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.rows.get(self.cursor)
    }

    /// Index of the first saved-out-of-range row, if any.
    pub fn saved_start(&self) -> Option<usize> {
        self.rows.iter().position(|e| matches!(e, Entry::Saved(_)))
    }

    pub fn adapter_name(&self, device: &str) -> Option<&str> {
        self.list
            .as_ref()?
            .devices
            .iter()
            .find(|d| d.id == device)
            .map(|d| d.name.as_str())
    }

    /// The signal of the nearby Network called `ssid`.
    pub fn signal_of(&self, ssid: &str) -> Option<u8> {
        self.rows.iter().find_map(|e| match e {
            Entry::Net(n) if n.ssid == ssid => Some(n.signal),
            _ => None,
        })
    }

    pub fn has_adapter(&self) -> bool {
        self.list.as_ref().is_some_and(|l| !l.devices.is_empty())
    }

    /// The Daemon has answered the current scan request.
    pub fn scan_acked(&self) -> bool {
        self.scan_acked
    }

    pub fn scanning(&self) -> bool {
        self.scanning
    }

    /// Where the sweep is, for drawing: steps through `0..period` and back.
    pub fn sweep(&self, period: usize) -> usize {
        let cycle = period * 2 - 2;
        let p = self.phase % cycle.max(1);
        if p < period { p } else { cycle - p }
    }

    pub fn move_to(&mut self, row: usize) -> bool {
        let row = row.min(self.rows.len().saturating_sub(1));
        let changed = row != self.cursor;
        self.cursor = row;
        changed
    }

    /// Takes a fresh list. Known rows keep their place and new ones go to
    /// the end of their section, so the list never reshuffles under the
    /// cursor; the order is only redone after a rescan the person asked for.
    /// The cursor follows its row. Returns whether anything visible changed.
    pub fn refresh(&mut self, list: WifiList) -> bool {
        let before = (self.rows.clone(), self.cursor, self.scanning);
        let keep = self.selected().map(Entry::key);
        let resort = self.resort || self.rows.is_empty();

        let nets: Vec<Entry> = list.networks.iter().cloned().map(Entry::Net).collect();
        let saved: Vec<Entry> = list.saved.iter().cloned().map(Entry::Saved).collect();
        let mut rows = self.settle(nets, resort);
        rows.extend(self.settle(saved, resort));
        self.rows = rows;
        self.list = Some(list);

        self.cursor = keep
            .and_then(|k| self.rows.iter().position(|e| e.key() == k))
            .unwrap_or_else(|| self.cursor.min(self.rows.len().saturating_sub(1)));
        // Scan results trickle in: each list restarts the quiet count.
        self.quiet = 0;
        before != (self.rows.clone(), self.cursor, self.scanning)
    }

    /// `fresh` in the previous order where known, then the rest.
    fn settle(&self, fresh: Vec<Entry>, resort: bool) -> Vec<Entry> {
        if resort {
            return fresh;
        }
        let mut by_key: HashMap<String, Entry> =
            fresh.iter().map(|e| (e.key(), e.clone())).collect();
        let mut out: Vec<Entry> = self
            .rows
            .iter()
            .filter_map(|old| by_key.remove(&old.key()))
            .collect();
        out.extend(fresh.into_iter().filter(|e| by_key.contains_key(&e.key())));
        out
    }

    /// The person asked for a rescan. False if one is already running.
    pub fn start_scan(&mut self) -> bool {
        if self.scanning {
            return false;
        }
        self.scanning = true;
        self.scan_acked = false;
        self.quiet = 0;
        self.resort = true;
        self.scan_deadline = Some(Instant::now() + SCAN_PATIENCE);
        true
    }

    /// The Daemon answered the scan request.
    pub fn scan_answered(&mut self, outcome: Option<&ScanOutcome>) {
        match outcome {
            // Every Device scanned recently: nothing will change.
            Some(o) if o.requested == 0 => self.stop_scan(),
            Some(_) => self.scan_acked = true,
            None => self.stop_scan(),
        }
    }

    pub fn stop_scan(&mut self) {
        self.scanning = false;
        self.scan_acked = false;
        self.resort = false;
    }

    /// Advances the sweep. Returns whether to redraw.
    pub fn tick(&mut self) -> bool {
        if !self.scanning {
            return false;
        }
        self.phase += 1;
        if self.scan_acked {
            self.quiet += 1;
            if self.quiet >= QUIET_FRAMES {
                self.stop_scan();
                return true;
            }
        }
        if self.scan_deadline.is_some_and(|d| Instant::now() >= d) {
            self.stop_scan();
        }
        true
    }
}
