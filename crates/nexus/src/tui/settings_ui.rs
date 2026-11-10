//! Drawing the Settings list and its detail pane. Pure like the rest of the
//! views: `App` in, cells out.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{HighlightSpacing, List, ListItem, ListState, Paragraph, Wrap};

use super::app::App;
use super::caps::Glyphs;
use super::hits::Hit;
use super::settings::{ITEMS, Item, Kind};
use super::theme::{Look, Theme};
use super::view::Sym;

/// What the list shows on the right of a row.
fn value(app: &App, item: &Item, look: &Look) -> Span<'static> {
    let tab = app.settings();
    match item.kind {
        Kind::Theme => Span::styled(app.theme_title(), look.accent()),
        Kind::Switch if tab.is_on(item) => Span::styled("on", look.ok()),
        Kind::Switch => Span::styled("off", look.dim()),
    }
}

pub(super) fn master(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let block = sym.block(look, "Settings", true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if !app.settings().has_data() {
        f.render_widget(
            Paragraph::new(Span::styled("Waiting for the daemon...", look.dim())),
            inner,
        );
        return;
    }
    let avail = (inner.width as usize).saturating_sub(2);
    let items: Vec<ListItem> = ITEMS
        .iter()
        .map(|item| {
            let mut v = value(app, item, look);
            // The label wins the room; a long value (a custom theme's name)
            // is the one that gives way.
            let room = avail.saturating_sub(item.label.chars().count() + 1);
            if v.content.chars().count() > room {
                let keep: String = v.content.chars().take(room.saturating_sub(1)).collect();
                let dots = if sym.glyphs == Glyphs::Ascii {
                    "~"
                } else {
                    "…"
                };
                v.content = format!("{keep}{dots}").into();
            }
            let pad = avail.saturating_sub(item.label.chars().count() + v.content.chars().count());
            ListItem::new(Line::from(vec![
                Span::raw(item.label),
                Span::raw(" ".repeat(pad.max(1))),
                v,
            ]))
        })
        .collect();
    let list = List::new(items)
        .highlight_style(look.selected())
        .highlight_symbol(sym.pointer())
        .highlight_spacing(HighlightSpacing::Always);
    let mut state = ListState::default().with_selected(Some(app.selected()));
    f.render_stateful_widget(list, inner, &mut state);
    for row in state.offset()..ITEMS.len() {
        let y = inner.y + (row - state.offset()) as u16;
        if y >= inner.bottom() {
            break;
        }
        app.hit(Rect::new(inner.x, y, inner.width, 1), Hit::Row(row));
    }
}

/// Four colour chips of a theme, drawn in that theme's own colours.
fn swatch(theme: Theme, look: &Look, sym: &Sym) -> Vec<Span<'static>> {
    let chip = if sym.glyphs == Glyphs::Ascii {
        "##"
    } else {
        "██"
    };
    let theirs = Look::new(theme, look.caps);
    [theirs.accent(), theirs.ok(), theirs.warn(), theirs.err()]
        .into_iter()
        .map(|s| Span::styled(chip, s))
        .collect()
}

fn theme_lines(app: &App, look: &Look, sym: &Sym) -> Vec<Line<'static>> {
    let tab = app.settings();
    let mut lines = vec![Line::default()];
    let now = tab.theme_name();
    let row = |lines: &mut Vec<Line<'static>>, name: &str, title: String, theme: Theme| {
        let here = name == now;
        let mut spans = vec![Span::styled(
            if here { sym.dot() } else { " " },
            if here { look.accent() } else { look.text() },
        )];
        spans.push(Span::raw(" "));
        spans.extend(swatch(theme, look, sym));
        spans.push(Span::styled(
            format!(" {title}"),
            if here { look.title() } else { look.text() },
        ));
        lines.push(Line::from(spans));
    };
    for t in Theme::BUILTIN {
        row(&mut lines, t.name, t.title.to_owned(), t);
    }
    for name in tab.custom_names() {
        if Theme::builtin(&name).is_some() {
            continue;
        }
        match tab
            .custom_tokens(&name)
            .map(|tokens| Theme::custom(&name, &tokens))
        {
            Some(Ok(theme)) => row(&mut lines, &name, format!("{name} (custom)"), theme),
            _ => lines.push(Line::styled(
                format!("  {name} (has a mistake, see below)"),
                look.warn(),
            )),
        }
    }
    lines.push(Line::default());
    lines.push(Line::styled("Make your own in config.toml:", look.muted()));
    for text in ["[themes.mine]", "base = \"nord\"", "accent = \"#ff8800\""] {
        lines.push(Line::styled(format!("  {text}"), look.dim()));
    }
    lines
}

pub(super) fn detail(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let item = ITEMS[app.selected().min(ITEMS.len() - 1)];
    let block = sym.block(look, item.label, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let tab = app.settings();
    if !tab.has_data() {
        f.render_widget(
            Paragraph::new(Span::styled("Waiting for the daemon...", look.dim())),
            inner,
        );
        return;
    }
    let mut lines = vec![Line::styled(item.help, look.text())];
    match item.kind {
        Kind::Theme => lines.extend(theme_lines(app, look, sym)),
        Kind::Switch => {
            lines.push(Line::default());
            let now = if tab.is_on(&item) { "on" } else { "off" };
            lines.push(Line::from(vec![
                Span::styled(format!("{:<12}", "Now"), look.muted()),
                Span::styled(now, look.text().bold()),
            ]));
            lines.push(Line::from(vec![
                Span::styled(format!("{:<12}", "Default"), look.muted()),
                Span::styled(if item.default { "on" } else { "off" }, look.text()),
            ]));
            if item.key == "connectivity.enabled" {
                lines.push(Line::default());
                let uri = tab.connectivity_uri().unwrap_or("not reported");
                lines.push(Line::from(vec![
                    Span::styled(format!("{:<12}", "Checks"), look.muted()),
                    Span::styled(super::surface::printable(uri), look.text()),
                ]));
                lines.push(Line::styled(
                    crate::text::tui::CONNECTIVITY_URI_HINT,
                    look.dim(),
                ));
            }
            if let Some(why) = app.setting_forced(&item) {
                lines.push(Line::default());
                lines.push(Line::styled(why, look.warn()));
            }
        }
    }
    lines.push(Line::default());
    lines.push(Line::styled(
        "enter changes it, r puts the default back.",
        look.dim(),
    ));
    if item.kind == Kind::Theme {
        for name in tab.custom_names() {
            if let Some(problem) = tab.custom_problem(&name) {
                lines.push(Line::default());
                lines.push(Line::styled(problem, look.warn()));
            }
        }
    }
    if let Some(note) = app.theme_note() {
        lines.push(Line::default());
        lines.push(Line::styled(note.to_owned(), look.warn()));
    }
    if let Some(w) = tab.warning() {
        lines.push(Line::default());
        lines.push(Line::styled(w.to_owned(), look.err()));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
