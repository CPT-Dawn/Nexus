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
use super::layout::{self, Areas, Mode};
use super::overview;
use super::theme::Look;
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
    if app.layer() == Layer::Help {
        help(f, area, &look, &sym);
    }
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
            left.push(Span::styled(label, style));
        }
    } else {
        left.push(Span::styled(
            format!(" {} {} ", app.tab().title(), sym.caret()),
            look.selected(),
        ));
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

fn footer(f: &mut Frame, app: &App, a: &Areas, look: &Look) {
    let mut spans = vec![Span::raw(" ")];
    for (key, what) in app.hints(!a.mode.split()) {
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

fn help(f: &mut Frame, area: Rect, look: &Look, sym: &Sym) {
    const KEYS: &[(&str, &str)] = &[
        ("j / k, Down / Up", "Move in the list"),
        ("g / G", "Jump to top / bottom"),
        ("h / l, Left / Right", "Previous / next tab"),
        ("Tab / Shift-Tab", "Next / previous tab"),
        ("1 - 6", "Jump to a tab"),
        ("Enter", "Open the selected card's tab"),
        ("d", "Details sheet (small windows)"),
        ("Wi-Fi: Enter", "Join the selected Network"),
        ("Wi-Fi: r", "Rescan"),
        ("Wi-Fi: x", "Disconnect"),
        ("Wi-Fi: f", "Forget (asks first)"),
        ("Wi-Fi: a", "Toggle autoconnect"),
        ("Wi-Fi: s", "Share as a QR code (asks first)"),
        ("Wi-Fi: u", "Undo the toast's action"),
        ("o", "Open a Portal's login page"),
        ("Esc", "Back out; never quits"),
        ("?", "Show or hide this help"),
        ("q, Ctrl-C", "Quit"),
    ];
    // Why a Portal's page may not show, when the window has room to say so.
    let note = if area.height >= KEYS.len() as u16 + 8 {
        PORTAL_NOTE
    } else {
        ""
    };
    let note_lines = if note.is_empty() { 0 } else { 5 };
    let height = KEYS.len() as u16 + 2 + note_lines;
    let width = 58.min(area.width);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height: height.min(area.height),
    };
    f.render_widget(Clear, rect);
    let block = sym.block(look, "All keys", true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let lines: Vec<Line> = KEYS
        .iter()
        .map(|(k, what)| {
            Line::from(vec![
                Span::styled(format!(" {k:<21}"), look.accent()),
                Span::styled(*what, look.text()),
            ])
        })
        .collect();
    let keys = Rect {
        height: KEYS.len() as u16,
        ..inner
    };
    f.render_widget(Paragraph::new(lines), keys);
    if !note.is_empty() {
        let below = Rect {
            y: keys.bottom() + 1,
            height: inner.height.saturating_sub(keys.height + 1),
            ..inner
        };
        f.render_widget(
            Paragraph::new(Span::styled(note, look.muted())).wrap(Wrap { trim: true }),
            below,
        );
    }
}

/// Browsers with secure DNS (DNS over HTTPS) skip the network's own resolver,
/// which is how a Portal steers you to its page.
const PORTAL_NOTE: &str = "Login page won't show? A browser with secure DNS (DNS over HTTPS) hides the Portal by skipping the network's resolver. Turn it off until you have signed in, or press o: Nexus opens a plain http:// page.";
