/// Braille dot spinner frames for scanning/connecting animation
const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Get the current spinner frame character based on the tick count
pub fn spinner_frame(tick_count: u64) -> char {
    let idx = (tick_count / 3) as usize % SPINNER_FRAMES.len();
    SPINNER_FRAMES[idx]
}

/// Get a spinning bar animation frame
const BAR_FRAMES: &[&str] = &["◐", "◓", "◑", "◒"];

pub fn bar_frame(tick_count: u64) -> &'static str {
    let idx = (tick_count / 4) as usize % BAR_FRAMES.len();
    BAR_FRAMES[idx]
}

/// Pulsing dot animation for connection indicator
const PULSE_FRAMES: &[&str] = &["●", "●", "●", "○", "○"];

pub fn pulse_frame(tick_count: u64) -> &'static str {
    let idx = (tick_count / 5) as usize % PULSE_FRAMES.len();
    PULSE_FRAMES[idx]
}
