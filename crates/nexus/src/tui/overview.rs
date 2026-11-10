//! The Overview: "am I online, and how" at a glance. Big-type hero, live
//! traffic graphs, summary cards and the recent-activity strip.
//!
//! Split layouts put the cards in the master column and the rest in the
//! detail pane; the narrow layout stacks everything and drops sections that
//! do not fit, last first.

use nexus_ipc::{ActivityKind, Connectivity, LinkKind, Status};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{HighlightSpacing, List, ListItem, ListState, Paragraph, Wrap};

use super::app::App;
use super::bigtype::{self, Px};
use super::caps::{ColorTier, Glyphs};
use super::doctor_view;
use super::graph;
use super::hits::Hit;
use super::layout::{Areas, Mode};
use super::theme::Look;
use super::view::{Sym, state_style};

/// A block is only drawn when it has room for a border and a line.
const MIN_BLOCK: u16 = 3;

/// Where each section goes. A zero-height rect means "does not fit".
struct Plan {
    cards: Rect,
    hero: Rect,
    traffic: Rect,
    activity: Rect,
    /// Cards get two lines each.
    roomy: bool,
}

/// The one place that decides what fits. Drawing and "are the graphs on
/// screen" (which gates the 1 Hz refresh) both read it.
fn plan(status: &Status, a: &Areas, look: &Look) -> Plan {
    match a.mode {
        Mode::Standard | Mode::Wide => {
            let area = a.detail.unwrap_or(a.master);
            let hero_h = hero_height(status, area.width, area.height / 2, look);
            let (hero, traffic, activity) = if a.mode == Mode::Wide {
                let [hero, lower] =
                    Layout::vertical([Constraint::Length(hero_h), Constraint::Fill(1)]).areas(area);
                let [traffic, activity] =
                    Layout::horizontal([Constraint::Percentage(58), Constraint::Fill(1)])
                        .areas(lower);
                (hero, traffic, activity)
            } else {
                let [hero, traffic, activity] = Layout::vertical([
                    Constraint::Length(hero_h),
                    Constraint::Length(traffic_height(area.height.saturating_sub(hero_h))),
                    Constraint::Fill(1),
                ])
                .areas(area);
                (hero, traffic, activity)
            };
            Plan {
                cards: a.master,
                hero,
                traffic,
                activity,
                roomy: true,
            }
        }
        // Narrow: hero, traffic, cards, activity from the top. When the
        // window is short, rows go to the hero first, then the cards (they
        // open the tabs), then traffic; the activity strip gets what is
        // left, if it can use it.
        _ => {
            let mut left = a.master.height;
            let mut take = |want: u16, least: u16| {
                let got = want.min(left);
                if got >= least {
                    left -= got;
                    got
                } else {
                    0
                }
            };
            let hero = take(
                3 + sentence_rows(status, a.master.width.saturating_sub(4)),
                3,
            );
            let cards = take(6, MIN_BLOCK);
            let traffic = take(4, 4);
            let activity = take(u16::MAX, MIN_BLOCK);
            let [hero, traffic, cards, activity] = Layout::vertical([
                Constraint::Length(hero),
                Constraint::Length(traffic),
                Constraint::Length(cards),
                Constraint::Length(activity),
            ])
            .areas(a.master);
            Plan {
                cards,
                hero,
                traffic,
                activity,
                roomy: false,
            }
        }
    }
}

/// Whether the traffic graphs are drawn at this size. Nothing else may ask
/// the Daemon for rates.
pub fn traffic_shown(look: &Look, status: &Status, width: u16, height: u16) -> bool {
    let a = super::layout::areas(Rect::new(0, 0, width, height));
    a.mode != Mode::TooSmall && plan(status, &a, look).traffic.height >= MIN_BLOCK
}

/// Where the hero sits (the focal element that pulses while connecting).
pub fn hero_rect(status: &Status, a: &Areas, look: &Look) -> Rect {
    plan(status, a, look).hero
}

pub fn draw(f: &mut Frame, app: &App, status: &Status, a: &Areas, look: &Look, sym: &Sym) {
    let p = plan(status, a, look);
    cards_panel(f, app, p.cards, look, sym, p.roomy);
    hero(f, status, app.doctor_trouble(), p.hero, look, sym);
    traffic(f, app, p.traffic, look, sym);
    // Trouble the Doctor found takes the top of the activity strip's room.
    let want = doctor_view::panel_height(app);
    let (diagnostics, activity_area) = if want == 0 || p.activity.height < MIN_BLOCK {
        (Rect::default(), p.activity)
    } else if p.activity.height >= want + MIN_BLOCK {
        let [d, rest] =
            Layout::vertical([Constraint::Length(want), Constraint::Fill(1)]).areas(p.activity);
        (d, rest)
    } else {
        (p.activity, Rect::default())
    };
    if diagnostics.height > 0 {
        doctor_view::panel(f, app, diagnostics, look, sym);
    }
    activity(f, app, activity_area, look, sym);
}

// ---- hero -----------------------------------------------------------------------

fn sentence_rows(status: &Status, width: u16) -> u16 {
    // Room for the longer of the two wordings, so a Doctor finding appearing
    // never moves the layout (and with it the graphs' gating).
    let len = [false, true]
        .into_iter()
        .map(|t| crate::text::tui::sentence(status, t).chars().count())
        .max()
        .unwrap_or(0) as u16;
    len.div_ceil(width.max(1)).clamp(1, 3)
}

/// Whether the big headline fits in a pane `width` wide, and at what scale.
fn big_scale(text: &str, width: u16, halo: bool) -> Option<u16> {
    let inner = width.saturating_sub(4);
    [2, 1]
        .into_iter()
        .find(|s| bigtype::width(text, *s, halo) <= inner)
}

fn wants_halo(look: &Look) -> bool {
    look.caps.color != ColorTier::None && look.caps.glyphs != Glyphs::Ascii
}

/// The hero's height: big type when the pane has the room, else a compact
/// hero. `budget` is the most the big one may take.
fn hero_height(status: &Status, width: u16, budget: u16, look: &Look) -> u16 {
    let halo = wants_halo(look);
    let sentence = sentence_rows(status, width.saturating_sub(4));
    let big = 2 + bigtype::height(halo) + 1 + sentence + 1;
    if big_scale(crate::text::tui::headline(status), width, halo).is_some() && big <= budget {
        big
    } else {
        2 + 1 + 1 + sentence + 1
    }
}

fn hero(f: &mut Frame, status: &Status, trouble: bool, area: Rect, look: &Look, sym: &Sym) {
    if area.height < MIN_BLOCK {
        return;
    }
    let block = sym.block(look, "Status", false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let style = state_style(look, status);
    let halo = wants_halo(look);
    let text = crate::text::tui::headline(status);
    let mut lines: Vec<Line> = Vec::new();
    let big_rows = bigtype::height(halo);
    let want_big = inner.height > big_rows + 2;
    match big_scale(text, area.width, halo).filter(|_| want_big) {
        Some(scale) => lines.extend(big_lines(text, scale, halo, style, sym)),
        None => lines.push(Line::from(vec![
            Span::styled(sym.dot(), style),
            Span::styled(format!(" {text}"), style.add_modifier(Modifier::BOLD)),
        ])),
    }
    let sentence = sentence_rows(status, inner.width) as usize;
    if inner.height as usize > lines.len() + sentence {
        lines.push(Line::default());
    }
    lines.push(Line::styled(
        crate::text::tui::sentence(status, trouble),
        look.text(),
    ));
    if inner.height as usize >= lines.len() + sentence {
        lines.push(Line::styled(facts(status), look.dim()));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn facts(status: &Status) -> String {
    let internet = match status.connectivity {
        Connectivity::None => "none",
        Connectivity::Portal => "sign-in required",
        Connectivity::Limited => "limited",
        Connectivity::Full => "full",
        _ => "unknown",
    };
    let backend = match &status.backend.version {
        Some(v) => format!("{} {v}", status.backend.name),
        None => status.backend.name.clone(),
    };
    format!("Internet: {internet}  |  Backend: {backend}")
}

fn big_lines(text: &str, scale: u16, halo: bool, style: Style, sym: &Sym) -> Vec<Line<'static>> {
    let lit = if sym.glyphs == Glyphs::Ascii {
        "#"
    } else {
        "█"
    };
    let glow = style.add_modifier(Modifier::DIM);
    bigtype::pixels(text, halo)
        .into_iter()
        .map(|row| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut run: Option<(Px, usize)> = None;
            let flush = |run: &mut Option<(Px, usize)>, spans: &mut Vec<Span<'static>>| {
                if let Some((px, n)) = run.take() {
                    let n = n * scale as usize;
                    spans.push(match px {
                        Px::Lit => Span::styled(lit.repeat(n), style),
                        Px::Halo => Span::styled("░".repeat(n), glow),
                        Px::Off => Span::raw(" ".repeat(n)),
                    });
                }
            };
            for px in row {
                match &mut run {
                    Some((p, n)) if *p == px => *n += 1,
                    _ => {
                        flush(&mut run, &mut spans);
                        run = Some((px, 1));
                    }
                }
            }
            flush(&mut run, &mut spans);
            Line::from(spans)
        })
        .collect()
}

// ---- traffic --------------------------------------------------------------------

fn traffic_height(room: u16) -> u16 {
    // Border + two graphs of two rows when there is room, one row otherwise.
    if room >= 14 { 6 } else { 4 }
}

fn traffic(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    if area.height < MIN_BLOCK {
        return;
    }
    let block = sym.block(look, "Traffic", false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let per = (inner.height / 2).clamp(1, 4);
    let label_w = 17.min(inner.width / 2);
    let rates = app.rates();
    let now = app.traffic_now().unwrap_or_default();
    for (i, (arrow, name, series, now_bps, style)) in [
        (sym.down(), "down", &rates.down, now.rx_bps, look.accent()),
        (sym.up(), "up", &rates.up, now.tx_bps, look.ok()),
    ]
    .into_iter()
    .enumerate()
    {
        let y = inner.y + i as u16 * per;
        if y + per > inner.y + inner.height {
            break;
        }
        let label = Rect::new(inner.x, y, label_w, per);
        let graph_area = Rect::new(inner.x + label_w, y, inner.width - label_w, per);
        let text = vec![
            Line::from(vec![
                Span::styled(format!("{arrow} "), style),
                Span::styled(graph::rate(now_bps), look.text()),
            ]),
            Line::styled(name, look.dim()),
        ];
        f.render_widget(Paragraph::new(text), label);
        let rows = if sym.glyphs == Glyphs::Ascii {
            graph::bars(series, graph_area.width, per)
        } else {
            graph::braille(series, graph_area.width, per)
        };
        f.render_widget(
            Paragraph::new(rows.into_iter().map(Line::from).collect::<Vec<_>>()).style(style),
            graph_area,
        );
    }
}

// ---- cards ----------------------------------------------------------------------

/// What one card says: its name, its value, and whether it is active.
pub fn card(app: &App, row: usize) -> (&'static str, String, bool) {
    let link = app.status().and_then(|s| s.link.as_ref());
    let (name, kinds, idle): (&str, &[LinkKind], &str) = match row {
        0 => ("Wi-Fi", &[LinkKind::Wifi], "Not connected"),
        1 => ("Ethernet", &[LinkKind::Ethernet], "Not connected"),
        2 => (
            "VPN",
            &[LinkKind::Vpn, LinkKind::Wireguard],
            "Not connected",
        ),
        _ => ("Hotspot", &[], "Not started"),
    };
    if row == 1
        && let Some(port) = app.ethernet().headline()
    {
        return ethernet_card(name, port);
    }
    if row == 3
        && let Some(h) = app.hotspot().info()
    {
        // Whether the machine *can* host one is the Hotspot tab's to say: the
        // picture arrives after the first frame, and a card that flips
        // between two wordings would be noise.
        if let (Some(ssid), true) = (&h.ssid, h.on) {
            return (name, ssid.clone(), true);
        }
    }
    if row == 0 && !app.radios().wifi.enabled {
        return (
            name,
            crate::text::tui::wifi_card_off(&app.radios()).into(),
            false,
        );
    }
    match link {
        Some(l) if kinds.contains(&l.kind) => (name, l.name.clone(), true),
        _ if app.status().is_none() => (name, "-".into(), false),
        _ => (name, idle.into(), false),
    }
}

/// The Ethernet card: the port's Link details, or why there is none.
fn ethernet_card(
    name: &'static str,
    port: &nexus_ipc::EthernetPort,
) -> (&'static str, String, bool) {
    use nexus_ipc::{EthernetProblem, EthernetState};
    // The card is narrow: the Link's speed and address, or the one thing wrong.
    // The Ethernet tab names the port.
    let value = match port.state {
        EthernetState::Connected => {
            let mut parts: Vec<String> = port.speed_text().into_iter().collect();
            parts.extend(port.addresses.first().map(|a| a.address.clone()));
            if parts.is_empty() {
                "Connected".to_owned()
            } else {
                parts.join(", ")
            }
        }
        EthernetState::Connecting => "Connecting".to_owned(),
        EthernetState::Unplugged => "Cable unplugged".to_owned(),
        EthernetState::Misconfigured => match port.trouble.as_ref().map(|t| t.kind) {
            Some(EthernetProblem::NoAddress) => "No address".to_owned(),
            Some(EthernetProblem::SignIn) => "Needs sign-in".to_owned(),
            _ => "Not working".to_owned(),
        },
        _ => "Not connected".to_owned(),
    };
    (name, value, port.state == EthernetState::Connected)
}

/// The four cards as a selectable list. `roomy` gives each two lines.
pub fn cards_panel(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym, roomy: bool) {
    if area.height < MIN_BLOCK {
        return;
    }
    let block = sym.block(look, "Connections", true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let roomy = roomy && inner.height >= 8;
    let items: Vec<ListItem> = (0..app.tab().rows())
        .map(|row| {
            let (name, value, active) = card(app, row);
            let dot = Span::styled(
                if active { sym.dot() } else { sym.hollow() },
                if active { look.ok() } else { look.dim() },
            );
            if roomy {
                ListItem::new(vec![
                    Line::from(vec![dot, Span::styled(format!(" {name}"), look.title())]),
                    Line::styled(format!("  {value}"), look.muted()),
                ])
            } else {
                ListItem::new(Line::from(vec![
                    dot,
                    Span::raw(format!(" {name:<9}")),
                    Span::styled(value, look.muted()),
                ]))
            }
        })
        .collect();
    let list = List::new(items)
        .highlight_style(look.selected())
        .highlight_symbol(sym.pointer())
        .highlight_spacing(HighlightSpacing::Always);
    let mut state = ListState::default().with_selected(Some(app.selected()));
    f.render_stateful_widget(list, inner, &mut state);
    let step = if roomy { 2 } else { 1 };
    for row in state.offset()..app.tab().rows() {
        let y = inner.y + ((row - state.offset()) as u16) * step;
        if y >= inner.bottom() {
            break;
        }
        let rect = Rect::new(inner.x, y, inner.width, step.min(inner.bottom() - y));
        app.hit(rect, Hit::Row(row));
    }
}

// ---- activity -------------------------------------------------------------------

fn activity(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    if area.height < MIN_BLOCK {
        return;
    }
    let block = sym.block(look, "Recent activity", false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if app.activity().is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "Nothing yet. Joins, losses, VPNs and sign-in pages show up here.",
                look.dim(),
            ))
            .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }
    let lines: Vec<Line> = app
        .activity()
        .iter()
        .take(inner.height as usize)
        .map(|e| {
            let style = match e.kind {
                ActivityKind::Joined | ActivityKind::VpnUp | ActivityKind::Online => look.ok(),
                ActivityKind::Lost => look.err(),
                ActivityKind::VpnDown | ActivityKind::Portal => look.warn(),
                _ => look.muted(),
            };
            Line::from(vec![
                Span::styled(format!("{} ", sym.dot()), style),
                Span::styled(e.text.clone(), look.text()),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}
