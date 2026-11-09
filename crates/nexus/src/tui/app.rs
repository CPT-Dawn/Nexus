//! TUI state and its single `update` function. No I/O here: input and Daemon
//! events arrive as [`Msg`]s, and `update` says whether the screen changed.

use std::collections::VecDeque;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use nexus_ipc::{ActivityEntry, Finding, Status, Traffic};

use super::caps::Caps;
use super::theme::{Look, Theme};

/// Everything that can happen to the TUI.
#[derive(Debug, Clone)]
pub enum Msg {
    Key(KeyEvent),
    Resize(u16, u16),
    /// A fresh status from the Daemon (first connect, event or poll).
    Status(Status),
    /// A traffic sample, asked for only while the graphs are visible.
    Traffic(Traffic),
    /// The Daemon's whole activity log (on connect), newest first.
    Activity(Vec<ActivityEntry>),
    /// One new activity entry.
    Happened(ActivityEntry),
    /// The Doctor's findings (or none: all clear, or nothing to check).
    Findings(Vec<Finding>),
    /// A fix the person confirmed ended: `Err` carries the words to show.
    FixDone(Result<(), String>),
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

    /// How many rows the tab's master list has.
    pub fn rows(self) -> usize {
        match self {
            Tab::Overview => 4,
            _ => 0,
        }
    }
}

/// How many samples of traffic history to keep (one per second).
pub const HISTORY: usize = 300;
/// How many activity entries to keep.
const ACTIVITY: usize = 20;

/// Recent traffic rates, oldest first.
#[derive(Debug, Default, Clone)]
pub struct Rates {
    pub down: VecDeque<u64>,
    pub up: VecDeque<u64>,
}

impl Rates {
    /// Adds a sample. Returns whether any graph would look different: a
    /// zero entering a window that drops a zero is a flat line staying flat.
    fn push(&mut self, t: Traffic) -> bool {
        let mut changed = false;
        for (series, v) in [(&mut self.down, t.rx_bps), (&mut self.up, t.tx_bps)] {
            series.push_back(v);
            let dropped = if series.len() > HISTORY {
                series.pop_front().unwrap_or(0)
            } else {
                0
            };
            changed |= v != 0 || dropped != 0;
        }
        changed
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
    /// "Run the fix?": waits for a yes or a no.
    Confirm,
}

/// Where a Doctor fix is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixState {
    Idle,
    /// The yes/no is on screen.
    Asking,
    Running,
    Done,
    Failed(String),
}

#[derive(Debug)]
pub struct App {
    look: Look,
    tab: Tab,
    selected: usize,
    layer: Layer,
    conn: Conn,
    status: Option<Status>,
    rates: Rates,
    samples: usize,
    activity: Vec<ActivityEntry>,
    findings: Vec<Finding>,
    fix: FixState,
    /// A confirmed fix the event loop has yet to hand to the Daemon link.
    fix_request: std::cell::RefCell<Option<String>>,
    size: (u16, u16),
    quit: bool,
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
            rates: Rates::default(),
            samples: 0,
            activity: Vec::new(),
            findings: Vec::new(),
            fix: FixState::Idle,
            fix_request: std::cell::RefCell::new(None),
            size: (80, 24),
            quit: false,
        }
    }

    pub fn look(&self) -> Look {
        self.look
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    pub fn selected(&self) -> usize {
        self.selected
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

    /// Recent traffic, for the graphs.
    pub fn rates(&self) -> &Rates {
        &self.rates
    }

    /// The latest traffic sample, if any since the graphs last appeared.
    pub fn traffic_now(&self) -> Option<Traffic> {
        Some(Traffic {
            rx_bps: *self.rates.down.back()?,
            tx_bps: *self.rates.up.back()?,
        })
    }

    /// How many samples arrived since the graphs last appeared.
    pub fn traffic_samples(&self) -> usize {
        self.samples
    }

    /// Recent activity, newest first.
    pub fn activity(&self) -> &[ActivityEntry] {
        &self.activity
    }

    /// The Doctor's latest findings (empty while the machine looks fine).
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    /// The first finding in trouble that has a fix the Daemon can run.
    pub fn runnable_fix(&self) -> Option<&Finding> {
        self.findings
            .iter()
            .find(|f| f.is_trouble() && f.fix.as_ref().is_some_and(|x| x.runnable))
    }

    pub fn fix_state(&self) -> &FixState {
        &self.fix
    }

    /// The id of a finding whose fix the person just confirmed. The event
    /// loop sends it to the Daemon once, after the frame.
    pub fn take_fix_request(&self) -> Option<String> {
        self.fix_request.borrow_mut().take()
    }

    /// Whether the first-run setup screen replaces the Overview: Nexus has
    /// no NetworkManager to talk to.
    pub fn needs_setup(&self) -> bool {
        self.conn == Conn::Up
            && self
                .status
                .as_ref()
                .is_some_and(|s| s.state == nexus_ipc::State::Unavailable)
    }

    /// Whether the traffic graphs are on screen. The Daemon link polls
    /// traffic at 1 Hz exactly while this holds, and not otherwise.
    pub fn traffic_visible(&self) -> bool {
        let (true, Some(status)) = (
            self.conn == Conn::Up && self.tab == Tab::Overview && self.layer == Layer::Root,
            &self.status,
        ) else {
            return false;
        };
        super::overview::traffic_shown(&self.look, status, self.size.0, self.size.1)
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// Whether the narrow layout is active, from the last known size.
    pub fn narrow(&self) -> bool {
        !super::layout::Mode::of(ratatui::layout::Rect::new(0, 0, self.size.0, self.size.1)).split()
    }

    /// When a running effect needs the next frame, if any. Nothing animates
    /// yet, so the loop sleeps until something happens (ADR-0003).
    pub fn next_tick(&self) -> Option<Duration> {
        None
    }

    /// Applies one message. Returns whether the screen needs a redraw: an
    /// event that changes nothing must not cost a frame.
    pub fn update(&mut self, msg: Msg) -> bool {
        let was_visible = self.traffic_visible();
        let dirty = self.apply(msg);
        if was_visible && !self.traffic_visible() {
            // The history would be stale and gappy on return: start over.
            self.rates = Rates::default();
            self.samples = 0;
        }
        dirty
    }

    fn apply(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Traffic(t) => {
                if !self.traffic_visible() {
                    return false; // a late sample from before the graphs left
                }
                self.samples += 1;
                self.rates.push(t)
            }
            Msg::Activity(list) => {
                let changed = self.activity != list;
                self.activity = list;
                self.activity.truncate(ACTIVITY);
                changed && self.tab == Tab::Overview
            }
            Msg::Happened(entry) => {
                self.activity.insert(0, entry);
                self.activity.truncate(ACTIVITY);
                self.tab == Tab::Overview
            }
            Msg::Findings(list) => {
                let changed = self.findings != list;
                self.findings = list;
                changed
            }
            Msg::FixDone(result) => {
                self.fix = match result {
                    Ok(()) => FixState::Done,
                    Err(why) => FixState::Failed(why),
                };
                true
            }
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
            Msg::Down => {
                let changed = self.conn != Conn::Down;
                self.conn = Conn::Down;
                changed
            }
            Msg::Tick => false,
        }
    }

    fn key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return true;
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
                _ => false,
            },
            Layer::Confirm => match key.code {
                KeyCode::Char('y' | 'Y') => {
                    self.layer = Layer::Root;
                    match self.runnable_fix().map(|f| f.id.clone()) {
                        Some(id) => {
                            self.fix = FixState::Running;
                            *self.fix_request.borrow_mut() = Some(id);
                        }
                        None => self.fix = FixState::Idle, // it healed meanwhile
                    }
                    true
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                    self.layer = Layer::Root;
                    self.fix = FixState::Idle;
                    true
                }
                _ => false,
            },
            Layer::Root => self.root_key(key),
        }
    }

    fn root_key(&mut self, key: KeyEvent) -> bool {
        let last_row = self.tab.rows().saturating_sub(1);
        match key.code {
            KeyCode::Char('q') => {
                self.quit = true;
                true
            }
            KeyCode::Char('?') => {
                self.layer = Layer::Help;
                true
            }
            KeyCode::Char('f')
                if self.tab == Tab::Overview
                    && self.fix != FixState::Running
                    && self.runnable_fix().is_some() =>
            {
                self.layer = Layer::Confirm;
                self.fix = FixState::Asking;
                true
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l' | ']') => self.step_tab(1),
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h' | '[') => self.step_tab(-1),
            KeyCode::Char(c @ '1'..='6') => {
                let i = c as usize - '1' as usize;
                self.go_tab(Tab::ALL[i])
            }
            KeyCode::Down | KeyCode::Char('j') => self.select((self.selected + 1).min(last_row)),
            KeyCode::Up | KeyCode::Char('k') => self.select(self.selected.saturating_sub(1)),
            KeyCode::Char('g') | KeyCode::Home => self.select(0),
            KeyCode::Char('G') | KeyCode::End => self.select(last_row),
            KeyCode::Enter if self.tab == Tab::Overview => {
                self.go_tab(Tab::ALL[1 + self.selected.min(3)])
            }
            KeyCode::Enter if self.narrow() => {
                self.layer = Layer::Sheet;
                true
            }
            // Esc at the root has nothing to back out of, and never quits.
            _ => false,
        }
    }

    fn select(&mut self, row: usize) -> bool {
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
        match self.layer {
            Layer::Help => vec![("esc", "close")],
            Layer::Confirm => vec![("y", "run"), ("n", "cancel")],
            Layer::Sheet => vec![("esc", "back"), ("?", "help")],
            Layer::Root => {
                let mut v = Vec::new();
                if self.tab.rows() > 0 && !self.needs_setup() {
                    v.push(("j/k", "move"));
                }
                if !self.needs_setup()
                    && (self.tab == Tab::Overview || (narrow && self.tab.rows() > 0))
                {
                    v.push(("enter", "open"));
                }
                if self.tab == Tab::Overview
                    && self.fix != FixState::Running
                    && self.runnable_fix().is_some()
                {
                    v.push(("f", "fix"));
                }
                v.push(("h/l", "tab"));
                v.push(("?", "help"));
                v.push(("q", "quit"));
                v
            }
        }
    }
}
