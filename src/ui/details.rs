use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use super::theme;
use crate::app::App;
use crate::network::types::ConnectionStatus;

/// Render the network detail panel (right side)
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(Line::from(Span::styled(
            " Details ",
            theme::style_accent_bold(),
        )))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::style_border())
        .style(theme::style_default());

    if app.networks.is_empty() {
        let para = Paragraph::new("No network selected")
            .block(block)
            .style(theme::style_dim())
            .alignment(Alignment::Center);
        frame.render_widget(para, area);
        return;
    }

    let selected = &app.networks[app.selected_index.min(app.networks.len().saturating_sub(1))];

    let mut lines: Vec<Line> = Vec::new();

    // SSID
    lines.push(Line::from(""));
    lines.push(detail_line("  SSID", &selected.ssid));
    lines.push(detail_line("  BSSID", &selected.bssid));
    lines.push(Line::from(""));

    // Signal
    let sig_color = theme::signal_color(selected.signal_strength);
    lines.push(Line::from(vec![
        Span::styled("  Signal      ", theme::style_dim()),
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
    lines.push(detail_line("  Frequency", &freq_str));
    let chan_str = format!("{}", channel);
    lines.push(detail_line("  Channel", &chan_str));
    lines.push(Line::from(""));

    // Security
    let sec_style = if selected.security == crate::network::types::SecurityType::Open {
        theme::style_warning()
    } else {
        theme::style_default()
    };
    lines.push(Line::from(vec![
        Span::styled("  Security    ", theme::style_dim()),
        Span::styled(selected.security.to_string(), sec_style),
    ]));

    // Saved
    lines.push(detail_line(
        "  Saved",
        if selected.is_saved { "Yes" } else { "No" },
    ));
    lines.push(detail_line(
        "  Status",
        if selected.is_active {
            "Connected"
        } else {
            "Not connected"
        },
    ));

    // Active connection details
    if selected.is_active
        && let ConnectionStatus::Connected(ref info) = app.connection_status {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ── Connection Info ──",
                theme::style_accent(),
            )));
            lines.push(Line::from(""));

            if let Some(ref ip) = info.ip4 {
                lines.push(detail_line("  IPv4", ip));
            }
            if let Some(ref gw) = info.gateway {
                lines.push(detail_line("  Gateway", gw));
            }
            lines.push(detail_line("  MAC", &info.mac));
            if info.speed > 0 {
                let speed_str = format!("{} Mbps", info.speed);
                lines.push(detail_line("  Speed", &speed_str));
            }
        }

    let para = Paragraph::new(lines)
        .block(block)
        .style(theme::style_default());

    frame.render_widget(para, area);
}

/// Build a key-value detail line (owns its data)
fn detail_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<14}", label), theme::style_dim()),
        Span::styled(value.to_string(), theme::style_default()),
    ])
}

/// Generate a text-based signal strength bar
fn signal_bar(strength: u8) -> String {
    let filled = (strength as usize * 10) / 100;
    let empty = 10 - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}
