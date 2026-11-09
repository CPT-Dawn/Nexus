//! Mouse input. Clicks are resolved against the rectangles the last frame
//! recorded (`hits.rs`); the wheel moves whatever list has the focus.

use crossterm::event::{KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use super::{App, Layer, Tab};
use crate::tui::hits::Hit;
use crate::tui::overlays;

impl App {
    /// Forgets last frame's rectangles; the next `draw` records new ones.
    pub(in crate::tui) fn hit_reset(&self) {
        self.hits.borrow_mut().clear();
    }

    /// Records that a click inside `rect` means `hit`. Later calls sit on top.
    pub(in crate::tui) fn hit(&self, rect: Rect, hit: Hit) {
        if rect.width > 0 && rect.height > 0 {
            self.hits.borrow_mut().push((rect, hit));
        }
    }

    fn hit_at(&self, x: u16, y: u16) -> Option<Hit> {
        let at = Position { x, y };
        self.hits
            .borrow()
            .iter()
            .rev()
            .find(|(r, _)| r.contains(at))
            .map(|(_, h)| *h)
    }

    pub(super) fn mouse(&mut self, ev: MouseEvent) -> bool {
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => self.click(ev.column, ev.row),
            MouseEventKind::ScrollUp => self.wheel(false),
            MouseEventKind::ScrollDown => self.wheel(true),
            _ => false,
        }
    }

    /// Whether a modal view (not the palette or help) has the screen.
    fn modal(&self) -> bool {
        self.field.is_some()
            || self.confirm.is_some()
            || self.editor.is_some()
            || self.share.is_some()
            || self.layer == Layer::Confirm
    }

    fn click(&mut self, x: u16, y: u16) -> bool {
        let hit = self.hit_at(x, y);
        // A footer hint is a button in every state: it presses its key.
        if let Some(Hit::Key(code, mods)) = hit {
            return self.key(KeyEvent::new(code, mods));
        }
        if self.palette.is_some() {
            return match hit {
                Some(Hit::Palette(i)) => {
                    if let Some(p) = self.palette.as_mut() {
                        p.selected = i;
                    }
                    self.run_selected();
                    true
                }
                Some(Hit::PaletteBox) => false,
                _ => {
                    self.palette = None; // a click outside dismisses it
                    true
                }
            };
        }
        if self.modal() {
            return false;
        }
        if self.layer == Layer::Help {
            return match hit {
                Some(Hit::HelpBox) => false,
                _ => {
                    self.layer = Layer::Root;
                    true
                }
            };
        }
        match hit {
            Some(Hit::Tab(tab)) => {
                self.layer = Layer::Root;
                self.go_tab(tab)
            }
            Some(Hit::NextTab) => {
                self.layer = Layer::Root;
                self.step_tab(1)
            }
            Some(Hit::Row(row)) => self.click_row(row),
            _ => false,
        }
    }

    /// A click on a list row: the Overview's cards open their tab (they are
    /// buttons); elsewhere the first click selects and a click on the
    /// selected row does what Enter does.
    fn click_row(&mut self, row: usize) -> bool {
        if self.layer != Layer::Root {
            return false;
        }
        if self.tab == Tab::Overview {
            self.select(row);
            return self.go_tab(Tab::ALL[1 + row.min(3)]);
        }
        if row == self.selected() {
            return self.key(KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                KeyModifiers::NONE,
            ));
        }
        self.select(row)
    }

    /// One wheel notch: moves the palette's, the help's or the list's cursor.
    fn wheel(&mut self, down: bool) -> bool {
        if let Some(p) = &self.palette {
            let last = self.matches().len().saturating_sub(1);
            let to = if down {
                (p.selected + 1).min(last)
            } else {
                p.selected.saturating_sub(1)
            };
            if let Some(p) = self.palette.as_mut() {
                let changed = p.selected != to;
                p.selected = to;
                return changed;
            }
        }
        if self.modal() {
            return false;
        }
        if self.layer == Layer::Help {
            return self.scroll_help(if down { 1 } else { -1 });
        }
        let last = self.rows().saturating_sub(1);
        let to = if down {
            (self.selected() + 1).min(last)
        } else {
            self.selected().saturating_sub(1)
        };
        self.select(to)
    }

    /// Scrolls the help overlay by `by` rows, clamped to its content.
    pub(super) fn scroll_help(&mut self, by: isize) -> bool {
        let max = overlays::help_max_scroll(self.size);
        let to = self.help_scroll.saturating_add_signed(by).min(max);
        let changed = to != self.help_scroll;
        self.help_scroll = to;
        changed
    }
}
