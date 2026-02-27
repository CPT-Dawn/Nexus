use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, AppMode};
use crate::ui::theme::Theme;

/// Render the bottom status bar with context-sensitive keybinding hints
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let hints = match &app.mode {
        AppMode::Normal | AppMode::Scanning => normal_hints(t),
        AppMode::PasswordInput { .. } => password_hints(t),
        AppMode::Hidden => hidden_hints(t),
        AppMode::Help => help_hints(t),
        AppMode::Connecting | AppMode::Disconnecting => busy_hints(t),
        AppMode::Error(_) => error_hints(t),
    };

    let line = Line::from(hints);
    let para = Paragraph::new(line).alignment(Alignment::Center);
    frame.render_widget(para, area);
}

fn normal_hints(t: &Theme) -> Vec<Span<'static>> {
    vec![
        key(t, "↑↓/jk"),
        desc(t, "Navigate "),
        key(t, "Enter"),
        desc(t, "Connect "),
        key(t, "d"),
        desc(t, "Disconnect "),
        key(t, "s"),
        desc(t, "Scan "),
        key(t, "f"),
        desc(t, "Forget "),
        key(t, "h"),
        desc(t, "Hidden "),
        key(t, "i"),
        desc(t, "Details "),
        key(t, "/"),
        desc(t, "Help "),
        key(t, "q"),
        desc(t, "Quit"),
    ]
}

fn password_hints(t: &Theme) -> Vec<Span<'static>> {
    vec![
        key(t, "Enter"),
        desc(t, "Submit "),
        key(t, "Esc"),
        desc(t, "Cancel "),
        key(t, "Ctrl+H"),
        desc(t, "Toggle visibility"),
    ]
}

fn hidden_hints(t: &Theme) -> Vec<Span<'static>> {
    vec![
        key(t, "Tab"),
        desc(t, "Switch field "),
        key(t, "Enter"),
        desc(t, "Connect "),
        key(t, "Esc"),
        desc(t, "Cancel"),
    ]
}

fn help_hints(t: &Theme) -> Vec<Span<'static>> {
    vec![
        key(t, "?"),
        desc(t, "Close "),
        key(t, "Esc"),
        desc(t, "Close"),
    ]
}

fn busy_hints(t: &Theme) -> Vec<Span<'static>> {
    vec![Span::styled("Please wait…", t.style_dim())]
}

fn error_hints(t: &Theme) -> Vec<Span<'static>> {
    vec![key(t, "Esc"), desc(t, "Close")]
}

fn key(t: &Theme, k: &'static str) -> Span<'static> {
    Span::styled(format!(" [{k}] "), t.style_key_hint())
}

fn desc(t: &Theme, d: &'static str) -> Span<'static> {
    Span::styled(d, t.style_key_desc())
}
