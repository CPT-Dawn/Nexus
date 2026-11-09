//! The two overlays generated from the Verb table: the `?` help (scrolls, so
//! it never clips however many keys later tickets add) and the `:` palette.

use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};

use super::app::App;
use super::caps::Glyphs;
use super::hits::Hit;
use super::theme::Look;
use super::verbs;
use super::view::Sym;

const HELP_WIDTH: u16 = 62;
const KEY_COL: usize = 21;
const PALETTE_WIDTH: u16 = 64;
const PALETTE_MAX_ROWS: u16 = 10;

/// Browsers with secure DNS (DNS over HTTPS) skip the network's own resolver,
/// which is how a Portal steers you to its page.
const PORTAL_NOTE: &str = "Login page won't show? A browser with secure DNS (DNS over HTTPS) hides the Portal by skipping the network's resolver. Turn it off until you have signed in, or press o: Nexus opens a plain http:// page.";

// ---- help -----------------------------------------------------------------------

enum HelpLine {
    Row(String, &'static str),
    Blank,
    Note(String),
}

/// Greedy word wrap to `width` cells.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn help_lines(inner_width: usize) -> Vec<HelpLine> {
    let mut lines: Vec<HelpLine> = verbs::help_rows()
        .into_iter()
        .map(|(k, w)| HelpLine::Row(k, w))
        .collect();
    lines.push(HelpLine::Blank);
    lines.extend(
        wrap(PORTAL_NOTE, inner_width.saturating_sub(2))
            .into_iter()
            .map(HelpLine::Note),
    );
    lines
}

fn help_inner_width(area: Rect) -> usize {
    (HELP_WIDTH.min(area.width) as usize).saturating_sub(4)
}

/// The help box: centred, as tall as its content, never taller than the frame.
fn help_box(area: Rect, total: usize) -> Rect {
    let width = HELP_WIDTH.min(area.width);
    let height = (total as u16).saturating_add(2).min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// The room the help has: between the header and the footer, so the footer's
/// buttons stay visible and clickable.
fn help_area(size: (u16, u16)) -> Rect {
    Rect::new(0, 1, size.0, size.1.saturating_sub(2))
}

/// How far the help can scroll in a window of `size` (0 when it all fits).
pub fn help_max_scroll(size: (u16, u16)) -> usize {
    let area = help_area(size);
    let total = help_lines(help_inner_width(area)).len();
    let visible = help_box(area, total).height.saturating_sub(2) as usize;
    total.saturating_sub(visible)
}

/// Rows the help shows at once in a window of `size` (a page).
pub fn help_page(size: (u16, u16)) -> usize {
    let area = help_area(size);
    let total = help_lines(help_inner_width(area)).len();
    (help_box(area, total).height.saturating_sub(2) as usize).max(1)
}

pub fn help(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let lines = help_lines(help_inner_width(area));
    let rect = help_box(area, lines.len());
    app.hit(rect, Hit::HelpBox);
    f.render_widget(Clear, rect);
    let visible = rect.height.saturating_sub(2) as usize;
    let scrolls = lines.len() > visible;
    let block = sym.block(
        look,
        if scrolls {
            "All keys (j/k scroll)"
        } else {
            "All keys"
        },
        true,
    );
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let from = app.help_scroll().min(lines.len().saturating_sub(visible));
    let shown: Vec<Line> = lines
        .iter()
        .skip(from)
        .take(visible)
        .map(|line| match line {
            HelpLine::Row(k, what) => Line::from(vec![
                Span::styled(format!(" {k:<KEY_COL$}"), look.accent()),
                Span::styled(*what, look.text()),
            ]),
            HelpLine::Blank => Line::default(),
            HelpLine::Note(text) => Line::styled(format!(" {text}"), look.muted()),
        })
        .collect();
    f.render_widget(Paragraph::new(shown), inner);
    if scrolls {
        let (track, thumb) = if sym.glyphs == Glyphs::Ascii {
            ("|", "#")
        } else {
            ("│", "█")
        };
        let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some(track))
            .thumb_symbol(thumb)
            .track_style(look.border_focus())
            .thumb_style(look.accent());
        let mut state = ScrollbarState::new(lines.len())
            .viewport_content_length(visible)
            .position(from);
        f.render_stateful_widget(
            bar,
            rect.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut state,
        );
    }
}

// ---- palette --------------------------------------------------------------------

/// First visible match so that `selected` is in view in `rows` rows.
pub fn palette_offset(selected: usize, rows: usize) -> usize {
    (selected + 1).saturating_sub(rows.max(1))
}

pub fn palette(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let Some(p) = app.palette() else {
        return;
    };
    let found = app.matches();
    let width = PALETTE_WIDTH.min(area.width.saturating_sub(2));
    // Borders, the input line, a rule and a status line frame the matches.
    let fixed = 5;
    let rows = (found.len().max(1) as u16)
        .min(PALETTE_MAX_ROWS)
        .min(area.height.saturating_sub(fixed + 2));
    let height = rows + fixed;
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + 1.min(area.height.saturating_sub(height)),
        width,
        height,
    };
    app.hit(rect, Hit::PaletteBox);
    f.render_widget(Clear, rect);
    let block = sym.block(look, "Command palette", true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let cursor = if sym.glyphs == Glyphs::Ascii {
        "_"
    } else {
        "▏"
    };
    let input = Line::from(vec![
        Span::styled(": ", look.accent()),
        Span::styled(p.query.clone(), look.text()),
        Span::styled(cursor, look.accent()),
    ]);
    let line = |n: u16| Rect {
        y: inner.y + n,
        height: 1,
        ..inner
    };
    f.render_widget(Paragraph::new(input), line(0));
    let rule = if sym.glyphs == Glyphs::Ascii {
        "-"
    } else {
        "─"
    };
    f.render_widget(
        Paragraph::new(Span::styled(
            rule.repeat(inner.width as usize),
            look.border(),
        )),
        line(1),
    );
    if found.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("No action matches that.", look.dim())),
            line(2),
        );
        return;
    }
    let visible = rows as usize;
    let from = palette_offset(p.selected, visible);
    let key_col = 8;
    let verb_col =
        (inner.width as usize).saturating_sub(sym.pointer().chars().count() + key_col + 1);
    for (i, found) in found.iter().enumerate().skip(from).take(visible) {
        let slot = i - from;
        let selected = i == p.selected;
        let (text, key) = if found.blocked.is_some() {
            (look.dim(), look.dim())
        } else {
            (look.text(), look.muted())
        };
        let pointer = if selected { sym.pointer() } else { "  " };
        let mut spans = vec![
            Span::styled(pointer, look.accent()),
            Span::styled(format!("{:<verb_col$}", found.verb.verb), text),
            Span::styled(format!(" {:>key_col$}", found.verb.key), key),
        ];
        if selected {
            spans = spans
                .into_iter()
                .map(|s| Span::styled(s.content, look.selected()))
                .collect();
        }
        let row = line(2 + slot as u16);
        f.render_widget(Paragraph::new(Line::from(spans)), row);
        app.hit(row, Hit::Palette(i));
    }
    // What the highlighted action does, or why it cannot run.
    let status = match found.get(p.selected) {
        Some(x) => match x.blocked {
            Some(why) => Span::styled(format!("  {why}"), look.warn()),
            None => Span::styled(format!("  {}", x.verb.help), look.muted()),
        },
        None => Span::raw(""),
    };
    f.render_widget(Paragraph::new(status), line(2 + rows));
}
