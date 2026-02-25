use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use super::theme;
use crate::animation::spinner;
use crate::app::App;
use crate::network::types::ConnectionStatus;

/// Render the application header bar
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let nerd = !app.config.no_nerd_fonts;

    // Build the title with WiFi icon
    let wifi_icon = if nerd {
        theme::ICON_WIFI
    } else {
        theme::PLAIN_WIFI
    };
    let title = Line::from(vec![
        Span::styled(format!(" {wifi_icon}"), theme::style_accent_bold()),
        Span::styled("Nexus ", theme::style_accent_bold()),
    ]);

    // Build connection status (right side)
    let status_spans = build_status_spans(app, nerd);

    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::style_border_focused())
        .style(theme::style_default());

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
        theme::style_dim(),
    )]);
    let iface_para = Paragraph::new(iface).alignment(Alignment::Left);
    frame.render_widget(iface_para, inner);
}

/// Build status indicator spans based on connection state
fn build_status_spans(app: &App, _nerd: bool) -> Vec<Span<'static>> {
    let tick = app.animation.tick_count;

    match &app.connection_status {
        ConnectionStatus::Connected(info) => {
            vec![
                Span::styled("● ", theme::style_connected()),
                Span::styled(format!("{}", info.ssid), theme::style_connected()),
                Span::styled(
                    format!(
                        " ({}{})",
                        info.ip4.as_deref().unwrap_or("no IP"),
                        if info.speed > 0 {
                            format!(" • {} Mbps", info.speed)
                        } else {
                            String::new()
                        }
                    ),
                    theme::style_dim(),
                ),
                Span::styled(" ", theme::style_default()),
            ]
        }
        ConnectionStatus::Connecting(ssid) => {
            let spin = spinner::spinner_frame(tick);
            vec![
                Span::styled(format!("{spin} "), theme::style_accent()),
                Span::styled("Connecting to ", theme::style_dim()),
                Span::styled(ssid.clone(), theme::style_accent()),
                Span::styled("… ", theme::style_dim()),
            ]
        }
        ConnectionStatus::Disconnecting => {
            let spin = spinner::spinner_frame(tick);
            vec![
                Span::styled(format!("{spin} "), theme::style_warning()),
                Span::styled("Disconnecting… ", theme::style_dim()),
            ]
        }
        ConnectionStatus::Disconnected => {
            vec![
                Span::styled("○ ", theme::style_dim()),
                Span::styled("Disconnected ", theme::style_dim()),
            ]
        }
        ConnectionStatus::Failed(msg) => {
            vec![
                Span::styled("✗ ", theme::style_error()),
                Span::styled(format!("Failed: {} ", msg), theme::style_error()),
            ]
        }
    }
}
