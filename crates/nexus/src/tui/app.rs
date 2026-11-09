//! TUI state and its single `update` function. Input and Daemon events
//! arrive as [`Msg`]s, and `update` says whether the screen changed. The only
//! I/O is [`Runner`] sending the commands the person asked for.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use nexus_ipc::{Event, PromptClosed, PromptKind, Secret, Status, WifiList};

use super::caps::Caps;
use super::theme::{Look, Theme};
use super::wifi::{Cmd, Confirm, Done, Entry, PasswordField, Runner, SWEEP_FRAME, Toast, WifiTab};

/// How long a toast stays when nobody answers it.
const TOAST_FOR: Duration = Duration::from_secs(6);

/// Everything that can happen to the TUI.
#[derive(Debug, Clone)]
pub enum Msg {
    Key(KeyEvent),
    Resize(u16, u16),
    /// A fresh status from the Daemon (first connect, event or poll).
    Status(Status),
    /// A fresh Wi-Fi list (first connect or `networks_changed`).
    Wifi(WifiList),
    /// A Secret prompt opened or closed.
    Prompt(Event),
    /// A command the person gave has finished.
    Done(Done),
    /// The Daemon cannot be reached (never connected, or the link dropped).
    Down,
    /// A running effect asked for a frame.
    Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    WiFi,
    Ethernet,
    Vpn,
    Hotspot,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Overview,
        Tab::WiFi,
        Tab::Ethernet,
        Tab::Vpn,
        Tab::Hotspot,
        Tab::Settings,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::WiFi => "Wi-Fi",
            Tab::Ethernet => "Ethernet",
            Tab::Vpn => "VPN",
            Tab::Hotspot => "Hotspot",
            Tab::Settings => "Settings",
        }
    }

    pub fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).expect("listed")
    }

    /// How many rows the tab's master list has (the Wi-Fi tab's depends on
    /// the Daemon: see [`App::rows`]).
    pub fn rows(self) -> usize {
        match self {
            Tab::Overview => 4,
            _ => 0,
        }
    }
}

/// Link between the TUI and the Daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conn {
    /// Not yet heard from the Daemon.
    Connecting,
    Up,
    Down,
}

/// Which layer of the screen has the keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Root,
    /// The slide-in detail sheet (narrow layout).
    Sheet,
    Help,
}

#[derive(Debug)]
pub struct App {
    look: Look,
    tab: Tab,
    selected: usize,
    layer: Layer,
    conn: Conn,
    status: Option<Status>,
    size: (u16, u16),
    quit: bool,
    wifi: WifiTab,
    /// An open Secret prompt: it has the keys until answered or cancelled.
    field: Option<PasswordField>,
    confirm: Option<Confirm>,
    toast: Option<Toast>,
    runner: Option<Runner>,
}

impl App {
    pub fn new(caps: Caps) -> App {
        App {
            look: Look::new(Theme::MIDNIGHT_BLOOM, caps),
            tab: Tab::Overview,
            selected: 0,
            layer: Layer::Root,
            conn: Conn::Connecting,
            status: None,
            size: (80, 24),
            quit: false,
            wifi: WifiTab::default(),
            field: None,
            confirm: None,
            toast: None,
            runner: None,
        }
    }

    /// Lets the app act on the Daemon: commands run on threads over `socket`
    /// and finish as [`Msg::Done`] on `tx`. Without it, commands go nowhere.
    pub fn attach(&mut self, socket: PathBuf, tx: Sender<Msg>) {
        self.runner = Some(Runner::new(socket, tx));
    }

    pub fn look(&self) -> Look {
        self.look
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    /// The cursor row in the current tab's master list.
    pub fn selected(&self) -> usize {
        match self.tab {
            Tab::WiFi => self.wifi.cursor(),
            _ => self.selected,
        }
    }

    /// How many rows the current tab's master list has.
    pub fn rows(&self) -> usize {
        match self.tab {
            Tab::WiFi => self.wifi.len(),
            other => other.rows(),
        }
    }

    pub fn wifi(&self) -> &WifiTab {
        &self.wifi
    }

    pub fn field(&self) -> Option<&PasswordField> {
        self.field.as_ref()
    }

    pub fn confirm(&self) -> Option<&Confirm> {
        self.confirm.as_ref()
    }

    pub fn toast(&self) -> Option<&Toast> {
        self.toast.as_ref()
    }

    pub fn layer(&self) -> Layer {
        self.layer
    }

    pub fn conn(&self) -> Conn {
        self.conn
    }

    pub fn is_down(&self) -> bool {
        self.conn == Conn::Down
    }

    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// Whether the narrow layout is active, from the last known size.
    pub fn narrow(&self) -> bool {
        !super::layout::Mode::of(ratatui::layout::Rect::new(0, 0, self.size.0, self.size.1)).split()
    }

    /// When a running effect needs the next frame, if any: the scan sweep
    /// while scanning, the toast's end while one shows (ADR-0003).
    pub fn next_tick(&self) -> Option<Duration> {
        let sweep = self.wifi.scanning().then_some(SWEEP_FRAME);
        let toast = self
            .toast
            .as_ref()
            .map(|t| t.until.saturating_duration_since(Instant::now()));
        match (sweep, toast) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Applies one message. Returns whether the screen needs a redraw: an
    /// event that changes nothing must not cost a frame.
    pub fn update(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Key(key) => self.key(key),
            Msg::Resize(w, h) => {
                self.size = (w, h);
                if !self.narrow() && self.layer == Layer::Sheet {
                    self.layer = Layer::Root;
                }
                true
            }
            Msg::Status(status) => {
                let changed = self.conn != Conn::Up || self.status.as_ref() != Some(&status);
                self.conn = Conn::Up;
                self.status = Some(status);
                changed
            }
            Msg::Wifi(list) => self.wifi.refresh(list),
            Msg::Prompt(event) => self.prompt_event(event),
            Msg::Done(done) => self.done(done),
            Msg::Down => {
                // A prompt dies with the link; the Daemon replays it if it lives.
                let changed = self.conn != Conn::Down || self.field.take().is_some();
                self.conn = Conn::Down;
                changed
            }
            Msg::Tick => {
                let mut dirty = self.wifi.tick();
                if self
                    .toast
                    .as_ref()
                    .is_some_and(|t| Instant::now() >= t.until)
                {
                    self.toast = None;
                    dirty = true;
                }
                dirty
            }
        }
    }

    fn send(&self, cmd: Cmd) {
        if let Some(runner) = &self.runner {
            runner.run(cmd);
        }
    }

    fn say(&mut self, text: impl Into<String>, bad: bool, undo: Option<Cmd>) {
        self.toast = Some(Toast {
            text: text.into(),
            bad,
            undo,
            until: Instant::now() + TOAST_FOR,
        });
    }

    fn prompt_event(&mut self, event: Event) -> bool {
        match event {
            Event::SecretPromptOpened { prompt } if prompt.kind == PromptKind::WifiPassword => {
                self.field = Some(PasswordField::new(prompt));
                true
            }
            Event::SecretPromptClosed { id, reason } => {
                // Our own answer or cancel already hid it; this covers a
                // prompt another Front end answered, or one that timed out.
                let ours = self.field.as_ref().is_some_and(|f| f.prompt.id == id);
                if ours {
                    self.field = None;
                    if reason == PromptClosed::Expired {
                        self.say("The password prompt timed out.", true, None);
                    }
                }
                ours
            }
            _ => false,
        }
    }

    fn done(&mut self, done: Done) -> bool {
        let Done { cmd, result } = done;
        match (&cmd, result) {
            (Cmd::Scan, Ok(outcome)) => {
                self.wifi.scan_answered(outcome.as_ref());
                if outcome.is_some_and(|o| o.requested == 0) {
                    self.say("Scanned a moment ago; the list is fresh.", false, None);
                }
            }
            (Cmd::Scan, Err(message)) => {
                self.wifi.stop_scan();
                self.say(message, true, None);
            }
            (Cmd::Join { ssid, .. }, result) => {
                self.wifi.busy = None;
                match result {
                    Ok(_) => self.say(format!("Joined {ssid}."), false, None),
                    Err(message) => self.say(message, true, None),
                }
            }
            (Cmd::Disconnect { network, ssid }, Ok(_)) => {
                let undo = Cmd::Join {
                    network: network.clone(),
                    ssid: ssid.clone(),
                };
                self.say(format!("Disconnected from {ssid}."), false, Some(undo));
            }
            (Cmd::Forget { ssid, .. }, Ok(_)) => {
                self.say(format!("Forgot {ssid}."), false, None);
            }
            (
                Cmd::Autoconnect {
                    network,
                    ssid,
                    enabled,
                },
                Ok(_),
            ) => {
                let undo = Cmd::Autoconnect {
                    network: network.clone(),
                    ssid: ssid.clone(),
                    enabled: !enabled,
                };
                let text = match enabled {
                    true => format!("{ssid} will join automatically."),
                    false => format!("{ssid} will not join automatically."),
                };
                self.say(text, false, Some(undo));
            }
            (_, Err(message)) => self.say(message, true, None),
            _ => return false,
        }
        true
    }

    fn key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return true;
        }
        if self.field.is_some() {
            return self.field_key(key);
        }
        if self.confirm.is_some() {
            return self.confirm_key(key);
        }
        match self.layer {
            Layer::Help => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
                    self.layer = Layer::Root;
                    return true;
                }
                false
            }
            Layer::Sheet => match key.code {
                KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                    self.layer = Layer::Root;
                    true
                }
                KeyCode::Char('?') => {
                    self.layer = Layer::Help;
                    true
                }
                _ => self.tab_key(key),
            },
            Layer::Root => self.root_key(key),
        }
    }

    /// Typing a password into the inline field.
    fn field_key(&mut self, key: KeyEvent) -> bool {
        let Some(field) = self.field.as_mut() else {
            return false;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                let prompt = field.prompt.id;
                self.field = None;
                self.send(Cmd::Cancel { prompt });
            }
            KeyCode::Enter => {
                if field.text().is_empty() {
                    return false;
                }
                let secret = Secret::new(field.text());
                let prompt = field.prompt.id;
                self.field = None;
                self.send(Cmd::Answer { prompt, secret });
            }
            KeyCode::Char('r') if ctrl => field.reveal = !field.reveal,
            KeyCode::Char('u') if ctrl => field.clear(),
            KeyCode::Backspace => field.pop(),
            KeyCode::Char(c) if !ctrl => field.push(c),
            _ => return false,
        }
        true
    }

    /// Forget waits for a yes.
    fn confirm_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('y' | 'Y') => {
                if let Some(Confirm { network, ssid }) = self.confirm.take() {
                    self.send(Cmd::Forget { network, ssid });
                }
                true
            }
            KeyCode::Esc | KeyCode::Char('n' | 'N') => {
                self.confirm = None;
                true
            }
            _ => false,
        }
    }

    fn root_key(&mut self, key: KeyEvent) -> bool {
        let last_row = self.rows().saturating_sub(1);
        match key.code {
            KeyCode::Char('q') => {
                self.quit = true;
                true
            }
            KeyCode::Char('?') => {
                self.layer = Layer::Help;
                true
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l' | ']') => self.step_tab(1),
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h' | '[') => self.step_tab(-1),
            KeyCode::Char(c @ '1'..='6') => {
                let i = c as usize - '1' as usize;
                self.go_tab(Tab::ALL[i])
            }
            KeyCode::Down | KeyCode::Char('j') => self.select((self.selected() + 1).min(last_row)),
            KeyCode::Up | KeyCode::Char('k') => self.select(self.selected().saturating_sub(1)),
            KeyCode::Char('g') | KeyCode::Home => self.select(0),
            KeyCode::Char('G') | KeyCode::End => self.select(last_row),
            KeyCode::Enter if self.narrow() && self.tab != Tab::WiFi => {
                self.layer = Layer::Sheet;
                true
            }
            KeyCode::Char('d') if self.narrow() && self.rows() > 0 => {
                self.layer = Layer::Sheet;
                true
            }
            // Esc at the root has nothing to back out of, and never quits.
            _ => self.tab_key(key),
        }
    }

    /// Keys that belong to the current tab (the sheet passes them through).
    fn tab_key(&mut self, key: KeyEvent) -> bool {
        match self.tab {
            Tab::WiFi => self.wifi_key(key),
            _ => false,
        }
    }

    fn wifi_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('r') => {
                if self.wifi.start_scan() {
                    self.send(Cmd::Scan);
                }
                true
            }
            KeyCode::Char('u') => match self.toast.take().and_then(|t| t.undo) {
                Some(undo) => {
                    self.run_wifi(undo);
                    true
                }
                None => false,
            },
            _ => {
                let Some(entry) = self.wifi.selected().cloned() else {
                    return false;
                };
                let (network, ssid) = (entry.network_id(), entry.name().to_owned());
                match key.code {
                    KeyCode::Enter if !entry.joined() && self.wifi.busy.is_none() => {
                        self.run_wifi(Cmd::Join { network, ssid });
                        true
                    }
                    KeyCode::Char('x') if entry.joined() => {
                        self.run_wifi(Cmd::Disconnect { network, ssid });
                        true
                    }
                    KeyCode::Char('f') if entry.saved() => {
                        self.confirm = Some(Confirm { network, ssid });
                        true
                    }
                    KeyCode::Char('a') if entry.autoconnect().is_some() => {
                        let enabled = !entry.autoconnect().unwrap_or(true);
                        self.run_wifi(Cmd::Autoconnect {
                            network,
                            ssid,
                            enabled,
                        });
                        true
                    }
                    _ => false,
                }
            }
        }
    }

    /// Sends a Wi-Fi command, marking a join as in flight.
    fn run_wifi(&mut self, cmd: Cmd) {
        if let Cmd::Join { ssid, .. } = &cmd {
            self.wifi.busy = Some(ssid.clone());
        }
        self.toast = None;
        self.send(cmd);
    }

    fn select(&mut self, row: usize) -> bool {
        if self.tab == Tab::WiFi {
            return self.wifi.move_to(row);
        }
        let changed = row != self.selected;
        self.selected = row;
        changed
    }

    fn step_tab(&mut self, by: isize) -> bool {
        let n = Tab::ALL.len() as isize;
        let next = (self.tab.index() as isize + by).rem_euclid(n) as usize;
        self.go_tab(Tab::ALL[next])
    }

    fn go_tab(&mut self, tab: Tab) -> bool {
        let changed = tab != self.tab;
        self.tab = tab;
        self.selected = 0;
        changed
    }

    /// The keys that work right now, as `(key, what it does)`, for the footer.
    pub fn hints(&self, narrow: bool) -> Vec<(&'static str, &'static str)> {
        if self.field.is_some() {
            return vec![("enter", "join"), ("ctrl-r", "show"), ("esc", "cancel")];
        }
        if self.confirm.is_some() {
            return vec![("y", "forget"), ("esc", "keep")];
        }
        match self.layer {
            Layer::Help => vec![("esc", "close")],
            Layer::Sheet => {
                let mut v = vec![("esc", "back")];
                if self.tab == Tab::WiFi {
                    v.extend(self.wifi_hints(false));
                }
                v.push(("?", "help"));
                v
            }
            Layer::Root => {
                let mut v = Vec::new();
                if self.rows() > 0 {
                    v.push(("j/k", "move"));
                }
                if self.tab == Tab::WiFi {
                    v.extend(self.wifi_hints(!narrow));
                } else if narrow && self.rows() > 0 {
                    v.push(("enter", "open"));
                }
                v.push(("h/l", "tab"));
                v.push(("?", "help"));
                v.push(("q", "quit"));
                v
            }
        }
    }

    /// Wi-Fi keys that apply to the selected row; `more` adds the less common ones.
    fn wifi_hints(&self, more: bool) -> Vec<(&'static str, &'static str)> {
        let mut v = Vec::new();
        let selected: Option<&Entry> = self.wifi.selected();
        if selected.is_some_and(|e| !e.joined()) {
            v.push(("enter", "join"));
        }
        v.push(("r", "scan"));
        if more && let Some(e) = selected {
            if e.joined() {
                v.push(("x", "disconnect"));
            }
            if e.saved() {
                v.push(("f", "forget"));
            }
            if e.autoconnect().is_some() {
                v.push(("a", "auto"));
            }
        }
        if self.toast.as_ref().is_some_and(|t| t.undo.is_some()) {
            v.push(("u", "undo"));
        }
        v
    }
}
