use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::theme;
use crate::app::{App, AppMode};

/// Render the bottom status bar with context-sensitive keybinding hints
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let hints = match &app.mode {
        AppMode::Normal | AppMode::Scanning => normal_hints(),
        AppMode::PasswordInput { .. } => password_hints(),
        AppMode::Hidden => hidden_hints(),
        AppMode::Help => help_hints(),
        AppMode::Connecting | AppMode::Disconnecting => busy_hints(),
        AppMode::Error(_) => error_hints(),
    };

    let line = Line::from(hints);
    let para = Paragraph::new(line).alignment(Alignment::Center);
    frame.render_widget(para, area);
}

fn normal_hints() -> Vec<Span<'static>> {
    vec![
        key("↑↓/jk"),
        desc("Navigate "),
        key("Enter"),
        desc("Connect "),
        key("d"),
        desc("Disconnect "),
        key("s"),
        desc("Scan "),
        key("f"),
        desc("Forget "),
        key("h"),
        desc("Hidden "),
        key("i"),
        desc("Details "),
        key("/"),
        desc("Help "),
        key("q"),
        desc("Quit"),
    ]
}

fn password_hints() -> Vec<Span<'static>> {
    vec![
        key("Enter"),
        desc("Submit "),
        key("Esc"),
        desc("Cancel "),
        key("Ctrl+H"),
        desc("Toggle visibility"),
    ]
}

fn hidden_hints() -> Vec<Span<'static>> {
    vec![
        key("Tab"),
        desc("Switch field "),
        key("Enter"),
        desc("Connect "),
        key("Esc"),
        desc("Cancel"),
    ]
}

fn help_hints() -> Vec<Span<'static>> {
    vec![key("?"), desc("Close "), key("Esc"), desc("Close")]
}

fn busy_hints() -> Vec<Span<'static>> {
    vec![Span::styled("Please wait…", theme::style_dim())]
}

fn error_hints() -> Vec<Span<'static>> {
    vec![key("Esc"), desc("Close")]
}

fn key(k: &'static str) -> Span<'static> {
    Span::styled(format!(" [{k}] "), theme::style_key_hint())
}

fn desc(d: &'static str) -> Span<'static> {
    Span::styled(d, theme::style_key_desc())
}
