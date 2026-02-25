use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::ui::theme::Theme;

/// A Yes/No confirmation dialog
#[derive(Debug, Clone)]
pub struct ConfirmDialog {
    pub title: String,
    pub message: String,
    pub visible: bool,
    /// Callback data — stores context info for the action to confirm
    pub context: Option<String>,
}

impl ConfirmDialog {
    pub fn new() -> Self {
        Self {
            title: String::new(),
            message: String::new(),
            visible: false,
            context: None,
        }
    }

    pub fn show(&mut self, title: &str, message: &str, context: Option<String>) {
        self.title = title.to_string();
        self.message = message.to_string();
        self.context = context;
        self.visible = true;
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.context = None;
    }

    pub fn render(&self, f: &mut Frame, theme: &Theme) {
        if !self.visible {
            return;
        }

        let area = centered_rect(40, 6, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .title(Span::styled(
                format!(" {} ", self.title),
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(theme.dialog_border);

        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Length(1)])
            .split(inner);

        // Message
        let msg = Paragraph::new(Line::from(Span::styled(
            &self.message,
            Style::default().fg(theme.fg),
        )))
        .alignment(Alignment::Center);
        f.render_widget(msg, chunks[0]);

        // Actions
        let actions = Line::from(vec![
            Span::styled(" y ", theme.help_key),
            Span::styled("Yes ", Style::default().fg(theme.fg_dim)),
            Span::styled("│", Style::default().fg(theme.border)),
            Span::styled(" n ", theme.help_key),
            Span::styled("No ", Style::default().fg(theme.fg_dim)),
        ]);
        let actions_p = Paragraph::new(actions).alignment(Alignment::Center);
        f.render_widget(actions_p, chunks[1]);
    }
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((area.height.saturating_sub(height)) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
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
