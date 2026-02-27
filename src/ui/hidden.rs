use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::theme;
use crate::app::App;

/// Render the hidden network connection modal
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let width = 56_u16.min(area.width.saturating_sub(4));
    let height = 11_u16.min(area.height.saturating_sub(4));

    let y_offset = app.animation.dialog_y_offset();
    let dialog = super::centered_rect_fixed(width, height, area);
    let dialog = Rect {
        y: dialog.y.saturating_add(y_offset),
        ..dialog
    };

    frame.render_widget(Clear, dialog);

    let nerd = app.config.nerd_fonts();
    let icon = if nerd {
        theme::ICON_HIDDEN
    } else {
        theme::PLAIN_HIDDEN
    };

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(format!(" {icon}"), t.style_accent()),
            Span::styled(" Connect to Hidden Network ", t.style_accent_bold()),
        ]))
        .borders(Borders::ALL)
        .border_type(t.border_type)
        .border_style(t.style_accent())
        .style(t.style_default());

    frame.render_widget(block, dialog);

    let cursor_char = if app.animation.cursor_visible() {
        "█"
    } else {
        " "
    };

    // SSID field
    let ssid_area = Rect {
        x: dialog.x + 3,
        y: dialog.y + 2,
        width: dialog.width.saturating_sub(6),
        height: 1,
    };

    let ssid_label_style = if app.hidden_field_focus == 0 {
        t.style_accent()
    } else {
        t.style_dim()
    };

    let ssid_line = Line::from(vec![
        Span::styled("SSID:     ", ssid_label_style),
        Span::styled(app.hidden_ssid_input.clone(), t.style_default()),
        if app.hidden_field_focus == 0 {
            Span::styled(cursor_char.to_string(), t.style_accent())
        } else {
            Span::raw("")
        },
    ]);
    frame.render_widget(Paragraph::new(ssid_line), ssid_area);

    // Password field
    let pwd_area = Rect {
        x: dialog.x + 3,
        y: dialog.y + 4,
        width: dialog.width.saturating_sub(6),
        height: 1,
    };

    let pwd_label_style = if app.hidden_field_focus == 1 {
        t.style_accent()
    } else {
        t.style_dim()
    };

    let pwd_display = if app.password_visible {
        app.hidden_password_input.clone()
    } else {
        "●".repeat(app.hidden_password_input.len())
    };

    let pwd_line = Line::from(vec![
        Span::styled("Password: ", pwd_label_style),
        Span::styled(pwd_display, t.style_default()),
        if app.hidden_field_focus == 1 {
            Span::styled(cursor_char.to_string(), t.style_accent())
        } else {
            Span::raw("")
        },
    ]);
    frame.render_widget(Paragraph::new(pwd_line), pwd_area);

    // Optional label
    let opt_area = Rect {
        x: dialog.x + 13,
        y: dialog.y + 5,
        width: dialog.width.saturating_sub(16),
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            "(leave empty for open networks)",
            t.style_dim(),
        )),
        opt_area,
    );

    // Hints
    let hint_area = Rect {
        x: dialog.x + 3,
        y: dialog.y + height.saturating_sub(3),
        width: dialog.width.saturating_sub(6),
        height: 1,
    };

    let hints = Line::from(vec![
        Span::styled("[Tab]", t.style_key_hint()),
        Span::styled(" Switch  ", t.style_key_desc()),
        Span::styled("[Enter]", t.style_key_hint()),
        Span::styled(" Connect  ", t.style_key_desc()),
        Span::styled("[Esc]", t.style_key_hint()),
        Span::styled(" Cancel ", t.style_key_desc()),
    ]);

    frame.render_widget(
        Paragraph::new(hints)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true }),
        hint_area,
    );
}
