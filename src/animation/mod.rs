pub mod spinner;
pub mod transitions;

use std::collections::HashSet;

/// Identifies different animations that can be active
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationId {
    Spinner,
    DialogSlideIn,
    SignalUpdate,
    CursorBlink,
    ListFadeIn,
}

/// Tracks animation state for the entire application
#[derive(Debug)]
pub struct AnimationState {
    /// Monotonically increasing tick counter
    pub tick_count: u64,
    /// Currently active animations
    pub active: HashSet<AnimationId>,
    /// Dialog slide-in offset (decreases from max to 0)
    pub dialog_offset: f32,
    /// Max dialog slide offset
    pub dialog_offset_max: f32,
}

impl Default for AnimationState {
    fn default() -> Self {
        Self {
            tick_count: 0,
            active: HashSet::new(),
            dialog_offset: 0.0,
            dialog_offset_max: 3.0,
        }
    }
}

impl AnimationState {
    /// Advance all animations by one tick
    pub fn tick(&mut self) {
        self.tick_count = self.tick_count.wrapping_add(1);

        // Advance dialog slide-in
        if self.active.contains(&AnimationId::DialogSlideIn) {
            self.dialog_offset = (self.dialog_offset - 0.6).max(0.0);
            if self.dialog_offset <= 0.0 {
                self.active.remove(&AnimationId::DialogSlideIn);
            }
        }
    }

    /// Check if any animation is currently running
    pub fn has_active_animation(&self) -> bool {
        !self.active.is_empty()
    }

    /// Start the dialog slide-in animation
    pub fn start_dialog_slide(&mut self) {
        self.dialog_offset = self.dialog_offset_max;
        self.active.insert(AnimationId::DialogSlideIn);
    }

    /// Start the scanning spinner
    pub fn start_spinner(&mut self) {
        self.active.insert(AnimationId::Spinner);
    }

    /// Stop the scanning spinner
    pub fn stop_spinner(&mut self) {
        self.active.remove(&AnimationId::Spinner);
    }

    /// Check if cursor should be visible (blink effect)
    pub fn cursor_visible(&self) -> bool {
        // 70% duty cycle: visible for 14 out of 20 ticks
        (self.tick_count % 20) < 14
    }

    /// Get dialog Y offset as integer for rendering
    pub fn dialog_y_offset(&self) -> u16 {
        self.dialog_offset.ceil() as u16
    }
}

/// Linear interpolation
pub fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t.clamp(0.0, 1.0)
}

/// Exponential ease-out interpolation (smooth approach)
pub fn ease_out(current: f32, target: f32, factor: f32) -> f32 {
    current + (target - current) * factor
}
