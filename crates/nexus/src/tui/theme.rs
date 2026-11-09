//! Semantic theme tokens and how they degrade across colour tiers.
//!
//! Views never name a colour: they ask [`Look`] for a token's style
//! (`look.accent()`, `look.dim()`). A token carries its truecolor value and a
//! hand-picked 16-colour fallback; the 256 tier is the nearest xterm index.

use std::collections::BTreeMap;

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
    /// The config name: `theme = "nord"`. `custom` for a user's own.
    pub name: &'static str,
    /// What the Settings tab calls it.
    pub title: &'static str,
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
        title: "Midnight Bloom",
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

    pub const CATPPUCCIN: Theme = Theme {
        name: "catppuccin",
        title: "Catppuccin",
        bg: tok(0x1e1e2e, Color::Black),
        fg: tok(0xcdd6f4, Color::Reset),
        muted: tok(0xa6adc8, Color::Gray),
        dim: tok(0x7f849c, Color::DarkGray),
        accent: tok(0xcba6f7, Color::LightMagenta),
        ok: tok(0xa6e3a1, Color::Green),
        warn: tok(0xf9e2af, Color::Yellow),
        err: tok(0xf38ba8, Color::LightRed),
        border: tok(0x45475a, Color::Magenta),
        selection: tok(0x313244, Color::DarkGray),
    };

    pub const GRUVBOX: Theme = Theme {
        name: "gruvbox",
        title: "Gruvbox",
        bg: tok(0x282828, Color::Black),
        fg: tok(0xebdbb2, Color::Reset),
        muted: tok(0xa89984, Color::Gray),
        dim: tok(0x7c6f64, Color::DarkGray),
        accent: tok(0xfabd2f, Color::LightYellow),
        ok: tok(0xb8bb26, Color::Green),
        warn: tok(0xfe8019, Color::Yellow),
        err: tok(0xff5c47, Color::LightRed),
        border: tok(0x504945, Color::Yellow),
        selection: tok(0x3c3836, Color::DarkGray),
    };

    pub const NORD: Theme = Theme {
        name: "nord",
        title: "Nord",
        bg: tok(0x2e3440, Color::Black),
        fg: tok(0xeceff4, Color::Reset),
        muted: tok(0xa3adc2, Color::Gray),
        dim: tok(0x7b88a1, Color::DarkGray),
        accent: tok(0x88c0d0, Color::LightCyan),
        ok: tok(0xa3be8c, Color::Green),
        warn: tok(0xebcb8b, Color::Yellow),
        err: tok(0xe0848d, Color::LightRed),
        border: tok(0x434c5e, Color::Cyan),
        selection: tok(0x3b4252, Color::DarkGray),
    };

    pub const TOKYO_NIGHT: Theme = Theme {
        name: "tokyo-night",
        title: "Tokyo Night",
        bg: tok(0x1a1b26, Color::Black),
        fg: tok(0xc0caf5, Color::Reset),
        muted: tok(0x9aa5ce, Color::Gray),
        dim: tok(0x6a739f, Color::DarkGray),
        accent: tok(0x7aa2f7, Color::LightBlue),
        ok: tok(0x9ece6a, Color::Green),
        warn: tok(0xe0af68, Color::Yellow),
        err: tok(0xf7768e, Color::LightRed),
        border: tok(0x3b4261, Color::Blue),
        selection: tok(0x292e42, Color::DarkGray),
    };

    pub const ROSE_PINE: Theme = Theme {
        name: "rose-pine",
        title: "Rosé Pine",
        bg: tok(0x191724, Color::Black),
        fg: tok(0xe0def4, Color::Reset),
        muted: tok(0x908caa, Color::Gray),
        dim: tok(0x6e6a86, Color::DarkGray),
        accent: tok(0xc4a7e7, Color::LightMagenta),
        ok: tok(0x9ccfd8, Color::Green),
        warn: tok(0xf6c177, Color::Yellow),
        err: tok(0xeb6f92, Color::LightRed),
        border: tok(0x403d52, Color::Magenta),
        selection: tok(0x26233a, Color::DarkGray),
    };

    /// Every text token clears a 7:1 contrast ratio on black (WCAG AAA).
    pub const HIGH_CONTRAST: Theme = Theme {
        name: "high-contrast",
        title: "High contrast",
        bg: tok(0x000000, Color::Black),
        fg: tok(0xffffff, Color::Reset),
        muted: tok(0xe0e0e0, Color::Gray),
        dim: tok(0xb8b8b8, Color::DarkGray),
        accent: tok(0xffd400, Color::LightYellow),
        ok: tok(0x40ff40, Color::LightGreen),
        warn: tok(0xffa040, Color::LightYellow),
        err: tok(0xff6060, Color::LightRed),
        border: tok(0x8c8c8c, Color::Gray),
        selection: tok(0x2a2a60, Color::Blue),
    };

    /// The built-in themes, the default first.
    pub const BUILTIN: [Theme; 7] = [
        Theme::MIDNIGHT_BLOOM,
        Theme::CATPPUCCIN,
        Theme::GRUVBOX,
        Theme::NORD,
        Theme::TOKYO_NIGHT,
        Theme::ROSE_PINE,
        Theme::HIGH_CONTRAST,
    ];

    /// A built-in theme by its config name.
    pub fn builtin(name: &str) -> Option<Theme> {
        Theme::BUILTIN.into_iter().find(|t| t.name == name)
    }

    /// A custom theme: `tokens` maps a semantic token (`bg`, `fg`, `muted`,
    /// `dim`, `accent`, `ok`, `warn`, `err`, `border`, `selection`) to
    /// `#rrggbb`, and `base` names the built-in theme the rest comes from
    /// (default Midnight Bloom). The error says which entry is wrong.
    pub fn custom(name: &str, tokens: &BTreeMap<String, String>) -> Result<Theme, String> {
        let bad = |what: String| format!("theme `{name}`: {what}");
        let mut theme = Theme::MIDNIGHT_BLOOM;
        if let Some(base) = tokens.get("base") {
            theme = Theme::builtin(base)
                .ok_or_else(|| bad(format!("base `{base}` is not a built-in theme")))?;
        }
        theme.name = "custom";
        theme.title = "Custom";
        for (key, value) in tokens {
            let slot = match key.as_str() {
                "base" => continue,
                "bg" => &mut theme.bg,
                "fg" => &mut theme.fg,
                "muted" => &mut theme.muted,
                "dim" => &mut theme.dim,
                "accent" => &mut theme.accent,
                "ok" => &mut theme.ok,
                "warn" => &mut theme.warn,
                "err" => &mut theme.err,
                "border" => &mut theme.border,
                "selection" => &mut theme.selection,
                other => return Err(bad(format!("`{other}` is not a theme token"))),
            };
            let rgb = parse_hex(value)
                .ok_or_else(|| bad(format!("`{key} = {value}` is not a #rrggbb colour")))?;
            *slot = Tok {
                rgb,
                ansi: nearest_ansi(rgb),
            };
        }
        if theme.accent.rgb == theme.border.rgb {
            return Err(bad(
                "accent and border must differ (the focused pane would not stand out)".into(),
            ));
        }
        Ok(theme)
    }
}

/// `#rrggbb` or `rrggbb` (also the short `#rgb`).
fn parse_hex(text: &str) -> Option<(u8, u8, u8)> {
    let h = text.strip_prefix('#').unwrap_or(text);
    if !h.is_ascii() {
        return None;
    }
    let digit = |s: &str| u8::from_str_radix(s, 16).ok();
    match h.len() {
        6 => Some((digit(&h[0..2])?, digit(&h[2..4])?, digit(&h[4..6])?)),
        3 => {
            let d = |i: usize| digit(&h[i..=i]).map(|v| v * 17);
            Some((d(0)?, d(1)?, d(2)?))
        }
        _ => None,
    }
}

/// The 16-colour terminal colour closest to an RGB value.
fn nearest_ansi(c: (u8, u8, u8)) -> Color {
    const PALETTE: [(Color, (u8, u8, u8)); 16] = [
        (Color::Black, (0, 0, 0)),
        (Color::Red, (170, 0, 0)),
        (Color::Green, (0, 170, 0)),
        (Color::Yellow, (170, 85, 0)),
        (Color::Blue, (0, 0, 170)),
        (Color::Magenta, (170, 0, 170)),
        (Color::Cyan, (0, 170, 170)),
        (Color::Gray, (170, 170, 170)),
        (Color::DarkGray, (85, 85, 85)),
        (Color::LightRed, (255, 85, 85)),
        (Color::LightGreen, (85, 255, 85)),
        (Color::LightYellow, (255, 255, 85)),
        (Color::LightBlue, (85, 85, 255)),
        (Color::LightMagenta, (255, 85, 255)),
        (Color::LightCyan, (85, 255, 255)),
        (Color::White, (255, 255, 255)),
    ];
    let dist = |p: (u8, u8, u8)| {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(p.0, c.0) + d(p.1, c.1) + d(p.2, c.2)
    };
    PALETTE
        .into_iter()
        .min_by_key(|(_, rgb)| dist(*rgb))
        .map_or(Color::Reset, |(color, _)| color)
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
pub(super) fn nearest_xterm((r, g, b): (u8, u8, u8)) -> u8 {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance((r, g, b): (u8, u8, u8)) -> f64 {
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn built_in_names_are_unique_and_found() {
        for t in Theme::BUILTIN {
            assert_eq!(Theme::builtin(t.name).map(|b| b.title), Some(t.title));
        }
        let mut names: Vec<_> = Theme::BUILTIN.iter().map(|t| t.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Theme::BUILTIN.len());
    }

    #[test]
    fn every_theme_keeps_the_focused_border_recognisable() {
        // surface.rs finds the focused border by the accent on a box glyph.
        for t in Theme::BUILTIN {
            assert_ne!(t.accent.rgb, t.border.rgb, "{}", t.name);
            assert_ne!(t.accent.rgb, t.fg.rgb, "{}", t.name);
        }
    }

    #[test]
    fn text_is_readable_on_every_theme() {
        for t in Theme::BUILTIN {
            for (what, tok) in [
                ("fg", t.fg),
                ("muted", t.muted),
                ("accent", t.accent),
                ("ok", t.ok),
                ("warn", t.warn),
                ("err", t.err),
            ] {
                assert!(contrast(tok.rgb, t.bg.rgb) >= 4.5, "{} {what}", t.name);
            }
            assert!(contrast(t.dim.rgb, t.bg.rgb) >= 3.0, "{} dim", t.name);
        }
    }

    #[test]
    fn high_contrast_clears_seven_to_one() {
        let t = Theme::HIGH_CONTRAST;
        for tok in [t.fg, t.muted, t.dim, t.accent, t.ok, t.warn, t.err] {
            assert!(contrast(tok.rgb, t.bg.rgb) >= 7.0, "{:?}", tok.rgb);
        }
        assert!(contrast(t.fg.rgb, t.selection.rgb) >= 7.0);
    }

    fn tokens(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn a_custom_theme_overrides_tokens_of_its_base() {
        let t = Theme::custom("mine", &tokens(&[("base", "nord"), ("accent", "#ff8800")])).unwrap();
        assert_eq!(t.accent.rgb, (255, 136, 0));
        assert_eq!(t.fg.rgb, Theme::NORD.fg.rgb);
        assert_eq!(t.name, "custom");
    }

    #[test]
    fn short_and_bare_hex_work() {
        let t = Theme::custom("m", &tokens(&[("accent", "f80"), ("ok", "00ff00")])).unwrap();
        assert_eq!(t.accent.rgb, (255, 136, 0));
        assert_eq!(t.ok.rgb, (0, 255, 0));
    }

    #[test]
    fn mistakes_are_named() {
        for (pairs, needle) in [
            (&[("accent", "teal")][..], "accent = teal"),
            (&[("accnt", "#ffffff")][..], "`accnt` is not a theme token"),
            (&[("base", "nope")][..], "base `nope`"),
            (&[("accent", "#2c4a4c")][..], "must differ"),
            (&[("accent", "é12345")][..], "not a #rrggbb"),
        ] {
            let err = Theme::custom("m", &tokens(pairs)).unwrap_err();
            assert!(err.contains(needle) && err.contains("theme `m`"), "{err}");
        }
    }

    #[test]
    fn ansi_fallback_picks_the_nearest_of_sixteen() {
        assert_eq!(nearest_ansi((250, 80, 80)), Color::LightRed);
        assert_eq!(nearest_ansi((2, 2, 2)), Color::Black);
    }
}
