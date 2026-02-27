use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::theme;
use crate::app::App;
use crate::network::types::{ConnectionStatus, FrequencyBand, channel_from_frequency};
use crate::ui::theme::Theme;

/// Render the network detail panel (right side)
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let nerd = app.config.nerd_fonts();
    let t = &app.theme;
    let info_icon = if nerd { theme::ICON_INFO } else { "(i) " };

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(format!(" {info_icon}"), t.style_accent()),
            Span::styled("Details ", t.style_accent_bold()),
        ]))
        .borders(Borders::ALL)
        .border_type(t.border_type)
        .border_style(t.style_border())
        .style(t.style_default());

    if app.networks.is_empty() {
        let para = Paragraph::new("No network selected")
            .block(block)
            .style(t.style_dim())
            .alignment(Alignment::Center);
        frame.render_widget(para, area);
        return;
    }

    let selected = match app.selected_network() {
        Some(net) => net,
        None => {
            let para = Paragraph::new("No network selected")
                .block(block)
                .style(t.style_dim())
                .alignment(Alignment::Center);
            frame.render_widget(para, area);
            return;
        }
    };

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        detail_line(t, "  SSID", &selected.ssid),
        detail_line(t, "  BSSID", &selected.bssid),
        detail_line(t, "  AP Path", &selected.ap_path),
        Line::from(""),
    ];

    // Signal
    let sig_color = t.signal_color(selected.signal_strength);
    lines.push(Line::from(vec![
        Span::styled("  Signal      ", t.style_dim()),
        Span::styled(
            format!("{}%", selected.signal_strength),
            ratatui::style::Style::default().fg(sig_color),
        ),
        Span::styled(
            format!("  {}", signal_bar(selected.signal_strength)),
            ratatui::style::Style::default().fg(sig_color),
        ),
    ]));

    // Frequency & Channel
    let band = selected.band();
    let channel = selected.channel();
    let freq_str = format!("{} MHz ({})", selected.frequency, band);
    lines.push(detail_line(t, "  Frequency", &freq_str));
    let chan_str = format!("{}", channel);
    lines.push(detail_line(t, "  Channel", &chan_str));
    lines.push(Line::from(""));

    // Security
    let sec_style = if selected.security == crate::network::types::SecurityType::Open {
        t.style_warning()
    } else {
        t.style_default()
    };
    lines.push(Line::from(vec![
        Span::styled("  Security    ", t.style_dim()),
        Span::styled(selected.security.to_string(), sec_style),
    ]));

    // Saved
    lines.push(detail_line(
        t,
        "  Saved",
        if selected.is_saved { "Yes" } else { "No" },
    ));
    lines.push(detail_line(
        t,
        "  Status",
        if selected.is_active {
            "Connected"
        } else {
            "Not connected"
        },
    ));

    // Active connection details
    if selected.is_active
        && let ConnectionStatus::Connected(ref info) = app.connection_status
    {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ── Connection Info ──",
            ratatui::style::Style::default().fg(t.accent2),
        )));
        lines.push(Line::from(""));

        if let Some(ref ip) = info.ip4 {
            lines.push(detail_line(t, "  IPv4", ip));
        }
        if let Some(ref ip6) = info.ip6 {
            lines.push(detail_line(t, "  IPv6", ip6));
        }
        if let Some(ref gw) = info.gateway {
            lines.push(detail_line(t, "  Gateway", gw));
        }
        if !info.dns.is_empty() {
            lines.push(detail_line(t, "  DNS", &info.dns.join(", ")));
        }
        lines.push(detail_line(t, "  MAC", &info.mac));
        lines.push(detail_line(t, "  BSSID", &info.bssid));
        lines.push(detail_line(t, "  Interface", &info.interface));
        if info.speed > 0 {
            let speed_str = format!("{} Mbps", info.speed);
            lines.push(detail_line(t, "  Speed", &speed_str));
        }
        if info.frequency > 0 {
            let band = FrequencyBand::from_mhz(info.frequency);
            let ch = channel_from_frequency(info.frequency);
            let freq_str = format!("{} MHz ({}, ch {})", info.frequency, band, ch);
            lines.push(detail_line(t, "  Frequency", &freq_str));
        }
        if info.signal > 0 {
            lines.push(detail_line(t, "  Signal", &format!("{}%", info.signal)));
        }
    }

    let para = Paragraph::new(lines).block(block).style(t.style_default());

    frame.render_widget(para, area);
}

/// Build a key-value detail line (owns its data)
fn detail_line(t: &Theme, label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<14}", label), t.style_dim()),
        Span::styled(value.to_string(), t.style_default()),
    ])
}

/// Generate a text-based signal strength bar
fn signal_bar(strength: u8) -> String {
    let filled = (strength as usize * 10) / 100;
    let empty = 10 - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}
