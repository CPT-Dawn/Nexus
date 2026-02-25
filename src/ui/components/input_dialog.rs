use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::ui::theme::Theme;

/// State for a text input dialog (e.g., WiFi password)
#[derive(Debug, Clone)]
pub struct InputDialog {
    pub title: String,
    pub prompt: String,
    pub input: String,
    pub cursor_pos: usize,
    pub masked: bool,
    pub visible: bool,
}

impl InputDialog {
    pub fn new(title: &str, prompt: &str, masked: bool) -> Self {
        Self {
            title: title.to_string(),
            prompt: prompt.to_string(),
            input: String::new(),
            cursor_pos: 0,
            masked,
            visible: false,
        }
    }

    pub fn show(&mut self) {
        self.input.clear();
        self.cursor_pos = 0;
        self.visible = true;
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.input.clear();
        self.cursor_pos = 0;
    }

    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor_pos, c);
        self.cursor_pos += 1;
    }

    pub fn delete_char(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            self.input.remove(self.cursor_pos);
        }
    }

    pub fn delete_forward(&mut self) {
        if self.cursor_pos < self.input.len() {
            self.input.remove(self.cursor_pos);
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor_pos < self.input.len() {
            self.cursor_pos += 1;
        }
    }

    pub fn move_start(&mut self) {
        self.cursor_pos = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor_pos = self.input.len();
    }

    pub fn value(&self) -> &str {
        &self.input
    }

    pub fn render(&self, f: &mut Frame, theme: &Theme) {
        if !self.visible {
            return;
        }

        let area = centered_rect(50, 7, f.area());

        // Clear background
        f.render_widget(Clear, area);

        let block = Block::default()
            .title(Span::styled(
                format!(" {} ", self.title),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(theme.dialog_border);

        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .margin(0)
            .split(inner);

        // Prompt (styled with inactive input theme)
        let prompt = Paragraph::new(Line::from(Span::styled(&self.prompt, theme.input_inactive)));
        f.render_widget(prompt, chunks[0]);

        // Input field
        let display_text = if self.masked {
            "●".repeat(self.input.len())
        } else {
            self.input.clone()
        };

        let input_line = Line::from(vec![
            Span::styled("❯ ", Style::default().fg(theme.accent)),
            Span::styled(display_text, theme.input_active),
        ]);

        let input = Paragraph::new(input_line);
        f.render_widget(input, chunks[1]);

        // Place cursor
        let cursor_x = chunks[1].x + 2 + self.cursor_pos as u16;
        let cursor_y = chunks[1].y;
        f.set_cursor_position((cursor_x, cursor_y));

        // Hint
        let hint = Paragraph::new(Line::from(Span::styled(
            "Enter to submit │ Esc to cancel",
            Style::default().fg(theme.fg_muted),
        )))
        .alignment(Alignment::Center);
        f.render_widget(hint, chunks[2]);
    }
}

/// Create a centered rectangle of given percentage width and fixed height
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
