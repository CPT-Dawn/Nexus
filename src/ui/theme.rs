use ratatui::style::{Color, Modifier, Style};

// ─── Nerd Font Icons ──────────────────────────────────────────────────────
// Signal strength icons (Nerd Font: nf-md-wifi_strength)
pub const SIGNAL_ICONS_NERD: &[&str] = &["󰤯 ", "󰤟 ", "󰤢 ", "󰤥 ", "󰤨 "];
// Fallback plain Unicode signal bars
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

// Plain fallbacks
pub const PLAIN_WIFI: &str = "[W]";
pub const PLAIN_WIFI_OFF: &str = "[X]";
pub const PLAIN_LOCK: &str = "[L]";
pub const PLAIN_LOCK_OPEN: &str = "[O]";
pub const PLAIN_CONNECTED: &str = "*";
pub const PLAIN_SAVED: &str = "*";
pub const PLAIN_ARROW: &str = ">";
pub const PLAIN_HIDDEN: &str = "[H]";

// ─── Color Palette (terminal-adaptive, transparency-friendly) ──────────

/// Background: always Reset (transparent) — respects terminal background
pub const BG: Color = Color::Reset;

/// Primary text color
pub const FG: Color = Color::White;

/// Dimmed text (labels, inactive elements)
pub const FG_DIM: Color = Color::DarkGray;

/// Accent color (selected items, active borders, keybinding hints)
pub const ACCENT: Color = Color::Cyan;

/// Secondary accent
pub const ACCENT2: Color = Color::Magenta;

/// Border color (inactive)
pub const BORDER: Color = Color::DarkGray;

/// Border color (focused/active panel)
pub const BORDER_FOCUSED: Color = Color::Cyan;

/// Connected/success indicator
pub const CONNECTED: Color = Color::Green;

/// Signal strength colors
pub const SIGNAL_EXCELLENT: Color = Color::Green;
pub const SIGNAL_GOOD: Color = Color::Green;
pub const SIGNAL_FAIR: Color = Color::Yellow;
pub const SIGNAL_WEAK: Color = Color::Red;
pub const SIGNAL_NONE: Color = Color::DarkGray;

/// Warning color (open networks, rfkill)
pub const WARNING: Color = Color::Yellow;

/// Error color
pub const ERROR: Color = Color::Red;

/// Selected item background (only element that gets a bg)
pub const SELECTED_BG: Color = Color::DarkGray;

// ─── Style Constructors ──────────────────────────────────────────────────

pub fn style_default() -> Style {
    Style::default().fg(FG).bg(BG)
}

pub fn style_dim() -> Style {
    Style::default().fg(FG_DIM).bg(BG)
}

pub fn style_accent() -> Style {
    Style::default().fg(ACCENT).bg(BG)
}

pub fn style_accent_bold() -> Style {
    Style::default()
        .fg(ACCENT)
        .bg(BG)
        .add_modifier(Modifier::BOLD)
}

pub fn style_selected() -> Style {
    Style::default()
        .fg(FG)
        .bg(SELECTED_BG)
        .add_modifier(Modifier::BOLD)
}

pub fn style_connected() -> Style {
    Style::default()
        .fg(CONNECTED)
        .bg(BG)
        .add_modifier(Modifier::BOLD)
}

pub fn style_error() -> Style {
    Style::default().fg(ERROR).bg(BG)
}

pub fn style_warning() -> Style {
    Style::default().fg(WARNING).bg(BG)
}

pub fn style_border() -> Style {
    Style::default().fg(BORDER).bg(BG)
}

pub fn style_border_focused() -> Style {
    Style::default().fg(BORDER_FOCUSED).bg(BG)
}

pub fn style_key_hint() -> Style {
    Style::default()
        .fg(ACCENT)
        .bg(BG)
        .add_modifier(Modifier::BOLD)
}

pub fn style_key_desc() -> Style {
    Style::default().fg(FG_DIM).bg(BG)
}

/// Get signal color based on strength percentage
pub fn signal_color(strength: u8) -> Color {
    match strength {
        0..=19 => SIGNAL_NONE,
        20..=39 => SIGNAL_WEAK,
        40..=59 => SIGNAL_FAIR,
        60..=79 => SIGNAL_GOOD,
        _ => SIGNAL_EXCELLENT,
    }
}

/// Get signal icon based on strength percentage
pub fn signal_icon(strength: u8, nerd_fonts: bool) -> &'static str {
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

/// Get lock icon based on security needs password
pub fn lock_icon(needs_password: bool, nerd_fonts: bool) -> &'static str {
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
