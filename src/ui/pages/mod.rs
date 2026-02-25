pub mod dashboard;
pub mod interfaces;
pub mod wifi;
pub mod connections;
pub mod diagnostics;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::Frame;

use crate::app::{App, Page};
use crate::ui::components;

/// Render the full screen: tabs + active page + status bar
pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),  // Tab bar
            Constraint::Min(0),    // Page content
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    // Tab bar
    components::tabs::render(f, &app.active_page, &app.theme, chunks[0]);

    // Active page content
    match app.active_page {
        Page::Dashboard => dashboard::render(f, app, chunks[1]),
        Page::Interfaces => interfaces::render(f, app, chunks[1]),
        Page::Wifi => wifi::render(f, app, chunks[1]),
        Page::Connections => connections::render(f, app, chunks[1]),
        Page::Diagnostics => diagnostics::render(f, app, chunks[1]),
    }

    // Status bar
    components::status_bar::render(f, app, &app.theme, chunks[2]);

    // Overlay dialogs
    app.input_dialog.render(f, &app.theme);
    app.confirm_dialog.render(f, &app.theme);

    // Toast notification
    if let Some(ref msg) = app.toast_message {
        render_toast(f, msg, app.toast_is_error, &app.theme);
    }
}

/// Render a temporary toast notification at the top-right
fn render_toast(f: &mut Frame, message: &str, is_error: bool, theme: &crate::ui::theme::Theme) {
    use ratatui::style::{Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Clear, Paragraph};

    let area = f.area();
    let width = (message.len() as u16 + 4).min(area.width - 4);
    let x = area.width.saturating_sub(width + 2);
    let toast_area = Rect::new(x, 1, width, 3);

    f.render_widget(Clear, toast_area);

    let color = if is_error { theme.error } else { theme.success };
    let icon = if is_error { " ✗ " } else { " ✓ " };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color));

    let text = Paragraph::new(Line::from(vec![
        Span::styled(icon, Style::default().fg(color).add_modifier(Modifier::BOLD)),
        Span::styled(message, Style::default().fg(theme.fg)),
    ]))
    .block(block);

    f.render_widget(text, toast_area);
}
