//! Event-driven effects (ADR-0003). Every effect starts from an event: a tab
//! change, a join, a connecting state, an error, a toast. Nothing animates on
//! its own, and [`Fx::running`] is false the moment the last effect ends, so
//! the event loop goes back to sleep and idle stays at zero frames.
//!
//! Effects are a post-pass over the frame the views drew ([`Fx::process`]):
//! the views stay pure. Time comes from the caller, so tests step a virtual
//! clock instead of racing a real one.

use std::time::{Duration as Wall, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tachyonfx::pattern::{RadialPattern, SweepPattern};
use tachyonfx::{CellFilter, Duration, Effect, EffectManager, Interpolation, fx};

use super::caps::ColorTier;
use super::theme::{Look, nearest_xterm};

/// The longest a single frame may advance an effect. After a pause (a stall,
/// a suspended terminal) an effect resumes rather than jumping to its end.
const MAX_STEP: Wall = Wall::from_millis(100);

/// How often a running effect asks for a frame.
pub const FRAME: Wall = Wall::from_millis(33);

/// How much motion the person gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Movement {
    /// Gliding, sliding, shaking and pulsing.
    Full,
    /// No movement: state changes stay legible through colour alone.
    Reduced,
    /// No effects: a terminal with 16 colours or none cannot fade.
    Off,
}

/// One slot per kind of effect: a new one replaces the old.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Slot {
    #[default]
    Tab,
    Bloom,
    Pulse,
    Toast,
}

#[derive(Debug)]
pub struct Fx {
    manager: EffectManager<Slot>,
    motion: Movement,
    /// A pulse is waiting to be cancelled (the connecting state ended).
    pulsing: bool,
    tier: ColorTier,
    accent: Color,
    bright: Color,
    bg: Color,
    /// When the last frame was drawn, while effects run.
    last: Option<Instant>,
}

fn rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb(r, g, b)
}

impl Fx {
    pub fn new(look: &Look) -> Fx {
        let motion = match look.caps.color {
            ColorTier::TrueColor | ColorTier::Ansi256 => Movement::Full,
            ColorTier::Ansi16 | ColorTier::None => Movement::Off,
        };
        Fx {
            manager: EffectManager::default(),
            motion,
            pulsing: false,
            tier: look.caps.color,
            accent: rgb(look.theme.accent.rgb),
            bright: rgb(look.theme.fg.rgb),
            bg: rgb(look.theme.bg.rgb),
            last: None,
        }
    }

    pub fn motion(&self) -> Movement {
        self.motion
    }

    /// Turns movement off (the person asked for reduced motion). A terminal
    /// that cannot fade stays off.
    pub fn reduce(&mut self, on: bool) {
        if self.motion == Movement::Off {
            return;
        }
        self.motion = if on {
            Movement::Reduced
        } else {
            Movement::Full
        };
        if on {
            self.clear();
        }
    }

    /// Whether any effect still wants frames.
    pub fn running(&self) -> bool {
        self.manager.is_running()
    }

    /// Drops everything (a resize invalidates every area).
    pub fn clear(&mut self) {
        self.manager = EffectManager::default();
        self.pulsing = false;
        self.last = None;
    }

    fn add(&mut self, slot: Slot, effect: Effect) {
        self.last.get_or_insert_with(Instant::now);
        self.manager.add_unique_effect(slot, effect);
    }

    /// The body glides in from the left after a tab change.
    pub fn tab_changed(&mut self, body: Rect) {
        if self.motion != Movement::Full {
            return;
        }
        // Text only: a sweep that fades the background too would leave
        // opaque cells on a transparent terminal while it runs.
        let glide = fx::fade_from_fg(self.bg, (280, Interpolation::QuadOut))
            .with_pattern(SweepPattern::left_to_right(14));
        self.add(Slot::Tab, glide.with_area(body));
    }

    /// A joined Network blooms: its surface lights up and settles. Reduced
    /// motion keeps the colour change and drops the spreading.
    pub fn joined(&mut self, area: Rect) {
        let bloom = fx::fade_from_fg(self.accent, (750, Interpolation::QuadOut));
        let bloom = match self.motion {
            Movement::Full => bloom.with_pattern(RadialPattern::center()),
            Movement::Reduced => bloom,
            Movement::Off => return,
        };
        self.add(Slot::Bloom, bloom.with_area(area));
    }

    /// The focal elements breathe while a join is in flight; `on` false
    /// lets them settle.
    pub fn connecting(&mut self, on: bool, focal: &[Rect]) {
        if self.motion != Movement::Full {
            return;
        }
        if on {
            let breath = |area: Rect| {
                fx::repeating(fx::ping_pong(fx::fade_to_fg(
                    self.bright,
                    (650, Interpolation::SineInOut),
                )))
                .with_area(area)
                .with_filter(CellFilter::Text)
            };
            let all: Vec<Effect> = focal.iter().map(|r| breath(*r)).collect();
            self.add(Slot::Pulse, fx::parallel(&all));
            self.pulsing = true;
        } else if self.pulsing {
            self.manager.cancel_unique_effect(Slot::Pulse);
            self.pulsing = false;
        }
    }

    /// A toast slides in from the right; a problem also shakes once.
    pub fn toast(&mut self, rect: Rect, bad: bool, frame: Rect) {
        if self.motion != Movement::Full {
            return;
        }
        let slide = fx::fade_from_fg(self.bg, (200, Interpolation::QuadOut))
            .with_pattern(SweepPattern::right_to_left(10));
        let effect = if bad {
            fx::sequence(&[slide, shake(240)])
        } else {
            slide
        };
        // The shake moves cells a column either way: give it room.
        let room = Rect {
            x: rect.x.saturating_sub(1),
            width: rect.width + 2,
            ..rect
        }
        .intersection(frame);
        self.add(Slot::Toast, effect.with_area(room));
    }

    /// Advances the clock by the time since the last frame (never more than
    /// [`MAX_STEP`]; zero for the first frame of an effect).
    pub fn step_from_clock(&mut self) -> Wall {
        let now = Instant::now();
        let step = self
            .last
            .map_or(Wall::ZERO, |l| now.saturating_duration_since(l))
            .min(MAX_STEP);
        self.last = Some(now);
        step
    }

    /// Runs every effect over the drawn frame for `step` of time.
    pub fn process(&mut self, step: Wall, buf: &mut Buffer) {
        if !self.manager.is_running() {
            self.last = None;
            return;
        }
        let area = buf.area;
        self.manager
            .process_effects(Duration::from(step), buf, area);
        if self.tier == ColorTier::Ansi256 {
            quantize(buf);
        }
        if !self.manager.is_running() {
            self.last = None;
        }
    }
}

/// A short horizontal wobble: the cells of the area swap sides, a column at
/// a time, and settle.
fn shake(ms: u32) -> Effect {
    fx::effect_fn_buf((), (ms, Interpolation::Linear), |_, ctx, buf| {
        let alpha = ctx.alpha();
        // Five beats: left, right, left, right (smaller), rest.
        let offset: i32 = match (alpha * 5.0) as u32 {
            0 => -1,
            1 => 1,
            2 => -1,
            3 => 1,
            _ => 0,
        };
        if offset == 0 {
            return;
        }
        let area = ctx.area.intersection(buf.area);
        for y in area.top()..area.bottom() {
            let row: Vec<_> = (area.left()..area.right())
                .map(|x| buf[(x, y)].clone())
                .collect();
            let n = row.len() as i32;
            for (i, x) in (area.left()..area.right()).enumerate() {
                let from = i as i32 - offset;
                buf[(x, y)] = if (0..n).contains(&from) {
                    row[from as usize].clone()
                } else {
                    ratatui::buffer::Cell::default()
                };
            }
        }
    })
}

/// Effects blend in RGB; a 256-colour terminal gets the nearest index.
fn quantize(buf: &mut Buffer) {
    for cell in &mut buf.content {
        if let Color::Rgb(r, g, b) = cell.fg {
            cell.fg = Color::Indexed(nearest_xterm((r, g, b)));
        }
        if let Color::Rgb(r, g, b) = cell.bg {
            cell.bg = Color::Indexed(nearest_xterm((r, g, b)));
        }
    }
}
