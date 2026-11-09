//! QR codes drawn in text cells: two modules per cell with the half-block
//! characters, so modules stay square. No image protocol is involved.
//!
//! Dark modules are the painted halves. Phone cameras expect dark on light,
//! so callers draw the lines black on white (a white quiet zone included),
//! whatever the terminal theme.

use qrcode::{Color, EcLevel, QrCode};

/// Light modules around the code. The standard asks for four; two keeps the
/// code small enough for narrow windows and phones read it fine.
const QUIET: usize = 2;

/// A QR code as rows of text cells, quiet zone included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawing {
    pub lines: Vec<String>,
}

impl Drawing {
    /// Columns the drawing needs.
    pub fn width(&self) -> usize {
        self.lines.first().map_or(0, |l| l.chars().count())
    }

    /// Rows the drawing needs.
    pub fn height(&self) -> usize {
        self.lines.len()
    }
}

/// Draws `payload`. `None` only when it is too long for any QR code.
/// Error correction is the lowest level: the code sits on a screen, not on
/// a scratched sticker, and the lower level keeps it small.
pub fn draw(payload: &str) -> Option<Drawing> {
    let code = QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::L).ok()?;
    let side = code.width();
    let total = side + 2 * QUIET;
    let dark = |x: usize, y: usize| -> bool {
        let (Some(x), Some(y)) = (x.checked_sub(QUIET), y.checked_sub(QUIET)) else {
            return false;
        };
        x < side && y < side && code[(x, y)] == Color::Dark
    };
    let lines = (0..total.div_ceil(2))
        .map(|row| {
            (0..total)
                .map(|x| match (dark(x, row * 2), dark(x, row * 2 + 1)) {
                    (false, false) => ' ',
                    (true, false) => '\u{2580}',
                    (false, true) => '\u{2584}',
                    (true, true) => '\u{2588}',
                })
                .collect()
        })
        .collect();
    Some(Drawing { lines })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wifi_payload_fits_a_small_square() {
        let d = draw("WIFI:T:WPA;S:Home;P:correct horse battery;;").unwrap();
        assert!(
            (29 + 2 * QUIET..=41 + 2 * QUIET).contains(&d.width()),
            "{}",
            d.width()
        );
        assert_eq!(d.height(), d.width().div_ceil(2));
    }

    #[test]
    fn the_quiet_zone_is_light_all_round() {
        let d = draw("WIFI:T:nopass;S:Cafe;;").unwrap();
        assert!(d.lines[0].chars().all(|c| c == ' '));
        let last = d.lines.last().unwrap();
        assert!(last.chars().take(QUIET).all(|c| c == ' '));
        assert!(last.chars().rev().take(QUIET).all(|c| c == ' '));
        // The finder pattern's dark corner sits right inside the quiet zone.
        assert!(d.lines[1].chars().nth(QUIET).is_some_and(|c| c != ' '));
    }

    #[test]
    fn too_much_text_is_none() {
        assert!(draw(&"x".repeat(10_000)).is_none());
    }
}
