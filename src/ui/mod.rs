pub mod details;
pub mod header;
pub mod help;
pub mod hidden;
pub mod network_list;
pub mod password;
pub mod status_bar;
pub mod theme;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::app::{App, AppMode};

/// Root render function — draws the entire UI
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Check minimum terminal size
    if area.width < 50 || area.height < 12 {
        render_too_small(frame, area);
        return;
    }

    // Main vertical layout: header | body | footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(6),    // Body
            Constraint::Length(1), // Footer / status bar
        ])
        .split(area);

    // Render header
    header::render(frame, app, chunks[0]);

    // Body: network list (+ optional detail panel)
    let show_details = app.detail_visible && area.width > 90;
    if show_details {
        let body_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(chunks[1]);

        network_list::render(frame, app, body_chunks[0]);
        details::render(frame, app, body_chunks[1]);
    } else {
        network_list::render(frame, app, chunks[1]);
    }

    // Render footer
    status_bar::render(frame, app, chunks[2]);

    // Render overlays (modals) on top
    match &app.mode {
        AppMode::PasswordInput { ssid } => {
            let ssid = ssid.clone();
            password::render(frame, app, area, &ssid);
        }
        AppMode::Hidden => {
            hidden::render(frame, app, area);
        }
        AppMode::Help => {
            help::render(frame, area);
        }
        AppMode::Error(msg) => {
            render_error_dialog(frame, area, msg);
        }
        _ => {}
    }
}

/// Render a "terminal too small" message
fn render_too_small(frame: &mut Frame, area: Rect) {
    use ratatui::text::Text;
    use ratatui::widgets::Paragraph;

    let msg = Text::styled("Terminal too small\nMinimum: 50×12", theme::style_warning());
    let para = Paragraph::new(msg).alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(para, area);
}

/// Render an error dialog overlay
fn render_error_dialog(frame: &mut Frame, area: Rect, message: &str) {
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

    let dialog = centered_rect(60, 30, area);
    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(" ", theme::style_error()),
            Span::styled(" Error ", theme::style_error()),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::style_error())
        .style(theme::style_default());

    let para = Paragraph::new(message.to_string())
        .block(block)
        .wrap(Wrap { trim: true })
        .style(theme::style_default());

    frame.render_widget(para, dialog);

    // Footer hint inside dialog
    let hint_area = Rect {
        x: dialog.x + 2,
        y: dialog.y + dialog.height - 2,
        width: dialog.width.saturating_sub(4),
        height: 1,
    };
    let hint = ratatui::text::Line::from(vec![
        Span::styled("[Esc]", theme::style_key_hint()),
        Span::styled(" Close", theme::style_key_desc()),
    ]);
    frame.render_widget(ratatui::widgets::Paragraph::new(hint), hint_area);
}

/// Create a centered rectangle within an area (percentage-based)
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Create a centered rectangle with fixed dimensions
pub fn centered_rect_fixed(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
