//! Keys and results of the Settings tab, kept apart from the rest of `App`.
//!
//! A change is a `config_set` op: the Daemon edits `config.toml` in place and
//! announces `config_reloaded`, which is when the screen changes. The file
//! stays the one source of truth, and an edit by hand shows up the same way.

use crossterm::event::{KeyCode, KeyEvent};
use nexus_ipc::Op;
use serde_json::{Value, json};

use super::{App, Msg};
use crate::text;
use crate::tui::caps::Glyphs;
use crate::tui::fx::{Fx, Movement};
use crate::tui::settings::{ITEMS, Item, Kind};
use crate::tui::theme::{Look, Theme};
use crate::tui::undo::Undo;

impl App {
    /// Keys of the Settings list: enter changes the row, r resets it.
    pub(super) fn settings_key(&mut self, key: KeyEvent) -> bool {
        let Some(item) = ITEMS.get(self.selected).copied() else {
            return false;
        };
        match key.code {
            KeyCode::Enter | KeyCode::Char(' ') => self.change_setting(&item),
            KeyCode::Char('r') => {
                let before = self.settings.raw(item.key);
                self.set_setting(item.key, Value::Null);
                if before.is_null() {
                    return false; // already the default
                }
                self.say(
                    text::tui::reset(item.label),
                    false,
                    Some(Undo::Setting {
                        key: item.key.to_owned(),
                        value: before,
                    }),
                );
                true
            }
            _ => false,
        }
    }

    /// Enter on a row: the next theme, or the opposite switch.
    pub(in crate::tui) fn change_setting(&mut self, item: &Item) -> bool {
        if !self.settings.has_data() {
            return false;
        }
        match item.kind {
            Kind::Theme => {
                let next = self.settings.next_theme();
                self.set_setting(item.key, json!(next))
            }
            Kind::Switch => {
                let now = self.settings.is_on(item);
                let before = self.settings.raw(item.key);
                self.set_setting(item.key, json!(!now));
                self.say(
                    text::tui::switched(item.label, !now),
                    false,
                    Some(Undo::Setting {
                        key: item.key.to_owned(),
                        value: before,
                    }),
                );
                true
            }
        }
    }

    /// The verbs of the palette: change the setting named `key`.
    pub(in crate::tui) fn change_setting_by_key(&mut self, key: &str) -> bool {
        match ITEMS.iter().find(|i| i.key == key).copied() {
            Some(item) => self.change_setting(&item),
            None => false,
        }
    }

    pub(super) fn set_setting(&mut self, key: &str, value: Value) -> bool {
        if let Some(runner) = &self.runner {
            let op = Op::ConfigSet {
                key: key.to_owned(),
                value,
            };
            runner.call(op, |r| Msg::Setting(r.map(|_| ())));
        }
        false
    }

    /// Takes the Daemon's config: re-themes, switches icons and motion, and
    /// says why a theme could not be used. Returns whether the screen changed.
    pub(super) fn apply_config(&mut self, config: Value, warning: Option<String>) -> bool {
        let mut dirty = self.settings.refresh(config, warning);
        let (theme, note) = self.settings.resolve_theme();
        let mut caps = self.flag_caps;
        if caps.glyphs == Glyphs::Unicode && self.setting_on("ui.nerd_font") {
            caps.glyphs = Glyphs::Nerd;
        }
        let look = Look::new(theme, caps);
        if look.theme.name != self.look.theme.name
            || look.caps != self.look.caps
            || !same_palette(&look.theme, &self.look.theme)
        {
            self.look = look;
            // Effects carry the theme's colours: start them afresh.
            self.fx = Fx::new(&self.look);
            self.fx.reduce(self.motion_reduced());
            self.sync_pulse();
            dirty = true;
        } else if (self.fx.motion() == Movement::Reduced) != self.motion_reduced() {
            self.fx.reduce(self.motion_reduced());
        }
        if note != self.theme_note {
            if let Some(why) = &note {
                self.say(why.clone(), true, None);
            }
            self.theme_note = note;
            dirty = true;
        }
        dirty
    }

    fn setting_on(&self, key: &str) -> bool {
        ITEMS
            .iter()
            .find(|i| i.key == key)
            .is_some_and(|i| self.settings.is_on(i))
    }

    /// Whether the config asks for reduced motion.
    pub(super) fn settings_motion(&self) -> bool {
        self.setting_on("ui.reduce_motion")
    }

    /// Reduced motion by the environment or by the config.
    fn motion_reduced(&self) -> bool {
        self.env_motion || self.settings_motion()
    }

    /// What the theme row shows.
    pub fn theme_title(&self) -> String {
        if self.theme_note.is_some() {
            return Theme::MIDNIGHT_BLOOM.title.to_owned();
        }
        match self.look.theme.name {
            "custom" => self.settings.theme_name().to_owned(),
            _ => self.look.theme.title.to_owned(),
        }
    }

    /// Why the configured theme is not the one in force, if so.
    pub fn theme_note(&self) -> Option<&str> {
        self.theme_note.as_deref()
    }

    /// Why a switch cannot be turned off from here: something outside the
    /// config file already turned it on.
    pub fn setting_forced(&self, item: &Item) -> Option<&'static str> {
        match item.key {
            "ui.reduce_motion" if self.env_motion => Some(
                "NEXUS_REDUCE_MOTION or REDUCE_MOTION is set in the environment, \
                 so motion stays reduced whatever this says.",
            ),
            "ui.nerd_font" if self.flag_caps.glyphs == Glyphs::Nerd => {
                Some("--nerd-font was given, so the icons stay on whatever this says.")
            }
            "ui.nerd_font" if self.flag_caps.glyphs == Glyphs::Ascii => {
                Some("This terminal is showing plain ASCII, which has no icons.")
            }
            _ => None,
        }
    }
}

fn same_palette(a: &Theme, b: &Theme) -> bool {
    [
        (a.bg, b.bg),
        (a.fg, b.fg),
        (a.muted, b.muted),
        (a.dim, b.dim),
        (a.accent, b.accent),
        (a.ok, b.ok),
        (a.warn, b.warn),
        (a.err, b.err),
        (a.border, b.border),
        (a.selection, b.selection),
    ]
    .iter()
    .all(|(x, y)| x.rgb == y.rgb)
}
