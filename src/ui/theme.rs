use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::BorderType;

use crate::config::{Config, ThemeConfig};

// ─── Nerd Font Icons ──────────────────────────────────────────────────────
// These are glyph constants — not configurable via TOML (they'd break
// alignment if users changed them). The nerd_fonts toggle switches between
// these and the PLAIN_* fallbacks.
pub const SIGNAL_ICONS_NERD: &[&str] = &["󰤯 ", "󰤟 ", "󰤢 ", "󰤥 ", "󰤨 "];
pub const SIGNAL_ICONS_PLAIN: &[&str] = &["▂   ", "▂▄  ", "▂▄▆ ", "▂▄▆█", "▂▄▆█"];

pub const ICON_WIFI: &str = "󰤨 ";
pub const ICON_WIFI_OFF: &str = "󰤭 ";
pub const ICON_LOCK: &str = "󰌾 ";
pub const ICON_LOCK_OPEN: &str = "󰴲 ";
pub const ICON_CONNECTED: &str = " ";
pub const ICON_SAVED: &str = "★";
pub const ICON_ARROW_RIGHT: &str = " ";
pub const ICON_HIDDEN: &str = "󰈈 ";
pub const ICON_SCAN: &str = "󰑐 ";
pub const ICON_ERROR: &str = " ";
pub const ICON_INFO: &str = " ";

pub const PLAIN_WIFI: &str = "[W]";
pub const PLAIN_WIFI_OFF: &str = "[X]";
pub const PLAIN_LOCK: &str = "[L]";
pub const PLAIN_LOCK_OPEN: &str = "[O]";
pub const PLAIN_CONNECTED: &str = "*";
pub const PLAIN_SAVED: &str = "*";
pub const PLAIN_ARROW: &str = ">";
pub const PLAIN_HIDDEN: &str = "[H]";

// ─── Theme (runtime, config-driven) ─────────────────────────────────────

/// Runtime theme struct. Built once from Config, then passed around by
/// reference. All colors come from the user's config.toml.
#[derive(Debug, Clone)]
pub struct Theme {
    // Core palette
    pub bg: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub accent: Color,
    pub accent2: Color,
    pub border: Color,
    pub border_focused: Color,

    // Semantic
    pub connected: Color,
    pub warning: Color,
    pub error: Color,
    pub selected_bg: Color,

    // Signal gradient
    pub signal_excellent: Color,
    pub signal_good: Color,
    pub signal_fair: Color,
    pub signal_weak: Color,
    pub signal_none: Color,

    // Border type
    pub border_type: BorderType,
}

impl Theme {
    /// Construct from the loaded Config.
    pub fn from_config(config: &Config) -> Self {
        let t: &ThemeConfig = &config.theme;

        let border_type = match config.appearance.border_style.as_str() {
            "plain" => BorderType::Plain,
            "thick" => BorderType::Thick,
            "double" => BorderType::Double,
            _ => BorderType::Rounded,
        };

        Self {
            bg: t.bg,
            fg: t.fg,
            fg_dim: t.fg_dim,
            accent: t.accent,
            accent2: t.accent_secondary,
            border: t.border,
            border_focused: t.border_focused,
            connected: t.semantic.connected,
            warning: t.semantic.warning,
            error: t.semantic.error,
            selected_bg: t.semantic.selected_bg,
            signal_excellent: t.signal.excellent,
            signal_good: t.signal.good,
            signal_fair: t.signal.fair,
            signal_weak: t.signal.weak,
            signal_none: t.signal.none,
            border_type,
        }
    }

    // ─── Style Constructors ─────────────────────────────────────────

    pub fn style_default(&self) -> Style {
        Style::default().fg(self.fg).bg(self.bg)
    }

    pub fn style_dim(&self) -> Style {
        Style::default().fg(self.fg_dim).bg(self.bg)
    }

    pub fn style_accent(&self) -> Style {
        Style::default().fg(self.accent).bg(self.bg)
    }

    pub fn style_accent_bold(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .bg(self.bg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn style_selected(&self) -> Style {
        Style::default()
            .fg(self.fg)
            .bg(self.selected_bg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn style_connected(&self) -> Style {
        Style::default()
            .fg(self.connected)
            .bg(self.bg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn style_error(&self) -> Style {
        Style::default().fg(self.error).bg(self.bg)
    }

    pub fn style_warning(&self) -> Style {
        Style::default().fg(self.warning).bg(self.bg)
    }

    pub fn style_border(&self) -> Style {
        Style::default().fg(self.border).bg(self.bg)
    }

    pub fn style_border_focused(&self) -> Style {
        Style::default().fg(self.border_focused).bg(self.bg)
    }

    pub fn style_key_hint(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .bg(self.bg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn style_key_desc(&self) -> Style {
        Style::default().fg(self.fg_dim).bg(self.bg)
    }

    // ─── Signal Helpers ─────────────────────────────────────────────

    pub fn signal_color(&self, strength: u8) -> Color {
        match strength {
            0..=19 => self.signal_none,
            20..=39 => self.signal_weak,
            40..=59 => self.signal_fair,
            60..=79 => self.signal_good,
            _ => self.signal_excellent,
        }
    }

    pub fn signal_icon(&self, strength: u8, nerd_fonts: bool) -> &'static str {
        let icons = if nerd_fonts {
            SIGNAL_ICONS_NERD
        } else {
            SIGNAL_ICONS_PLAIN
        };
        match strength {
            0..=19 => icons[0],
            20..=39 => icons[1],
            40..=59 => icons[2],
            60..=79 => icons[3],
            _ => icons[4],
        }
    }

    pub fn lock_icon(&self, needs_password: bool, nerd_fonts: bool) -> &'static str {
        if nerd_fonts {
            if needs_password {
                ICON_LOCK
            } else {
                ICON_LOCK_OPEN
            }
        } else if needs_password {
            PLAIN_LOCK
        } else {
            PLAIN_LOCK_OPEN
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::from_config(&Config::default())
    }
}
