//! State of the Profile settings editor (Wi-Fi tab, `e`): the form, its inline
//! validation and the "Keep these settings?" countdown. No I/O here: `App`
//! turns the keys into Daemon ops through the [`Runner`](super::wifi::Runner).

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_ipc::{
    Event, Field, Ipv4Method, Ipv6Method, MacMode, PendingChange, ProfileApplied, ProfileSettings,
    ProfileView, Settled, profile,
};

/// What comes back from the Daemon for the editor.
#[derive(Debug, Clone)]
pub enum ProfileMsg {
    /// `profile_get` answered.
    Loaded(Result<ProfileView, String>),
    /// `profile_update` answered.
    Applied(Result<ProfileApplied, String>),
    /// `profile_confirm` or `profile_revert` failed (success shows as an event).
    Refused(String),
    /// `profile_pending` or `profile_settled`.
    Event(Event),
}

/// One line of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Ipv4,
    Ipv4Address,
    Ipv4Gateway,
    Ipv6,
    Ipv6Address,
    Ipv6Gateway,
    Dns,
    Mac,
    Metered,
    Priority,
}

impl Row {
    pub fn label(self) -> &'static str {
        match self {
            Row::Ipv4 => "IPv4",
            Row::Ipv4Address => "  address",
            Row::Ipv4Gateway => "  gateway",
            Row::Ipv6 => "IPv6",
            Row::Ipv6Address => "  address",
            Row::Ipv6Gateway => "  gateway",
            Row::Dns => "DNS",
            Row::Mac => "MAC address",
            Row::Metered => "Metered",
            Row::Priority => "Priority",
        }
    }

    /// Typed into, as opposed to cycled with the arrows.
    pub fn is_text(self) -> bool {
        matches!(
            self,
            Row::Ipv4Address
                | Row::Ipv4Gateway
                | Row::Ipv6Address
                | Row::Ipv6Gateway
                | Row::Dns
                | Row::Priority
        )
    }

    fn field(self) -> Option<Field> {
        Some(match self {
            Row::Ipv4Address => Field::Ipv4Address,
            Row::Ipv4Gateway => Field::Ipv4Gateway,
            Row::Ipv6Address => Field::Ipv6Address,
            Row::Ipv6Gateway => Field::Ipv6Gateway,
            Row::Dns => Field::Dns,
            Row::Priority => Field::Priority,
            _ => return None,
        })
    }
}

/// Where the editor is in its life.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for `profile_get`.
    Loading,
    Editing,
    /// `profile_update` is in flight.
    Applying,
    /// Applied; waiting for "keep" or the clock.
    Pending {
        id: u64,
        deadline: Instant,
        secs_left: u32,
        /// The answer was sent; the outcome comes as an event.
        answered: bool,
    },
}

/// What the keys asked of the app.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// Nothing visible changed.
    Idle,
    Redraw,
    Close,
    Apply,
    Confirm(u64),
    Revert(u64),
}

#[derive(Debug, Clone)]
pub struct Editor {
    pub network: String,
    pub ssid: String,
    pub phase: Phase,
    /// The Profile as loaded; its version goes back with the update.
    view: Option<ProfileView>,
    focus: usize,
    ipv4: Ipv4Method,
    ipv4_address: String,
    ipv4_gateway: String,
    ipv6: Ipv6Method,
    ipv6_address: String,
    ipv6_gateway: String,
    dns: String,
    mac: MacMode,
    metered: bool,
    priority: String,
    /// The Daemon's last complaint (a conflict, say), shown in the form.
    pub message: Option<String>,
}

impl Editor {
    pub fn loading(network: String, ssid: String) -> Self {
        Self {
            network,
            ssid,
            phase: Phase::Loading,
            view: None,
            focus: 0,
            ipv4: Ipv4Method::Auto,
            ipv4_address: String::new(),
            ipv4_gateway: String::new(),
            ipv6: Ipv6Method::Auto,
            ipv6_address: String::new(),
            ipv6_gateway: String::new(),
            dns: String::new(),
            mac: MacMode::Permanent,
            metered: false,
            priority: "0".into(),
            message: None,
        }
    }

    pub fn view(&self) -> Option<&ProfileView> {
        self.view.as_ref()
    }

    pub fn active(&self) -> bool {
        self.view.as_ref().is_some_and(|v| v.active)
    }

    /// Fills the form from the Daemon's answer.
    pub fn load(&mut self, result: Result<ProfileView, String>) {
        match result {
            Err(message) => self.message = Some(message),
            Ok(view) => {
                let s = &view.settings;
                self.ipv4 = s.ipv4.method;
                self.ipv4_address = profile::addresses_text(&s.ipv4.addresses);
                self.ipv4_gateway = s.ipv4.gateway.clone().unwrap_or_default();
                self.ipv6 = s.ipv6.method;
                self.ipv6_address = profile::addresses_text(&s.ipv6.addresses);
                self.ipv6_gateway = s.ipv6.gateway.clone().unwrap_or_default();
                self.dns = s.dns.join(", ");
                self.mac = s.mac;
                self.metered = s.metered;
                self.priority = s.priority.to_string();
                self.view = Some(view);
                self.phase = Phase::Editing;
                self.message = None;
            }
        }
    }

    /// The lines on screen, in order.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Ipv4];
        if self.ipv4 == Ipv4Method::Manual {
            rows.extend([Row::Ipv4Address, Row::Ipv4Gateway]);
        }
        rows.push(Row::Ipv6);
        if self.ipv6 == Ipv6Method::Manual {
            rows.extend([Row::Ipv6Address, Row::Ipv6Gateway]);
        }
        rows.extend([Row::Dns, Row::Mac, Row::Metered, Row::Priority]);
        rows
    }

    pub fn focused(&self) -> Row {
        let rows = self.rows();
        rows[self.focus.min(rows.len() - 1)]
    }

    pub fn focus_index(&self) -> usize {
        self.focus.min(self.rows().len() - 1)
    }

    pub fn text_of(&self, row: Row) -> &str {
        match row {
            Row::Ipv4Address => &self.ipv4_address,
            Row::Ipv4Gateway => &self.ipv4_gateway,
            Row::Ipv6Address => &self.ipv6_address,
            Row::Ipv6Gateway => &self.ipv6_gateway,
            Row::Dns => &self.dns,
            Row::Priority => &self.priority,
            _ => "",
        }
    }

    /// The words shown for a row that is cycled rather than typed.
    pub fn choice_of(&self, row: Row) -> &'static str {
        match row {
            Row::Ipv4 => match self.ipv4 {
                Ipv4Method::Auto => "automatic (DHCP)",
                Ipv4Method::Manual => "static",
                _ => "set up outside Nexus",
            },
            Row::Ipv6 => match self.ipv6 {
                Ipv6Method::Auto => "automatic",
                Ipv6Method::Manual => "static",
                Ipv6Method::Disabled => "disabled",
                _ => "set up outside Nexus",
            },
            Row::Mac => match self.mac {
                MacMode::PerNetwork => "random per Network",
                MacMode::EveryTime => "random every time",
                MacMode::Permanent => "permanent (hardware)",
                _ => "set up outside Nexus",
            },
            Row::Metered => {
                if self.metered {
                    "yes"
                } else {
                    "no"
                }
            }
            _ => "",
        }
    }

    fn text_mut(&mut self, row: Row) -> Option<&mut String> {
        Some(match row {
            Row::Ipv4Address => &mut self.ipv4_address,
            Row::Ipv4Gateway => &mut self.ipv4_gateway,
            Row::Ipv6Address => &mut self.ipv6_address,
            Row::Ipv6Gateway => &mut self.ipv6_gateway,
            Row::Dns => &mut self.dns,
            Row::Priority => &mut self.priority,
            _ => return None,
        })
    }

    /// The settings the form says, or what is wrong with them, row by row.
    /// Rows hidden behind "automatic" are not checked.
    pub fn build(&self) -> Result<ProfileSettings, Vec<(Row, String)>> {
        let mut problems: Vec<(Row, String)> = Vec::new();
        let base = self.view.as_ref().map(|v| v.settings.clone());
        let mut s = base.unwrap_or_default();
        s.ipv4.method = self.ipv4;
        s.ipv6.method = self.ipv6;
        s.mac = self.mac;
        s.metered = self.metered;
        if self.ipv4 == Ipv4Method::Manual {
            match profile::parse_addresses(&self.ipv4_address, false) {
                Ok(a) => s.ipv4.addresses = a,
                Err(m) => problems.push((Row::Ipv4Address, m)),
            }
            s.ipv4.gateway = non_empty(&self.ipv4_gateway);
        } else {
            s.ipv4.addresses.clear();
            s.ipv4.gateway = None;
        }
        if self.ipv6 == Ipv6Method::Manual {
            match profile::parse_addresses(&self.ipv6_address, true) {
                Ok(a) => s.ipv6.addresses = a,
                Err(m) => problems.push((Row::Ipv6Address, m)),
            }
            s.ipv6.gateway = non_empty(&self.ipv6_gateway);
        } else {
            s.ipv6.addresses.clear();
            s.ipv6.gateway = None;
        }
        match profile::parse_dns(&self.dns) {
            Ok(d) => s.dns = d,
            Err(m) => problems.push((Row::Dns, m)),
        }
        match self.priority.trim().parse::<i32>() {
            Ok(p) => s.priority = p,
            Err(_) => problems.push((Row::Priority, "Priority is a whole number, like 10".into())),
        }
        if problems.is_empty() {
            for e in profile::validate(&s) {
                let row = match e.field {
                    Field::Ipv4Address => Row::Ipv4Address,
                    Field::Ipv4Gateway => Row::Ipv4Gateway,
                    Field::Ipv6Address => Row::Ipv6Address,
                    Field::Ipv6Gateway => Row::Ipv6Gateway,
                    Field::Dns => Row::Dns,
                    Field::Priority => Row::Priority,
                };
                problems.push((row, e.message));
            }
        }
        if problems.is_empty() {
            Ok(s)
        } else {
            Err(problems)
        }
    }

    /// Complaints for the form, in row order.
    pub fn problems(&self) -> Vec<(Row, String)> {
        let mut p = self.build().err().unwrap_or_default();
        let order = self.rows();
        p.sort_by_key(|(row, _)| order.iter().position(|r| r == row));
        p
    }

    /// What is changed from the loaded Profile.
    pub fn dirty(&self) -> bool {
        match (&self.view, self.build()) {
            (Some(v), Ok(s)) => v.settings != s,
            (Some(_), Err(_)) => true,
            _ => false,
        }
    }

    pub fn secs_left(&self) -> Option<u32> {
        match self.phase {
            Phase::Pending { secs_left, .. } => Some(secs_left),
            _ => None,
        }
    }

    pub fn is_pending(&self) -> bool {
        matches!(self.phase, Phase::Pending { .. })
    }

    /// The update to send, or `None` (and the reason on screen) when the form
    /// is not valid.
    pub fn update_op(&mut self) -> Option<nexus_ipc::Op> {
        let view = self.view.as_ref()?;
        match self.build() {
            Ok(settings) => Some(nexus_ipc::Op::ProfileUpdate {
                network: self.network.clone(),
                version: view.version,
                settings,
                rollback_secs: None,
            }),
            Err(problems) => {
                // Take the person to the first complaint.
                let order = self.rows();
                if let Some(at) = problems
                    .iter()
                    .filter_map(|(row, _)| order.iter().position(|r| r == row))
                    .min()
                {
                    self.focus = at;
                }
                None
            }
        }
    }

    pub fn applying(&mut self) {
        self.phase = Phase::Applying;
        self.message = None;
    }

    /// The Daemon says a change is waiting. Starts the countdown.
    pub fn pending(&mut self, change: &PendingChange, now: Instant) {
        self.phase = Phase::Pending {
            id: change.id,
            deadline: now + Duration::from_secs(u64::from(change.rollback_secs)),
            secs_left: change.rollback_secs,
            answered: false,
        };
    }

    /// Moves the countdown. Returns whether the number on screen changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        let Phase::Pending {
            deadline,
            secs_left,
            ..
        } = &mut self.phase
        else {
            return false;
        };
        let left = deadline.saturating_duration_since(now).as_secs_f64().ceil() as u32;
        let changed = left != *secs_left;
        *secs_left = left;
        changed
    }

    /// The Daemon settled the pending change `id`: the way it ended, if it is ours.
    pub fn settled(&self, id: u64) -> bool {
        matches!(self.phase, Phase::Pending { id: mine, .. } if mine == id)
    }

    pub fn applied_failed(&mut self, message: String) {
        self.phase = Phase::Editing;
        self.message = Some(message);
    }

    /// A key while the form (or the countdown) has the keyboard.
    pub fn key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match &mut self.phase {
            Phase::Loading => match key.code {
                KeyCode::Esc => Action::Close,
                _ => Action::None,
            },
            Phase::Applying => Action::None,
            Phase::Pending { id, answered, .. } => {
                if *answered {
                    return Action::None;
                }
                let id = *id;
                match key.code {
                    KeyCode::Char('y' | 'Y' | 'k') | KeyCode::Enter => {
                        *answered = true;
                        Action::Confirm(id)
                    }
                    KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                        *answered = true;
                        Action::Revert(id)
                    }
                    _ => Action::None,
                }
            }
            Phase::Editing => self.edit_key(key, ctrl),
        }
    }

    fn edit_key(&mut self, key: KeyEvent, ctrl: bool) -> Action {
        let rows = self.rows();
        let row = self.focused();
        let last = rows.len() - 1;
        match key.code {
            KeyCode::Esc => Action::Close,
            KeyCode::Enter => Action::Apply,
            KeyCode::Char('s') if ctrl => Action::Apply,
            KeyCode::Tab | KeyCode::Down => {
                self.focus = (self.focus_index() + 1).min(last);
                Action::Redraw
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.focus = self.focus_index().saturating_sub(1);
                Action::Redraw
            }
            KeyCode::Char('u') if ctrl && row.is_text() => {
                if let Some(t) = self.text_mut(row) {
                    t.clear();
                }
                Action::Redraw
            }
            KeyCode::Backspace if row.is_text() => {
                if let Some(t) = self.text_mut(row) {
                    t.pop();
                }
                Action::Redraw
            }
            KeyCode::Char(c) if row.is_text() && !ctrl => {
                if let Some(t) = self.text_mut(row) {
                    t.push(c);
                }
                Action::Redraw
            }
            KeyCode::Char('j') if !row.is_text() => {
                self.focus = (self.focus_index() + 1).min(last);
                Action::Redraw
            }
            KeyCode::Char('k') if !row.is_text() => {
                self.focus = self.focus_index().saturating_sub(1);
                Action::Redraw
            }
            KeyCode::Right | KeyCode::Char(' ' | 'l') if !row.is_text() => self.cycle(row, 1),
            KeyCode::Left | KeyCode::Char('h') if !row.is_text() => self.cycle(row, -1),
            _ => Action::Idle,
        }
    }

    fn cycle(&mut self, row: Row, by: isize) -> Action {
        fn step<T: Copy + PartialEq>(all: &[T], now: T, by: isize) -> T {
            let at = all.iter().position(|x| *x == now).unwrap_or(0) as isize;
            all[(at + by).rem_euclid(all.len() as isize) as usize]
        }
        match row {
            Row::Ipv4 => {
                self.ipv4 = step(&[Ipv4Method::Auto, Ipv4Method::Manual], self.ipv4, by);
            }
            Row::Ipv6 => {
                self.ipv6 = step(
                    &[Ipv6Method::Auto, Ipv6Method::Manual, Ipv6Method::Disabled],
                    self.ipv6,
                    by,
                );
            }
            Row::Mac => {
                self.mac = step(
                    &[MacMode::PerNetwork, MacMode::EveryTime, MacMode::Permanent],
                    self.mac,
                    by,
                );
            }
            Row::Metered => self.metered = !self.metered,
            _ => return Action::Idle,
        }
        Action::Redraw
    }

    /// How a pending change ended, in words for a toast.
    pub fn outcome_text(&self, outcome: Settled) -> (String, bool) {
        if outcome == Settled::Confirmed {
            (format!("Keeping the new settings of {}.", self.ssid), false)
        } else {
            (
                format!("Reverted: {} has its old settings again.", self.ssid),
                true,
            )
        }
    }

    /// The complaint belonging to the focused row, else the first one.
    pub fn complaint(&self) -> Option<String> {
        let problems = self.problems();
        let focused = self.focused();
        problems
            .iter()
            .find(|(row, _)| *row == focused)
            .or_else(|| problems.first())
            .map(|(_, m)| m.clone())
    }

    /// Whether a row has something wrong with it.
    pub fn is_wrong(&self, row: Row) -> bool {
        row.field().is_some() && self.problems().iter().any(|(r, _)| *r == row)
    }
}

fn non_empty(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn loaded() -> Editor {
        let mut e = Editor::loading("6e".into(), "Net".into());
        e.load(Ok(ProfileView {
            ssid: "Net".into(),
            uuid: "u".into(),
            version: 3,
            active: true,
            settings: ProfileSettings::default(),
        }));
        e
    }

    fn pending(id: u64, secs: u32) -> PendingChange {
        PendingChange {
            id,
            ssid: "Net".into(),
            rollback_secs: secs,
        }
    }

    #[test]
    fn the_countdown_follows_the_clock_and_only_redraws_when_the_number_moves() {
        let mut e = loaded();
        let t0 = Instant::now();
        e.pending(&pending(1, 15), t0);
        assert_eq!(e.secs_left(), Some(15));
        assert!(!e.tick(t0 + Duration::from_millis(200)));
        assert!(e.tick(t0 + Duration::from_millis(3200)));
        assert_eq!(e.secs_left(), Some(12));
        assert!(e.tick(t0 + Duration::from_secs(60)));
        assert_eq!(e.secs_left(), Some(0));
    }

    #[test]
    fn keep_and_revert_are_answered_once() {
        let mut e = loaded();
        e.pending(&pending(7, 15), Instant::now());
        assert_eq!(e.key(key(KeyCode::Char('y'))), Action::Confirm(7));
        assert_eq!(e.key(key(KeyCode::Char('n'))), Action::None);
        assert!(e.settled(7) && !e.settled(8));
    }

    #[test]
    fn static_ipv4_rows_appear_and_are_checked_only_then() {
        let mut e = loaded();
        assert_eq!(e.rows().len(), 6);
        assert!(e.build().is_ok());
        e.key(key(KeyCode::Char(' '))); // IPv4 -> static
        assert_eq!(e.rows().len(), 8);
        let problems = e.problems();
        assert_eq!(problems[0].0, Row::Ipv4Address, "{problems:?}");
        e.key(key(KeyCode::Char(' '))); // static -> automatic again
        assert!(e.build().is_ok());
    }

    #[test]
    fn apply_with_a_typo_moves_focus_to_it_and_sends_nothing() {
        let mut e = loaded();
        e.key(key(KeyCode::Char(' ')));
        for _ in 0..3 {
            e.key(key(KeyCode::Tab));
        }
        assert_ne!(e.focused(), Row::Ipv4Address);
        assert!(e.update_op().is_none());
        assert_eq!(e.focused(), Row::Ipv4Address);
    }

    #[test]
    fn a_priority_that_is_not_a_number_is_a_complaint() {
        let mut e = loaded();
        for _ in 0..6 {
            e.key(key(KeyCode::Tab));
        }
        assert_eq!(e.focused(), Row::Priority);
        e.key(key(KeyCode::Char('x')));
        assert!(e.problems().iter().any(|(r, _)| *r == Row::Priority));
    }
}
