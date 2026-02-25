use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Sparkline, Table};
use ratatui::Frame;

use crate::app::App;
use crate::network::types::*;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    let state = match &app.network_state {
        Some(s) => s,
        None => {
            let msg = Paragraph::new("Connecting to NetworkManager...")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.fg_dim));
            f.render_widget(msg, area);
            return;
        }
    };

    if !state.nm_running {
        let msg = Paragraph::new(Line::from(vec![
            Span::styled("✗ ", Style::default().fg(theme.error)),
            Span::raw("NetworkManager is not running"),
        ]))
        .alignment(Alignment::Center);
        f.render_widget(msg, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // Header: connectivity + hostname + version
            Constraint::Min(8),    // Interface table
            Constraint::Length(6), // Bandwidth sparklines
        ])
        .split(area);

    // ── Header section ────────────────────────────────────────────────
    render_header(f, state, theme, chunks[0]);

    // ── Interface table ───────────────────────────────────────────────
    render_interface_table(f, state, theme, chunks[1]);

    // ── Bandwidth sparklines ──────────────────────────────────────────
    render_bandwidth(f, app, state, theme, chunks[2]);
}

fn render_header(f: &mut Frame, state: &NetworkState, theme: &crate::ui::theme::Theme, area: Rect) {
    let block = Block::default()
        .title(Span::styled(
            " 󰈀 Overview ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(34),
            Constraint::Percentage(33),
        ])
        .split(inner);

    // Connectivity
    let conn_style = theme.connectivity_style(&state.connectivity);
    let conn_icon = match state.connectivity {
        ConnectivityState::Full => "󰖩 ",
        ConnectivityState::Limited => "󰖪 ",
        ConnectivityState::Portal => "󰖧 ",
        ConnectivityState::None => "󰖪 ",
        ConnectivityState::Unknown => "? ",
    };
    let connectivity = Paragraph::new(vec![
        Line::from(Span::styled(
            "Connectivity",
            Style::default()
                .fg(theme.fg_dim)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(conn_icon, conn_style),
            Span::styled(state.connectivity.label(), conn_style),
        ]),
    ]);
    f.render_widget(connectivity, cols[0]);

    // Hostname & Version
    let info = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Host: ", Style::default().fg(theme.fg_dim)),
            Span::styled(&state.hostname, Style::default().fg(theme.fg)),
        ]),
        Line::from(vec![
            Span::styled("NM: ", Style::default().fg(theme.fg_dim)),
            Span::styled(
                format!("v{}", state.nm_version),
                Style::default().fg(theme.fg),
            ),
        ]),
    ])
    .alignment(Alignment::Center);
    f.render_widget(info, cols[1]);

    // Active connections count + WiFi status
    let active_count = state.active_connections.len();
    let wifi_label = if state.wireless_enabled {
        Span::styled("WiFi: ON", Style::default().fg(theme.success))
    } else {
        Span::styled("WiFi: OFF", Style::default().fg(theme.error))
    };
    let net_label = if state.networking_enabled {
        Span::styled("Net: ON", Style::default().fg(theme.success))
    } else {
        Span::styled("Net: OFF", Style::default().fg(theme.error))
    };

    let summary = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Active: ", Style::default().fg(theme.fg_dim)),
            Span::styled(
                format!(
                    "{} connection{}",
                    active_count,
                    if active_count == 1 { "" } else { "s" }
                ),
                Style::default().fg(theme.fg),
            ),
        ]),
        Line::from(vec![wifi_label, Span::raw("  "), net_label]),
    ])
    .alignment(Alignment::Right);
    f.render_widget(summary, cols[2]);
}

fn render_interface_table(
    f: &mut Frame,
    state: &NetworkState,
    theme: &crate::ui::theme::Theme,
    area: Rect,
) {
    let block = Block::default()
        .title(Span::styled(
            " Interfaces ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border));

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("Interface"),
        Cell::from("Type"),
        Cell::from("State"),
        Cell::from("IP Address"),
        Cell::from("Connection"),
        Cell::from("↓ RX/s"),
        Cell::from("↑ TX/s"),
    ])
    .style(theme.table_header)
    .height(1);

    let rows: Vec<Row> = state
        .devices
        .iter()
        .map(|dev| {
            let state_color = theme.device_state_color(&dev.state);
            let stats = state.stats.get(&dev.interface);
            let rx_rate = stats
                .map(|s| InterfaceStats::format_rate(s.rx_rate))
                .unwrap_or_else(|| "—".to_string());
            let tx_rate = stats
                .map(|s| InterfaceStats::format_rate(s.tx_rate))
                .unwrap_or_else(|| "—".to_string());

            Row::new(vec![
                Cell::from(dev.device_type.icon()),
                Cell::from(dev.interface.clone()),
                Cell::from(dev.device_type.label()),
                Cell::from(Span::styled(
                    dev.state.label(),
                    Style::default().fg(state_color),
                )),
                Cell::from(dev.display_ip()),
                Cell::from(
                    dev.connection_name
                        .clone()
                        .unwrap_or_else(|| "—".to_string()),
                ),
                Cell::from(rx_rate),
                Cell::from(tx_rate),
            ])
            .style(theme.table_row)
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Length(14),
        Constraint::Length(16),
        Constraint::Length(16),
        Constraint::Length(12),
        Constraint::Length(12),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(theme.table_row_selected);

    f.render_widget(table, area);
}

fn render_bandwidth(
    f: &mut Frame,
    _app: &App,
    state: &NetworkState,
    theme: &crate::ui::theme::Theme,
    area: Rect,
) {
    let block = Block::default()
        .title(Span::styled(" 󰓅 Bandwidth ", theme.title_style()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Find the primary connected interface
    let primary_iface = state
        .devices
        .iter()
        .find(|d| d.state.is_connected() && d.device_type != DeviceType::Loopback);

    if let Some(dev) = primary_iface {
        if let Some(stats) = state.stats.get(&dev.interface) {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(inner);

            // RX sparkline
            let rx_data: Vec<u64> = stats.rx_history.iter().map(|v| *v as u64).collect();
            let rx_label = format!(
                "↓ {} ({}) ",
                dev.interface,
                InterfaceStats::format_rate(stats.rx_rate)
            );
            let rx_spark = Sparkline::default()
                .block(
                    Block::default()
                        .title(Span::styled(rx_label, Style::default().fg(theme.success))),
                )
                .data(&rx_data)
                .style(Style::default().fg(theme.sparkline_fg));
            f.render_widget(rx_spark, cols[0]);

            // TX sparkline
            let tx_data: Vec<u64> = stats.tx_history.iter().map(|v| *v as u64).collect();
            let tx_label = format!(
                "↑ {} ({}) ",
                dev.interface,
                InterfaceStats::format_rate(stats.tx_rate)
            );
            let tx_spark = Sparkline::default()
                .block(
                    Block::default().title(Span::styled(tx_label, Style::default().fg(theme.info))),
                )
                .data(&tx_data)
                .style(Style::default().fg(theme.sparkline_fg));
            f.render_widget(tx_spark, cols[1]);

            return;
        }
    }

    let msg = Paragraph::new("No active connection")
        .alignment(Alignment::Center)
        .style(Style::default().fg(theme.fg_dim));
    f.render_widget(msg, inner);
}
