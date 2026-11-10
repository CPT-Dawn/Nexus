//! The command palette (`:`): type plain English, pick a Verb, run it.
//! The Verbs themselves live in `verbs.rs`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{App, Layer};
use crate::tui::verbs::{self, Found, Run, Verb};

/// The palette while it is open.
#[derive(Debug, Default)]
pub struct Palette {
    pub query: String,
    /// Index into the current matches.
    pub selected: usize,
}

impl App {
    pub fn palette(&self) -> Option<&Palette> {
        self.palette.as_ref()
    }

    /// The Verbs matching what has been typed, best first.
    pub fn matches(&self) -> Vec<Found> {
        let query = self.palette.as_ref().map_or("", |p| p.query.as_str());
        verbs::search(self, query)
    }

    pub(super) fn open_palette(&mut self) -> bool {
        self.toast = None;
        self.filtering = false;
        self.palette = Some(Palette::default());
        true
    }

    pub(super) fn palette_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let last = self.matches().len().saturating_sub(1);
        let Some(p) = self.palette.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => {
                self.palette = None;
                true
            }
            KeyCode::Enter => {
                self.run_selected();
                true
            }
            KeyCode::Up | KeyCode::BackTab => move_to(p, p.selected.saturating_sub(1)),
            KeyCode::Char('p') if ctrl => move_to(p, p.selected.saturating_sub(1)),
            KeyCode::Down | KeyCode::Tab => move_to(p, (p.selected + 1).min(last)),
            KeyCode::Char('n') if ctrl => move_to(p, (p.selected + 1).min(last)),
            KeyCode::PageUp => move_to(p, p.selected.saturating_sub(5)),
            KeyCode::PageDown => move_to(p, (p.selected + 5).min(last)),
            KeyCode::Backspace => {
                let had = p.query.pop().is_some();
                p.selected = 0;
                had
            }
            KeyCode::Char('u') if ctrl => {
                let had = !p.query.is_empty();
                p.query.clear();
                p.selected = 0;
                had
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                p.query.push(c);
                p.selected = 0;
                true
            }
            _ => false,
        }
    }

    /// Runs the highlighted match (a click on a match selects it first).
    pub(super) fn run_selected(&mut self) {
        let at = self.palette.as_ref().map_or(0, |p| p.selected);
        let Some(found) = self.matches().get(at).copied() else {
            return;
        };
        self.palette = None;
        match found.blocked {
            Some(why) => self.say(why, false, None),
            None => {
                self.run_verb(found.verb);
            }
        }
    }

    /// Does what `verb` says, as if its key had been pressed.
    fn run_verb(&mut self, verb: &'static Verb) -> bool {
        self.layer = Layer::Root;
        match verb.run {
            Run::Root(code) => self.root_key(plain(code)),
            Run::Key(tab, code) => self.keys_of(tab, plain(code)),
            Run::Go(tab) => self.go_tab(tab),
            Run::Call(f) => f(self),
        }
    }
}

fn plain(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn move_to(p: &mut Palette, to: usize) -> bool {
    let changed = p.selected != to;
    p.selected = to;
    changed
}
