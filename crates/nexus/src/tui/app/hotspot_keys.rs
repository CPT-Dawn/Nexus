//! Keys and results of the Hotspot panel, kept apart from the rest of `App`.
//!
//! On the Hotspot tab: `n` starts one with a made-up password, `c` opens the
//! form to choose a name and password, `s` asks to show the join QR code
//! (nothing is read until the reveal key), `S` stops it. The Tethering keys
//! of the phone list work beside them.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_ipc::{Op, Secret};

use super::{App, Msg};
use crate::text;
use crate::tui::hotspot::{Busy, Done, Field, Form};
use crate::tui::share::Share;

impl App {
    /// Keys of the Hotspot tab: the Hotspot's own, then the phone list's.
    pub(in crate::tui) fn hotspot_tab_key(&mut self, key: KeyEvent) -> bool {
        self.hotspot_key(key) || self.tether_key(key)
    }

    fn hotspot_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('n') => self.hotspot_start(None, None),
            KeyCode::Char('c') => self.hotspot_form(),
            KeyCode::Char('S') => self.hotspot_stop(),
            KeyCode::Char('s') => self.hotspot_share(),
            _ => false,
        }
    }

    /// Why a start is not possible right now, in words, if it is not.
    fn hotspot_cannot_start(&self) -> Option<String> {
        let tab = &self.hotspot;
        if let Some(why) = tab.why() {
            return Some(why.to_owned());
        }
        if tab.on() {
            return Some("A Hotspot is already running. Stop it first (S).".into());
        }
        if tab.busy != Busy::No {
            return Some("A Hotspot is already starting.".into());
        }
        if !tab.has_data() {
            return Some("Still checking whether this machine can host a Hotspot.".into());
        }
        None
    }

    /// Starts a Hotspot. Without a name and password the Daemon makes them up.
    pub(in crate::tui) fn hotspot_start(
        &mut self,
        ssid: Option<String>,
        password: Option<Secret>,
    ) -> bool {
        if let Some(why) = self.hotspot_cannot_start() {
            self.say(why, true, None);
            return true;
        }
        self.toast = None;
        self.hotspot.busy = Busy::Starting;
        if let Some(runner) = &self.runner {
            runner.call(Op::HotspotStart { ssid, password }, |r| {
                Msg::HotspotDone(Done::Started(r.map(|v| ssid_of(&v))))
            });
        }
        true
    }

    pub(in crate::tui) fn hotspot_form(&mut self) -> bool {
        if let Some(why) = self.hotspot_cannot_start() {
            self.say(why, true, None);
            return true;
        }
        self.toast = None;
        self.hotspot.form = Some(Form::new());
        true
    }

    pub(in crate::tui) fn hotspot_stop(&mut self) -> bool {
        if !self.hotspot.on() {
            self.say(text::tui::NO_HOTSPOT_RUNNING, true, None);
            return true;
        }
        if self.hotspot.busy != Busy::No {
            return false;
        }
        self.toast = None;
        self.hotspot.busy = Busy::Stopping;
        if let Some(runner) = &self.runner {
            runner.call(Op::HotspotStop, |r| {
                Msg::HotspotDone(Done::Stopped(r.map(|v| ssid_of(&v))))
            });
        }
        true
    }

    /// Opens the QR view; it only asks, the reveal key reads.
    pub(in crate::tui) fn hotspot_share(&mut self) -> bool {
        if !self.hotspot.ready() {
            self.say(text::tui::NO_HOTSPOT_TO_SHARE, true, None);
            return true;
        }
        let ssid = self.hotspot.ssid().unwrap_or_default().to_owned();
        self.toast = None;
        self.share = Some(Share::ask_hotspot(ssid));
        true
    }

    /// Keys while the form is open.
    pub(in crate::tui) fn hotspot_form_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(form) = self.hotspot.form.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => {
                self.hotspot.form = None;
                return true;
            }
            _ if form.busy => return false,
            KeyCode::Enter if form.focus == Field::Name => form.focus = Field::Password,
            KeyCode::Enter => {
                let ssid = form.ssid.trim().to_owned();
                let password = form.password.trim().to_owned();
                form.busy = true;
                form.error = None;
                self.hotspot.busy = Busy::Starting;
                if let Some(runner) = &self.runner {
                    let ssid = (!ssid.is_empty()).then_some(ssid);
                    let password = (!password.is_empty()).then(|| Secret::new(password));
                    runner.call(Op::HotspotStart { ssid, password }, |r| {
                        Msg::HotspotDone(Done::Started(r.map(|v| ssid_of(&v))))
                    });
                }
            }
            KeyCode::Tab | KeyCode::Down | KeyCode::BackTab | KeyCode::Up => {
                form.focus = match form.focus {
                    Field::Name => Field::Password,
                    Field::Password => Field::Name,
                };
            }
            KeyCode::Backspace => {
                form.focused().pop();
            }
            KeyCode::Char('r') if ctrl => form.letters = !form.letters,
            KeyCode::Char('u') if ctrl => form.focused().clear(),
            KeyCode::Char(c) if !ctrl => form.focused().push(c),
            _ => return false,
        }
        true
    }

    /// A start or stop finished.
    pub(in crate::tui) fn hotspot_done(&mut self, done: Done) -> bool {
        self.hotspot.busy = Busy::No;
        match done {
            Done::Started(Ok(ssid)) => {
                self.hotspot.form = None;
                self.say(text::tui::hotspot_on(&ssid), false, None);
            }
            Done::Started(Err(message)) => match self.hotspot.form.as_mut() {
                Some(form) => {
                    form.busy = false;
                    form.error = Some(message);
                }
                None => self.say(message, true, None),
            },
            Done::Stopped(Ok(ssid)) => {
                // The details of a Hotspot that is gone are not worth showing.
                if self.share.as_ref().is_some_and(|s| s.hotspot) {
                    self.share = None;
                }
                self.say(text::hotspot_stopped(&ssid), false, None);
            }
            Done::Stopped(Err(message)) => self.say(message, true, None),
        }
        true
    }
}

fn ssid_of(value: &serde_json::Value) -> String {
    value
        .get("ssid")
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .to_owned()
}
