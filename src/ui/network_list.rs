use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::theme;
use crate::animation::spinner;
use crate::animation::transitions::fade_in_opacity;
use crate::app::{App, AppMode};

/// Truncate a string to `max_chars` grapheme-safe width, appending `…` if truncated.
/// Never slices into the middle of a multi-byte character.
fn truncate_ssid(s: &str, max_chars: usize) -> String {
    if s.width() <= max_chars {
        return format!("{:<width$}", s, width = max_chars);
    }
    let mut result = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw >= max_chars {
            break;
        }
        result.push(ch);
        w += cw;
    }
    result.push('…');
    // pad to max_chars
    let rw = result.width();
    if rw < max_chars {
        for _ in 0..(max_chars - rw) {
            result.push(' ');
        }
    }
    result
}

/// Render the WiFi network list
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let nerd = app.config.nerd_fonts();
    let t = &app.theme;
    let is_scanning = matches!(app.mode, AppMode::Scanning);
    let is_search = matches!(app.mode, AppMode::Search);

    // Reserve one line at the bottom for the search bar when in search mode
    let (list_area, search_area) = if is_search || !app.search_query.is_empty() {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(1)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    // Build title
    let visible_count = app.filtered_indices.len();
    let total_count = app.networks.len();
    let sort_label = app.sort_mode.label();

    let title_text = if is_scanning {
        let scan_icon = if nerd { theme::ICON_SCAN } else { "" };
        let spin = spinner::spinner_frame(app.animation.tick_count);
        format!(" {scan_icon}{spin} Scanning… ")
    } else if !app.search_query.is_empty() {
        format!(" WiFi Networks ({visible_count}/{total_count}) [{sort_label}] ")
    } else {
        format!(" WiFi Networks ({total_count}) [{sort_label}] ")
    };

    let block = Block::default()
        .title(Line::from(Span::styled(title_text, t.style_accent_bold())))
        .borders(Borders::ALL)
        .border_type(t.border_type)
        .border_style(t.style_border())
        .style(t.style_default());

    // Use the filtered visible list
    let visible = app.visible_networks();

    if visible.is_empty() {
        let empty_msg = if is_scanning {
            "Scanning for networks…"
        } else if !app.search_query.is_empty() {
            "No matching networks"
        } else {
            "No networks found. Press [s] to scan."
        };
        let para = ratatui::widgets::Paragraph::new(empty_msg)
            .block(block)
            .style(t.style_dim())
            .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(para, list_area);

        // Render search bar even when list is empty
        if let Some(sa) = search_area {
            render_search_bar(frame, app, sa);
        }
        return;
    }

    // Build list items from filtered view
    let items: Vec<ListItem> = visible
        .iter()
        .enumerate()
        .map(|(vis_idx, net)| {
            let is_selected = vis_idx == app.selected_index;
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

            // SSID with padding (char-boundary-safe truncation)
            let ssid_width = 28;
            let ssid_display = truncate_ssid(&net.ssid, ssid_width);

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
                let band_str = match net.band() {
                    crate::network::types::FrequencyBand::FiveGhz => "5G",
                    crate::network::types::FrequencyBand::SixGhz => "6G",
                    _ => "  ",
                };
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

    frame.render_stateful_widget(list, list_area, &mut state);

    // Render search bar
    if let Some(sa) = search_area {
        render_search_bar(frame, app, sa);
    }
}

/// Render the inline search/filter bar at the bottom of the network list
fn render_search_bar(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let is_active = matches!(app.mode, AppMode::Search);

    let cursor = if is_active && app.animation.cursor_visible() {
        "█"
    } else {
        ""
    };

    let line = Line::from(vec![
        Span::styled(" /", t.style_accent_bold()),
        Span::styled(&app.search_query, t.style_default()),
        Span::styled(cursor, t.style_accent()),
    ]);

    let para = Paragraph::new(line).style(t.style_default());
    frame.render_widget(para, area);
}
