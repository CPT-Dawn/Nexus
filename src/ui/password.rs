use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::App;

/// Render the password input modal dialog
pub fn render(frame: &mut Frame, app: &App, area: Rect, ssid: &str) {
    let t = &app.theme;
    let width = 56_u16.min(area.width.saturating_sub(4));
    let height = 8_u16.min(area.height.saturating_sub(4));

    let y_offset = app.animation.dialog_y_offset();
    let dialog = super::centered_rect_fixed(width, height, area);
    let dialog = Rect {
        y: dialog.y.saturating_add(y_offset),
        ..dialog
    };

    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(" 󰌾 ", t.style_accent()),
            Span::styled(format!("Connect to \"{ssid}\" "), t.style_accent_bold()),
        ]))
        .borders(Borders::ALL)
        .border_type(t.border_type)
        .border_style(t.style_accent())
        .style(t.style_default());

    frame.render_widget(block, dialog);

    // Password input field
    let inner = Rect {
        x: dialog.x + 3,
        y: dialog.y + 2,
        width: dialog.width.saturating_sub(6),
        height: 1,
    };

    let label = Span::styled("Password: ", t.style_dim());

    let password_display = if app.password_visible {
        app.password_input.clone()
    } else {
        "●".repeat(app.password_input.len())
    };

    // Cursor
    let cursor_char = if app.animation.cursor_visible() {
        "█"
    } else {
        " "
    };

    let input_line = Line::from(vec![
        label,
        Span::styled(password_display, t.style_default()),
        Span::styled(cursor_char.to_string(), t.style_accent()),
    ]);

    frame.render_widget(Paragraph::new(input_line), inner);

    // Show/hide hint
    let toggle_hint = if app.password_visible {
        "[Ctrl+H] Hide"
    } else {
        "[Ctrl+H] Show"
    };

    let hint_area = Rect {
        x: dialog.x + 3,
        y: dialog.y + height.saturating_sub(3),
        width: dialog.width.saturating_sub(6),
        height: 1,
    };

    let hints = Line::from(vec![
        Span::styled("[Enter]", t.style_key_hint()),
        Span::styled(" Connect  ", t.style_key_desc()),
        Span::styled("[Esc]", t.style_key_hint()),
        Span::styled(" Cancel  ", t.style_key_desc()),
        Span::styled(toggle_hint, t.style_key_desc()),
    ]);

    frame.render_widget(
        Paragraph::new(hints)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true }),
        hint_area,
    );
}
