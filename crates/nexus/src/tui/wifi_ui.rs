//! Drawing the Wi-Fi tab: the list, the detail pane and the layers over it
//! (password field, Forget confirmation, toast). Pure like the rest of the
//! views: `App` in, cells out.

use nexus_ipc::{Security, WifiNetwork};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, HighlightSpacing, List, ListItem, ListState, Paragraph, Wrap};

use super::app::App;
use super::caps::Glyphs;
use super::hits::Hit;
use super::theme::Look;
use super::view::Sym;
use super::wifi::Entry;

/// Rows at least this wide also show the band.
const BAND_AT: usize = 34;
const SWEEP_CELLS: usize = 6;

fn bars_filled(signal: u8) -> usize {
    match signal {
        75.. => 4,
        50..=74 => 3,
        25..=49 => 2,
        _ => 1,
    }
}

fn signal_style(look: &Look, signal: u8) -> Style {
    match signal {
        50.. => look.ok(),
        25..=49 => look.warn(),
        _ => look.err(),
    }
}

/// Four signal bars; the ones beyond the signal are dim.
fn bars(look: &Look, sym: &Sym, signal: u8) -> Vec<Span<'static>> {
    let glyphs: [&str; 4] = match sym.glyphs {
        Glyphs::Ascii => ["#", "#", "#", "#"],
        _ => ["▂", "▄", "▆", "█"],
    };
    let on = bars_filled(signal);
    glyphs
        .iter()
        .enumerate()
        .map(|(i, g)| {
            if i < on {
                Span::styled(*g, signal_style(look, signal))
            } else {
                let off = if sym.glyphs == Glyphs::Ascii { "." } else { *g };
                Span::styled(off, look.dim())
            }
        })
        .collect()
}

fn short_security(s: Security) -> &'static str {
    match s {
        Security::Enterprise => "802.1x",
        other => other.label(),
    }
}

fn security_words(s: Security) -> &'static str {
    match s {
        Security::Open => "Open, no password",
        Security::Owe => "Enhanced Open, encrypted without a password",
        Security::Wep => "WEP (old and weak)",
        Security::Wpa => "WPA",
        Security::Wpa2 => "WPA2 personal",
        Security::Wpa3 => "WPA3 personal",
        Security::Wpa3Transition => "WPA2 and WPA3 personal",
        Security::Enterprise => "Enterprise (802.1X)",
        _ => "Unknown",
    }
}

fn band_short(n: &WifiNetwork) -> &'static str {
    match n.band {
        nexus_ipc::Band::Ghz2_4 => "2.4G",
        nexus_ipc::Band::Ghz5 => "5G",
        nexus_ipc::Band::Ghz6 => "6G",
        _ => "?",
    }
}

/// `text` cut to `width` cells, with an ellipsis when it had to be.
fn fit(text: &str, width: usize, sym: &Sym) -> String {
    if text.chars().count() <= width {
        return format!("{text:<width$}");
    }
    let dots = if sym.glyphs == Glyphs::Ascii {
        "~"
    } else {
        "…"
    };
    let keep: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{keep}{dots}")
}

fn row_line(entry: &Entry, busy: bool, avail: usize, look: &Look, sym: &Sym) -> Line<'static> {
    match entry {
        Entry::Net(n) => {
            let band = avail >= BAND_AT;
            // marker, gap, name, gap, bars, gap + security, [gap + band]
            let name_w = avail.saturating_sub(2 + 1 + 4 + 7 + if band { 5 } else { 0 });
            let (marker, marker_style) = if n.joined {
                (sym.dot(), look.ok())
            } else if busy {
                ("~", look.warn())
            } else {
                (" ", look.text())
            };
            let mut spans = vec![
                Span::styled(marker, marker_style),
                Span::raw(" "),
                Span::raw(fit(&n.ssid, name_w, sym)),
                Span::raw(" "),
            ];
            spans.extend(bars(look, sym, n.signal));
            spans.push(Span::styled(
                format!(" {:<6}", short_security(n.security)),
                look.muted(),
            ));
            if band {
                spans.push(Span::styled(format!(" {:>4}", band_short(n)), look.muted()));
            }
            Line::from(spans)
        }
        Entry::Saved(s) => {
            let name_w = avail.saturating_sub(6 + 3);
            let auto = if s.autoconnect { "auto" } else { "manual" };
            Line::from(vec![
                Span::raw("  "),
                Span::styled(fit(&s.ssid, name_w, sym), look.muted()),
                Span::styled(format!(" {auto:>6}"), look.dim()),
            ])
        }
    }
}

pub(super) fn master(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let wifi = app.wifi();
    let block = sym.block(look, "Wi-Fi", true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    sweep(f, app, area, look, sym);
    if wifi.len() == 0 {
        let msg = if !wifi.has_list() {
            "Looking for networks..."
        } else if !wifi.has_adapter() {
            "No Wi-Fi adapter found."
        } else {
            "No networks in range.\nPress r to scan."
        };
        f.render_widget(
            Paragraph::new(Span::styled(msg, look.dim())).wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    let avail = (inner.width as usize).saturating_sub(2);
    let saved_start = wifi.saved_start();
    let mut items = Vec::new();
    // The list item behind each row (the "Saved" rule is an item too).
    let mut item_of_row = Vec::new();
    for (i, entry) in wifi.rows().iter().enumerate() {
        if saved_start == Some(i) {
            let rule = if sym.glyphs == Glyphs::Ascii {
                "--"
            } else {
                "──"
            };
            items.push(ListItem::new(Line::styled(
                format!("{rule} Saved, out of range"),
                look.dim(),
            )));
        }
        let busy = wifi.busy.as_deref() == Some(entry.name());
        item_of_row.push(items.len());
        items.push(ListItem::new(row_line(entry, busy, avail, look, sym)));
    }
    let list = List::new(items)
        .highlight_style(look.selected())
        .highlight_symbol(sym.pointer())
        .highlight_spacing(HighlightSpacing::Always);
    let shown = wifi.cursor() + usize::from(saved_start.is_some_and(|s| s <= wifi.cursor()));
    let mut state = ListState::default().with_selected(Some(shown));
    f.render_stateful_widget(list, inner, &mut state);
    for (row, item) in item_of_row.iter().enumerate() {
        if *item < state.offset() {
            continue;
        }
        let y = inner.y + (*item - state.offset()) as u16;
        if y >= inner.bottom() {
            break;
        }
        app.hit(Rect::new(inner.x, y, inner.width, 1), Hit::Row(row));
    }
}

/// The scan sweep, set into the list's top border while scanning.
fn sweep(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let wifi = app.wifi();
    if !wifi.scanning() || area.width < 20 {
        return;
    }
    let at = wifi.sweep(SWEEP_CELLS);
    let (lit, off) = if sym.glyphs == Glyphs::Ascii {
        ("#", "-")
    } else {
        ("█", "░")
    };
    let mut spans = vec![Span::raw(" ")];
    for i in 0..SWEEP_CELLS {
        let near = i.abs_diff(at) <= 1;
        spans.push(if i == at {
            Span::styled(lit, look.accent())
        } else if near {
            Span::styled(off, look.accent())
        } else {
            Span::styled(off, look.dim())
        });
    }
    spans.push(Span::styled(" scanning ", look.muted()));
    f.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Right),
        Rect {
            x: area.x + 1,
            y: area.y,
            width: area.width - 2,
            height: 1,
        },
    );
}

pub(super) fn detail(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let wifi = app.wifi();
    let title = wifi.selected().map_or("Wi-Fi", Entry::name);
    let block = sym.block(look, title, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(entry) = wifi.selected() else {
        f.render_widget(
            Paragraph::new(Span::styled("Nothing selected.", look.dim())),
            inner,
        );
        return;
    };
    let row = |k: &str, v: Vec<Span<'static>>| {
        let mut spans = vec![Span::styled(format!("{k:<14}"), look.muted())];
        spans.extend(v);
        Line::from(spans)
    };
    let text = |s: String| vec![Span::styled(s, look.text())];
    let busy = wifi.busy.as_deref() == Some(entry.name());
    let mut lines = Vec::new();
    match entry {
        Entry::Net(n) => {
            let (state, style) = if n.joined {
                (format!("{} Connected", sym.dot()), look.ok().bold())
            } else if busy {
                ("~ Joining...".to_string(), look.warn().bold())
            } else if n.profile.is_some() {
                ("Saved".to_string(), look.text().bold())
            } else {
                ("Not saved".to_string(), look.muted().bold())
            };
            lines.push(Line::styled(state, style));
            lines.push(Line::default());
            let mut signal = bars(look, sym, n.signal);
            signal.insert(0, Span::raw(format!("{:>3}%  ", n.signal)));
            lines.push(row("Signal", signal));
            lines.push(row("Security", text(security_words(n.security).into())));
            lines.push(row("Band", text(n.band.label().into())));
            lines.push(row("Access points", text(n.access_points.to_string())));
            if let Some(name) = wifi.adapter_name(&n.device) {
                lines.push(row("Adapter", text(name.into())));
            }
            if let Some(auto) = n.autoconnect {
                lines.push(row(
                    "Autoconnect",
                    text(if auto { "on" } else { "off" }.into()),
                ));
            }
            lines.push(Line::default());
            let secured = !matches!(n.security, Security::Open | Security::Owe);
            let hint = if n.joined {
                "Press x to disconnect."
            } else if n.profile.is_none() && secured {
                "Press enter to join. You will be asked for the password."
            } else {
                "Press enter to join."
            };
            lines.push(Line::styled(hint, look.dim()));
        }
        Entry::Saved(s) => {
            lines.push(Line::styled("Saved, out of range", look.muted().bold()));
            lines.push(Line::default());
            let auto = if s.autoconnect { "on" } else { "off" };
            lines.push(row("Autoconnect", text(auto.into())));
            lines.push(Line::default());
            lines.push(Line::styled(
                "Nexus joins it again when it is back in range.",
                look.dim(),
            ));
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// The layers over the tab: password field, Forget confirmation, toast.
/// They show on any tab, because a Secret prompt can open anywhere.
pub(super) fn overlays(f: &mut Frame, app: &App, body: Rect, look: &Look, sym: &Sym) {
    if app.field().is_none()
        && let Some(share) = app.share()
    {
        super::share::draw(f, share, body, look, sym);
    }
    if let Some(field) = app.field() {
        let rect = centered(body, 62, 7);
        f.render_widget(Clear, rect);
        let title = format!("Join {}", field.prompt.network);
        let block = sym.block(look, &title, true);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        let mask = if sym.glyphs == Glyphs::Ascii {
            '*'
        } else {
            '•'
        };
        let shown: String = if field.reveal {
            field.text().to_string()
        } else {
            field.text().chars().map(|_| mask).collect()
        };
        // Keep the end of a long password in view.
        let room = (inner.width as usize).saturating_sub(12);
        let skip = shown.chars().count().saturating_sub(room);
        let shown: String = shown.chars().skip(skip).collect();
        let message_style = if field.prompt.retry {
            look.warn()
        } else {
            look.text()
        };
        let lines = vec![
            Line::styled(field.prompt.message.clone(), message_style),
            Line::default(),
            Line::from(vec![
                Span::styled("Password  ", look.muted()),
                Span::styled(shown, look.text()),
                Span::styled("_", look.accent()),
            ]),
            Line::default(),
            Line::styled("enter join   ctrl-r show   esc cancel", look.dim()),
        ];
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    } else if let Some(confirm) = app.confirm() {
        let rect = centered(body, 52, 6);
        f.render_widget(Clear, rect);
        let block = sym.block(look, "Forget", true);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        let lines = vec![
            Line::from(vec![
                Span::styled("Forget ", look.text()),
                Span::styled(confirm.ssid.clone(), look.accent().bold()),
                Span::styled("?", look.text()),
            ]),
            Line::styled("Its saved password goes with it.", look.muted()),
            Line::default(),
            Line::styled("y forget   esc keep", look.dim()),
        ];
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }

    if let Some(toast) = app.toast() {
        let undo = if toast.undo.is_some() { "  u undo" } else { "" };
        let width = (toast.text.chars().count() + undo.len() + 4)
            .min(body.width.saturating_sub(2) as usize) as u16;
        if body.height < 5 || width < 8 {
            return;
        }
        let rect = Rect {
            // Inside the frame: one column and one row clear of the border.
            x: body.right().saturating_sub(width + 2),
            y: body.bottom().saturating_sub(4),
            width,
            height: 3,
        };
        f.render_widget(Clear, rect);
        let block = sym.block(look, if toast.bad { "Problem" } else { "Done" }, false);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        let style = if toast.bad { look.err() } else { look.ok() };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(toast.text.clone(), style),
                Span::styled(undo, look.accent()),
            ])),
            inner,
        );
    }
}
