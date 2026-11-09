//! TUI state and its single `update` function. Input and Daemon events
//! arrive as [`Msg`]s, and `update` says whether the screen changed. The only
//! I/O is [`Runner`] sending the commands the person asked for.

mod eth_keys;
mod hotspot_keys;
mod settings_keys;
mod vpn_keys;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent};
use nexus_ipc::{
    ActivityEntry, ActivityKind, ApList, Connectivity, EthernetList, Event, Finding, Hotspot, Op,
    PromptClosed, PromptKind, Secret, State, Status, Tethers, Traffic, Trust, VpnList, WifiList,
    WifiShare,
};

use super::aps::{Aps, Stage as ApsStage};
use super::caps::Caps;
use super::eap_form::{EapForm, Typed};
use super::ethernet::{self, EthernetTab};
use super::fx::{self, Fx};
use super::hidden::{HiddenForm, Typed as HiddenTyped};
use super::hits::Hit;
use super::hotspot::{self, HotspotTab};
use super::profile::{Action, Editor, ProfileMsg};
use super::settings::SettingsTab;
use super::share::{Share, Stage};
use super::tether::TetherTab;
use super::theme::{Look, Theme};
use super::vpn::{self, VpnTab};
use super::wifi::{Cmd, Confirm, Done, Entry, PasswordField, Runner, SWEEP_FRAME, Toast, WifiTab};

mod mouse;
mod palette;

pub use self::palette::Palette;

/// How long a toast stays when nobody answers it.
const TOAST_FOR: Duration = Duration::from_secs(6);

/// Everything that can happen to the TUI.
#[derive(Debug, Clone)]
pub enum Msg {
    Key(KeyEvent),
    /// A click or wheel notch (mouse capture is on in the real terminal).
    Mouse(MouseEvent),
    Resize(u16, u16),
    /// A fresh status from the Daemon (first connect, event or poll).
    Status(Status),
    /// A fresh Wi-Fi list (first connect or `networks_changed`).
    Wifi(WifiList),
    /// The VPN list (first connect or `vpns_changed`).
    Vpns(VpnList),
    /// A VPN command the person gave has finished.
    Vpn(Box<vpn::Done>),
    /// The Daemon's picture of Bluetooth and the paired phones.
    Tethers(Tethers),
    /// The Ethernet ports (first connect or `ethernet_changed`).
    Ethernet(EthernetList),
    /// An Ethernet command the person gave has finished.
    Wired(Box<ethernet::Done>),
    /// Whether this machine can host a Hotspot, and the one that runs.
    Hotspot(Hotspot),
    /// A Hotspot start or stop the person asked for has finished.
    HotspotDone(hotspot::Done),
    /// A Secret prompt opened or closed.
    Prompt(Event),
    /// A command the person gave has finished.
    Done(Done),
    /// The Daemon answered a reveal for the share view.
    Shared(Result<WifiShare, String>),
    /// The Daemon answered for the access point details.
    Aps(Result<ApList, String>),
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
    /// Something about the Profile settings editor.
    Profile(ProfileMsg),
    /// The Daemon's config (first connect, or `config_reloaded`), with the
    /// reason it is being ignored, if it is.
    Config {
        config: serde_json::Value,
        warning: Option<String>,
    },
    /// A setting the person changed was refused (`Err`) or taken.
    Setting(Result<(), String>),
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
            Tab::Settings => super::settings::ITEMS.len(),
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
    wifi: WifiTab,
    vpn: VpnTab,
    vpn_runner: Option<vpn::Runner>,
    tether: TetherTab,
    settings: SettingsTab,
    /// What the flags and terminal allowed before the config had a say.
    flag_caps: Caps,
    /// The environment asked for reduced motion.
    env_motion: bool,
    /// Why the configured theme is not in force, if it is not.
    theme_note: Option<String>,
    ethernet: EthernetTab,
    hotspot: HotspotTab,
    /// An open Secret prompt: it has the keys until answered or cancelled.
    field: Option<PasswordField>,
    confirm: Option<Confirm>,
    /// The 802.1X sign-in form: while open it has the keys.
    eap: Option<EapForm>,
    /// The hidden-network form: while open it has the keys.
    hidden: Option<HiddenForm>,
    /// The access point details of a Network, while open.
    aps: Option<Aps>,
    /// The Profile settings editor, while open: it has the keys.
    editor: Option<Editor>,
    /// The share view (QR code) of a saved Network.
    share: Option<Share>,
    toast: Option<Toast>,
    runner: Option<Runner>,
    /// The command palette, while open: it has the keys.
    palette: Option<Palette>,
    /// First help row shown (the overlay scrolls).
    help_scroll: usize,
    /// Event-driven effects (ADR-0003); empty, and so silent, when idle.
    fx: Fx,
    /// Clickable rectangles of the last frame (see `hits.rs`).
    hits: std::cell::RefCell<Vec<(ratatui::layout::Rect, Hit)>>,
}

impl App {
    pub fn new(caps: Caps) -> App {
        let look = Look::new(Theme::MIDNIGHT_BLOOM, caps);
        App {
            look,
            fx: Fx::new(&look),
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
            wifi: WifiTab::default(),
            vpn: VpnTab::default(),
            vpn_runner: None,
            tether: TetherTab::default(),
            settings: SettingsTab::default(),
            flag_caps: caps,
            env_motion: false,
            theme_note: None,
            ethernet: EthernetTab::default(),
            hotspot: HotspotTab::default(),
            field: None,
            confirm: None,
            eap: None,
            hidden: None,
            aps: None,
            editor: None,
            share: None,
            toast: None,
            runner: None,
            palette: None,
            help_scroll: 0,
            hits: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// Lets the app act on the Daemon: commands run on threads over `socket`
    /// and finish as [`Msg::Done`] on `tx`. Without it, commands go nowhere.
    pub fn attach(&mut self, socket: PathBuf, tx: Sender<Msg>) {
        self.vpn_runner = Some(vpn::Runner::new(socket.clone(), tx.clone()));
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
            Tab::Vpn => self.vpn.cursor(),
            Tab::Hotspot => self.tether.cursor(),
            Tab::Ethernet => self.ethernet.cursor(),
            _ => self.selected,
        }
    }

    /// How many rows the current tab's master list has.
    pub fn rows(&self) -> usize {
        match self.tab {
            Tab::WiFi => self.wifi.len(),
            Tab::Vpn => self.vpn.len(),
            Tab::Hotspot => self.tether.len(),
            Tab::Ethernet => self.ethernet.len(),
            other => other.rows(),
        }
    }

    pub fn vpn(&self) -> &VpnTab {
        &self.vpn
    }

    pub fn wifi(&self) -> &WifiTab {
        &self.wifi
    }

    pub fn settings(&self) -> &SettingsTab {
        &self.settings
    }

    pub fn tether(&self) -> &TetherTab {
        &self.tether
    }

    pub fn ethernet(&self) -> &EthernetTab {
        &self.ethernet
    }

    pub fn hotspot(&self) -> &HotspotTab {
        &self.hotspot
    }

    pub fn field(&self) -> Option<&PasswordField> {
        self.field.as_ref()
    }

    pub fn eap_form(&self) -> Option<&EapForm> {
        self.eap.as_ref()
    }

    pub fn hidden_form(&self) -> Option<&HiddenForm> {
        self.hidden.as_ref()
    }

    pub(super) fn aps(&self) -> Option<&Aps> {
        self.aps.as_ref()
    }

    pub fn editor(&self) -> Option<&Editor> {
        self.editor.as_ref()
    }

    pub fn confirm(&self) -> Option<&Confirm> {
        self.confirm.as_ref()
    }

    pub(super) fn share(&self) -> Option<&Share> {
        self.share.as_ref()
    }

    pub fn help_scroll(&self) -> usize {
        self.help_scroll
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

    /// The Network whose Portal wants a login, while the Link is behind one.
    pub fn portal(&self) -> Option<&str> {
        let status = self.status.as_ref().filter(|_| self.conn == Conn::Up)?;
        match (status.connectivity, &status.link) {
            (Connectivity::Portal, Some(link)) => Some(&link.name),
            _ => None,
        }
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

    /// When a running effect needs the next frame, if any: the scan sweep
    /// while scanning, the toast's end while one shows (ADR-0003).
    pub fn next_tick(&self) -> Option<Duration> {
        let sweep = self.wifi.scanning().then_some(SWEEP_FRAME);
        let countdown = self
            .editor
            .as_ref()
            .is_some_and(Editor::is_pending)
            .then_some(Duration::from_secs(1));
        let sweep = match (sweep, countdown) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let toast = self
            .toast
            .as_ref()
            .map(|t| t.until.saturating_duration_since(Instant::now()));
        let wait = match (sweep, toast) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        // A running effect wants its next frame; once the last one ends this
        // is None again and the loop sleeps (ADR-0003).
        match (wait, self.fx.running().then_some(fx::FRAME)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Whether an effect is still running (frames are being asked for).
    pub fn animating(&self) -> bool {
        self.fx.running()
    }

    /// How much motion this App allows (off on 16 colours and below).
    pub fn movement(&self) -> super::fx::Movement {
        self.fx.motion()
    }

    /// Reduced motion: no gliding, sliding, shaking or pulsing; the colour
    /// changes that say what happened stay.
    pub fn reduce_motion(&mut self, on: bool) {
        self.env_motion = on;
        self.fx.reduce(self.env_motion || self.settings_motion());
    }

    /// Runs the effects over the frame just drawn into `buf`, advancing them
    /// by the real time since the last frame.
    pub fn fx_frame(&mut self, buf: &mut ratatui::buffer::Buffer) {
        let step = self.fx.step_from_clock();
        self.fx.process(step, buf);
    }

    /// Like [`App::fx_frame`] with the clock in the caller's hands (tests).
    pub fn fx_step(&mut self, buf: &mut ratatui::buffer::Buffer, step: Duration) {
        self.fx.process(step, buf);
    }

    fn layout(&self) -> super::layout::Areas {
        super::layout::areas(ratatui::layout::Rect::new(0, 0, self.size.0, self.size.1))
    }

    /// The room between the header and the footer.
    fn body_rect(&self) -> ratatui::layout::Rect {
        let a = self.layout();
        ratatui::layout::Rect {
            y: a.header.bottom(),
            height: a.footer.y.saturating_sub(a.header.bottom()),
            ..a.header
        }
    }

    /// Starts or stops the connecting pulse on what the person is looking at:
    /// the header's status, and the hero on the Overview.
    fn sync_pulse(&mut self) {
        let connecting = self
            .status
            .as_ref()
            .is_some_and(|s| s.state == State::Connecting);
        let a = self.layout();
        let mut focal = vec![ratatui::layout::Rect {
            x: a.header.x + a.header.width / 2,
            width: a.header.width - a.header.width / 2,
            ..a.header
        }];
        if self.tab == Tab::Overview
            && let Some(status) = &self.status
        {
            focal.push(super::overview::hero_rect(status, &a, &self.look));
        }
        self.fx.connecting(connecting, &focal);
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
                let online = entry.kind == ActivityKind::Online;
                if online {
                    self.say("You're online.", false, None);
                    self.bloom();
                }
                self.activity.insert(0, entry);
                self.activity.truncate(ACTIVITY);
                online || self.tab == Tab::Overview
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
            Msg::Mouse(ev) => self.mouse(ev),
            Msg::Resize(w, h) => {
                self.size = (w, h);
                // Every effect area is stale now; the pulse restarts below.
                self.fx.clear();
                self.sync_pulse();
                if !self.narrow() && self.layer == Layer::Sheet {
                    self.layer = Layer::Root;
                }
                true
            }
            Msg::Status(status) => {
                let changed = self.conn != Conn::Up || self.status.as_ref() != Some(&status);
                let was = self
                    .status
                    .as_ref()
                    .is_some_and(|s| s.state == State::Connecting);
                self.conn = Conn::Up;
                self.status = Some(status);
                if was
                    != self
                        .status
                        .as_ref()
                        .is_some_and(|s| s.state == State::Connecting)
                {
                    self.sync_pulse();
                }
                changed
            }
            Msg::Wifi(list) => self.wifi.refresh(list),
            Msg::Vpns(list) => self.vpn.refresh(list) && self.tab == Tab::Vpn,
            Msg::Vpn(done) => self.vpn_done(*done),
            Msg::Tethers(tethers) => self.tether.refresh(tethers),
            Msg::Config { config, warning } => self.apply_config(config, warning),
            Msg::Setting(Err(message)) => {
                self.say(message, true, None);
                true
            }
            Msg::Setting(Ok(())) => false,
            Msg::Ethernet(list) => self.ethernet.refresh(list),
            Msg::Wired(done) => self.eth_done(*done),
            Msg::Hotspot(info) => {
                let changed = self.hotspot.refresh(info);
                // The details of a Hotspot that went away are not worth showing.
                let gone = self.share.as_ref().is_some_and(|s| s.hotspot) && !self.hotspot.on();
                if gone {
                    self.share = None;
                }
                changed || gone
            }
            Msg::HotspotDone(done) => self.hotspot_done(done),
            Msg::Prompt(event) => self.prompt_event(event),
            Msg::Done(done) => self.done(done),
            Msg::Profile(m) => self.profile_msg(m),
            Msg::Aps(result) => match self.aps.as_mut() {
                Some(aps) if matches!(aps.stage, ApsStage::Loading) => {
                    aps.stage = match result {
                        Ok(list) => ApsStage::Shown(list),
                        Err(message) => ApsStage::Failed(message),
                    };
                    true
                }
                _ => false, // closed meanwhile
            },
            Msg::Shared(result) => match self.share.as_mut() {
                Some(share) if matches!(share.stage, Stage::Reading) => {
                    share.arrived(result);
                    true
                }
                _ => false, // closed meanwhile: the details drop here, wiped
            },
            Msg::Down => {
                // A prompt dies with the link; the Daemon replays it if it lives.
                let mut changed = self.conn != Conn::Down
                    || self.field.take().is_some()
                    || self.share.take().is_some()
                    || self.vpn.drop_secrets();
                match self.editor.take() {
                    Some(e) if e.is_pending() => {
                        self.say(
                            "Lost the daemon. NetworkManager reverts the change on its own.",
                            true,
                            None,
                        );
                        changed = true;
                    }
                    Some(_) => changed = true,
                    None => {}
                }
                self.conn = Conn::Down;
                changed
            }
            Msg::Tick => {
                // A running effect needs its next frame.
                let mut dirty = self.fx.running() | self.wifi.tick();
                if let Some(editor) = self.editor.as_mut() {
                    dirty |= editor.tick(Instant::now());
                }
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
        if let Some(toast) = &self.toast {
            let body = self.body_rect();
            if let Some(rect) = super::wifi_ui::toast_rect(toast, body) {
                self.fx.toast(rect, toast.bad, body);
            }
        }
    }

    /// A joined Network blooms across the body.
    fn bloom(&mut self) {
        let body = self.body_rect();
        self.fx.joined(body);
    }

    fn prompt_event(&mut self, event: Event) -> bool {
        match event {
            Event::SecretPromptOpened { prompt }
                if matches!(
                    prompt.kind,
                    PromptKind::WifiPassword | PromptKind::VpnSecret
                ) =>
            {
                self.palette = None;
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
                    Ok(_) => {
                        self.say(format!("Joined {ssid}."), false, None);
                        self.bloom();
                    }
                    Err(message) => self.say(message, true, None),
                }
            }
            (
                Cmd::JoinHidden {
                    ssid,
                    security,
                    password,
                    ..
                },
                result,
            ) => {
                self.wifi.busy = None;
                match result {
                    Ok(_) => {
                        self.say(format!("Joined {ssid}."), false, None);
                        self.bloom();
                    }
                    // Back to the form, nothing typed lost, to try again.
                    Err(message) => {
                        self.hidden = Some(HiddenForm::reopened(
                            ssid.clone(),
                            *security,
                            password.as_ref(),
                            message,
                        ));
                    }
                }
            }
            (
                Cmd::JoinEap {
                    network, ssid, eap, ..
                },
                result,
            ) => {
                self.wifi.busy = None;
                match result {
                    Ok(_) => {
                        self.say(format!("Joined {ssid}."), false, None);
                        self.bloom();
                    }
                    // Back to the form, nothing typed lost, to try again.
                    Err(message) => {
                        self.eap = Some(EapForm::reopened(
                            network.clone(),
                            ssid.clone(),
                            eap,
                            message,
                        ));
                    }
                }
            }
            (Cmd::Disconnect { network, ssid }, Ok(_)) => {
                let undo = Cmd::Join {
                    network: network.clone(),
                    ssid: ssid.clone(),
                    device: None,
                };
                self.say(format!("Disconnected from {ssid}."), false, Some(undo));
            }
            (Cmd::Tether { phone, name }, result) => {
                self.tether.busy = None;
                match result {
                    Ok(_) => {
                        let undo = Cmd::Untether {
                            phone: phone.clone(),
                            name: name.clone(),
                        };
                        self.say(format!("Tethered through {name}."), false, Some(undo));
                    }
                    Err(message) => self.say(message, true, None),
                }
            }
            (Cmd::Untether { phone, name }, Ok(_)) => {
                let undo = Cmd::Tether {
                    phone: phone.clone(),
                    name: name.clone(),
                };
                self.say(format!("Dropped the tether to {name}."), false, Some(undo));
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
            (
                Cmd::Trust {
                    network,
                    ssid,
                    trust,
                },
                Ok(_),
            ) => {
                let other = match trust {
                    Trust::Untrusted => Trust::Trusted,
                    _ => Trust::Untrusted,
                };
                let undo = Cmd::Trust {
                    network: network.clone(),
                    ssid: ssid.clone(),
                    trust: other,
                };
                let text = match trust {
                    Trust::Untrusted => format!("{ssid} is now untrusted."),
                    _ => format!("{ssid} is now trusted."),
                };
                self.say(text, false, Some(undo));
            }
            (Cmd::OpenPortal, Ok(_)) => {
                self.say("Opened the login page in your browser.", false, None);
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
        if let Some(form) = self.eap.as_mut() {
            match form.key(key) {
                Typed::Editing => {}
                Typed::Cancel => self.eap = None,
                Typed::Submit(eap) => {
                    if let Some(form) = self.eap.take() {
                        if form.wired {
                            self.run_eth(ethernet::Cmd::SignIn {
                                port: form.network.clone(),
                                eap,
                            });
                        } else {
                            let (network, ssid) = (form.network.clone(), form.name.clone());
                            self.run_wifi(Cmd::JoinEap { network, ssid, eap });
                        }
                    }
                }
            }
            return true;
        }
        if let Some(form) = self.hidden.as_mut() {
            match form.key(key) {
                HiddenTyped::Editing => {}
                HiddenTyped::Cancel => self.hidden = None,
                HiddenTyped::Submit {
                    name,
                    security,
                    password,
                } => {
                    self.hidden = None;
                    let device = self.wifi.card().map(str::to_owned);
                    self.run_wifi(Cmd::JoinHidden {
                        ssid: name,
                        security,
                        password,
                        device,
                    });
                }
            }
            return true;
        }
        if self.aps.is_some() {
            return match key.code {
                KeyCode::Esc | KeyCode::Char('i' | 'q') | KeyCode::Enter => {
                    self.aps = None;
                    true
                }
                _ => false,
            };
        }
        if self.field.is_some() {
            return self.field_key(key);
        }
        if self.confirm.is_some() {
            return self.confirm_key(key);
        }
        if self.editor.is_some() {
            return self.editor_key(key);
        }

        if self.hotspot.form.is_some() {
            return self.hotspot_form_key(key);
        }
        if self.share.is_some() {
            return self.share_key(key);
        }
        if self.tab == Tab::Vpn && self.vpn.layer.is_some() {
            return self.vpn_layer_key(key);
        }
        if self.palette.is_some() {
            return self.palette_key(key);
        }
        match self.layer {
            Layer::Help => self.help_key(key),
            Layer::Sheet => match key.code {
                KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                    self.layer = Layer::Root;
                    true
                }
                KeyCode::Char('?') => self.open_help(),
                KeyCode::Char(':') => self.open_palette(),
                _ => self.tab_key(key),
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

    fn open_help(&mut self) -> bool {
        self.layer = Layer::Help;
        self.help_scroll = 0;
        true
    }

    /// The help overlay: close it, or scroll it.
    fn help_key(&mut self, key: KeyEvent) -> bool {
        let page = super::overlays::help_page(self.size) as isize;
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') => {
                self.layer = Layer::Root;
                true
            }
            KeyCode::Down | KeyCode::Char('j') => self.scroll_help(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_help(-1),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_help(page),
            KeyCode::PageUp => self.scroll_help(-page),
            KeyCode::Home | KeyCode::Char('g') => self.scroll_help(isize::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => self.scroll_help(isize::MAX / 2),
            _ => false,
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

    /// The share view: Enter reveals, Esc drops everything.
    fn share_key(&mut self, key: KeyEvent) -> bool {
        let Some(share) = self.share.as_mut() else {
            return false;
        };
        match (&mut share.stage, key.code) {
            (_, KeyCode::Esc) => self.share = None,
            (Stage::Ask | Stage::Failed(_), KeyCode::Enter) => {
                share.stage = Stage::Reading;
                if let Some(runner) = &self.runner {
                    if share.hotspot {
                        runner.reveal_hotspot();
                    } else {
                        runner.reveal(share.network.clone());
                    }
                }
            }
            (
                Stage::Shown {
                    letters, share: s, ..
                },
                KeyCode::Char('p'),
            ) if s.password.is_some() => {
                *letters = !*letters;
            }
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
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Char(':') => self.open_palette(),
            KeyCode::Char('f')
                if self.tab == Tab::Overview
                    && self.fix != FixState::Running
                    && self.runnable_fix().is_some() =>
            {
                self.layer = Layer::Confirm;
                self.fix = FixState::Asking;
                true
            }
            KeyCode::Char('o') if self.portal().is_some() => {
                self.send(Cmd::OpenPortal);
                false
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
            KeyCode::Enter if self.tab == Tab::Overview => {
                self.go_tab(Tab::ALL[1 + self.selected.min(3)])
            }
            KeyCode::Enter
                if self.narrow()
                    && !matches!(
                        self.tab,
                        Tab::WiFi | Tab::Ethernet | Tab::Vpn | Tab::Hotspot | Tab::Settings
                    ) =>
            {
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
        self.keys_of(self.tab, key)
    }

    /// Presses `key` as `tab`'s handler sees it. The one place that maps a
    /// tab to its handler, so the palette cannot miss a tab: a new tab is a
    /// compile error here, not a Verb that quietly does nothing.
    pub(super) fn keys_of(&mut self, tab: Tab, key: KeyEvent) -> bool {
        match tab {
            Tab::WiFi => self.wifi_key(key),
            Tab::Vpn => self.vpn_key(key),
            Tab::Ethernet => self.eth_key(key),
            Tab::Hotspot => self.hotspot_tab_key(key),
            Tab::Settings => self.settings_key(key),
            Tab::Overview => false,
        }
    }

    /// Keys of the Tethering list: enter tethers, x drops it, u undoes.
    fn tether_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Char('u') {
            return match self.toast.take().and_then(|t| t.undo) {
                Some(undo) => {
                    self.run_tether(undo);
                    true
                }
                None => false,
            };
        }
        let Some(phone) = self.tether.selected().cloned() else {
            return false;
        };
        let (id, name) = (phone.id.clone(), phone.name.clone());
        match key.code {
            KeyCode::Enter if !phone.connected && self.tether.busy.is_none() => {
                self.run_tether(Cmd::Tether { phone: id, name });
                true
            }
            KeyCode::Char('x') if phone.connected => {
                self.run_tether(Cmd::Untether { phone: id, name });
                true
            }
            _ => false,
        }
    }

    /// Sends a Tethering command, marking a connect as in flight.
    fn run_tether(&mut self, cmd: Cmd) {
        if let Cmd::Tether { phone, .. } = &cmd {
            self.tether.busy = Some(phone.clone());
        }
        self.toast = None;
        self.send(cmd);
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
            KeyCode::Char('n') => {
                self.toast = None;
                self.hidden = Some(HiddenForm::new());
                true
            }
            KeyCode::Char('c') => self.wifi.cycle_card(),
            _ => {
                let Some(entry) = self.wifi.selected().cloned() else {
                    return false;
                };
                let (network, ssid) = (entry.network_id(), entry.name().to_owned());
                match key.code {
                    KeyCode::Enter
                        if entry.enterprise() && !entry.saved() && self.wifi.busy.is_none() =>
                    {
                        self.eap = Some(EapForm::new(network, ssid));
                        true
                    }
                    KeyCode::Enter if !entry.joined() && self.wifi.busy.is_none() => {
                        let device = self.wifi.card().map(str::to_owned);
                        self.run_wifi(Cmd::Join {
                            network,
                            ssid,
                            device,
                        });
                        true
                    }
                    KeyCode::Char('i') if matches!(entry, Entry::Net(_)) => {
                        self.toast = None;
                        self.aps = Some(Aps::loading(ssid));
                        if let Some(runner) = &self.runner {
                            runner.call(Op::WifiAps { network }, |r| {
                                Msg::Aps(r.and_then(|v| {
                                    serde_json::from_value(v).map_err(|_| {
                                        "The daemon sent details Nexus cannot read.".to_string()
                                    })
                                }))
                            });
                        }
                        true
                    }
                    KeyCode::Char('x') if entry.joined() => {
                        self.run_wifi(Cmd::Disconnect { network, ssid });
                        true
                    }
                    KeyCode::Char('s') if entry.saved() => {
                        self.toast = None;
                        self.share = Some(Share::ask(network, ssid));
                        true
                    }
                    KeyCode::Char('f') if entry.saved() => {
                        self.confirm = Some(Confirm { network, ssid });
                        true
                    }
                    KeyCode::Char('e') if entry.saved() => {
                        self.open_editor(network, ssid);
                        true
                    }
                    KeyCode::Char('t') if entry.trust().is_some() => {
                        let trust = match entry.trust() {
                            Some(Trust::Untrusted) => Trust::Trusted,
                            _ => Trust::Untrusted,
                        };
                        self.run_wifi(Cmd::Trust {
                            network,
                            ssid,
                            trust,
                        });
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

    fn open_editor(&mut self, network: String, ssid: String) {
        self.start_editor(Editor::loading(network, ssid));
    }

    /// Opens the Profile editor on `editor` and asks the Daemon for its Profile.
    fn start_editor(&mut self, editor: Editor) {
        self.toast = None;
        let op = editor.get_op();
        self.editor = Some(editor);
        if let Some(runner) = &self.runner {
            runner.call(op, |r| {
                Msg::Profile(ProfileMsg::Loaded(r.and_then(|v| {
                    serde_json::from_value(v)
                        .map_err(|_| "The daemon sent a Profile this nexus cannot read.".into())
                })))
            });
        }
    }

    /// Keys while the Profile editor is open.
    fn editor_key(&mut self, key: KeyEvent) -> bool {
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        match editor.key(key) {
            Action::None | Action::Idle => false,
            Action::Redraw => true,
            Action::Close => {
                self.editor = None;
                true
            }
            Action::Apply => {
                if let Some(op) = editor.update_op() {
                    editor.applying();
                    if let Some(runner) = &self.runner {
                        runner.call(op, |r| {
                            Msg::Profile(ProfileMsg::Applied(r.and_then(|v| {
                                serde_json::from_value(v).map_err(|_| {
                                    "The daemon's answer was not understood.".to_string()
                                })
                            })))
                        });
                    }
                }
                true
            }
            Action::Confirm(pending) => {
                self.answer_pending(Op::ProfileConfirm { pending });
                true
            }
            Action::Revert(pending) => {
                self.answer_pending(Op::ProfileRevert { pending });
                true
            }
        }
    }

    fn answer_pending(&self, op: Op) {
        if let Some(runner) = &self.runner {
            runner.call(op, |r| match r {
                // Success arrives as the `profile_settled` event.
                Ok(_) => Msg::Tick,
                Err(m) => Msg::Profile(ProfileMsg::Refused(m)),
            });
        }
    }

    fn profile_msg(&mut self, msg: ProfileMsg) -> bool {
        match msg {
            ProfileMsg::Loaded(result) => {
                let Some(editor) = self.editor.as_mut() else {
                    return false;
                };
                if let Err(message) = &result {
                    let message = message.clone();
                    self.editor = None;
                    self.say(message, true, None);
                    return true;
                }
                editor.load(result);
                true
            }
            ProfileMsg::Applied(result) => {
                let Some(editor) = self.editor.as_mut() else {
                    return false;
                };
                match result {
                    Err(message) => editor.applied_failed(message),
                    Ok(applied) => match applied.pending {
                        Some(change) => editor.pending(&change, Instant::now()),
                        None => {
                            let ssid = editor.ssid.clone();
                            self.editor = None;
                            self.say(format!("Saved the settings of {ssid}."), false, None);
                        }
                    },
                }
                true
            }
            ProfileMsg::Refused(message) => {
                // Too late to keep it, say: the settled event follows.
                self.say(message, true, None);
                true
            }
            ProfileMsg::Event(Event::ProfilePending { change }) => {
                // Our own update answers with the same change; this covers
                // the event winning the race against the answer.
                match self.editor.as_mut() {
                    Some(e) if e.ssid == change.ssid && !e.is_pending() => {
                        e.pending(&change, Instant::now());
                        true
                    }
                    _ => false,
                }
            }
            ProfileMsg::Event(Event::ProfileSettled { id, outcome, .. }) => {
                let Some(editor) = self.editor.as_ref().filter(|e| e.settled(id)) else {
                    return false;
                };
                let (text, bad) = editor.outcome_text(outcome);
                self.editor = None;
                self.say(text, bad, None);
                true
            }
            ProfileMsg::Event(_) => false,
        }
    }

    /// Sends a Wi-Fi command, marking a join as in flight.
    fn run_wifi(&mut self, cmd: Cmd) {
        if let Cmd::Join { ssid, .. } | Cmd::JoinEap { ssid, .. } | Cmd::JoinHidden { ssid, .. } =
            &cmd
        {
            self.wifi.busy = Some(ssid.clone());
        }
        self.toast = None;
        self.send(cmd);
    }

    fn select(&mut self, row: usize) -> bool {
        if self.tab == Tab::WiFi {
            return self.wifi.move_to(row);
        }
        if self.tab == Tab::Vpn {
            return self.vpn.move_to(row);
        }
        if self.tab == Tab::Hotspot {
            return self.tether.move_to(row);
        }
        if self.tab == Tab::Ethernet {
            return self.ethernet.move_to(row);
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
        if changed {
            self.fx.tab_changed(self.body_rect());
            self.sync_pulse();
        }
        if changed && tab == Tab::Hotspot {
            // A card plugged in or a radio switched is not pushed: look again.
            if let Some(runner) = &self.runner {
                runner.call(Op::Hotspot, |r| match r.map(serde_json::from_value) {
                    Ok(Ok(info)) => Msg::Hotspot(info),
                    _ => Msg::Tick,
                });
            }
        }
        changed
    }

    /// The keys that work right now, as `(key, what it does)`, for the footer.
    pub fn hints(&self, narrow: bool) -> Vec<(&'static str, &'static str)> {
        if self.palette.is_some() {
            return vec![("enter", "run"), ("esc", "close")];
        }
        if self.eap.is_some() || self.hidden.is_some() {
            return vec![("tab", "next"), ("enter", "join"), ("esc", "cancel")];
        }
        if self.aps.is_some() {
            return vec![("esc", "close")];
        }
        if self.hotspot.form.is_some() {
            return vec![
                ("tab", "next"),
                ("enter", "start"),
                ("ctrl-r", "show"),
                ("esc", "cancel"),
            ];
        }
        if let Some(field) = &self.field {
            let go = if field.prompt.kind == PromptKind::VpnSecret {
                "send"
            } else {
                "join"
            };
            return vec![("enter", go), ("ctrl-r", "show"), ("esc", "cancel")];
        }
        if self.confirm.is_some() {
            return vec![("y", "forget"), ("esc", "keep")];
        }
        if let Some(editor) = &self.editor {
            return editor_hints(editor);
        }

        if let Some(share) = &self.share {
            return share.hints();
        }
        if self.tab == Tab::Vpn && self.vpn.layer.is_some() {
            return self.vpn_hints(false);
        }
        match self.layer {
            Layer::Help => vec![("j/k", "scroll"), ("esc", "close")],
            Layer::Confirm => vec![("y", "run"), ("n", "cancel")],
            Layer::Sheet => {
                let mut v = vec![("esc", "back")];
                if self.tab == Tab::WiFi {
                    v.extend(self.wifi_hints(false));
                } else if self.tab == Tab::Vpn {
                    v.extend(self.vpn_hints(false));
                } else if self.tab == Tab::Hotspot {
                    v.extend(self.tether_hints());
                } else if self.tab == Tab::Settings {
                    v.extend(self.settings_hints());
                } else if self.tab == Tab::Ethernet {
                    v.extend(self.eth_hints());
                }
                v.push(("?", "help"));
                v
            }
            Layer::Root => {
                let mut v = Vec::new();
                if self.rows() > 0 && !self.needs_setup() {
                    v.push(("j/k", "move"));
                }
                if self.needs_setup() {
                    // No list and nothing to open on the setup screen.
                } else if self.tab == Tab::WiFi {
                    v.extend(self.wifi_hints(!narrow));
                } else if self.tab == Tab::Vpn {
                    v.extend(self.vpn_hints(!narrow));
                } else if self.tab == Tab::Settings {
                    v.extend(self.settings_hints());
                    if narrow && self.rows() > 0 {
                        v.push(("d", "details"));
                    }
                } else if self.tab == Tab::Hotspot {
                    v.extend(self.tether_hints());
                    if narrow && self.rows() > 0 {
                        v.push(("d", "details"));
                    }
                } else if self.tab == Tab::Ethernet {
                    v.extend(self.eth_hints());
                    if narrow && self.rows() > 0 {
                        v.push(("d", "details"));
                    }
                } else if self.tab == Tab::Overview || (narrow && self.rows() > 0) {
                    v.push(("enter", "open"));
                }
                if self.tab == Tab::Overview
                    && self.fix != FixState::Running
                    && self.runnable_fix().is_some()
                {
                    v.push(("f", "fix"));
                }
                if self.portal().is_some() {
                    v.push(("o", "login"));
                }
                v.push(("h/l", "tab"));
                v.push((":", "palette"));
                v.push(("?", "help"));
                v.push(("q", "quit"));
                v
            }
        }
    }

    /// Hotspot keys, then the Tethering keys of the selected phone.
    fn tether_hints(&self) -> Vec<(&'static str, &'static str)> {
        let mut v = Vec::new();
        if self.hotspot.on() {
            if self.hotspot.ready() {
                v.push(("s", "hotspot QR"));
            }
            v.push(("S", "stop hotspot"));
        } else if self.hotspot.available() {
            v.push(("n", "hotspot"));
            v.push(("c", "custom"));
        }
        if let Some(phone) = self.tether.selected() {
            if phone.connected {
                v.push(("x", "drop tether"));
            } else {
                v.push(("enter", "tether"));
            }
        }
        if self.toast.as_ref().is_some_and(|t| t.undo.is_some()) {
            v.push(("u", "undo"));
        }
        v
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
            if e.saved() {
                v.push(("s", "share"));
            }
            if e.autoconnect().is_some() {
                v.push(("a", "auto"));
            }
            if e.trust().is_some() {
                v.push(("t", "trust"));
            }
            if e.saved() {
                v.push(("e", "settings"));
            }
        }
        if more {
            v.push(("n", "other"));
            if !self.wifi.cards().is_empty() {
                v.push(("c", "card"));
            }
        }
        if self.toast.as_ref().is_some_and(|t| t.undo.is_some()) {
            v.push(("u", "undo"));
        }
        v
    }
}

/// Footer keys of the Profile editor.
fn editor_hints(editor: &Editor) -> Vec<(&'static str, &'static str)> {
    use super::profile::Phase;
    match editor.phase {
        Phase::Loading | Phase::Applying => vec![("esc", "close")],
        Phase::Pending { .. } => vec![("y", "keep"), ("n", "revert now")],
        Phase::Editing => {
            let mut v = vec![("tab", "next")];
            if editor.focused().is_text() {
                v.push(("type", "edit"));
            } else {
                v.push(("space", "change"));
            }
            v.push(("enter", "apply"));
            v.push(("esc", "cancel"));
            v
        }
    }
}
