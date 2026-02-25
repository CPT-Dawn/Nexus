use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use crate::app::App;
use crate::network::types::*;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    let state = match &app.network_state {
        Some(s) => s,
        None => {
            let msg = Paragraph::new("Loading interfaces...")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.fg_dim));
            f.render_widget(msg, area);
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    // ── Left: Interface list ──────────────────────────────────────────
    render_interface_list(f, app, state, chunks[0]);

    // ── Right: Detail panel ───────────────────────────────────────────
    render_detail_panel(f, app, state, chunks[1]);
}

fn render_interface_list(f: &mut Frame, app: &App, state: &NetworkState, area: Rect) {
    let theme = &app.theme;

    let block = Block::default()
        .title(Span::styled(
            " Interfaces ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(theme.block_style(true));

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("Name"),
        Cell::from("Type"),
        Cell::from("State"),
        Cell::from("IP"),
    ])
    .style(theme.table_header)
    .height(1);

    let rows: Vec<Row> = state
        .devices
        .iter()
        .enumerate()
        .map(|(i, dev)| {
            let state_color = theme.device_state_color(&dev.state);
            let selected = app.interfaces_state.selected_index == i;

            let row = Row::new(vec![
                Cell::from(dev.device_type.icon()),
                Cell::from(dev.interface.clone()),
                Cell::from(dev.device_type.label()),
                Cell::from(Span::styled(
                    dev.state.label(),
                    Style::default().fg(state_color),
                )),
                Cell::from(dev.display_ip()),
            ]);

            if selected {
                row.style(theme.table_row_selected)
            } else {
                row
            }
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Length(14),
        Constraint::Min(10),
    ];

    let mut table_state = TableState::default();
    table_state.select(Some(app.interfaces_state.selected_index));

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(theme.table_row_selected);

    f.render_stateful_widget(table, area, &mut table_state);
}

fn render_detail_panel(f: &mut Frame, app: &App, state: &NetworkState, area: Rect) {
    let theme = &app.theme;

    let block = Block::default()
        .title(Span::styled(
            " Details ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(theme.block_style(false));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let device = match state.devices.get(app.interfaces_state.selected_index) {
        Some(d) => d,
        None => {
            let msg = Paragraph::new("No interface selected")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.fg_dim));
            f.render_widget(msg, inner);
            return;
        }
    };

    let mut lines = Vec::new();

    // Interface name with icon
    lines.push(Line::from(vec![Span::styled(
        format!("{}{}", device.device_type.icon(), device.interface),
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    // Status
    let state_color = theme.device_state_color(&device.state);
    add_detail_line(
        &mut lines,
        "State",
        device.state.label(),
        state_color,
        theme,
    );
    add_detail_line(
        &mut lines,
        "Type",
        device.device_type.label(),
        theme.fg,
        theme,
    );

    if let Some(ref name) = device.connection_name {
        add_detail_line(&mut lines, "Connection", name, theme.fg, theme);
    }

    lines.push(Line::from(""));

    // Addresses
    add_detail_line(&mut lines, "MAC", &device.hw_address, theme.fg, theme);

    if let Some(ref ip) = device.ip4_address {
        let ip_str = match &device.ip4_subnet {
            Some(prefix) => format!("{}{}", ip, prefix),
            None => ip.clone(),
        };
        add_detail_line(&mut lines, "IPv4", &ip_str, theme.success, theme);
    }

    if let Some(ref ip) = device.ip6_address {
        add_detail_line(&mut lines, "IPv6", ip, theme.info, theme);
    }

    if let Some(ref gw) = device.ip4_gateway {
        add_detail_line(&mut lines, "Gateway", gw, theme.fg, theme);
    }

    if !device.ip4_dns.is_empty() {
        add_detail_line(
            &mut lines,
            "DNS",
            &device.ip4_dns.join(", "),
            theme.fg,
            theme,
        );
    }

    lines.push(Line::from(""));

    // Hardware
    add_detail_line(&mut lines, "MTU", &device.mtu.to_string(), theme.fg, theme);
    add_detail_line(&mut lines, "Driver", &device.driver, theme.fg, theme);

    if device.speed > 0 {
        add_detail_line(
            &mut lines,
            "Speed",
            &format!("{} Mbps", device.speed),
            theme.fg,
            theme,
        );
    }

    add_detail_line(
        &mut lines,
        "Autoconnect",
        if device.autoconnect { "Yes" } else { "No" },
        theme.fg,
        theme,
    );

    // Stats
    if let Some(stats) = state.stats.get(&device.interface) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "── Traffic ──",
            Style::default().fg(theme.fg_dim),
        )));
        add_detail_line(
            &mut lines,
            "RX",
            &format!(
                "{} ({}/s)",
                InterfaceStats::format_bytes(stats.rx_bytes),
                InterfaceStats::format_rate(stats.rx_rate)
            ),
            theme.success,
            theme,
        );
        add_detail_line(
            &mut lines,
            "TX",
            &format!(
                "{} ({}/s)",
                InterfaceStats::format_bytes(stats.tx_bytes),
                InterfaceStats::format_rate(stats.tx_rate)
            ),
            theme.info,
            theme,
        );
        add_detail_line(
            &mut lines,
            "Packets",
            &format!("↓{} ↑{}", stats.rx_packets, stats.tx_packets),
            theme.fg,
            theme,
        );
        if stats.rx_errors > 0 || stats.tx_errors > 0 {
            add_detail_line(
                &mut lines,
                "Errors",
                &format!("↓{} ↑{}", stats.rx_errors, stats.tx_errors),
                theme.error,
                theme,
            );
        }
        if stats.rx_dropped > 0 || stats.tx_dropped > 0 {
            add_detail_line(
                &mut lines,
                "Dropped",
                &format!("↓{} ↑{}", stats.rx_dropped, stats.tx_dropped),
                theme.warning,
                theme,
            );
        }
    }

    let detail = Paragraph::new(lines);
    f.render_widget(detail, inner);
}

fn add_detail_line(
    lines: &mut Vec<Line<'_>>,
    label: &str,
    value: &str,
    value_color: ratatui::style::Color,
    theme: &crate::ui::theme::Theme,
) {
    lines.push(Line::from(vec![
        Span::styled(format!("{:<14}", label), Style::default().fg(theme.fg_dim)),
        Span::styled(value.to_string(), Style::default().fg(value_color)),
    ]));
}
