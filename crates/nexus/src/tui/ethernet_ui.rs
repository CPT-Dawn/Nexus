//! Drawing the Ethernet ports and the Link details of the selected one.
//! Pure like the rest of the views: `App` in, cells out.

use nexus_ipc::{EthernetPort, EthernetState};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{HighlightSpacing, List, ListItem, ListState, Paragraph, Wrap};

use super::app::App;
use super::hits::Hit;
use super::theme::Look;
use super::view::Sym;

/// One word for where a port stands.
pub fn state_word(state: EthernetState, busy: bool) -> &'static str {
    match state {
        EthernetState::Unplugged => "unplugged",
        EthernetState::Disconnected if busy => "connecting",
        EthernetState::Disconnected => "ready",
        EthernetState::Connecting => "connecting",
        EthernetState::Connected => "connected",
        EthernetState::Misconfigured if busy => "connecting",
        EthernetState::Misconfigured => "problem",
        _ => "unknown",
    }
}

fn marker(state: EthernetState, busy: bool, sym: &Sym, look: &Look) -> Span<'static> {
    match state {
        EthernetState::Connected => Span::styled(sym.dot(), look.ok()),
        EthernetState::Connecting => Span::styled("~", look.warn()),
        _ if busy => Span::styled("~", look.warn()),
        EthernetState::Misconfigured => Span::styled("!", look.err()),
        EthernetState::Unplugged => Span::styled(sym.hollow(), look.dim()),
        _ => Span::styled(" ", look.text()),
    }
}

pub(super) fn master(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let tab = app.ethernet();
    let block = sym.block(look, "Ethernet", true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 1 {
        return;
    }
    if !tab.has_data() {
        f.render_widget(
            Paragraph::new(Span::styled("Looking for ports...", look.dim())),
            inner,
        );
        return;
    }
    if tab.len() == 0 {
        f.render_widget(
            Paragraph::new(Span::styled(
                "No Ethernet port found on this computer.",
                look.dim(),
            ))
            .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    let items: Vec<ListItem> = tab
        .ports()
        .iter()
        .map(|p| {
            let busy = tab.busy.as_deref() == Some(p.id.as_str());
            let word = state_word(p.state, busy);
            let name_w = (inner.width as usize).saturating_sub(2 + 1 + 1 + 1 + 10);
            let name: String = p.id.chars().take(name_w).collect();
            ListItem::new(Line::from(vec![
                marker(p.state, busy, sym, look),
                Span::raw(format!(" {name:<name_w$}")),
                Span::styled(format!(" {word:>10}"), look.muted()),
            ]))
        })
        .collect();
    let list = List::new(items)
        .highlight_style(look.selected())
        .highlight_symbol(sym.pointer())
        .highlight_spacing(HighlightSpacing::Always);
    let mut state = ListState::default().with_selected(Some(tab.cursor()));
    f.render_stateful_widget(list, inner, &mut state);
    for row in state.offset()..tab.len() {
        let y = inner.y + (row - state.offset()) as u16;
        if y >= inner.bottom() {
            break;
        }
        app.hit(Rect::new(inner.x, y, inner.width, 1), Hit::Row(row));
    }
}

fn row(k: &str, v: Span<'static>, look: &Look) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{k:<10}"), look.muted()), v])
}

/// The lines under the headline: the Link, when there is one.
fn link_lines(p: &EthernetPort, look: &Look) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let text = |s: String| Span::styled(s, look.text());
    if let Some(speed) = p.speed_text() {
        lines.push(row("Speed", text(speed), look));
    }
    for (i, a) in p.addresses.iter().enumerate() {
        let label = if i == 0 { "Address" } else { "" };
        lines.push(row(
            label,
            text(format!("{}/{}", a.address, a.prefix)),
            look,
        ));
    }
    if let Some(g) = &p.gateway {
        lines.push(row("Gateway", text(g.clone()), look));
    }
    for (i, d) in p.dns.iter().enumerate() {
        let label = if i == 0 { "DNS" } else { "" };
        lines.push(row(label, text(d.clone()), look));
    }
    lines
}

pub(super) fn detail(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let tab = app.ethernet();
    let selected = tab.selected();
    let title = selected.map_or("Ethernet", |p| p.id.as_str());
    let block = sym.block(look, title, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(p) = selected else {
        let msg = if tab.has_data() {
            "No Ethernet port found on this computer."
        } else {
            "Looking for ports..."
        };
        f.render_widget(
            Paragraph::new(Span::styled(msg, look.dim())).wrap(Wrap { trim: false }),
            inner,
        );
        return;
    };
    let busy = tab.busy.as_deref() == Some(p.id.as_str());
    let mut lines = Vec::new();
    let (headline, style) = match p.state {
        EthernetState::Connected => (format!("{} Connected", sym.dot()), look.ok().bold()),
        EthernetState::Connecting => ("~ Connecting...".to_owned(), look.warn().bold()),
        _ if busy => ("~ Connecting...".to_owned(), look.warn().bold()),
        EthernetState::Misconfigured => ("! Cable in, not working".to_owned(), look.err().bold()),
        EthernetState::Unplugged => (
            format!("{} Cable unplugged", sym.hollow()),
            look.warn().bold(),
        ),
        _ => ("Cable in, not joined".to_owned(), look.text().bold()),
    };
    lines.push(Line::styled(headline, style));
    lines.push(Line::default());
    match p.state {
        EthernetState::Unplugged => {
            lines.push(Line::styled(
                nexus_ipc::unplugged_message(&p.id),
                look.warn(),
            ));
            lines.push(Line::default());
        }
        EthernetState::Misconfigured if !busy => {
            if let Some(t) = &p.trouble {
                lines.push(Line::styled(t.message.clone(), look.err()));
                lines.push(Line::default());
            }
        }
        _ => {}
    }
    lines.extend(link_lines(p, look));
    if let Some(mac) = &p.mac {
        lines.push(row("MAC", Span::styled(mac.clone(), look.text()), look));
    }
    if let Some(profile) = &p.profile {
        let tag = if profile.enterprise { "  (802.1X)" } else { "" };
        lines.push(row(
            "Profile",
            Span::styled(format!("{}{tag}", clean(&profile.name)), look.text()),
            look,
        ));
    }
    lines.push(Line::default());
    lines.push(Line::styled(hint(p, busy), look.dim()));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// A Profile's name is the person's (or the office's) to choose: no control
/// characters on screen.
fn clean(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

fn hint(p: &EthernetPort, busy: bool) -> &'static str {
    if busy {
        return "Waiting for the port to come up...";
    }
    match p.state {
        EthernetState::Unplugged => "Nothing to join until a cable is in.",
        EthernetState::Connected => {
            "x drops the Link. s signs in (802.1X). e changes the Profile settings."
        }
        EthernetState::Misconfigured => {
            "Enter tries again. s signs in (802.1X). e changes the Profile settings."
        }
        EthernetState::Connecting => "x cancels.",
        _ => "Enter joins. s signs in (802.1X). e changes the Profile settings.",
    }
}
