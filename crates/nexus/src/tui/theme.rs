//! Semantic theme tokens and how they degrade across colour tiers.
//!
//! Views never name a colour: they ask [`Look`] for a token's style
//! (`look.accent()`, `look.dim()`). A token carries its truecolor value and a
//! hand-picked 16-colour fallback; the 256 tier is the nearest xterm index.

use ratatui::style::{Color, Modifier, Style};

use super::caps::{Caps, ColorTier};

/// One colour token: exact RGB, and what to use on a 16-colour terminal.
#[derive(Debug, Clone, Copy)]
pub struct Tok {
    pub rgb: (u8, u8, u8),
    pub ansi: Color,
}

const fn tok(rgb: u32, ansi: Color) -> Tok {
    Tok {
        rgb: ((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8),
        ansi,
    }
}

/// A palette of semantic tokens.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    /// Painted only on an opaque surface; the default is transparent.
    pub bg: Tok,
    pub fg: Tok,
    pub muted: Tok,
    pub dim: Tok,
    pub accent: Tok,
    pub ok: Tok,
    pub warn: Tok,
    pub err: Tok,
    pub border: Tok,
    /// Tint behind the selected row.
    pub selection: Tok,
}

impl Theme {
    /// The CPT Dawn suite look, shared with Zenith and Hoarder.
    pub const MIDNIGHT_BLOOM: Theme = Theme {
        name: "midnight-bloom",
        bg: tok(0x0b1214, Color::Black),
        fg: tok(0xe4f2f0, Color::Reset),
        muted: tok(0x9db4b5, Color::Gray),
        dim: tok(0x5d7377, Color::DarkGray),
        accent: tok(0x8de3dc, Color::LightCyan),
        ok: tok(0x9ece6a, Color::Green),
        warn: tok(0xe0af68, Color::Yellow),
        err: tok(0xf7768e, Color::LightRed),
        border: tok(0x2c4a4c, Color::Cyan),
        selection: tok(0x17302f, Color::DarkGray),
    };
}

/// A theme resolved for one terminal: hands out ready-made styles.
#[derive(Debug, Clone, Copy)]
pub struct Look {
    pub theme: Theme,
    pub caps: Caps,
}

impl Look {
    pub fn new(theme: Theme, caps: Caps) -> Look {
        Look { theme, caps }
    }

    fn color(&self, t: Tok) -> Option<Color> {
        let (r, g, b) = t.rgb;
        match self.caps.color {
            ColorTier::TrueColor => Some(Color::Rgb(r, g, b)),
            ColorTier::Ansi256 => Some(Color::Indexed(nearest_xterm(t.rgb))),
            ColorTier::Ansi16 => Some(t.ansi),
            ColorTier::None => None,
        }
    }

    fn fg(&self, t: Tok) -> Style {
        match self.color(t) {
            Some(c) => Style::new().fg(c),
            None => Style::new(),
        }
    }

    pub fn text(&self) -> Style {
        self.fg(self.theme.fg)
    }

    pub fn muted(&self) -> Style {
        self.fg(self.theme.muted)
    }

    pub fn dim(&self) -> Style {
        match self.caps.color {
            ColorTier::None => Style::new().add_modifier(Modifier::DIM),
            _ => self.fg(self.theme.dim),
        }
    }

    pub fn accent(&self) -> Style {
        let s = self.fg(self.theme.accent);
        if self.caps.color == ColorTier::None {
            s.add_modifier(Modifier::BOLD)
        } else {
            s
        }
    }

    pub fn ok(&self) -> Style {
        self.fg(self.theme.ok)
    }

    pub fn warn(&self) -> Style {
        self.fg(self.theme.warn)
    }

    /// Errors are never colour alone: bold carries them on a mono terminal.
    pub fn err(&self) -> Style {
        self.fg(self.theme.err).add_modifier(Modifier::BOLD)
    }

    pub fn border(&self) -> Style {
        match self.caps.color {
            ColorTier::None => Style::new(),
            _ => self.fg(self.theme.border),
        }
    }

    /// The border of the focused pane.
    pub fn border_focus(&self) -> Style {
        self.accent()
    }

    pub fn title(&self) -> Style {
        self.text().add_modifier(Modifier::BOLD)
    }

    /// The selected row: a tint, or reverse video when there is no colour.
    pub fn selected(&self) -> Style {
        match self.caps.color {
            ColorTier::None => Style::new().add_modifier(Modifier::REVERSED),
            ColorTier::Ansi16 => Style::new().add_modifier(Modifier::REVERSED),
            _ => {
                let mut s = self.text().add_modifier(Modifier::BOLD);
                if let Some(bg) = self.color(self.theme.selection) {
                    s = s.bg(bg);
                }
                s
            }
        }
    }

    /// Fill for an opaque surface (not used while transparent).
    pub fn surface(&self) -> Style {
        match self.color(self.theme.bg) {
            Some(c) => Style::new().bg(c),
            None => Style::new(),
        }
    }
}

/// The xterm-256 index closest to an RGB colour (cube or grey ramp).
fn nearest_xterm((r, g, b): (u8, u8, u8)) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let near = |v: u8| {
        (0..6)
            .min_by_key(|&i| (i32::from(LEVELS[i]) - i32::from(v)).abs())
            .expect("six levels")
    };
    let dist = |a: (u8, u8, u8)| {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(a.0, r) + d(a.1, g) + d(a.2, b)
    };
    let (ri, gi, bi) = (near(r), near(g), near(b));
    let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);
    let cube_idx = 16 + 36 * ri + 6 * gi + bi;

    let avg = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
    let step = (avg.saturating_sub(8) / 10).min(23) as u8;
    let grey_v = 8 + 10 * step;
    let grey = (grey_v, grey_v, grey_v);

    if dist(grey) < dist(cube) {
        232 + step
    } else {
        cube_idx as u8
    }
}
