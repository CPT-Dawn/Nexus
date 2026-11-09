//! Drawing. Pure: `App` in, cells out. Colours come only from [`Look`] tokens,
//! areas only from the layout engine.

use nexus_ipc::{Connectivity, LinkKind, State, Status};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, HighlightSpacing, List, ListItem, ListState, Padding,
    Paragraph, Wrap,
};

use super::app::{App, Conn, Layer, Tab};
use super::caps::Glyphs;
use super::layout::{self, Areas, Mode};
use super::theme::Look;
use super::wifi_ui;
use super::words;

/// The symbols for one glyph set.
pub(super) struct Sym {
    pub(super) glyphs: Glyphs,
}

impl Sym {
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
    let a = layout::areas(area);
    if a.mode == Mode::TooSmall {
        too_small(f, area, &look);
        return;
    }
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

fn state_style(look: &Look, status: &Status) -> Style {
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
    if a.mode.split() {
        master(f, app, a.master, look, sym);
        if let Some(detail) = a.detail {
            self::detail(f, app, detail, a.mode == Mode::Wide, look, sym);
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
        detail(f, app, area, false, look, sym);
    } else {
        master(f, app, a.master, look, sym);
    }
}

/// The summary of one Overview card.
fn card(app: &App, row: usize) -> (&'static str, String) {
    let link = app.status().and_then(|s| s.link.as_ref());
    let (name, kinds): (&str, &[LinkKind]) = match row {
        0 => ("Wi-Fi", &[LinkKind::Wifi]),
        1 => ("Ethernet", &[LinkKind::Ethernet]),
        2 => ("VPN", &[LinkKind::Vpn, LinkKind::Wireguard]),
        _ => ("Hotspot", &[]),
    };
    let value = match link {
        Some(l) if kinds.contains(&l.kind) => l.name.clone(),
        _ if row == 3 || app.status().is_none() => "Off".into(),
        _ => "Idle".into(),
    };
    (name, value)
}

fn master(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    if app.tab() == Tab::WiFi {
        return wifi_ui::master(f, app, area, look, sym);
    }
    let title = app.tab().title();
    let block = sym.block(look, title, true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if app.tab().rows() == 0 {
        f.render_widget(
            Paragraph::new(Span::styled("Nothing to list yet.", look.dim())),
            inner,
        );
        return;
    }
    let items: Vec<ListItem> = (0..app.tab().rows())
        .map(|row| {
            let (name, value) = card(app, row);
            ListItem::new(Line::from(vec![
                Span::raw(format!("{name:<10}")),
                Span::styled(value, look.muted()),
            ]))
        })
        .collect();
    let list = List::new(items)
        .highlight_style(look.selected())
        .highlight_symbol(sym.pointer())
        .highlight_spacing(HighlightSpacing::Always);
    let mut state = ListState::default().with_selected(Some(app.selected()));
    f.render_stateful_widget(list, inner, &mut state);
}

fn detail(f: &mut Frame, app: &App, area: Rect, wide: bool, look: &Look, sym: &Sym) {
    let title = match app.tab() {
        Tab::Overview => card(app, app.selected()).0,
        other => other.title(),
    };
    if app.conn() != Conn::Up {
        let block = sym.block(look, title, false);
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(
            Paragraph::new(daemon_lines(app, look)).wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    if app.tab() == Tab::WiFi {
        return wifi_ui::detail(f, app, area, look, sym);
    }
    let (Tab::Overview, Some(status)) = (app.tab(), app.status()) else {
        let block = sym.block(look, title, false);
        let inner = block.inner(area);
        f.render_widget(block, area);
        let msg = format!("{} arrives in a later release.", app.tab().title());
        f.render_widget(Paragraph::new(Span::styled(msg, look.dim())), inner);
        return;
    };

    if wide {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(55), Constraint::Fill(1)]).areas(area);
        let block = sym.block(look, "Status", false);
        let inner = block.inner(left);
        f.render_widget(block, left);
        f.render_widget(
            Paragraph::new(hero(status, look, sym)).wrap(Wrap { trim: false }),
            inner,
        );
        let block = sym.block(look, title, false);
        let inner = block.inner(right);
        f.render_widget(block, right);
        f.render_widget(Paragraph::new(facts(app, status, look)), inner);
    } else {
        let block = sym.block(look, title, false);
        let inner = block.inner(area);
        f.render_widget(block, area);
        let mut lines = hero(status, look, sym);
        lines.push(Line::default());
        lines.extend(facts(app, status, look));
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
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

fn hero(status: &Status, look: &Look, sym: &Sym) -> Vec<Line<'static>> {
    vec![
        Line::default(),
        Line::from(vec![Span::styled(
            format!("{} {}", sym.dot(), words::headline(status)),
            state_style(look, status).bold(),
        )]),
        Line::default(),
        Line::styled(words::sentence(status), look.text()),
    ]
}

fn facts(app: &App, status: &Status, look: &Look) -> Vec<Line<'static>> {
    let row = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!("{k:<14}"), look.muted()),
            Span::styled(v, look.text()),
        ])
    };
    let (name, value) = card(app, app.selected());
    let link = match &status.link {
        Some(l) => l.name.clone(),
        None => "none".into(),
    };
    let connectivity = match status.connectivity {
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
    vec![
        row(name, value),
        row("Primary link", link),
        row("Internet", connectivity.into()),
        row("Backend", backend),
    ]
}

fn help(f: &mut Frame, area: Rect, look: &Look, sym: &Sym) {
    const KEYS: [(&str, &str); 15] = [
        ("j / k, Down / Up", "Move in the list"),
        ("g / G", "Jump to top / bottom"),
        ("h / l, Left / Right", "Previous / next tab"),
        ("Tab / Shift-Tab", "Next / previous tab"),
        ("1 - 6", "Jump to a tab"),
        ("Enter, d", "Open details (small windows)"),
        ("Wi-Fi: Enter", "Join the selected Network"),
        ("Wi-Fi: r", "Rescan"),
        ("Wi-Fi: x", "Disconnect"),
        ("Wi-Fi: f", "Forget (asks first)"),
        ("Wi-Fi: a", "Toggle autoconnect"),
        ("Wi-Fi: u", "Undo the toast's action"),
        ("Esc", "Back out; never quits"),
        ("?", "Show or hide this help"),
        ("q, Ctrl-C", "Quit"),
    ];
    let height = KEYS.len() as u16 + 2;
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
    f.render_widget(Paragraph::new(lines), inner);
}
