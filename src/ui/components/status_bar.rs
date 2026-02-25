use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, Mode, Page};
use crate::auth::PermissionLevel;
use crate::ui::theme::Theme;

/// Render the bottom status bar with contextual keybindings and status info
pub fn render(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);

    // Left: keybindings
    let keys = get_keybindings(app);
    let key_spans: Vec<Span> = keys
        .iter()
        .flat_map(|(key, desc)| {
            vec![
                Span::styled(format!(" {} ", key), theme.help_key),
                Span::styled(format!("{} ", desc), theme.help_desc),
                Span::styled("│", Style::default().fg(theme.border)),
            ]
        })
        .collect();

    let help_line = Line::from(key_spans);
    let help = Paragraph::new(help_line).style(theme.status_bar);
    f.render_widget(help, chunks[0]);

    // Right: status info
    let perm_label = match app.permission_level {
        PermissionLevel::Full => Span::styled(" ● Full", Style::default().fg(theme.success)),
        PermissionLevel::ReadOnly => {
            Span::styled(" ○ Read-Only", Style::default().fg(theme.warning))
        }
        PermissionLevel::Unknown => Span::styled(" … Checking", Style::default().fg(theme.fg_dim)),
    };

    let connectivity = if let Some(ref state) = app.network_state {
        let style = theme.connectivity_style(&state.connectivity);
        Span::styled(format!(" {} ", state.connectivity), style)
    } else {
        Span::styled(" No NM ", Style::default().fg(theme.error))
    };

    let status_line = Line::from(vec![connectivity, Span::raw("│"), perm_label, Span::raw(" ")]);
    let status = Paragraph::new(status_line)
        .style(theme.status_bar)
        .alignment(ratatui::layout::Alignment::Right);
    f.render_widget(status, chunks[1]);
}

fn get_keybindings(app: &App) -> Vec<(&'static str, &'static str)> {
    match app.mode {
        Mode::Normal => {
            let mut keys = vec![
                ("q", "Quit"),
                ("Tab", "Next"),
                ("1-5", "Page"),
                ("r", "Refresh"),
            ];
            match app.active_page {
                Page::Dashboard => {}
                Page::Interfaces => {
                    keys.extend([("↑↓", "Select"), ("Enter", "Details"), ("d", "Disconnect")]);
                }
                Page::Wifi => {
                    keys.extend([
                        ("↑↓", "Select"),
                        ("Enter", "Connect"),
                        ("s", "Scan"),
                        ("/", "Filter"),
                    ]);
                }
                Page::Connections => {
                    keys.extend([
                        ("↑↓", "Select"),
                        ("Enter", "Activate"),
                        ("d", "Deactivate"),
                        ("x", "Delete"),
                    ]);
                }
                Page::Diagnostics => {
                    keys.extend([("↑↓", "Select"), ("Enter", "Run"), ("c", "Clear")]);
                }
            }
            keys
        }
        Mode::Input => vec![("Enter", "Submit"), ("Esc", "Cancel")],
        Mode::Dialog => vec![("y", "Yes"), ("n/Esc", "No")],
        Mode::Filtering => vec![("Enter", "Apply"), ("Esc", "Clear")],
    }
}
