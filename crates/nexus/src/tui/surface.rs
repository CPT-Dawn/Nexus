//! Premium surfaces (ADR-0003): focused panes get a gradient border, and on a
//! truecolor terminal a faint glow around it. A post-pass over the drawn
//! frame, so views keep asking the theme for one border style; the focused
//! border is recognised by its token (the accent) on a border glyph.
//!
//! Tiers: truecolor gets the smooth gradient and the glow; 256 colours get
//! the same gradient stepped to the nearest palette index and no glow; 16
//! colours, no colour and the ASCII glyph set keep the plain accent border
//! (bold where there is no colour).

use ratatui::buffer::Buffer;
use ratatui::style::Color;

use super::caps::ColorTier;
use super::theme::{Look, nearest_xterm};

const BORDER_GLYPHS: &[&str] = &[
    "╭", "╮", "╰", "╯", "─", "│", "┌", "┐", "└", "┘", "+", "-", "|",
];

/// How far the gradient turns the accent's hue, in degrees.
const HUE_SWING: f32 = 58.0;

/// Gradient bands across the screen.
const STEPS: f32 = 12.0;

/// How strongly the glow tints the cells beside a focused border.
const GLOW: f32 = 0.10;

pub(super) fn finish(buf: &mut Buffer, look: &Look) {
    let tier = look.caps.color;
    if !matches!(tier, ColorTier::TrueColor | ColorTier::Ansi256) {
        return;
    }
    let accent = look.theme.accent.rgb;
    let accent_color = match tier {
        ColorTier::TrueColor => Color::Rgb(accent.0, accent.1, accent.2),
        _ => Color::Indexed(nearest_xterm(accent)),
    };
    let area = buf.area;
    if area.width == 0 || area.height == 0 {
        return;
    }
    let span = f32::from(area.width) + 2.0 * f32::from(area.height);
    let mut ring: Vec<(u16, u16, (u8, u8, u8))> = Vec::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &buf[(x, y)];
            if cell.fg != accent_color || !BORDER_GLYPHS.contains(&cell.symbol()) {
                continue;
            }
            // Banded, not smooth: a dozen steps read as a gradient, and the
            // frame keeps a handful of styles instead of one per column.
            let t = (f32::from(x - area.x) + 2.0 * f32::from(y - area.y)) / span;
            let t = (t * STEPS).floor() / STEPS;
            let shade = turn_hue(accent, HUE_SWING * (t - 0.5));
            buf[(x, y)].fg = match tier {
                ColorTier::TrueColor => Color::Rgb(shade.0, shade.1, shade.2),
                _ => Color::Indexed(nearest_xterm(shade)),
            };
            ring.push((x, y, shade));
        }
    }
    if tier != ColorTier::TrueColor {
        return;
    }
    // Glow: the plain cells beside a focused border take a hint of its colour.
    let base = look.theme.bg.rgb;
    for (x, y, shade) in ring {
        let beside = [
            (x.wrapping_sub(1), y),
            (x + 1, y),
            (x, y.wrapping_sub(1)),
            (x, y + 1),
        ];
        for (nx, ny) in beside {
            if nx >= area.right() || ny >= area.bottom() || nx < area.left() || ny < area.top() {
                continue;
            }
            let cell = &mut buf[(nx, ny)];
            if cell.bg != Color::Reset || BORDER_GLYPHS.contains(&cell.symbol()) {
                continue;
            }
            let tint = mix(base, shade, GLOW);
            cell.bg = Color::Rgb(tint.0, tint.1, tint.2);
        }
    }
}

fn mix(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// `c` with its hue turned by `degrees` (HSL, lightness and saturation kept).
fn turn_hue(c: (u8, u8, u8), degrees: f32) -> (u8, u8, u8) {
    let (r, g, b) = (
        f32::from(c.0) / 255.0,
        f32::from(c.1) / 255.0,
        f32::from(c.2) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return c;
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let h = (h + degrees).rem_euclid(360.0);
    let k = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = k * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - k / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (k, x, 0.0),
        1 => (x, k, 0.0),
        2 => (0.0, k, x),
        3 => (0.0, x, k),
        4 => (x, 0.0, k),
        _ => (k, 0.0, x),
    };
    let to = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to(r), to(g), to(b))
}

// ---- small helpers the views share ------------------------------------------

/// `text` safe to show in a terminal: control and direction-override
/// characters (which could reorder the text around them) become U+FFFD.
pub(super) fn printable(text: &str) -> String {
    nexus_ipc::display_ssid(text.as_bytes())
}

/// `n` mask characters for a hidden password.
pub(super) fn dots(n: usize, sym: &super::view::Sym) -> String {
    let dot = if sym.glyphs == super::caps::Glyphs::Ascii {
        '*'
    } else {
        '\u{2022}'
    };
    std::iter::repeat_n(dot, n).collect()
}

/// A `width` by `height` rectangle in the middle of `area` (smaller if
/// `area` is).
pub(super) fn centered(
    area: ratatui::layout::Rect,
    width: u16,
    height: u16,
) -> ratatui::layout::Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    ratatui::layout::Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}
