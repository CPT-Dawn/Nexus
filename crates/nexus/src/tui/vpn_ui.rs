//! Drawing the VPN tab: the list, the detail pane and the layers over it
//! (forms, public key, config QR). Pure like the other views.

use nexus_ipc::{Vpn, VpnKind, VpnState};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, HighlightSpacing, List, ListItem, ListState, Paragraph, Wrap};

use super::app::{App, Tab};
use super::caps::Glyphs;
use super::hits::Hit;
use super::share::{centered, code_style};
use super::theme::Look;
use super::view::Sym;
use super::vpn::{Form, Layer, Stage};

/// A Profile name is untrusted text: control characters would drive the terminal.
fn printable(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

fn state_word(state: VpnState) -> &'static str {
    match state {
        VpnState::Up => "up",
        VpnState::Activating => "starting",
        VpnState::Down => "down",
    }
}

fn marker<'a>(vpn: &Vpn, busy: bool, look: &Look, sym: &Sym) -> Span<'a> {
    match (vpn.state, busy) {
        (VpnState::Up, _) => Span::styled(sym.dot(), look.ok()),
        (VpnState::Activating, _) | (VpnState::Down, true) => Span::styled("~", look.warn()),
        (VpnState::Down, false) => Span::styled(sym.hollow(), look.dim()),
    }
}

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

pub(super) fn master(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let vpn = app.vpn();
    let mut title = "VPN".to_owned();
    if app.filtering() || !vpn.filter().is_empty() {
        title.push_str(&crate::text::tui::filter_title(
            vpn.filter(),
            app.filtering(),
        ));
    }
    let block = sym.block(look, &title, true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if vpn.is_empty() {
        let msg = if !vpn.filter().is_empty() && vpn.has_list() {
            return super::view::empty_filter(
                f,
                inner,
                look,
                crate::text::tui::no_match("VPNs", vpn.filter()),
            );
        } else if vpn.has_list() {
            "No VPNs yet.\nPress i to import a .conf or .ovpn,\nor c to create a tunnel."
        } else {
            "Looking for VPNs..."
        };
        let lines: Vec<Line> = msg.lines().map(|l| Line::styled(l, look.dim())).collect();
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }
    let avail = (inner.width as usize).saturating_sub(2);
    let items: Vec<ListItem> = vpn
        .rows()
        .iter()
        .map(|v| {
            let busy = vpn.busy.as_deref() == Some(v.id.as_str());
            let name_w = avail.saturating_sub(2 + 1 + 8);
            ListItem::new(Line::from(vec![
                marker(v, busy, look, sym),
                Span::raw(" "),
                Span::raw(fit(&printable(&v.name), name_w, sym)),
                Span::styled(format!(" {:>8}", state_word(v.state)), look.muted()),
            ]))
        })
        .collect();
    let list = List::new(items)
        .highlight_style(look.selected())
        .highlight_symbol(sym.pointer())
        .highlight_spacing(HighlightSpacing::Always);
    let mut state = ListState::default().with_selected(Some(vpn.cursor()));
    f.render_stateful_widget(list, inner, &mut state);
    // A click selects a row; a click on the selected one brings it up or down.
    for row in state.offset()..vpn.len() {
        let y = inner.y + (row - state.offset()) as u16;
        if y >= inner.bottom() {
            break;
        }
        app.hit(Rect::new(inner.x, y, inner.width, 1), Hit::Row(row));
    }
}

pub(super) fn detail(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let vpn = app.vpn();
    let title = vpn
        .selected()
        .map_or_else(|| "VPN".to_owned(), |v| printable(&v.name));
    let block = sym.block(look, &title, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(v) = vpn.selected() else {
        f.render_widget(
            Paragraph::new(Span::styled("Nothing selected.", look.dim())),
            inner,
        );
        return;
    };
    let row = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!("{k:<12}"), look.muted()),
            Span::styled(v, look.text()),
        ])
    };
    let busy = vpn.busy.as_deref() == Some(v.id.as_str());
    let (state, style) = match (v.state, busy) {
        (VpnState::Up, _) => (format!("{} Up", sym.dot()), look.ok().bold()),
        (VpnState::Activating, _) | (VpnState::Down, true) => {
            ("~ Starting...".to_owned(), look.warn().bold())
        }
        (VpnState::Down, false) => (format!("{} Down", sym.hollow()), look.muted().bold()),
    };
    let mut lines = vec![Line::styled(state, style), Line::default()];
    let kind = match v.kind {
        VpnKind::Wireguard => "WireGuard",
        VpnKind::Plugin => "Plugin VPN",
    };
    lines.push(row("Kind", kind.into()));
    if !v.addresses.is_empty() {
        lines.push(row("Address", v.addresses.join(", ")));
    }
    if let Some(e) = &v.endpoint {
        lines.push(row("Endpoint", e.clone()));
    }
    lines.push(Line::default());
    lines.push(Line::styled(
        if v.state == VpnState::Down {
            "Press enter to bring it up."
        } else {
            "Press enter to bring it down."
        },
        look.dim(),
    ));
    if v.kind == VpnKind::Wireguard {
        lines.push(Line::styled(
            "p shows its public key, s its config as a QR code.",
            look.dim(),
        ));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// The layers over the VPN tab.
pub(super) fn overlays(f: &mut Frame, app: &App, body: Rect, look: &Look, sym: &Sym) {
    if app.tab() != Tab::Vpn {
        return;
    }
    match &app.vpn().layer {
        Some(Layer::Form(form)) => form_view(f, form, body, look, sym),
        Some(Layer::Key { name, key, saved }) => key_view(f, name, key, *saved, body, look, sym),
        Some(Layer::Share(view)) => share_view(f, view, body, look, sym),
        None => {}
    }
}

fn form_view(f: &mut Frame, form: &Form, body: Rect, look: &Look, sym: &Sym) {
    let height = form.fields.len() as u16 + 6;
    let rect = centered(body, 66, height);
    f.render_widget(Clear, rect);
    let block = sym.block(look, form.title(), true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let room = (inner.width as usize).saturating_sub(12);
    let mut lines = Vec::new();
    for (i, field) in form.fields.iter().enumerate() {
        let focused = i == form.focus && !form.busy;
        let shown = if focused || !field.value.is_empty() {
            let skip = field.value.chars().count().saturating_sub(room);
            field.value.chars().skip(skip).collect::<String>()
        } else {
            String::new()
        };
        let mut spans = vec![Span::styled(
            format!("{:<10}", field.label),
            if focused { look.accent() } else { look.muted() },
        )];
        if shown.is_empty() && !focused {
            spans.push(Span::styled(field.hint, look.dim()));
        } else {
            spans.push(Span::styled(shown, look.text()));
            if focused {
                spans.push(Span::styled("_", look.accent()));
            }
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::default());
    lines.push(match (&form.error, form.busy) {
        (_, true) => Line::styled("Saving...", look.warn()),
        (Some(why), _) => Line::styled(why.clone(), look.err()),
        (None, false) => Line::styled(
            match form.kind {
                super::vpn::FormKind::Import => "Keys and passwords stay inside NetworkManager.",
                super::vpn::FormKind::Create => "Nexus makes the key pair for you.",
            },
            look.muted(),
        ),
    });
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn key_view(f: &mut Frame, name: &str, key: &str, saved: bool, body: Rect, look: &Look, sym: &Sym) {
    let rect = centered(body, 62, 8);
    f.render_widget(Clear, rect);
    let title = printable(name);
    let block = sym.block(look, &title, true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let lines = vec![
        Line::styled(
            if saved {
                "Saved. Give this public key to the other side:"
            } else {
                "Public key (safe to share):"
            },
            look.text(),
        ),
        Line::default(),
        Line::styled(key.to_owned(), look.accent().bold()),
        Line::default(),
        Line::styled("esc close", look.dim()),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn share_view(f: &mut Frame, view: &super::vpn::ShareView, body: Rect, look: &Look, sym: &Sym) {
    let title = format!("Share {}", printable(&view.name));
    let small = |f: &mut Frame, height: u16, lines: Vec<Line<'static>>| {
        let rect = centered(body, 66, height);
        f.render_widget(Clear, rect);
        let block = sym.block(look, &title, true);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    };
    match &view.stage {
        Stage::Ask => small(
            f,
            8,
            vec![
                Line::styled("Show this config's QR code?", look.text()),
                Line::styled(
                    "It contains the PRIVATE key. Anyone who can see this screen can use the tunnel.",
                    look.warn(),
                ),
                Line::default(),
                Line::styled("enter reveal   esc cancel", look.dim()),
            ],
        ),
        Stage::Reading => small(
            f,
            6,
            vec![
                Line::styled("Asking NetworkManager for the keys...", look.text()),
                Line::styled("It may ask you to authorize this first.", look.muted()),
            ],
        ),
        Stage::Failed(message) => small(
            f,
            7,
            vec![
                Line::styled(message.clone(), look.err()),
                Line::default(),
                Line::styled("enter try again   esc close", look.dim()),
            ],
        ),
        Stage::Shown { share, drawing } => {
            let ascii = sym.glyphs == Glyphs::Ascii;
            let need = drawing
                .as_ref()
                .map(|d| (d.width() as u16 + 4, d.height() as u16 + 3));
            let fits = !ascii && need.is_some_and(|(w, h)| w <= body.width && h <= body.height);
            let (Some(drawing), true) = (drawing, fits) else {
                let why = match need {
                    _ if ascii => "The QR code needs block characters, and --ascii is on.".into(),
                    Some((w, h)) => format!(
                        "The QR code needs a window of at least {w} by {} cells.",
                        h + 2
                    ),
                    None => "This config is too long for a QR code.".to_owned(),
                };
                small(
                    f,
                    9,
                    vec![
                        Line::styled(why, look.warn()),
                        Line::styled(
                            "Make the window bigger, or use `nexus vpn share --qr`.",
                            look.muted(),
                        ),
                        Line::default(),
                        Line::styled(format!("Public key  {}", share.public_key), look.text()),
                        Line::default(),
                        Line::styled("esc close", look.dim()),
                    ],
                );
                return;
            };
            let (w, h) = need.expect("a drawing has a size");
            let rect = centered(body, w.max(title.chars().count() as u16 + 4), h);
            f.render_widget(Clear, rect);
            let block = sym.block(look, &title, true);
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            let style = code_style(look);
            let mut lines: Vec<Line> = drawing
                .lines
                .iter()
                .map(|l| Line::styled(l.clone(), style).centered())
                .collect();
            lines.push(
                Line::styled(
                    format!(
                        "Scan with the WireGuard app to copy {} to a phone.",
                        printable(&view.name)
                    ),
                    look.muted(),
                )
                .centered(),
            );
            f.render_widget(Paragraph::new(lines), inner);
        }
    }
}
