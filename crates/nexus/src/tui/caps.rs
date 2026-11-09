//! What the terminal can show: colour tier and glyph set.

/// How many colours to use, richest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTier {
    TrueColor,
    Ansi256,
    Ansi16,
    /// No colour at all (`NO_COLOR`, a dumb terminal). Bold, dim and
    /// reverse video still carry the meaning.
    None,
}

/// Which symbols to draw with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyphs {
    /// Box drawing, bars and dots that any Unicode font has. The default.
    Unicode,
    /// Plain ASCII only.
    Ascii,
    /// Unicode plus Nerd Font icons. Opt-in.
    Nerd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub color: ColorTier,
    pub glyphs: Glyphs,
}

impl Caps {
    /// Reads the environment through `env` (so tests can fake it).
    /// `ascii` and `nerd` are the user's explicit flags; ASCII wins.
    pub fn detect(env: impl Fn(&str) -> Option<String>, ascii: bool, nerd: bool) -> Caps {
        let get = |k: &str| env(k).filter(|v| !v.is_empty());
        let term = get("TERM").unwrap_or_default();
        let dumb = term == "dumb";

        let color = if get("NO_COLOR").is_some() || dumb {
            ColorTier::None
        } else if matches!(get("COLORTERM").as_deref(), Some("truecolor" | "24bit")) {
            ColorTier::TrueColor
        } else if term.contains("256color") {
            ColorTier::Ansi256
        } else {
            ColorTier::Ansi16
        };

        let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
            .into_iter()
            .find_map(get)
            .unwrap_or_default()
            .to_lowercase();
        let utf8 = locale.contains("utf-8") || locale.contains("utf8");
        let glyphs = if ascii || dumb || !utf8 {
            Glyphs::Ascii
        } else if nerd {
            Glyphs::Nerd
        } else {
            Glyphs::Unicode
        };
        Caps { color, glyphs }
    }

    /// Detects from the process environment.
    pub fn from_env(ascii: bool, nerd: bool) -> Caps {
        Caps::detect(|k| std::env::var(k).ok(), ascii, nerd)
    }
}
