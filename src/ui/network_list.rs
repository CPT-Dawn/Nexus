use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use unicode_width::UnicodeWidthStr;

use super::theme;
use crate::animation::spinner;
use crate::animation::transitions::fade_in_opacity;
use crate::app::{App, AppMode};

/// Render the WiFi network list
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let nerd = app.config.nerd_fonts();
    let t = &app.theme;
    let is_scanning = matches!(app.mode, AppMode::Scanning);

    // Build title
    let title_text = if is_scanning {
        let scan_icon = if nerd { theme::ICON_SCAN } else { "" };
        let spin = spinner::spinner_frame(app.animation.tick_count);
        format!(" {scan_icon}{spin} Scanning… ")
    } else {
        let count = app.networks.len();
        format!(" WiFi Networks ({count}) ")
    };

    let block = Block::default()
        .title(Line::from(Span::styled(title_text, t.style_accent_bold())))
        .borders(Borders::ALL)
        .border_type(t.border_type)
        .border_style(t.style_border())
        .style(t.style_default());

    if app.networks.is_empty() {
        let empty_msg = if is_scanning {
            "Scanning for networks…"
        } else {
            "No networks found. Press [s] to scan."
        };
        let para = ratatui::widgets::Paragraph::new(empty_msg)
            .block(block)
            .style(t.style_dim())
            .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(para, area);
        return;
    }

    // Build list items
    let items: Vec<ListItem> = app
        .networks
        .iter()
        .enumerate()
        .map(|(idx, net)| {
            let is_selected = idx == app.selected_index;
            let opacity = fade_in_opacity(net.seen_ticks);

            // Selection indicator
            let selector = if is_selected {
                if nerd {
                    Span::styled(format!("{} ", theme::ICON_ARROW_RIGHT), t.style_accent())
                } else {
                    Span::styled(format!("{} ", theme::PLAIN_ARROW), t.style_accent())
                }
            } else {
                Span::styled("  ", t.style_default())
            };

            // Connection status dot
            let status_dot = if net.is_active {
                Span::styled("● ", t.style_connected())
            } else {
                Span::styled("  ", t.style_default())
            };

            // SSID with padding
            let ssid_width = 28;
            let ssid_display = if net.ssid.width() > ssid_width {
                format!("{}…", &net.ssid[..ssid_width - 1])
            } else {
                format!("{:<width$}", net.ssid, width = ssid_width)
            };

            let ssid_style = if net.is_active {
                t.style_connected()
            } else if is_selected {
                t.style_selected()
            } else if opacity < 1.0 {
                t.style_dim()
            } else {
                t.style_default()
            };

            // Signal strength
            let signal_display = net.display_signal.round() as u8;
            let sig_icon = t.signal_icon(signal_display, nerd);
            let sig_color = t.signal_color(signal_display);
            let signal_span = Span::styled(
                sig_icon.to_string(),
                ratatui::style::Style::default().fg(sig_color),
            );

            // Signal percentage
            let pct = Span::styled(
                format!("{:>3}%", signal_display),
                ratatui::style::Style::default().fg(sig_color),
            );

            // Security badge
            let sec_str = format!(" {:<6}", net.security.to_string());
            let sec_style = if net.security == crate::network::types::SecurityType::Open {
                t.style_warning()
            } else {
                t.style_dim()
            };
            let security = Span::styled(sec_str, sec_style);

            // Lock icon
            let lock = t.lock_icon(net.security.needs_password(), nerd);
            let lock_span = Span::styled(
                format!("{lock} "),
                if net.security.needs_password() {
                    t.style_dim()
                } else {
                    t.style_warning()
                },
            );

            // Saved indicator
            let saved = if net.is_saved {
                Span::styled(
                    if nerd {
                        theme::ICON_SAVED
                    } else {
                        theme::PLAIN_SAVED
                    },
                    t.style_accent(),
                )
            } else {
                Span::raw(" ")
            };

            // Band indicator
            let band = {
                let level = net.signal_level();
                let band_str = match net.band() {
                    crate::network::types::FrequencyBand::FiveGhz => "5G",
                    crate::network::types::FrequencyBand::SixGhz => "6G",
                    _ => "  ",
                };
                let _ = level; // signal level available for future use
                Span::styled(format!(" {band_str}"), t.style_dim())
            };

            let line = Line::from(vec![
                selector,
                status_dot,
                Span::styled(ssid_display, ssid_style),
                Span::raw(" "),
                signal_span,
                pct,
                Span::raw(" "),
                lock_span,
                security,
                saved,
                band,
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(t.style_selected())
        .highlight_symbol("");

    let mut state = ListState::default();
    state.select(Some(app.selected_index));

    frame.render_stateful_widget(list, area, &mut state);
}
