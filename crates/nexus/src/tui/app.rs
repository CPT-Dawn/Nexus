//! TUI state and its single `update` function. No I/O here: input and Daemon
//! events arrive as [`Msg`]s, and `update` says whether the screen changed.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use nexus_ipc::Status;

use super::caps::Caps;
use super::theme::{Look, Theme};

/// Everything that can happen to the TUI.
#[derive(Debug, Clone)]
pub enum Msg {
    Key(KeyEvent),
    Resize(u16, u16),
    /// A fresh status from the Daemon (first connect, event or poll).
    Status(Status),
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
            Layer::Sheet => vec![("esc", "back"), ("?", "help")],
            Layer::Root => {
                let mut v = Vec::new();
                if self.tab.rows() > 0 {
                    v.push(("j/k", "move"));
                }
                if narrow && self.tab.rows() > 0 {
                    v.push(("enter", "open"));
                }
                v.push(("h/l", "tab"));
                v.push(("?", "help"));
                v.push(("q", "quit"));
                v
            }
        }
    }
}
