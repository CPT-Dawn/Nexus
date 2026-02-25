use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use crate::app::App;
use crate::network::types::*;

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let theme = &app.theme;

    let state = match &app.network_state {
        Some(s) => s,
        None => {
            let msg = Paragraph::new("Loading WiFi data...")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.fg_dim));
            f.render_widget(msg, area);
            return;
        }
    };

    if !state.wireless_enabled {
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "󰖪  WiFi is disabled",
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press 'w' to enable WiFi",
                Style::default().fg(theme.fg_dim),
            )),
        ])
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.border))
                .title(Span::styled(
                    " 󰖩 WiFi ",
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )),
        );
        f.render_widget(msg, area);
        return;
    }

    // Check if WiFi device exists
    let has_wifi = state
        .devices
        .iter()
        .any(|d| d.device_type == DeviceType::WiFi);

    if !has_wifi {
        let msg = Paragraph::new("No WiFi device found")
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.error))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.border))
                    .title(" 󰖩 WiFi "),
            );
        f.render_widget(msg, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // WiFi status header
            Constraint::Min(5),    // AP list
        ])
        .split(area);

    // ── WiFi status header ────────────────────────────────────────────
    render_wifi_status(f, app, state, chunks[0]);

    // ── AP list ───────────────────────────────────────────────────────
    render_ap_list(f, &*app, state, chunks[1]);
}

fn render_wifi_status(f: &mut Frame, app: &App, state: &NetworkState, area: Rect) {
    let theme = &app.theme;

    // Find active WiFi connection
    let active_wifi = state
        .devices
        .iter()
        .find(|d| d.device_type == DeviceType::WiFi && d.state.is_connected());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            " 󰖩 WiFi ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let content = if let Some(dev) = active_wifi {
        let conn_name = dev
            .connection_name
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());

        Line::from(vec![
            Span::styled("Connected to: ", Style::default().fg(theme.fg_dim)),
            Span::styled(
                conn_name,
                Style::default()
                    .fg(theme.success)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  │  ", Style::default().fg(theme.border)),
            Span::styled("IP: ", Style::default().fg(theme.fg_dim)),
            Span::styled(dev.display_ip(), Style::default().fg(theme.fg)),
            Span::styled("  │  ", Style::default().fg(theme.border)),
            Span::styled(
                format!("APs: {}", state.wifi_access_points.len()),
                Style::default().fg(theme.fg_dim),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled("Not connected", Style::default().fg(theme.warning)),
            Span::styled("  │  ", Style::default().fg(theme.border)),
            Span::styled(
                format!("Available: {}", state.wifi_access_points.len()),
                Style::default().fg(theme.fg_dim),
            ),
            Span::styled("  │  Press ", Style::default().fg(theme.fg_dim)),
            Span::styled(
                "s",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to scan", Style::default().fg(theme.fg_dim)),
        ])
    };

    let p = Paragraph::new(content);
    f.render_widget(p, inner);
}

fn render_ap_list(f: &mut Frame, app: &App, state: &NetworkState, area: Rect) {
    let theme = &app.theme;

    let block = Block::default()
        .title(Span::styled(
            " Available Networks ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(theme.block_style(true));

    // Apply filter
    let filtered_aps: Vec<&WifiAccessPoint> = if app.wifi_state.filter.is_empty() {
        state.wifi_access_points.iter().collect()
    } else {
        let filter_lower = app.wifi_state.filter.to_lowercase();
        state
            .wifi_access_points
            .iter()
            .filter(|ap| ap.ssid.to_lowercase().contains(&filter_lower))
            .collect()
    };

    if filtered_aps.is_empty() {
        let msg = if state.wifi_access_points.is_empty() {
            "No networks found. Press 's' to scan."
        } else {
            "No networks match your filter."
        };
        let p = Paragraph::new(msg)
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.fg_dim))
            .block(block);
        f.render_widget(p, area);
        return;
    }

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("SSID"),
        Cell::from("Signal"),
        Cell::from("Security"),
        Cell::from("Band"),
        Cell::from("Ch"),
        Cell::from("Saved"),
    ])
    .style(theme.table_header)
    .height(1);

    let rows: Vec<Row> = filtered_aps
        .iter()
        .enumerate()
        .map(|(i, ap)| {
            let signal_color = theme.signal_color(ap.strength);
            let active_indicator = if ap.is_active { "󰖩 " } else { "  " };

            let security_style = match ap.security {
                WifiSecurity::Open => theme.security_open,
                WifiSecurity::Enterprise => theme.security_enterprise,
                _ => theme.security_wpa,
            };

            let saved_icon = if ap.is_saved { "✓" } else { "" };

            let row = Row::new(vec![
                Cell::from(active_indicator),
                Cell::from(Span::styled(
                    ap.ssid.clone(),
                    if ap.is_active {
                        Style::default()
                            .fg(theme.success)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.fg)
                    },
                )),
                Cell::from(Span::styled(
                    format!("{} {}%", ap.signal_bars(), ap.strength),
                    Style::default().fg(signal_color),
                )),
                Cell::from(Span::styled(ap.security.label(), security_style)),
                Cell::from(ap.band()),
                Cell::from(ap.channel.to_string()),
                Cell::from(Span::styled(saved_icon, Style::default().fg(theme.success))),
            ]);

            if i == app.wifi_state.selected_index {
                row.style(theme.table_row_selected)
            } else {
                row
            }
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Min(15),
        Constraint::Length(14),
        Constraint::Length(10),
        Constraint::Length(5),
        Constraint::Length(4),
        Constraint::Length(5),
    ];

    let mut table_state = TableState::default();
    table_state.select(Some(app.wifi_state.selected_index));

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(theme.table_row_selected);

    f.render_stateful_widget(table, area, &mut table_state);
}
