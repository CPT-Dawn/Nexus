pub mod spinner;
pub mod transitions;

// ─── Animation Flags (replaces HashSet<AnimationId>) ────────────────────
const FLAG_SPINNER: u8 = 0b0000_0001;
const FLAG_DIALOG_SLIDE: u8 = 0b0000_0010;

/// Tracks animation state for the entire application.
/// Uses a simple bitflag `u8` instead of `HashSet` — zero allocation,
/// cache-friendly, and there are only a handful of animation types.
#[derive(Debug)]
pub struct AnimationState {
    /// Monotonically increasing tick counter
    pub tick_count: u64,
    /// Bitflags for active animations
    active: u8,
    /// Dialog slide-in progress: 0.0 (done) → 1.0 (just started)
    dialog_t: f32,
    /// Duration of dialog slide in ticks
    dialog_duration: f32,
    /// Elapsed ticks since dialog slide started
    dialog_elapsed: f32,
}

impl Default for AnimationState {
    fn default() -> Self {
        Self {
            tick_count: 0,
            active: 0,
            dialog_t: 0.0,
            dialog_duration: 12.0, // ~200ms at 60 FPS
            dialog_elapsed: 0.0,
        }
    }
}

impl AnimationState {
    /// Advance all animations by one tick
    pub fn tick(&mut self) {
        self.tick_count = self.tick_count.wrapping_add(1);

        // Advance dialog slide-in using cubic ease-out
        if self.active & FLAG_DIALOG_SLIDE != 0 {
            self.dialog_elapsed += 1.0;
            let t = (self.dialog_elapsed / self.dialog_duration).min(1.0);
            self.dialog_t = ease_out_cubic(t);
            if t >= 1.0 {
                self.dialog_t = 1.0;
                self.active &= !FLAG_DIALOG_SLIDE;
            }
        }
    }

    /// Start the dialog slide-in animation
    pub fn start_dialog_slide(&mut self) {
        self.dialog_t = 0.0;
        self.dialog_elapsed = 0.0;
        self.active |= FLAG_DIALOG_SLIDE;
    }

    /// Start the scanning spinner
    pub fn start_spinner(&mut self) {
        self.active |= FLAG_SPINNER;
    }

    /// Stop the scanning spinner
    pub fn stop_spinner(&mut self) {
        self.active &= !FLAG_SPINNER;
    }

    /// Check if cursor should be visible (blink effect)
    pub fn cursor_visible(&self) -> bool {
        // 70% duty cycle: visible for 14 out of 20 ticks
        (self.tick_count % 20) < 14
    }

    /// Get dialog Y offset as integer for rendering.
    /// Returns pixels of remaining offset (largest when animation just started,
    /// shrinks to 0 when complete).
    pub fn dialog_y_offset(&self) -> u16 {
        let max_offset: f32 = 4.0;
        let remaining = max_offset * (1.0 - self.dialog_t);
        remaining.ceil() as u16
    }
}

/// Exponential ease-out interpolation (smooth approach for signal smoothing)
pub fn ease_out(current: f32, target: f32, factor: f32) -> f32 {
    current + (target - current) * factor
}

/// Cubic ease-out: fast start, smooth deceleration.
/// `t` in [0, 1] → output in [0, 1].
#[inline]
pub fn ease_out_cubic(t: f32) -> f32 {
    let inv = 1.0 - t;
    1.0 - inv * inv * inv
}
