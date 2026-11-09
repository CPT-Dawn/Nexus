//! Rate graphs: braille columns (two samples per cell) or, in ASCII mode, one
//! sample per cell with plain bars. Pure: samples in, text rows out.

use std::collections::VecDeque;

/// The graph never zooms in below this rate, so a whisper of traffic does
/// not look like a flood.
const FLOOR_BPS: u64 = 32 * 1024;

/// Braille dot bits for one column of a cell, top to bottom.
const LEFT: [u32; 4] = [0x01, 0x02, 0x04, 0x40];
const RIGHT: [u32; 4] = [0x08, 0x10, 0x20, 0x80];

/// The most recent samples that fit, oldest first, left-padded with zeros
/// (nothing seen yet reads as a quiet line).
fn window(samples: &VecDeque<u64>, n: usize) -> Vec<u64> {
    let have = samples.len().min(n);
    let mut out = vec![0; n - have];
    out.extend(samples.iter().skip(samples.len() - have));
    out
}

fn scale(window: &[u64]) -> u64 {
    window.iter().copied().max().unwrap_or(0).max(FLOOR_BPS)
}

/// Dots (0..=max) a sample fills; any traffic shows at least one dot, and a
/// quiet line keeps a baseline so the graph never vanishes.
fn dots(v: u64, top: u64, max: u32) -> u32 {
    if v == 0 {
        return 1;
    }
    (((u128::from(v) * u128::from(max)).div_ceil(u128::from(top))) as u32).clamp(1, max)
}

/// Braille rows, `width` cells wide and `rows` tall, newest sample at the
/// right edge.
pub fn braille(samples: &VecDeque<u64>, width: u16, rows: u16) -> Vec<String> {
    let (w, rows) = (width as usize, rows as usize);
    let win = window(samples, w * 2);
    let top = scale(&win);
    let max = (rows * 4) as u32;
    let heights: Vec<u32> = win.iter().map(|v| dots(*v, top, max)).collect();
    (0..rows)
        .map(|row| {
            // Dots below this row's bottom edge are filled by lower rows.
            let base = ((rows - 1 - row) * 4) as u32;
            (0..w)
                .map(|col| {
                    let mut bits = 0;
                    for (side, table) in [(0, LEFT), (1, RIGHT)] {
                        let h = heights[col * 2 + side];
                        for k in 0..4u32 {
                            if base + k < h {
                                // k counts up from the bottom of the cell.
                                bits |= table[3 - k as usize];
                            }
                        }
                    }
                    char::from_u32(0x2800 + bits).unwrap_or(' ')
                })
                .collect()
        })
        .collect()
}

/// The same graph for terminals that cannot be trusted with braille.
pub fn bars(samples: &VecDeque<u64>, width: u16, rows: u16) -> Vec<String> {
    let (w, rows) = (width as usize, rows as usize);
    let win = window(samples, w);
    let top = scale(&win);
    (0..rows)
        .map(|row| {
            let from_bottom = (rows - 1 - row) as u32;
            win.iter()
                .map(|v| {
                    let h = dots(*v, top, rows as u32 * 4).div_ceil(4);
                    if *v == 0 && from_bottom == 0 {
                        '_'
                    } else if h > from_bottom && *v > 0 {
                        '#'
                    } else {
                        ' '
                    }
                })
                .collect()
        })
        .collect()
}

/// "0 B/s", "950 B/s", "12.3 KB/s", "3.5 MB/s": decimal units, one decimal.
pub fn rate(bps: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    if bps < 1000 {
        return format!("{bps} B/s");
    }
    let mut v = bps as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit < UNITS.len() - 1 {
        v /= 1000.0;
        unit += 1;
    }
    format!("{v:.1} {}/s", UNITS[unit])
}
