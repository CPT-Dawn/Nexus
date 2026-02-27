use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::theme;
use crate::animation::spinner;
use crate::app::App;
use crate::network::types::{ConnectionStatus, FrequencyBand};

/// Render the application header bar
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let nerd = app.config.nerd_fonts();
    let t = &app.theme;

    // Build the title with WiFi icon
    let wifi_icon = if nerd {
        theme::ICON_WIFI
    } else {
        theme::PLAIN_WIFI
    };
    let title = Line::from(vec![
        Span::styled(format!(" {wifi_icon}"), t.style_accent_bold()),
        Span::styled("Nexus ", t.style_accent_bold()),
    ]);

    // Build connection status (right side)
    let status_spans = build_status_spans(app, nerd);

    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_type(t.border_type)
        .border_style(t.style_border_focused())
        .style(t.style_default());

    // Render the block
    frame.render_widget(block, area);

    // Render the status line inside the block (right-aligned)
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: 1,
    };

    let status_line = Line::from(status_spans);
    let status = Paragraph::new(status_line).alignment(Alignment::Right);
    frame.render_widget(status, inner);

    // Render interface name on the left inside the block
    let iface = Line::from(vec![Span::styled(
        format!("  {}", app.interface_name),
        t.style_dim(),
    )]);
    let iface_para = Paragraph::new(iface).alignment(Alignment::Left);
    frame.render_widget(iface_para, inner);
}

/// Build status indicator spans based on connection state
fn build_status_spans(app: &App, nerd: bool) -> Vec<Span<'static>> {
    let tick = app.animation.tick_count;
    let t = &app.theme;

    match &app.connection_status {
        ConnectionStatus::Connected(info) => {
            let connected_icon = if nerd {
                theme::ICON_CONNECTED
            } else {
                theme::PLAIN_CONNECTED
            };
            let pulse = spinner::pulse_frame(tick);
            let band_str = match FrequencyBand::from_mhz(info.frequency) {
                FrequencyBand::FiveGhz => " 5G",
                FrequencyBand::SixGhz => " 6G",
                _ => "",
            };
            vec![
                Span::styled(format!("{connected_icon}{pulse} "), t.style_connected()),
                Span::styled(info.ssid.clone(), t.style_connected()),
                Span::styled(
                    format!(
                        " ({}{}{})",
                        info.ip4.as_deref().unwrap_or("no IP"),
                        if info.speed > 0 {
                            format!(" • {} Mbps", info.speed)
                        } else {
                            String::new()
                        },
                        band_str,
                    ),
                    t.style_dim(),
                ),
                Span::styled(" ", t.style_default()),
            ]
        }
        ConnectionStatus::Connecting(ssid) => {
            let spin = spinner::spinner_frame(tick);
            vec![
                Span::styled(format!("{spin} "), t.style_accent()),
                Span::styled("Connecting to ", t.style_dim()),
                Span::styled(ssid.clone(), t.style_accent()),
                Span::styled("… ", t.style_dim()),
            ]
        }
        ConnectionStatus::Disconnecting => {
            let bar = spinner::bar_frame(tick);
            vec![
                Span::styled(format!("{bar} "), t.style_warning()),
                Span::styled("Disconnecting… ", t.style_dim()),
            ]
        }
        ConnectionStatus::Disconnected => {
            let wifi_off = if nerd {
                theme::ICON_WIFI_OFF
            } else {
                theme::PLAIN_WIFI_OFF
            };
            vec![
                Span::styled(wifi_off.to_string(), t.style_dim()),
                Span::styled("Disconnected ", t.style_dim()),
            ]
        }
        ConnectionStatus::Failed(msg) => {
            let err_icon = if nerd { theme::ICON_ERROR } else { "[!] " };
            vec![
                Span::styled(err_icon.to_string(), t.style_error()),
                Span::styled(format!("Failed: {} ", msg), t.style_error()),
            ]
        }
    }
}
