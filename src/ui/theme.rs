use ratatui::style::{Color, Modifier, Style};

/// Adaptive terminal theme using ANSI 16 colors.
/// Respects the user's terminal colorscheme — no hardcoded RGB values.
#[derive(Debug, Clone)]
pub struct Theme {
    // ── Base ──────────────────────────────────────────────────
    pub bg: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub fg_muted: Color,

    // ── Semantic ──────────────────────────────────────────────
    pub success: Color, // Connected, OK
    pub warning: Color, // Connecting, degraded
    pub error: Color,   // Disconnected, failed
    pub info: Color,    // Informational highlights
    pub accent: Color,  // Primary accent (selection, active tab)

    // ── Borders ──────────────────────────────────────────────
    pub border: Color,
    pub border_focused: Color,

    // ── Components ───────────────────────────────────────────
    pub tab_active: Style,
    pub tab_inactive: Style,
    pub table_header: Style,
    pub table_row: Style,
    pub table_row_selected: Style,
    pub status_bar: Style,
    pub help_key: Style,
    pub help_desc: Style,
    pub dialog_border: Style,
    pub input_active: Style,
    pub input_inactive: Style,
    pub signal_excellent: Color,
    pub signal_good: Color,
    pub signal_fair: Color,
    pub signal_weak: Color,
    pub sparkline_fg: Color,

    // ── WiFi Security ────────────────────────────────────────
    pub security_open: Style,
    pub security_wpa: Style,
    pub security_enterprise: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            // Base
            bg: Color::Reset,
            fg: Color::Reset,
            fg_dim: Color::DarkGray,
            fg_muted: Color::Gray,

            // Semantic
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            info: Color::Blue,
            accent: Color::Cyan,

            // Borders
            border: Color::DarkGray,
            border_focused: Color::Cyan,

            // Tabs
            tab_active: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            tab_inactive: Style::default().fg(Color::Gray),

            // Table
            table_header: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            table_row: Style::default().fg(Color::Reset),
            table_row_selected: Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),

            // Status bar
            status_bar: Style::default().fg(Color::Black).bg(Color::DarkGray),
            help_key: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            help_desc: Style::default().fg(Color::Gray),

            // Dialog
            dialog_border: Style::default().fg(Color::Yellow),

            // Input
            input_active: Style::default().fg(Color::Cyan),
            input_inactive: Style::default().fg(Color::DarkGray),

            // Signal strength colors
            signal_excellent: Color::Green,
            signal_good: Color::LightGreen,
            signal_fair: Color::Yellow,
            signal_weak: Color::Red,

            // Sparkline
            sparkline_fg: Color::Cyan,

            // WiFi security
            security_open: Style::default().fg(Color::Yellow),
            security_wpa: Style::default().fg(Color::Green),
            security_enterprise: Style::default().fg(Color::Magenta),
        }
    }
}

impl Theme {
    /// Get color for a device state
    pub fn device_state_color(&self, state: &crate::network::DeviceState) -> Color {
        if state.is_connected() {
            self.success
        } else if state.is_connecting() {
            self.warning
        } else {
            match state {
                crate::network::DeviceState::Failed => self.error,
                crate::network::DeviceState::Unavailable
                | crate::network::DeviceState::Unmanaged => self.fg_muted,
                _ => self.fg_dim,
            }
        }
    }

    /// Get color for WiFi signal strength
    pub fn signal_color(&self, strength: u8) -> Color {
        match strength {
            75..=100 => self.signal_excellent,
            50..=74 => self.signal_good,
            25..=49 => self.signal_fair,
            _ => self.signal_weak,
        }
    }

    /// Get style for connectivity state
    pub fn connectivity_style(&self, state: &crate::network::ConnectivityState) -> Style {
        match state {
            crate::network::ConnectivityState::Full => Style::default().fg(self.success),
            crate::network::ConnectivityState::Limited => Style::default().fg(self.warning),
            crate::network::ConnectivityState::Portal => Style::default().fg(self.warning),
            crate::network::ConnectivityState::None => Style::default().fg(self.error),
            crate::network::ConnectivityState::Unknown => Style::default().fg(self.fg_dim),
        }
    }

    /// Get a style for the title block
    pub fn title_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Get a bordered block style
    pub fn block_style(&self, focused: bool) -> Style {
        Style::default().fg(if focused {
            self.border_focused
        } else {
            self.border
        })
    }
}
