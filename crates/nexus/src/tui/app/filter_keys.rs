//! The `/` filter of the list tabs (Wi-Fi, VPN), kept apart from the rest of
//! `App`. `/` starts typing; the list narrows as you go; Enter keeps the
//! filter and gives the keys back; Esc clears it. A tab remembers its own.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{App, Tab};

impl App {
    /// Whether the current tab has a list `/` can narrow.
    pub fn filterable(&self) -> bool {
        matches!(self.tab, Tab::WiFi | Tab::Vpn)
    }

    /// Whether the filter is being typed (it has the keys).
    pub fn filtering(&self) -> bool {
        self.filtering
    }

    /// The current tab's filter text (empty when none).
    pub fn filter_text(&self) -> &str {
        match self.tab {
            Tab::WiFi => self.wifi.filter(),
            Tab::Vpn => self.vpn.filter(),
            _ => "",
        }
    }

    fn set_filter(&mut self, text: &str) -> bool {
        match self.tab {
            Tab::WiFi => self.wifi.set_filter(text),
            Tab::Vpn => self.vpn.set_filter(text),
            _ => false,
        }
    }

    /// `/`: start typing a filter (the old text stays to be edited).
    pub(in crate::tui) fn start_filter(&mut self) -> bool {
        if !self.filterable() {
            return false;
        }
        self.toast = None;
        self.filtering = true;
        true
    }

    /// Esc on a list that has a filter: drop it.
    pub(super) fn clear_filter(&mut self) -> bool {
        let had = !self.filter_text().is_empty() || self.filtering;
        self.filtering = false;
        self.set_filter("");
        had
    }

    /// Keys while the filter is being typed.
    pub(super) fn filter_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let text = self.filter_text().to_owned();
        match key.code {
            KeyCode::Esc => self.clear_filter(),
            KeyCode::Enter => {
                self.filtering = false;
                true
            }
            KeyCode::Backspace => {
                let mut text = text;
                if text.pop().is_none() {
                    self.filtering = false;
                    return true;
                }
                self.set_filter(&text)
            }
            KeyCode::Char('u') if ctrl => self.set_filter(""),
            KeyCode::Down => self.select((self.selected() + 1).min(self.rows().saturating_sub(1))),
            KeyCode::Char('n') if ctrl => {
                self.select((self.selected() + 1).min(self.rows().saturating_sub(1)))
            }
            KeyCode::Up => self.select(self.selected().saturating_sub(1)),
            KeyCode::Char('p') if ctrl => self.select(self.selected().saturating_sub(1)),
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.set_filter(&format!("{text}{c}"))
            }
            _ => false,
        }
    }
}
