//! What a mouse click can land on. Drawing records one [`Hit`] per clickable
//! rectangle each frame (`App::hit`), and the click handler looks the cell up
//! in the last frame's list, so the mouse always agrees with what is on screen.

use crossterm::event::{KeyCode, KeyModifiers};

use super::app::Tab;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// A tab label in the header (Standard and Wide).
    Tab(Tab),
    /// The tab name in the Narrow header: steps to the next tab.
    NextTab,
    /// A row of the current tab's master list.
    Row(usize),
    /// A footer hint: a button that presses the key it names.
    Key(KeyCode, KeyModifiers),
    /// A match in the command palette.
    Palette(usize),
    /// The palette's frame (swallows clicks that land inside it).
    PaletteBox,
    /// The help overlay's frame.
    HelpBox,
}

/// The key a footer hint names, when pressing it makes sense. `None` for
/// hints that describe a group ("j/k") or typing.
pub fn hint_key(key: &str) -> Option<(KeyCode, KeyModifiers)> {
    let none = KeyModifiers::NONE;
    Some(match key {
        "enter" => (KeyCode::Enter, none),
        "esc" => (KeyCode::Esc, none),
        "tab" => (KeyCode::Tab, none),
        "space" => (KeyCode::Char(' '), none),
        "ctrl-r" => (KeyCode::Char('r'), KeyModifiers::CONTROL),
        k => {
            let mut chars = k.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => (KeyCode::Char(c), none),
                _ => return None,
            }
        }
    })
}
