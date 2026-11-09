//! Drawing the 802.1X form over the tab. Pure: `EapForm` in, cells out.

use nexus_ipc::EapMethod;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::caps::Glyphs;
use super::eap_form::{EapForm, Row};
use super::theme::Look;
use super::view::Sym;

const LABEL: usize = 15;

pub(super) fn draw(f: &mut Frame, form: &EapForm, body: Rect, look: &Look, sym: &Sym) {
    let width = body.width.min(74);
    let height = body.height.min(26);
    let rect = Rect {
        x: body.x + (body.width - width) / 2,
        y: body.y + (body.height - height) / 2,
        width,
        height,
    };
    f.render_widget(Clear, rect);
    let title = format!("Sign in to {}", form.name);
    let block = sym.block(look, &title, true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    if inner.height < 3 {
        return;
    }
    let ascii = sym.glyphs == Glyphs::Ascii;
    let mask = if ascii { '*' } else { '•' };
    let value_room = (inner.width as usize).saturating_sub(LABEL + 3);

    let mut lines: Vec<Line> = Vec::new();
    let mut focus_line = 0;
    if let Some(banner) = &form.banner {
        lines.push(Line::styled(banner.clone(), look.err()));
        lines.push(Line::default());
    }
    let focused = form.focused();
    for row in form.rows() {
        let here = row == focused;
        if here {
            focus_line = lines.len();
        }
        let pointer = if here { sym.pointer() } else { "  " };
        let label_style = if here { look.accent() } else { look.muted() };
        let (open, close) = if ascii { ("< ", " >") } else { ("‹ ", " ›") };
        let value: Vec<Span> = match row {
            Row::Method => vec![Span::styled(
                format!("{open}{}{close}", form.method().label()),
                look.text(),
            )],
            Row::Phase2 => vec![Span::styled(
                format!(
                    "{open}{}{close}",
                    form.phase2().map_or("-", |p| p.label())
                ),
                look.text(),
            )],
            Row::SkipCheck => {
                let (mark, text) = if form.skips_check() {
                    ("[x] ", "don't check the server")
                } else {
                    ("[ ] ", "check the server (recommended)")
                };
                let style = if form.skips_check() { look.warn() } else { look.text() };
                vec![Span::styled(format!("{mark}{text}"), style)]
            }
            _ => {
                let text = form.text(row);
                let shown: String = if row.secret() && !form.reveal {
                    text.chars().map(|_| mask).collect()
                } else {
                    text.to_owned()
                };
                // Keep the end of a long value in view.
                let skip = shown.chars().count().saturating_sub(value_room.saturating_sub(1));
                let shown: String = shown.chars().skip(skip).collect();
                let mut spans = Vec::new();
                if shown.is_empty() && !here {
                    spans.push(Span::styled(row.hint(), look.dim()));
                } else {
                    spans.push(Span::styled(shown, look.text()));
                }
                if here {
                    spans.push(Span::styled("_", look.accent()));
                }
                spans
            }
        };
        let mut spans = vec![
            Span::styled(pointer, look.accent()),
            Span::styled(format!("{:<LABEL$}", row.label()), label_style),
        ];
        spans.extend(value);
        lines.push(Line::from(spans));
        if let Some(error) = form.error(row) {
            lines.push(Line::styled(format!("  {error}"), look.err()));
        }
        if row == Row::SkipCheck && form.skips_check() {
            lines.push(Line::styled(
                "  Anyone nearby can pose as this network and read your password.",
                look.warn(),
            ));
        }
        if row == Row::Domain && form.method() != EapMethod::Tls && here {
            lines.push(Line::styled(
                format!("  {:LABEL$}Ask your IT desk; it stops fake networks.", ""),
                look.dim(),
            ));
        }
    }

    // The keys stay put; the form scrolls so the focused row is in view.
    let room = inner.height as usize - 1;
    let offset = (focus_line + 1).saturating_sub(room);
    let visible: Vec<Line> = lines.into_iter().skip(offset).take(room).collect();
    let (list, keys) = inner.split_at_row(inner.height - 1);
    f.render_widget(Paragraph::new(visible).wrap(Wrap { trim: false }), list);
    f.render_widget(
        Paragraph::new(Line::styled(
            if ascii {
                "tab next  </> change  ^r show  enter join  esc cancel"
            } else {
                "tab next  ←→ change  ^r show  enter join  esc cancel"
            },
            look.dim(),
        )),
        keys,
    );
}

trait SplitRow {
    fn split_at_row(self, row: u16) -> (Rect, Rect);
}

impl SplitRow for Rect {
    fn split_at_row(self, row: u16) -> (Rect, Rect) {
        (
            Rect { height: row, ..self },
            Rect {
                y: self.y + row,
                height: self.height - row,
                ..self
            },
        )
    }
}
