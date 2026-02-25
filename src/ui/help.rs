use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use super::theme;

/// Keybinding entries: (key, description)
const KEYBINDINGS: &[(&str, &str)] = &[
    ("↑ / k", "Move up"),
    ("↓ / j", "Move down"),
    ("g", "Go to top"),
    ("G", "Go to bottom"),
    ("Enter", "Connect to selected network"),
    ("d", "Disconnect from current network"),
    ("s", "Scan for networks"),
    ("f", "Forget selected network"),
    ("h", "Connect to hidden network"),
    ("i", "Toggle detail panel"),
    ("r", "Refresh connection info"),
    ("Ctrl+H", "Show/hide password"),
    ("Tab", "Switch fields (in dialogs)"),
    ("Esc", "Close dialog / cancel"),
    ("?", "Toggle this help"),
    ("q", "Quit Nexus"),
];

/// Render the help overlay
pub fn render(frame: &mut Frame, area: Rect) {
    let width = 52_u16.min(area.width.saturating_sub(4));
    let height = (KEYBINDINGS.len() as u16 + 6).min(area.height.saturating_sub(2));

    let dialog = super::centered_rect_fixed(width, height, area);
    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled("  ", theme::style_accent()),
            Span::styled(" Keybindings ", theme::style_accent_bold()),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::style_accent())
        .style(theme::style_default());

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    for (key, desc) in KEYBINDINGS {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<12}", key), theme::style_key_hint()),
            Span::styled(*desc, theme::style_default()),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Press ", theme::style_dim()),
        Span::styled("?", theme::style_key_hint()),
        Span::styled(" or ", theme::style_dim()),
        Span::styled("Esc", theme::style_key_hint()),
        Span::styled(" to close", theme::style_dim()),
    ]));

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, dialog);
}
