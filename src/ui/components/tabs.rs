use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Tabs as RataTabs};
use ratatui::Frame;

use crate::app::Page;
use crate::ui::theme::Theme;

/// Tab titles for the top navigation bar
const TAB_TITLES: &[(&str, &str)] = &[
    ("1", " Dashboard "),
    ("2", " Interfaces "),
    ("3", " WiFi "),
    ("4", " Connections "),
    ("5", " Diagnostics "),
];

/// Render the top navigation tab bar
pub fn render(f: &mut Frame, active_page: &Page, theme: &Theme, area: Rect) {
    let active_idx = match active_page {
        Page::Dashboard => 0,
        Page::Interfaces => 1,
        Page::Wifi => 2,
        Page::Connections => 3,
        Page::Diagnostics => 4,
    };

    let titles: Vec<Line> = TAB_TITLES
        .iter()
        .enumerate()
        .map(|(i, (num, label))| {
            let style = if i == active_idx {
                theme.tab_active
            } else {
                theme.tab_inactive
            };
            Line::from(vec![
                Span::styled(*num, style.add_modifier(Modifier::BOLD)),
                Span::styled(*label, style),
            ])
        })
        .collect();

    let tabs = RataTabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(theme.border))
                .title(Span::styled(
                    " 󰛳 nexus-nm ",
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .select(active_idx)
        .highlight_style(theme.tab_active)
        .divider(Span::styled(" │ ", Style::default().fg(theme.border)));

    f.render_widget(tabs, area);
}
