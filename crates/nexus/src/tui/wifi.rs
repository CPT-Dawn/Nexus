//! State of the Wi-Fi tab: the list in a stable order, the scan sweep, the
//! inline password field and the commands the tab sends to the Daemon.
//! No I/O here; [`Runner`] is the only part that talks to the socket.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use nexus_ipc::{Eap, Op, Prompt, SavedWifi, ScanOutcome, Secret, WifiList, WifiNetwork};
use zeroize::Zeroizing;

use super::app::Msg;
use super::runner;
use super::undo::Undo;

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
        /// The Wi-Fi Device to join through, if the person picked one.
        device: Option<String>,
    },
    /// Join a hidden Network by name and security. Without a password the
    /// Daemon opens a Secret prompt for it.
    JoinHidden {
        ssid: String,
        security: nexus_ipc::Security,
        password: Option<Secret>,
        device: Option<String>,
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
    /// Wi-Fi radio on or off.
    WifiPower {
        on: bool,
    },
    /// Airplane mode on or off.
    Airplane {
        on: bool,
    },
}

impl Cmd {
    fn op(&self) -> Op {
        match self {
            Cmd::Scan => Op::WifiScan,
            Cmd::Join {
                network, device, ..
            } => Op::WifiJoin {
                network: network.clone(),
                password: None,
                device: device.clone(),
                hidden: None,
            },
            Cmd::JoinHidden {
                ssid,
                security,
                password,
                device,
            } => Op::WifiJoin {
                network: nexus_ipc::network_id(ssid.as_bytes()),
                password: password.clone(),
                device: device.clone(),
                hidden: Some(*security),
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
            Cmd::WifiPower { on } => Op::WifiPower { on: *on },
            Cmd::Airplane { on } => Op::Airplane { on: *on },
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
        self.reveal_op(Op::WifiReveal { network });
    }

    /// Like [`Runner::reveal`], for the running Hotspot.
    pub fn reveal_hotspot(&self) {
        self.reveal_op(Op::HotspotReveal);
    }

    fn reveal_op(&self, op: Op) {
        let (socket, tx) = (self.socket.clone(), self.tx.clone());
        runner::spawn("nexus-reveal", move || {
            let shared = runner::call(&socket, op).and_then(|value| {
                serde_json::from_value(value)
                    .map_err(|_| "The daemon sent details Nexus cannot read.".to_string())
            });
            let _ = tx.send(Msg::Shared(shared));
        });
    }

    pub fn run(&self, cmd: Cmd) {
        let (socket, tx) = (self.socket.clone(), self.tx.clone());
        runner::spawn("nexus-command", move || {
            let result = execute(&socket, &cmd);
            if cmd.reports() {
                let _ = tx.send(Msg::Done(Done { cmd, result }));
            }
        });
    }

    /// Runs one op on its own connection and reports as the message `wrap`
    /// makes of the result (the Daemon's words on a refusal).
    pub fn call(
        &self,
        op: Op,
        wrap: impl FnOnce(Result<serde_json::Value, String>) -> Msg + Send + 'static,
    ) {
        let (socket, tx) = (self.socket.clone(), self.tx.clone());
        runner::spawn("nexus-command", move || {
            let _ = tx.send(wrap(runner::call(&socket, op)));
        });
    }
}

fn execute(socket: &std::path::Path, cmd: &Cmd) -> Result<Option<ScanOutcome>, String> {
    let value = runner::call(socket, cmd.op())?;
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
    pub undo: Option<Undo>,
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
    /// Every row, in the order the person has been looking at.
    all: Vec<Entry>,
    /// What the list shows: `all` narrowed by the `/` filter.
    rows: Vec<Entry>,
    filter: String,
    cursor: usize,
    /// A rescan the person asked for is running: when it ends, take the
    /// Daemon's order (strongest first), unless the cursor has moved since.
    reorder_after_scan: bool,
    /// The cursor moved while the scan ran.
    moved: bool,
    scanning: bool,
    /// The scan request was answered; the sweep ends once lists stop coming.
    scan_acked: bool,
    /// Sweep frames since the last list arrived (counted once acked).
    quiet: u8,
    scan_deadline: Option<Instant>,
    phase: usize,
    /// The SSID being joined.
    pub busy: Option<String>,
    /// The Wi-Fi Device (id) joins go through; `None` lets the Daemon pick
    /// the one that hears the Network best.
    card: Option<String>,
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

    /// The Wi-Fi Devices, when there is a choice between several.
    pub fn cards(&self) -> &[nexus_ipc::WifiDevice] {
        match &self.list {
            Some(l) if l.devices.len() > 1 => &l.devices,
            _ => &[],
        }
    }

    /// The Device joins are pinned to.
    pub fn card(&self) -> Option<&str> {
        self.card.as_deref()
    }

    /// Its interface name.
    pub fn card_name(&self) -> Option<&str> {
        self.card().and_then(|c| self.adapter_name(c))
    }

    /// Steps through "any card", then each card in turn. False with no choice.
    pub fn cycle_card(&mut self) -> bool {
        let cards = self.cards();
        if cards.is_empty() {
            return false;
        }
        let at = self
            .card
            .as_deref()
            .and_then(|c| cards.iter().position(|d| d.id == c));
        self.card = match at {
            None => Some(cards[0].id.clone()),
            Some(i) => cards.get(i + 1).map(|d| d.id.clone()),
        };
        true
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
        self.moved |= changed && self.scanning;
        changed
    }

    /// The `/` filter as typed (empty: none).
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Narrows the list to the Networks whose name contains `text`, ignoring
    /// case. The cursor stays on its row if that row is still shown.
    pub fn set_filter(&mut self, text: &str) -> bool {
        if self.filter == text {
            return false;
        }
        let keep = self.selected().map(Entry::key);
        self.filter = text.to_owned();
        self.show(keep);
        true
    }

    /// `rows` from `all` through the filter; the cursor follows `keep`.
    fn show(&mut self, keep: Option<String>) {
        let want = self.filter.to_lowercase();
        self.rows = self
            .all
            .iter()
            .filter(|e| want.is_empty() || e.name().to_lowercase().contains(&want))
            .cloned()
            .collect();
        self.cursor = keep
            .and_then(|k| self.rows.iter().position(|e| e.key() == k))
            .unwrap_or_else(|| self.cursor.min(self.rows.len().saturating_sub(1)));
    }

    /// `all` from `list`: in the Daemon's order when `resort`, else known
    /// rows keep their place and new ones go to the end of their section.
    fn arrange(&mut self, list: &WifiList, resort: bool) {
        let nets: Vec<Entry> = list.networks.iter().cloned().map(Entry::Net).collect();
        let saved: Vec<Entry> = list.saved.iter().cloned().map(Entry::Saved).collect();
        let mut all = self.settle(nets, resort);
        all.extend(self.settle(saved, resort));
        self.all = all;
    }

    /// Takes a fresh list. Known rows keep their place and new ones go to
    /// the end of their section, so the list never reshuffles under the
    /// cursor, scan or no scan (the order is only redone when a scan the
    /// person asked for ends, see [`WifiTab::stop_scan`]).
    /// The cursor follows its row. Returns whether anything visible changed.
    pub fn refresh(&mut self, list: WifiList) -> bool {
        let before = (self.rows.clone(), self.cursor, self.scanning);
        let keep = self.selected().map(Entry::key);
        let first = self.all.is_empty();
        self.arrange(&list, first);
        // A card that went away no longer pins joins.
        if self
            .card
            .as_deref()
            .is_some_and(|c| !list.devices.iter().any(|d| d.id == c))
        {
            self.card = None;
        }
        self.list = Some(list);
        self.show(keep);
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
            .all
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
        self.reorder_after_scan = true;
        self.moved = false;
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

    /// Ends the sweep. A scan the person asked for then puts the list in the
    /// Daemon's order (strongest first), but only if the cursor stayed put
    /// while it ran: nothing may re-sort under a cursor the person moved
    /// (the order is offered again at the next scan they start).
    pub fn stop_scan(&mut self) {
        self.scanning = false;
        self.scan_acked = false;
        let reorder = std::mem::take(&mut self.reorder_after_scan) && !self.moved;
        if reorder && let Some(list) = self.list.clone() {
            let keep = self.selected().map(Entry::key);
            self.arrange(&list, true);
            self.show(keep);
        }
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
