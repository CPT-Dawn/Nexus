//! Sharing a saved Network as a QR code. Nothing secret is read until the
//! person presses the reveal key (Enter on the question); closing the view
//! drops the details, which wipe themselves from memory.

use nexus_ipc::WifiShare;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::caps::{ColorTier, Glyphs};
use super::theme::Look;
use super::view::Sym;
use crate::qr::{self, Drawing};

/// Where the share view is.
#[derive(Debug)]
pub enum Stage {
    /// Asking whether to show the password; nothing has been read.
    Ask,
    /// The Daemon is reading the stored password (NetworkManager may be
    /// waiting for the person to authorize it).
    Reading,
    /// Revealed.
    Shown {
        share: WifiShare,
        /// `None` when the details are too long for a QR code.
        drawing: Option<Drawing>,
        /// The password's letters are on screen, not dots.
        letters: bool,
    },
    /// Refused or failed, in words.
    Failed(String),
}

#[derive(Debug)]
pub struct Share {
    pub network: String,
    pub ssid: String,
    pub stage: Stage,
}

impl Share {
    pub fn ask(network: String, ssid: String) -> Share {
        Share {
            network,
            ssid,
            stage: Stage::Ask,
        }
    }

    /// The answer of the Daemon to the reveal.
    pub fn arrived(&mut self, result: Result<WifiShare, String>) {
        self.stage = match result {
            Ok(share) => Stage::Shown {
                drawing: qr::draw(&share.payload()),
                share,
                letters: false,
            },
            Err(message) => Stage::Failed(message),
        };
    }

    /// The keys that work right now, for the footer.
    pub fn hints(&self) -> Vec<(&'static str, &'static str)> {
        match &self.stage {
            Stage::Ask => vec![("enter", "reveal"), ("esc", "cancel")],
            Stage::Reading => vec![("esc", "cancel")],
            Stage::Shown { share, .. } if share.password.is_some() => {
                vec![("p", "show password"), ("esc", "close")]
            }
            Stage::Shown { .. } => vec![("esc", "close")],
            Stage::Failed(_) => vec![("enter", "try again"), ("esc", "close")],
        }
    }
}

fn dots(n: usize, sym: &Sym) -> String {
    let dot = if sym.glyphs == Glyphs::Ascii {
        '*'
    } else {
        '\u{2022}'
    };
    std::iter::repeat_n(dot, n).collect()
}

/// QR cells are black on white so a phone reads them on any theme.
pub(super) fn code_style(look: &Look) -> Style {
    if look.caps.color == ColorTier::None {
        Style::new()
    } else {
        Style::new().fg(Color::Black).bg(Color::White)
    }
}

pub(super) fn draw(f: &mut Frame, share: &Share, body: Rect, look: &Look, sym: &Sym) {
    let title = format!("Share {}", share.ssid);
    let wrap = |lines: Vec<Line<'static>>| Paragraph::new(lines).wrap(Wrap { trim: false });
    match &share.stage {
        Stage::Ask | Stage::Reading | Stage::Failed(_) => {
            let rect = centered(body, 62, 7);
            f.render_widget(Clear, rect);
            let block = sym.block(look, &title, true);
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            let lines = match &share.stage {
                Stage::Ask => vec![
                    Line::styled("Show the password and a QR code for a phone?", look.text()),
                    Line::styled(
                        "Anyone who can see this screen can read them.",
                        look.muted(),
                    ),
                    Line::default(),
                    Line::styled("enter reveal   esc cancel", look.dim()),
                ],
                Stage::Reading => vec![
                    Line::styled("Asking NetworkManager for the password...", look.text()),
                    Line::styled("It may ask you to authorize this first.", look.muted()),
                ],
                Stage::Failed(message) => vec![
                    Line::styled(message.clone(), look.err()),
                    Line::default(),
                    Line::styled("enter try again   esc close", look.dim()),
                ],
                Stage::Shown { .. } => Vec::new(),
            };
            f.render_widget(wrap(lines), inner);
        }
        Stage::Shown {
            share: details,
            drawing,
            letters,
        } => {
            // The QR rows plus the border and one info line.
            let need = drawing
                .as_ref()
                .map(|d| (d.width() as u16 + 4, d.height() as u16 + 3));
            let ascii = sym.glyphs == Glyphs::Ascii;
            let fits = !ascii && need.is_some_and(|(w, h)| w <= body.width && h <= body.height);
            let password = details.password.as_ref().map(|p| {
                if *letters {
                    p.expose().to_owned()
                } else {
                    dots(p.expose().chars().count(), sym)
                }
            });
            let info = match (&password, details.password.is_some()) {
                (Some(p), _) => Line::from(vec![
                    Span::styled("Password  ", look.muted()),
                    Span::styled(p.clone(), look.text()),
                ]),
                (None, _) => Line::styled("Open network: no password.", look.muted()),
            };
            let (Some(drawing), true) = (drawing, fits) else {
                let rect = centered(body, 62, 9);
                f.render_widget(Clear, rect);
                let block = sym.block(look, &title, true);
                let inner = block.inner(rect);
                f.render_widget(block, rect);
                let why = match need {
                    _ if ascii => "The QR code needs block characters, and --ascii is on.".into(),
                    Some((w, h)) => format!(
                        "The QR code needs a window of at least {w} by {} cells.",
                        h + 2
                    ),
                    None => "These details are too long for a QR code.".to_string(),
                };
                f.render_widget(
                    wrap(vec![
                        Line::styled(why, look.warn()),
                        Line::styled(
                            "Make the window bigger, or use `nexus wifi share`.",
                            look.muted(),
                        ),
                        Line::default(),
                        info,
                    ]),
                    inner,
                );
                return;
            };
            let (w, h) = need.expect("a drawing has a size");
            let rect = centered(body, w.max(title.chars().count() as u16 + 4), h);
            f.render_widget(Clear, rect);
            let block = sym.block(look, &title, true);
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            let style = code_style(look);
            let mut lines: Vec<Line> = drawing
                .lines
                .iter()
                .map(|l| Line::styled(l.clone(), style).centered())
                .collect();
            lines.push(info.centered());
            f.render_widget(Paragraph::new(lines), inner);
        }
    }
}

pub(super) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}
