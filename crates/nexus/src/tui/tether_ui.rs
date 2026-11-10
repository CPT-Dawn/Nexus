//! Drawing the Tethering list and its detail pane (on the Hotspot tab).
//! Pure like the rest of the views: `App` in, cells out.

use nexus_ipc::{Bluetooth, Phone};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{HighlightSpacing, List, ListItem, ListState, Paragraph, Wrap};

use super::app::App;
use super::caps::Glyphs;
use super::hits::Hit;
use super::surface;
use super::theme::Look;
use super::view::Sym;

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

fn state_word(p: &Phone, busy: bool) -> &'static str {
    if p.connected {
        "connected"
    } else if p.connecting || busy {
        "connecting"
    } else if p.saved {
        "saved"
    } else {
        "paired"
    }
}

fn bluetooth_line(b: Bluetooth, look: &Look, sym: &Sym) -> Line<'static> {
    match b {
        Bluetooth::On => Line::from(vec![
            Span::styled(sym.dot(), look.ok()),
            Span::styled(" Bluetooth is on", look.text()),
        ]),
        Bluetooth::Off => Line::from(vec![
            Span::styled(sym.hollow(), look.warn()),
            Span::styled(" Bluetooth is off", look.warn()),
        ]),
        _ => Line::from(vec![
            Span::styled(sym.hollow(), look.err()),
            Span::styled(" No Bluetooth", look.err()),
        ]),
    }
}

fn row_line(p: &Phone, busy: bool, avail: usize, look: &Look, sym: &Sym) -> Line<'static> {
    let state = state_word(p, busy);
    let (marker, style) = if p.connected {
        (sym.dot(), look.ok())
    } else if p.connecting || busy {
        ("~", look.warn())
    } else {
        (" ", look.text())
    };
    let name_w = avail.saturating_sub(2 + 1 + 10);
    Line::from(vec![
        Span::styled(marker, style),
        Span::raw(" "),
        Span::raw(fit(&surface::printable(&p.name), name_w, sym)),
        Span::styled(format!(" {state:>10}"), look.muted()),
    ])
}

pub(super) fn master(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let tether = app.tether();
    let block = sym.block(look, "Tethering", true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 2 {
        return;
    }
    let [status, rest] = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(inner);
    if !tether.has_data() {
        f.render_widget(
            Paragraph::new(Span::styled("Looking for phones...", look.dim())),
            inner,
        );
        return;
    }
    f.render_widget(
        Paragraph::new(bluetooth_line(tether.bluetooth(), look, sym)),
        status,
    );
    if tether.len() == 0 {
        f.render_widget(
            Paragraph::new(Span::styled(
                "No paired phone offers network access.",
                look.dim(),
            ))
            .wrap(Wrap { trim: false }),
            rest,
        );
        return;
    }
    let avail = (rest.width as usize).saturating_sub(2);
    let items: Vec<ListItem> = tether
        .phones()
        .iter()
        .map(|p| {
            let busy = tether.busy.as_deref() == Some(p.id.as_str());
            ListItem::new(row_line(p, busy, avail, look, sym))
        })
        .collect();
    let list = List::new(items)
        .highlight_style(look.selected())
        .highlight_symbol(sym.pointer())
        .highlight_spacing(HighlightSpacing::Always);
    let mut state = ListState::default().with_selected(Some(tether.cursor()));
    f.render_stateful_widget(list, rest, &mut state);
    for row in state.offset()..tether.len() {
        let y = rest.y + (row - state.offset()) as u16;
        if y >= rest.bottom() {
            break;
        }
        app.hit(Rect::new(rest.x, y, rest.width, 1), Hit::Row(row));
    }
}

pub(super) fn detail(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let tether = app.tether();
    let selected = tether.selected();
    let title = selected.map_or_else(|| "Tethering".to_owned(), |p| surface::printable(&p.name));
    let block = sym.block(look, &title, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let bluetooth = tether.bluetooth();
    let mut lines = Vec::new();
    let Some(phone) = selected else {
        let (text, style) = match (tether.has_data(), bluetooth) {
            (false, _) => ("Looking for phones...", look.dim()),
            (true, Bluetooth::Off) => (
                "Bluetooth is off. Turn it on (bluetoothctl power on, or rfkill unblock \
                 bluetooth) and your paired phones will show here.",
                look.warn(),
            ),
            (true, Bluetooth::On) => (
                "No paired phone offers network access.\n\nPair your phone (bluetoothctl), \
                 then switch on Bluetooth tethering in its settings.",
                look.dim(),
            ),
            (true, _) => (
                "No Bluetooth Device was found, or the Bluetooth service is not running \
                 (systemctl start bluetooth).",
                look.err(),
            ),
        };
        f.render_widget(
            Paragraph::new(Span::styled(text, style)).wrap(Wrap { trim: false }),
            inner,
        );
        return;
    };
    let busy = tether.busy.as_deref() == Some(phone.id.as_str());
    let row = |k: &str, v: Span<'static>| {
        Line::from(vec![Span::styled(format!("{k:<12}"), look.muted()), v])
    };
    let (state, style) = if phone.connected {
        (format!("{} Connected", sym.dot()), look.ok().bold())
    } else if phone.connecting || busy {
        ("~ Connecting...".to_owned(), look.warn().bold())
    } else if phone.saved {
        ("Saved".to_owned(), look.text().bold())
    } else {
        ("Paired".to_owned(), look.muted().bold())
    };
    lines.push(Line::styled(state, style));
    lines.push(Line::default());
    lines.push(row("Address", Span::styled(phone.id.clone(), look.text())));
    let (bt, bt_style) = match bluetooth {
        Bluetooth::On => ("on", look.text()),
        Bluetooth::Off => ("off", look.warn()),
        _ => ("missing", look.err()),
    };
    lines.push(row("Bluetooth", Span::styled(bt, bt_style)));
    lines.push(row(
        "Profile",
        Span::styled(if phone.saved { "saved" } else { "none yet" }, look.text()),
    ));
    lines.push(Line::default());
    let hint = if phone.connected {
        "Press x to drop the tether."
    } else if bluetooth != Bluetooth::On {
        "Press enter to see how to turn Bluetooth on."
    } else {
        "Press enter to tether. Switch on Bluetooth tethering on the phone first."
    };
    lines.push(Line::styled(hint, look.dim()));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
