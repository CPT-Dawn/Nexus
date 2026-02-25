use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use std::collections::VecDeque;

use crate::app::App;

/// Available diagnostic tools
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticTool {
    Ping,
    DnsLookup,
    RouteTable,
    DnsServers,
    InterfaceStats,
}

impl DiagnosticTool {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Ping => "󰣐  Ping",
            Self::DnsLookup => "󰇖  DNS Lookup",
            Self::RouteTable => "󰑪  Route Table",
            Self::DnsServers => "  DNS Servers",
            Self::InterfaceStats => "󰈐  Interface Stats",
        }
    }

    pub fn all() -> &'static [DiagnosticTool] {
        &[
            Self::Ping,
            Self::DnsLookup,
            Self::RouteTable,
            Self::DnsServers,
            Self::InterfaceStats,
        ]
    }
}

/// State for the diagnostics page
#[derive(Debug, Clone, Default)]
pub struct DiagnosticsState {
    pub selected_tool: usize,
    pub output: VecDeque<String>,
    pub running: bool,
    pub target: String,
}

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let _theme = &app.theme;

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(24), Constraint::Min(30)])
        .split(area);

    // ── Left: Tool selector ───────────────────────────────────────────
    render_tool_list(f, app, chunks[0]);

    // ── Right: Output ─────────────────────────────────────────────────
    render_output(f, app, chunks[1]);
}

fn render_tool_list(f: &mut Frame, app: &mut App, area: Rect) {
    let theme = &app.theme;

    let block = Block::default()
        .title(Span::styled(
            " Tools ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(theme.block_style(true));

    let items: Vec<ListItem> = DiagnosticTool::all()
        .iter()
        .enumerate()
        .map(|(i, tool)| {
            let style = if i == app.diagnostics_state.selected_tool {
                theme.table_row_selected
            } else {
                Style::default().fg(theme.fg)
            };
            ListItem::new(Line::from(Span::styled(tool.label(), style)))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(app.diagnostics_state.selected_tool));

    let list = List::new(items)
        .block(block)
        .highlight_style(theme.table_row_selected)
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, area, &mut list_state);
}

fn render_output(f: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    let selected_tool = DiagnosticTool::all()
        .get(app.diagnostics_state.selected_tool)
        .copied()
        .unwrap_or(DiagnosticTool::Ping);

    let title = if app.diagnostics_state.running {
        format!(" {} ⏳ Running... ", selected_tool.label())
    } else {
        format!(" {} ", selected_tool.label())
    };

    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(theme.block_style(false));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.diagnostics_state.output.is_empty() {
        let hint = match selected_tool {
            DiagnosticTool::Ping => "Press Enter to ping a host",
            DiagnosticTool::DnsLookup => "Press Enter to look up a domain",
            DiagnosticTool::RouteTable => "Press Enter to show routing table",
            DiagnosticTool::DnsServers => "Press Enter to show DNS servers",
            DiagnosticTool::InterfaceStats => "Press Enter to show interface statistics",
        };

        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(hint, Style::default().fg(theme.fg_dim))),
            Line::from(""),
            Line::from(Span::styled(
                "Press 'c' to clear output",
                Style::default().fg(theme.fg_muted),
            )),
        ])
        .alignment(Alignment::Center);
        f.render_widget(msg, inner);
        return;
    }

    let lines: Vec<Line> = app
        .diagnostics_state
        .output
        .iter()
        .map(|line| {
            // Color code the output
            let style = if line.starts_with(">>>") || line.starts_with("---") {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else if line.contains("error") || line.contains("Error") || line.contains("FAIL") {
                Style::default().fg(theme.error)
            } else if line.contains("OK") || line.contains("success") || line.starts_with("  ") {
                Style::default().fg(theme.fg)
            } else {
                Style::default().fg(theme.fg_dim)
            };
            Line::from(Span::styled(line.as_str(), style))
        })
        .collect();

    let output = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(output, inner);
}
