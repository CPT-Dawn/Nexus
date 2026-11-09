//! The one layout engine. Every screen asks it for areas; no view looks at
//! the terminal width itself.
//!
//! Breakpoints (columns): below 40 wide or 10 tall, a hint; below 80, one
//! list with the detail in a slide-in sheet; 80 and up, tabs with
//! master/detail; 120 and up, the master column and a second detail block
//! grow.

use ratatui::layout::{Constraint, Layout as Split, Rect};

pub const MIN_WIDTH: u16 = 40;
pub const MIN_HEIGHT: u16 = 10;
pub const SPLIT_AT: u16 = 80;
pub const WIDE_AT: u16 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Too small to draw anything sensible.
    TooSmall,
    /// One list; Enter slides the detail in as a sheet.
    Narrow,
    /// Tabs, master and detail side by side.
    Standard,
    /// Standard with room for extra detail.
    Wide,
}

impl Mode {
    pub fn of(area: Rect) -> Mode {
        if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
            Mode::TooSmall
        } else if area.width < SPLIT_AT {
            Mode::Narrow
        } else if area.width < WIDE_AT {
            Mode::Standard
        } else {
            Mode::Wide
        }
    }

    /// Whether the master and the detail share the screen.
    pub fn split(self) -> bool {
        matches!(self, Mode::Standard | Mode::Wide)
    }
}

/// Where everything goes in one frame.
#[derive(Debug, Clone, Copy)]
pub struct Areas {
    pub mode: Mode,
    pub header: Rect,
    pub footer: Rect,
    /// The list (the whole body when narrow).
    pub master: Rect,
    /// The detail pane; `None` when narrow (it is a sheet there).
    pub detail: Option<Rect>,
    /// Where the slide-in sheet goes when narrow.
    pub sheet: Rect,
}

pub fn areas(area: Rect) -> Areas {
    let mode = Mode::of(area);
    let [header, body, footer] = Split::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);

    let (master, detail) = match mode {
        Mode::Standard | Mode::Wide => {
            let side = if mode == Mode::Wide { 40 } else { 30 };
            let [m, d] =
                Split::horizontal([Constraint::Length(side), Constraint::Fill(1)]).areas(body);
            (m, Some(d))
        }
        _ => (body, None),
    };
    let sheet = Rect {
        x: body.x + 4,
        width: body.width.saturating_sub(4),
        ..body
    };
    Areas {
        mode,
        header,
        footer,
        master,
        detail,
        sheet,
    }
}
