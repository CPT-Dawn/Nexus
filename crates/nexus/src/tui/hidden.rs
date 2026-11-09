//! The "other network" form: join a hidden Network, one that does not
//! broadcast its name, by typing the name and picking how it is protected.
//! State and drawing in one place; no I/O. `App` feeds it keys and sends the
//! result as a [`super::wifi::Cmd::JoinHidden`].

use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_ipc::{Secret, Security};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use zeroize::Zeroizing;

use super::caps::Glyphs;
use super::theme::Look;
use super::view::Sym;

/// What a hidden Network can be protected with, as the form offers them.
pub const CHOICES: [Security; 5] = [
    Security::Wpa2,
    Security::Wpa3,
    Security::Wpa3Transition,
    Security::Owe,
    Security::Open,
];

/// An SSID is at most 32 bytes.
const MAX_NAME: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Name,
    Security,
    Password,
}

impl Row {
    fn label(self) -> &'static str {
        match self {
            Row::Name => "Network name",
            Row::Security => "Security",
            Row::Password => "Password",
        }
    }
}

/// What a key did to the form.
#[derive(Debug)]
pub enum Typed {
    Editing,
    Cancel,
    /// Join: the name, how it is protected and the password if one was typed
    /// (without one, the Daemon asks for it).
    Submit {
        name: String,
        security: Security,
        password: Option<Secret>,
    },
}

pub struct HiddenForm {
    name: String,
    choice: usize,
    password: Zeroizing<String>,
    focus: Row,
    pub reveal: bool,
    /// Why the last attempt failed, in the Daemon's words.
    pub banner: Option<String>,
    error: Option<&'static str>,
}

impl fmt::Debug for HiddenForm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HiddenForm")
            .field("name", &self.name)
            .field("security", &CHOICES[self.choice])
            .field("password", &"<redacted>")
            .finish()
    }
}

impl HiddenForm {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            choice: 0,
            password: Zeroizing::new(String::new()),
            focus: Row::Name,
            reveal: false,
            banner: None,
            error: None,
        }
    }

    /// The form again after a join failed, with what was typed and why.
    pub fn reopened(
        name: String,
        security: Security,
        password: Option<&Secret>,
        banner: String,
    ) -> Self {
        let mut form = Self::new();
        form.name = name;
        form.choice = CHOICES.iter().position(|s| *s == security).unwrap_or(0);
        if let Some(p) = password {
            form.password = Zeroizing::new(p.expose().to_owned());
        }
        form.banner = Some(banner);
        form
    }

    pub fn security(&self) -> Security {
        CHOICES[self.choice]
    }

    fn secured(&self) -> bool {
        !matches!(self.security(), Security::Open | Security::Owe)
    }

    /// The rows now shown: no password row for Networks that have none.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Name, Row::Security];
        if self.secured() {
            rows.push(Row::Password);
        }
        rows
    }

    pub fn focused(&self) -> Row {
        self.focus
    }

    fn go(&mut self, by: isize) {
        let rows = self.rows();
        let at = rows.iter().position(|r| *r == self.focus).unwrap_or(0) as isize;
        let next = (at + by).rem_euclid(rows.len() as isize) as usize;
        self.focus = rows[next];
    }

    fn cycle(&mut self, forward: bool) {
        if self.focus != Row::Security {
            return;
        }
        let n = CHOICES.len();
        self.choice = if forward {
            (self.choice + 1) % n
        } else {
            (self.choice + n - 1) % n
        };
        self.error = None;
    }

    fn check(&mut self) -> bool {
        self.error = if self.name.is_empty() {
            Some("Type the network's name.")
        } else if self.name.len() > MAX_NAME {
            Some("A network name is at most 32 bytes.")
        } else if self.secured()
            && self.security() != Security::Wpa3
            && !self.password.is_empty()
            && !(8..=63).contains(&self.password.chars().count())
        {
            Some("A WPA2 password is 8 to 63 characters.")
        } else {
            None
        };
        self.error.is_none()
    }

    pub fn error(&self, row: Row) -> Option<&'static str> {
        match (row, self.error) {
            (Row::Name, Some(e)) if e.contains("name") => Some(e),
            (Row::Password, Some(e)) if e.contains("password") => Some(e),
            _ => None,
        }
    }

    pub fn text(&self, row: Row) -> &str {
        match row {
            Row::Name => &self.name,
            Row::Password => &self.password,
            Row::Security => "",
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Typed {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return Typed::Cancel,
            KeyCode::Enter if self.check() => {
                let password = (self.secured() && !self.password.is_empty())
                    .then(|| Secret::new(self.password.as_str()));
                return Typed::Submit {
                    name: self.name.clone(),
                    security: self.security(),
                    password,
                };
            }
            KeyCode::Tab | KeyCode::Down => self.go(1),
            KeyCode::BackTab | KeyCode::Up => self.go(-1),
            KeyCode::Left => self.cycle(false),
            KeyCode::Right => self.cycle(true),
            KeyCode::Char('r') if ctrl => self.reveal = !self.reveal,
            KeyCode::Char('u') if ctrl => match self.focus {
                Row::Name => self.name.clear(),
                Row::Password => self.password.clear(),
                Row::Security => {}
            },
            KeyCode::Char(' ') if self.focus == Row::Security => self.cycle(true),
            KeyCode::Backspace => match self.focus {
                Row::Name => {
                    self.name.pop();
                }
                Row::Password => {
                    self.password.pop();
                }
                Row::Security => {}
            },
            KeyCode::Char(c) if !ctrl => match self.focus {
                Row::Name => {
                    self.name.push(c);
                    self.error = None;
                }
                Row::Password => {
                    self.password.push(c);
                    self.error = None;
                }
                Row::Security => {}
            },
            _ => {}
        }
        Typed::Editing
    }
}

const LABEL: usize = 14;

fn choice_label(s: Security) -> &'static str {
    match s {
        Security::Wpa2 => "WPA2",
        Security::Wpa3 => "WPA3",
        Security::Wpa3Transition => "WPA2/WPA3",
        Security::Owe => "Enhanced Open (OWE)",
        _ => "Open",
    }
}

pub(super) fn draw(f: &mut Frame, form: &HiddenForm, body: Rect, look: &Look, sym: &Sym) {
    let width = body.width.min(66);
    let height = body.height.min(12);
    let rect = Rect {
        x: body.x + (body.width - width) / 2,
        y: body.y + (body.height - height) / 2,
        width,
        height,
    };
    f.render_widget(Clear, rect);
    let block = sym.block(look, "Join a hidden network", true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    if inner.height < 3 {
        return;
    }
    let ascii = sym.glyphs == Glyphs::Ascii;
    let mask = if ascii { '*' } else { '•' };
    let room = (inner.width as usize).saturating_sub(LABEL + 3);
    let mut lines: Vec<Line> = Vec::new();
    if let Some(banner) = &form.banner {
        lines.push(Line::styled(banner.clone(), look.err()));
        lines.push(Line::default());
    }
    let focused = form.focused();
    for row in form.rows() {
        let here = row == focused;
        let pointer = if here { sym.pointer() } else { "  " };
        let label_style = if here { look.accent() } else { look.muted() };
        let value: Vec<Span> = match row {
            Row::Security => {
                let (open, close) = if ascii {
                    ("< ", " >")
                } else {
                    ("‹ ", " ›")
                };
                vec![Span::styled(
                    format!("{open}{}{close}", choice_label(form.security())),
                    look.text(),
                )]
            }
            _ => {
                let text = form.text(row);
                let shown: String = if row == Row::Password && !form.reveal {
                    text.chars().map(|_| mask).collect()
                } else {
                    text.to_owned()
                };
                let skip = shown.chars().count().saturating_sub(room.saturating_sub(1));
                let shown: String = shown.chars().skip(skip).collect();
                let mut spans = Vec::new();
                if shown.is_empty() && !here {
                    let hint = if row == Row::Password {
                        "empty: you will be asked"
                    } else {
                        "as the router spells it"
                    };
                    spans.push(Span::styled(hint, look.dim()));
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
    }
    let (list, keys) = (
        Rect {
            height: inner.height - 1,
            ..inner
        },
        Rect {
            y: inner.bottom() - 1,
            height: 1,
            ..inner
        },
    );
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), list);
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
