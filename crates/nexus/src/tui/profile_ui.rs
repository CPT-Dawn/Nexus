//! Drawing the Profile settings editor: the form, its inline complaints and
//! the "Keep these settings?" countdown. Pure like the other views.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use super::app::App;
use super::caps::Glyphs;
use super::profile::{Editor, Phase};
use super::theme::Look;
use super::view::Sym;

/// Widest the form gets; narrower terminals shrink it to fit.
const WIDTH: u16 = 64;
/// Width of the label column.
const LABEL: usize = 13;

pub(super) fn overlay(f: &mut Frame, app: &App, body: Rect, look: &Look, sym: &Sym) {
    let Some(editor) = app.editor() else { return };
    let width = WIDTH.min(body.width);
    let wanted = match editor.phase {
        Phase::Editing => editor.rows().len() as u16 + 5,
        Phase::Pending { .. } => 8,
        _ => 5,
    };
    let height = wanted.min(body.height);
    if width < 24 || height < 5 {
        return;
    }
    let rect = Rect {
        x: body.x + (body.width - width) / 2,
        y: body.y + (body.height - height) / 2,
        width,
        height,
    };
    f.render_widget(Clear, rect);
    let title = format!("{} settings", editor.ssid);
    let block = sym.block(look, &title, true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let lines = match &editor.phase {
        Phase::Loading => vec![Line::styled("Reading the Profile...", look.muted())],
        Phase::Applying => vec![Line::styled("Applying...", look.muted())],
        Phase::Pending {
            secs_left,
            answered,
            ..
        } => pending(editor, *secs_left, *answered, look),
        Phase::Editing => form(editor, inner, look, sym),
    };
    f.render_widget(Paragraph::new(lines), inner);
}

fn pending(editor: &Editor, secs_left: u32, answered: bool, look: &Look) -> Vec<Line<'static>> {
    let s = editor
        .build()
        .ok()
        .map(|s| {
            let dns = if s.dns.is_empty() {
                "DNS from the Network".to_owned()
            } else {
                format!("DNS {}", s.dns.join(", "))
            };
            let ip = match s.ipv4.method {
                nexus_ipc::Ipv4Method::Manual => format!(
                    "IPv4 {}",
                    nexus_ipc::profile::addresses_text(&s.ipv4.addresses)
                ),
                _ => "IPv4 automatic".to_owned(),
            };
            format!("{ip}, {dns}")
        })
        .unwrap_or_default();
    let ask = if answered {
        Line::styled("Waiting for the daemon...", look.muted())
    } else {
        Line::styled("y keep   n revert now", look.dim())
    };
    vec![
        Line::styled("Keep these settings?", look.text().bold()),
        Line::styled(format!("Reverting in {secs_left}s"), look.warn()),
        Line::default(),
        Line::styled(s, look.muted()),
        Line::default(),
        ask,
    ]
}

fn form(editor: &Editor, inner: Rect, look: &Look, sym: &Sym) -> Vec<Line<'static>> {
    let rows = editor.rows();
    let focus = editor.focus_index();
    // Two lines are kept for the complaint and the keys; the rest holds rows,
    // scrolled to keep the focused one in view.
    let room = (inner.height as usize).saturating_sub(3).max(1);
    let width = inner.width as usize;
    let start = if rows.len() <= room {
        0
    } else {
        focus.saturating_sub(room - 1).min(rows.len() - room)
    };
    let (open, close) = match sym.glyphs {
        Glyphs::Ascii => ("< ", " >"),
        _ => ("‹ ", " ›"),
    };
    let mut lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(room)
        .map(|(i, row)| {
            let here = i == focus;
            let pointer = if here { sym.pointer() } else { "  " };
            let wrong = editor.is_wrong(*row);
            let label_style = if wrong {
                look.err()
            } else if here {
                look.accent()
            } else {
                look.muted()
            };
            let mut spans = vec![
                Span::styled(pointer, look.accent()),
                Span::styled(format!("{:<LABEL$}", row.label()), label_style),
            ];
            if row.is_text() {
                let text = editor.text_of(*row).to_owned();
                let value_style = if wrong { look.err() } else { look.text() };
                spans.push(Span::styled(text, value_style));
                if here {
                    spans.push(Span::styled("_", look.accent()));
                }
            } else {
                let value = editor.choice_of(*row);
                if here {
                    spans.push(Span::styled(open, look.accent()));
                    spans.push(Span::styled(value, look.text()));
                    spans.push(Span::styled(close, look.accent()));
                } else {
                    spans.push(Span::styled(value, look.text()));
                }
            }
            Line::from(spans)
        })
        .collect();
    let (text, style) = match (editor.complaint(), &editor.message) {
        (Some(why), _) => (why, look.err()),
        (None, Some(message)) => (message.clone(), look.err()),
        (None, None) if editor.active() => (
            "In use: you will be asked to keep the change.".to_owned(),
            look.muted(),
        ),
        (None, None) => (String::new(), look.muted()),
    };
    // Exactly two lines, so the keys below never move.
    let mut said = wrap2(&text, width.saturating_sub(1));
    said.resize(2, String::new());
    lines.extend(
        said.into_iter()
            .map(|l| Line::styled(format!(" {l}"), style)),
    );
    lines.push(Line::styled(
        "tab next  space change  enter apply  esc cancel",
        look.dim(),
    ));
    lines
}

/// `text` in at most two lines of `width` cells, broken at spaces; what does
/// not fit ends in an ellipsis.
fn wrap2(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.len() > 2 {
        lines.truncate(2);
        let last = &mut lines[1];
        while last.chars().count() + 3 > width {
            last.pop();
        }
        last.push_str("...");
    }
    lines
}
