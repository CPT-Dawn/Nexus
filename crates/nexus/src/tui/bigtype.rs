//! Big-type headlines: a 4x5 pixel font, optionally with a soft halo around
//! the lit pixels (the hero's "glow"). Pure: text in, a grid of cells out.
//!
//! Glyphs are four pixels wide so the round letters can round off: an O is a
//! ring with clipped corners, an N carries its diagonal, and nothing reads as
//! a lowercase letter. Every glyph is drawn upper case.

/// One pixel of the rendered headline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Px {
    Off,
    Lit,
    /// Unlit, but touching a lit pixel: drawn as a faint halo.
    Halo,
}

/// Pixels across one glyph, and the gap pixel between two.
const GLYPH_W: usize = 4;
const PITCH: usize = GLYPH_W + 1;

/// Letters A..Z, five rows of four bits each (bit 3 is the left column).
const FONT: [[u8; 5]; 26] = [
    [0b0110, 0b1001, 0b1111, 0b1001, 0b1001], // A
    [0b1110, 0b1001, 0b1110, 0b1001, 0b1110], // B
    [0b0111, 0b1000, 0b1000, 0b1000, 0b0111], // C
    [0b1110, 0b1001, 0b1001, 0b1001, 0b1110], // D
    [0b1111, 0b1000, 0b1110, 0b1000, 0b1111], // E
    [0b1111, 0b1000, 0b1110, 0b1000, 0b1000], // F
    [0b0111, 0b1000, 0b1011, 0b1001, 0b0111], // G
    [0b1001, 0b1001, 0b1111, 0b1001, 0b1001], // H
    [0b1110, 0b0100, 0b0100, 0b0100, 0b1110], // I
    [0b0011, 0b0001, 0b0001, 0b1001, 0b0110], // J
    [0b1001, 0b1010, 0b1100, 0b1010, 0b1001], // K
    [0b1000, 0b1000, 0b1000, 0b1000, 0b1111], // L
    [0b1001, 0b1111, 0b1111, 0b1001, 0b1001], // M
    [0b1001, 0b1101, 0b1111, 0b1011, 0b1001], // N
    [0b0110, 0b1001, 0b1001, 0b1001, 0b0110], // O
    [0b1110, 0b1001, 0b1110, 0b1000, 0b1000], // P
    [0b0110, 0b1001, 0b1001, 0b1011, 0b0111], // Q
    [0b1110, 0b1001, 0b1110, 0b1010, 0b1001], // R
    [0b0111, 0b1000, 0b0110, 0b0001, 0b1110], // S
    [0b1111, 0b0110, 0b0110, 0b0110, 0b0110], // T
    [0b1001, 0b1001, 0b1001, 0b1001, 0b0110], // U
    [0b1001, 0b1001, 0b1001, 0b0110, 0b0110], // V
    [0b1001, 0b1001, 0b1111, 0b1111, 0b1001], // W
    [0b1001, 0b0110, 0b0110, 0b0110, 0b1001], // X
    [0b1001, 0b1001, 0b0110, 0b0110, 0b0110], // Y
    [0b1111, 0b0001, 0b0110, 0b1000, 0b1111], // Z
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
        n * PITCH as u16 - 1 + if halo { 2 } else { 0 }
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
        glyphs.len() * PITCH - 1 + 2 * margin
    };
    let rows = ROWS + 2 * margin;
    let mut lit = vec![vec![false; cols]; rows];
    for (i, g) in glyphs.iter().enumerate() {
        for (r, bits) in g.iter().enumerate() {
            for c in 0..GLYPH_W {
                if bits & (0b1000 >> c) != 0 {
                    lit[r + margin][i * PITCH + c + margin] = true;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn art(text: &str) -> Vec<String> {
        pixels(text, false)
            .iter()
            .map(|row| {
                row.iter()
                    .map(|p| if *p == Px::Lit { '#' } else { '.' })
                    .collect()
            })
            .collect()
    }

    /// The hero's first word read as a notched D/Ø and a lowercase n (#29).
    #[test]
    fn o_is_a_round_ring_and_n_an_upper_case_with_its_diagonal() {
        assert_eq!(
            art("ON"),
            [
                ".##..#..#",
                "#..#.##.#",
                "#..#.####",
                "#..#.#.##",
                ".##..#..#",
            ]
        );
    }

    #[test]
    fn every_letter_has_a_distinct_shape_and_fits_the_grid() {
        let mut seen = std::collections::BTreeSet::new();
        for c in 'A'..='Z' {
            let g = glyph(c);
            assert!(g.iter().any(|r| *r != 0), "{c} is blank");
            assert!(
                g.iter().all(|r| *r < 1 << GLYPH_W),
                "{c} overflows the grid"
            );
            assert!(seen.insert(g), "{c} duplicates another letter");
        }
    }

    #[test]
    fn width_and_height_agree_with_the_grid() {
        for halo in [false, true] {
            let grid = pixels("Online", halo);
            assert_eq!(grid.len() as u16, height(halo));
            assert_eq!(grid[0].len() as u16, width("Online", 1, halo));
            assert_eq!(width("Online", 2, halo), 2 * width("Online", 1, halo));
        }
    }
}
