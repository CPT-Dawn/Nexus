use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use crate::app::App;
use crate::network::types::*;

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let theme = &app.theme;

    let state = match app.network_state.clone() {
        Some(s) => s,
        None => {
            let msg = Paragraph::new("Loading connections...")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.fg_dim));
            f.render_widget(msg, area);
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    // ── Left: Connection list ─────────────────────────────────────────
    render_connection_list(f, &*app, &state, chunks[0]);

    // ── Right: Connection detail ──────────────────────────────────────
    render_connection_detail(f, app, &state, chunks[1]);
}

fn render_connection_list(f: &mut Frame, app: &App, state: &NetworkState, area: Rect) {
    let theme = &app.theme;

    let block = Block::default()
        .title(Span::styled(
            " Saved Connections ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(theme.block_style(true));

    if state.saved_connections.is_empty() {
        let msg = Paragraph::new("No saved connections")
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.fg_dim))
            .block(block);
        f.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("Name"),
        Cell::from("Type"),
        Cell::from("Auto"),
        Cell::from("Status"),
    ])
    .style(theme.table_header)
    .height(1);

    // Determine which connections are currently active
    let active_uuids: std::collections::HashSet<String> = state
        .active_connections
        .iter()
        .map(|ac| ac.uuid.clone())
        .collect();

    let rows: Vec<Row> = state
        .saved_connections
        .iter()
        .enumerate()
        .map(|(i, conn)| {
            let is_active = active_uuids.contains(&conn.uuid);
            let status = if is_active { "● Active" } else { "" };
            let status_color = if is_active {
                theme.success
            } else {
                theme.fg_dim
            };

            let row = Row::new(vec![
                Cell::from(conn.type_icon()),
                Cell::from(conn.id.clone()),
                Cell::from(conn.type_label()),
                Cell::from(if conn.autoconnect { "✓" } else { "✗" }),
                Cell::from(Span::styled(status, Style::default().fg(status_color))),
            ]);

            if i == app.connections_state.selected_index {
                row.style(theme.table_row_selected)
            } else {
                row
            }
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Min(12),
        Constraint::Length(10),
        Constraint::Length(5),
        Constraint::Length(10),
    ];

    let mut table_state = TableState::default();
    table_state.select(Some(app.connections_state.selected_index));

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(theme.table_row_selected);

    f.render_stateful_widget(table, area, &mut table_state);
}

fn render_connection_detail(f: &mut Frame, app: &App, state: &NetworkState, area: Rect) {
    let theme = &app.theme;

    let block = Block::default()
        .title(Span::styled(
            " Connection Details ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(theme.block_style(false));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let conn = match state
        .saved_connections
        .get(app.connections_state.selected_index)
    {
        Some(c) => c,
        None => {
            let msg = Paragraph::new("No connection selected")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.fg_dim));
            f.render_widget(msg, inner);
            return;
        }
    };

    let active_uuids: std::collections::HashSet<String> = state
        .active_connections
        .iter()
        .map(|ac| ac.uuid.clone())
        .collect();
    let is_active = active_uuids.contains(&conn.uuid);

    let mut lines = Vec::new();

    // Name
    lines.push(Line::from(vec![Span::styled(
        format!("{}{}", conn.type_icon(), conn.id),
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    // Status
    if is_active {
        lines.push(Line::from(Span::styled(
            "● Currently Active",
            Style::default()
                .fg(theme.success)
                .add_modifier(Modifier::BOLD),
        )));
        // Show active connection runtime details
        if let Some(active) = state
            .active_connections
            .iter()
            .find(|ac| ac.uuid == conn.uuid)
        {
            add_detail(&mut lines, "Active Name", &active.id, theme);
            add_detail(&mut lines, "Active Type", &active.conn_type, theme);
            add_detail(&mut lines, "State", &format!("{:?}", active.state), theme);
            add_detail(
                &mut lines,
                "Devices",
                &format!("{}", active.devices.len()),
                theme,
            );
        }
    } else {
        lines.push(Line::from(Span::styled(
            "○ Inactive",
            Style::default().fg(theme.fg_dim),
        )));
    }
    lines.push(Line::from(""));

    add_detail(&mut lines, "UUID", &conn.uuid, theme);
    add_detail(&mut lines, "Type", conn.type_label(), theme);

    if let Some(ref iface) = conn.interface {
        add_detail(&mut lines, "Interface", iface, theme);
    }

    add_detail(
        &mut lines,
        "Autoconnect",
        if conn.autoconnect { "Yes" } else { "No" },
        theme,
    );

    if conn.timestamp > 0 {
        let dt = chrono::DateTime::from_timestamp(conn.timestamp as i64, 0)
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        add_detail(&mut lines, "Last Used", &dt, theme);
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "── Actions ──",
        Style::default().fg(theme.fg_dim),
    )));

    if is_active {
        lines.push(Line::from(vec![
            Span::styled(" d ", theme.help_key),
            Span::styled(" Deactivate", Style::default().fg(theme.fg_dim)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(" Enter ", theme.help_key),
            Span::styled(" Activate", Style::default().fg(theme.fg_dim)),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled(" x ", theme.help_key),
        Span::styled(" Delete", Style::default().fg(theme.fg_dim)),
    ]));

    let detail = Paragraph::new(lines);
    f.render_widget(detail, inner);
}

fn add_detail(
    lines: &mut Vec<Line<'_>>,
    label: &str,
    value: &str,
    theme: &crate::ui::theme::Theme,
) {
    lines.push(Line::from(vec![
        Span::styled(format!("{:<14}", label), Style::default().fg(theme.fg_dim)),
        Span::styled(value.to_string(), Style::default().fg(theme.fg)),
    ]));
}
