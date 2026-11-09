//! Drawing the Hotspot panel and the form that chooses its name and
//! password. Pure like the rest of the views: `App` in, cells out.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::app::App;
use super::caps::Glyphs;
use super::hotspot::{Busy, Field, Form};
use super::share::centered;
use super::theme::Look;
use super::view::Sym;

/// Rows the panel takes in the Hotspot tab's list column, borders included.
pub(super) const PANEL_HEIGHT: u16 = 6;

/// A name is the owner's to choose, but a stranger's Hotspot is not: no
/// control characters on screen.
fn clean(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

/// How tall the panel wants to be at `width`: an explanation of why this
/// machine cannot host a Hotspot needs room to wrap.
pub(super) fn panel_height(app: &App, width: u16) -> u16 {
    match app.hotspot().info() {
        Some(h) if !h.available => {
            let chars = h.why.as_deref().map_or(0, |w| w.chars().count());
            // Words wrap early, so count a few columns less than there are.
            let room = usize::from(width.saturating_sub(6)).max(10);
            let wrapped = u16::try_from(chars.div_ceil(room)).unwrap_or(u16::MAX);
            (3 + wrapped).clamp(PANEL_HEIGHT, 10)
        }
        _ => PANEL_HEIGHT,
    }
}

fn devices(n: Option<u32>) -> String {
    match n {
        Some(1) => "1 device joined".into(),
        Some(n) => format!("{n} devices joined"),
        None => "device count unknown".into(),
    }
}

pub(super) fn panel(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let tab = app.hotspot();
    let block = sym.block(look, "Hotspot", false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let lines = match tab.info() {
        None => vec![Line::styled("Checking Wi-Fi...", look.dim())],
        Some(h) if tab.busy == Busy::Starting && !h.on => vec![
            Line::from(vec![
                Span::styled("~ ", look.warn()),
                Span::styled("Starting...", look.warn()),
            ]),
            Line::styled("Waiting for NetworkManager.", look.dim()),
        ],
        Some(h) if h.on => {
            let ssid = clean(h.ssid.as_deref().unwrap_or(""));
            let state = if h.starting {
                Line::from(vec![
                    Span::styled("~ ", look.warn()),
                    Span::styled(ssid, look.text().bold()),
                    Span::styled("  starting...", look.warn()),
                ])
            } else {
                Line::from(vec![
                    Span::styled(sym.dot(), look.ok()),
                    Span::raw(" "),
                    Span::styled(ssid, look.text().bold()),
                ])
            };
            let count = if h.starting {
                Line::default()
            } else {
                Line::styled(devices(h.devices), look.muted())
            };
            let keys = if tab.busy == Busy::Stopping {
                Line::styled("Stopping...", look.warn())
            } else {
                Line::styled("s QR code   S stop", look.dim())
            };
            vec![state, count, keys]
        }
        Some(h) if !h.available => vec![
            Line::from(vec![
                Span::styled(sym.hollow(), look.err()),
                Span::styled(" Not available", look.err()),
            ]),
            Line::styled(h.why.clone().unwrap_or_default(), look.muted()),
        ],
        Some(_) => vec![
            Line::from(vec![
                Span::styled(sym.hollow(), look.dim()),
                Span::styled(" Off", look.text()),
            ]),
            Line::styled("n start, strong password", look.dim()),
            Line::styled("c choose name, password", look.dim()),
        ],
    };
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn dots(n: usize, sym: &Sym) -> String {
    let dot = if sym.glyphs == Glyphs::Ascii {
        '*'
    } else {
        '\u{2022}'
    };
    std::iter::repeat_n(dot, n).collect()
}

/// The form over the tab.
pub(super) fn form_view(f: &mut Frame, form: &Form, body: Rect, look: &Look, sym: &Sym) {
    let rect = centered(body, 66, 9);
    f.render_widget(Clear, rect);
    let block = sym.block(look, "Start a Hotspot", true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let room = (inner.width as usize).saturating_sub(12);
    let tail = |s: &str| -> String {
        let skip = s.chars().count().saturating_sub(room);
        s.chars().skip(skip).collect()
    };
    let line = |field: Field, label: &str, shown: String, hint: &'static str| {
        let focused = form.focus == field && !form.busy;
        let mut spans = vec![Span::styled(
            format!("{label:<10}"),
            if focused { look.accent() } else { look.muted() },
        )];
        if shown.is_empty() && !focused {
            spans.push(Span::styled(hint, look.dim()));
        } else {
            spans.push(Span::styled(shown, look.text()));
            if focused {
                spans.push(Span::styled("_", look.accent()));
            }
        }
        Line::from(spans)
    };
    let password = if form.letters {
        tail(&form.password)
    } else {
        tail(&dots(form.password.chars().count(), sym))
    };
    let lines = vec![
        line(
            Field::Name,
            "Name",
            tail(&form.ssid),
            "empty: Nexus-XXXX",
        ),
        line(
            Field::Password,
            "Password",
            password,
            "empty: a strong one is made up",
        ),
        Line::default(),
        match (&form.error, form.busy) {
            (_, true) => Line::styled("Starting...", look.warn()),
            (Some(why), _) => Line::styled(why.clone(), look.err()),
            (None, false) => Line::styled(
                "8 to 63 characters. The password shows as a QR code later (s).",
                look.muted(),
            ),
        },
        Line::default(),
        Line::styled(
            "tab next   enter start   ctrl-r show password   esc cancel",
            look.dim(),
        ),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
