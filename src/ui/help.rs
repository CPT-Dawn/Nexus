use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::App;

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
    ("/", "Search / filter networks"),
    ("S", "Cycle sort mode"),
    ("Ctrl+H", "Show/hide password"),
    ("Tab", "Switch fields (in dialogs)"),
    ("Esc", "Close dialog / cancel"),
    ("?", "Toggle this help"),
    ("q", "Quit Nexus"),
];

/// Render the help overlay
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let width = 52_u16.min(area.width.saturating_sub(4));
    let height = (KEYBINDINGS.len() as u16 + 6).min(area.height.saturating_sub(2));

    let dialog = super::centered_rect_fixed(width, height, area);
    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled("  ", t.style_accent()),
            Span::styled(" Keybindings ", t.style_accent_bold()),
        ]))
        .borders(Borders::ALL)
        .border_type(t.border_type)
        .border_style(t.style_accent())
        .style(t.style_default());

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    for (key, desc) in KEYBINDINGS {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<12}", key), t.style_key_hint()),
            Span::styled(*desc, t.style_default()),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Press ", t.style_dim()),
        Span::styled("?", t.style_key_hint()),
        Span::styled(" or ", t.style_dim()),
        Span::styled("Esc", t.style_key_hint()),
        Span::styled(" to close", t.style_dim()),
    ]));

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, dialog);
}
