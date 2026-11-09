//! Drawing. Pure: `App` in, cells out. Colours come only from [`Look`] tokens,
//! areas only from the layout engine.

use nexus_ipc::{Connectivity, State, Status};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap};

use super::app::{App, Conn, Layer, Tab};
use super::caps::Glyphs;
use super::doctor_view;
use super::eap_ui;
use super::hits::{Hit, hint_key};
use super::layout::{self, Areas, Mode};
use super::overlays;
use super::overview;
use super::profile_ui;
use super::tether_ui;
use super::theme::Look;
use super::vpn_ui;
use super::wifi_ui;
use super::words;

/// The symbols for one glyph set.
pub(super) struct Sym {
    pub(super) glyphs: Glyphs,
}

impl Sym {
    pub(super) fn hollow(&self) -> &'static str {
        match self.glyphs {
            Glyphs::Ascii => "o",
            _ => "○",
        }
    }

    pub(super) fn down(&self) -> &'static str {
        match self.glyphs {
            Glyphs::Ascii => "v",
            _ => "↓",
        }
    }

    pub(super) fn up(&self) -> &'static str {
        match self.glyphs {
            Glyphs::Ascii => "^",
            _ => "↑",
        }
    }

    pub(super) fn dot(&self) -> &'static str {
        match self.glyphs {
            Glyphs::Ascii => "*",
            _ => "●",
        }
    }

    fn caret(&self) -> &'static str {
        match self.glyphs {
            Glyphs::Ascii => "v",
            _ => "▾",
        }
    }

    pub(super) fn pointer(&self) -> &'static str {
        match self.glyphs {
            Glyphs::Ascii => "> ",
            _ => "▸ ",
        }
    }

    /// Nerd Font tab icons, only when opted in.
    fn tab_icon(&self, tab: Tab) -> &'static str {
        if self.glyphs != Glyphs::Nerd {
            return "";
        }
        match tab {
            Tab::Overview => "\u{f015} ",
            Tab::WiFi => "\u{f1eb} ",
            Tab::Ethernet => "\u{f0ec} ",
            Tab::Vpn => "\u{f132} ",
            Tab::Hotspot => "\u{f09e} ",
            Tab::Settings => "\u{f013} ",
        }
    }

    pub(super) fn block<'a>(&self, look: &Look, title: &'a str, focused: bool) -> Block<'a> {
        let border_style = if focused {
            look.border_focus()
        } else {
            look.border()
        };
        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(border_style)
            .padding(Padding::horizontal(1))
            .title(Span::styled(format!(" {title} "), look.title()));
        match self.glyphs {
            Glyphs::Ascii => block.border_set(border::Set {
                top_left: "+",
                top_right: "+",
                bottom_left: "+",
                bottom_right: "+",
                vertical_left: "|",
                vertical_right: "|",
                horizontal_top: "-",
                horizontal_bottom: "-",
            }),
            _ => block.border_type(BorderType::Rounded),
        }
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    app.hit_reset();
    let look = app.look();
    let sym = Sym {
        glyphs: look.caps.glyphs,
    };
    let area = f.area();
    if layout::Mode::of(area) == Mode::TooSmall {
        too_small(f, area, &look);
        return;
    }
    // A Portal's banner takes the top row whenever there is room to spare.
    // The Overview's hero already says it (and keeps its big type).
    let (banner, below) = match app.portal() {
        Some(network) if area.height > layout::MIN_HEIGHT && app.tab() != Tab::Overview => {
            let (top, rest) = (
                Rect { height: 1, ..area },
                Rect {
                    y: area.y + 1,
                    height: area.height - 1,
                    ..area
                },
            );
            (Some((top, network)), rest)
        }
        _ => (None, area),
    };
    if let Some((top, network)) = banner {
        let text = format!(" Login required for {network}. Press o to open it.");
        f.render_widget(Paragraph::new(Span::styled(text, look.warn().bold())), top);
    }
    let a = layout::areas(below);
    header(f, app, &a, &look, &sym);
    body(f, app, &a, &look, &sym);
    footer(f, app, &a, &look);
    let body = Rect {
        y: a.header.bottom(),
        height: a.footer.y.saturating_sub(a.header.bottom()),
        ..a.header
    };
    wifi_ui::overlays(f, app, body, &look, &sym);
    vpn_ui::overlays(f, app, body, &look, &sym);
    if let Some(form) = app.eap_form() {
        eap_ui::draw(f, form, body, &look, &sym);
    }
    profile_ui::overlay(f, app, body, &look, &sym);
    if app.layer() == Layer::Help {
        overlays::help(f, app, body, &look, &sym);
    }
    overlays::palette(f, app, area, &look, &sym);
    if app.layer() == Layer::Confirm {
        doctor_view::confirm(f, app, area, &look, &sym);
    }
}

fn too_small(f: &mut Frame, area: Rect, look: &Look) {
    let text = format!(
        "Terminal too small\nNexus needs {}x{}",
        layout::MIN_WIDTH,
        layout::MIN_HEIGHT
    );
    f.render_widget(
        Paragraph::new(text)
            .style(look.warn())
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        area,
    );
}

pub(super) fn state_style(look: &Look, status: &Status) -> Style {
    match (status.state, status.connectivity) {
        (State::Online, Connectivity::Full | Connectivity::Unknown) => look.ok(),
        (State::Online | State::Connecting | State::Disconnecting, _) => look.warn(),
        (State::Local | State::Site | State::Offline | State::Unavailable, _) => look.err(),
        _ => look.muted(),
    }
}

fn header(f: &mut Frame, app: &App, a: &Areas, look: &Look, sym: &Sym) {
    let mut left = vec![Span::styled(" Nexus ", look.accent().bold())];
    if a.mode.split() {
        for tab in Tab::ALL {
            let label = format!(" {}{} ", sym.tab_icon(tab), tab.title());
            let style = if tab == app.tab() {
                look.selected()
            } else {
                look.muted()
            };
            let span = Span::styled(label, style);
            let x = a.header.x + Line::from(left.clone()).width() as u16;
            let rect = Rect::new(x, a.header.y, span.width() as u16, 1);
            app.hit(rect.intersection(a.header), Hit::Tab(tab));
            left.push(span);
        }
    } else {
        let name = Span::styled(
            format!(" {} {} ", app.tab().title(), sym.caret()),
            look.selected(),
        );
        let x = a.header.x + Line::from(left.clone()).width() as u16;
        let rect = Rect::new(x, a.header.y, name.width() as u16, 1);
        app.hit(rect.intersection(a.header), Hit::NextTab);
        left.push(name);
        left.push(Span::styled(
            format!(" {}/{}", app.tab().index() + 1, Tab::ALL.len()),
            look.dim(),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(left)), a.header);

    let right = match (app.conn(), app.status()) {
        (Conn::Up, Some(s)) => Line::from(vec![
            Span::styled(sym.dot(), state_style(look, s)),
            Span::styled(format!(" {} ", words::headline(s)), look.text()),
        ]),
        _ if a.mode.split() => Line::from(Span::styled("? help ", look.dim())),
        _ => Line::default(),
    };
    f.render_widget(
        Paragraph::new(right).alignment(Alignment::Right),
        Rect {
            // Keep clear of the narrow header's tab name.
            x: a.header.x + a.header.width / 2,
            width: a.header.width - a.header.width / 2,
            ..a.header
        },
    );
}

/// Drops the least needed hints (quit, tab, move: all known or on screen
/// elsewhere) until the row fits. Everything else stays.
fn fit_hints(
    mut hints: Vec<(&'static str, &'static str)>,
    width: u16,
) -> Vec<(&'static str, &'static str)> {
    let used = |h: &[(&str, &str)]| {
        1 + h
            .iter()
            .map(|(k, w)| k.chars().count() + 1 + w.chars().count() + 2)
            .sum::<usize>()
    };
    for drop in ["q", "h/l", "j/k"] {
        if used(&hints) <= width as usize {
            break;
        }
        hints.retain(|(k, _)| *k != drop);
    }
    hints
}

fn footer(f: &mut Frame, app: &App, a: &Areas, look: &Look) {
    let mut spans = vec![Span::raw(" ")];
    for (key, what) in fit_hints(app.hints(!a.mode.split()), a.footer.width) {
        // Each hint is a button: a click presses the key it names.
        if let Some((code, mods)) = hint_key(key) {
            let x = a.footer.x + Line::from(spans.clone()).width() as u16;
            let w = (key.chars().count() + 1 + what.chars().count()) as u16;
            app.hit(
                Rect::new(x, a.footer.y, w, 1).intersection(a.footer),
                Hit::Key(code, mods),
            );
        }
        spans.push(Span::styled(key, look.accent()));
        spans.push(Span::styled(format!(" {what}  "), look.muted()));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), a.footer);

    let chip = match app.conn() {
        Conn::Up => return,
        Conn::Connecting => "connecting to the daemon ",
        Conn::Down => "daemon unreachable, retrying ",
    };
    f.render_widget(
        Paragraph::new(Span::styled(chip, look.warn())).alignment(Alignment::Right),
        Rect {
            x: a.footer.x + a.footer.width / 2,
            width: a.footer.width - a.footer.width / 2,
            ..a.footer
        },
    );
}

fn body(f: &mut Frame, app: &App, a: &Areas, look: &Look, sym: &Sym) {
    if app.tab() == Tab::Overview && app.needs_setup() {
        let whole = a.detail.map_or(a.master, |d| a.master.union(d));
        doctor_view::setup(f, app, whole, look, sym);
        return;
    }
    if let (Tab::Overview, Conn::Up, Some(status)) = (app.tab(), app.conn(), app.status()) {
        overview::draw(f, app, status, a, look, sym);
        return;
    }
    if a.mode.split() {
        master(f, app, a.master, look, sym);
        if let Some(detail) = a.detail {
            self::detail(f, app, detail, look, sym);
        }
        return;
    }
    // Narrow: the list, or the sheet over it.
    if app.layer() == Layer::Sheet || app.conn() != Conn::Up {
        let area = if app.layer() == Layer::Sheet {
            f.render_widget(Clear, a.sheet);
            a.sheet
        } else {
            a.master
        };
        detail(f, app, area, look, sym);
    } else {
        master(f, app, a.master, look, sym);
    }
}

fn master(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    if app.tab() == Tab::WiFi {
        return wifi_ui::master(f, app, area, look, sym);
    }
    if app.tab() == Tab::Vpn {
        return vpn_ui::master(f, app, area, look, sym);
    }
    if app.tab() == Tab::Hotspot {
        return tether_ui::master(f, app, area, look, sym);
    }
    if app.tab().rows() > 0 {
        overview::cards_panel(f, app, area, look, sym, true);
        return;
    }
    let block = sym.block(look, app.tab().title(), true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(
        Paragraph::new(Span::styled("Nothing to list yet.", look.dim())),
        inner,
    );
}

fn detail(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    if app.tab() == Tab::WiFi && app.conn() == Conn::Up {
        return wifi_ui::detail(f, app, area, look, sym);
    }
    if app.tab() == Tab::Vpn && app.conn() == Conn::Up {
        return vpn_ui::detail(f, app, area, look, sym);
    }
    if app.tab() == Tab::Hotspot && app.conn() == Conn::Up {
        return tether_ui::detail(f, app, area, look, sym);
    }
    let title = match app.tab() {
        Tab::Overview => "Status",
        other => other.title(),
    };
    let block = sym.block(look, title, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if app.conn() != Conn::Up {
        f.render_widget(
            Paragraph::new(daemon_lines(app, look)).wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    let msg = format!("{} arrives in a later release.", app.tab().title());
    f.render_widget(Paragraph::new(Span::styled(msg, look.dim())), inner);
}

fn daemon_lines(app: &App, look: &Look) -> Vec<Line<'static>> {
    match app.conn() {
        Conn::Down => vec![
            Line::styled("Can't reach the Nexus daemon.", look.err()),
            Line::default(),
            Line::styled("Reconnecting automatically.", look.text()),
            Line::styled(
                "Start it with `systemctl --user start nexus.socket`, or run `nexus daemon`.",
                look.muted(),
            ),
        ],
        _ => vec![Line::styled(
            "Connecting to the Nexus daemon...",
            look.muted(),
        )],
    }
}
