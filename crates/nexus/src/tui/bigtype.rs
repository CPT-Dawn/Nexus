//! Big-type headlines: a 3x5 pixel font, optionally with a soft halo around
//! the lit pixels (the hero's "glow"). Pure: text in, a grid of cells out.

/// One pixel of the rendered headline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Px {
    Off,
    Lit,
    /// Unlit, but touching a lit pixel: drawn as a faint halo.
    Halo,
}

/// Letters A..Z, five rows of three bits each (bit 2 is the left column).
const FONT: [[u8; 5]; 26] = [
    [0b010, 0b101, 0b111, 0b101, 0b101], // A
    [0b110, 0b101, 0b110, 0b101, 0b110], // B
    [0b011, 0b100, 0b100, 0b100, 0b011], // C
    [0b110, 0b101, 0b101, 0b101, 0b110], // D
    [0b111, 0b100, 0b110, 0b100, 0b111], // E
    [0b111, 0b100, 0b110, 0b100, 0b100], // F
    [0b011, 0b100, 0b101, 0b101, 0b011], // G
    [0b101, 0b101, 0b111, 0b101, 0b101], // H
    [0b111, 0b010, 0b010, 0b010, 0b111], // I
    [0b001, 0b001, 0b001, 0b101, 0b010], // J
    [0b101, 0b101, 0b110, 0b101, 0b101], // K
    [0b100, 0b100, 0b100, 0b100, 0b111], // L
    [0b101, 0b111, 0b111, 0b101, 0b101], // M
    [0b110, 0b101, 0b101, 0b101, 0b101], // N
    [0b010, 0b101, 0b101, 0b101, 0b010], // O
    [0b110, 0b101, 0b110, 0b100, 0b100], // P
    [0b010, 0b101, 0b101, 0b111, 0b011], // Q
    [0b110, 0b101, 0b110, 0b101, 0b101], // R
    [0b011, 0b100, 0b010, 0b001, 0b110], // S
    [0b111, 0b010, 0b010, 0b010, 0b010], // T
    [0b101, 0b101, 0b101, 0b101, 0b111], // U
    [0b101, 0b101, 0b101, 0b101, 0b010], // V
    [0b101, 0b101, 0b111, 0b111, 0b101], // W
    [0b101, 0b101, 0b010, 0b101, 0b101], // X
    [0b101, 0b101, 0b010, 0b010, 0b010], // Y
    [0b111, 0b001, 0b010, 0b100, 0b111], // Z
];

const ROWS: usize = 5;

fn glyph(c: char) -> [u8; 5] {
    match c.to_ascii_uppercase() {
        c @ 'A'..='Z' => FONT[c as usize - 'A' as usize],
        _ => [0; 5], // spaces and anything unknown are blank
    }
}

/// How many text columns `text` takes at `scale` columns per pixel,
/// halo margin included.
pub fn width(text: &str, scale: u16, halo: bool) -> u16 {
    let n = text.chars().count() as u16;
    let pixels = if n == 0 {
        0
    } else {
        n * 4 - 1 + if halo { 2 } else { 0 }
    };
    pixels * scale
}

/// How many text rows the headline takes.
pub fn height(halo: bool) -> u16 {
    ROWS as u16 + if halo { 2 } else { 0 }
}

/// The pixel grid for `text`, row by row, with a one-pixel halo margin when
/// `halo` is set.
pub fn pixels(text: &str, halo: bool) -> Vec<Vec<Px>> {
    let glyphs: Vec<[u8; 5]> = text.chars().map(glyph).collect();
    let margin = usize::from(halo);
    let cols = if glyphs.is_empty() {
        0
    } else {
        glyphs.len() * 4 - 1 + 2 * margin
    };
    let rows = ROWS + 2 * margin;
    let mut lit = vec![vec![false; cols]; rows];
    for (i, g) in glyphs.iter().enumerate() {
        for (r, bits) in g.iter().enumerate() {
            for c in 0..3 {
                if bits & (0b100 >> c) != 0 {
                    lit[r + margin][i * 4 + c + margin] = true;
                }
            }
        }
    }
    let mut grid = vec![vec![Px::Off; cols]; rows];
    for r in 0..rows {
        for c in 0..cols {
            grid[r][c] = if lit[r][c] {
                Px::Lit
            } else if halo && touches_lit(&lit, r, c) {
                Px::Halo
            } else {
                Px::Off
            };
        }
    }
    grid
}

fn touches_lit(lit: &[Vec<bool>], r: usize, c: usize) -> bool {
    for dr in -1i32..=1 {
        for dc in -1i32..=1 {
            if dr != 0 && dc != 0 {
                continue; // corners stay dark: a tighter glow
            }
            let (rr, cc) = (r as i32 + dr, c as i32 + dc);
            if rr >= 0
                && cc >= 0
                && lit
                    .get(rr as usize)
                    .and_then(|row| row.get(cc as usize))
                    .copied()
                    .unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}
