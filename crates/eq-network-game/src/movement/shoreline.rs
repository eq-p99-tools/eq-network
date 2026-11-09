//! Local Titanium shoreline input. None of these fields are sent on the wire.

use super::{swim_speed, PROGRAM_STEP_SECONDS};
use std::time::Duration;

/// Native rearm interval and a conservative lifetime for a single local arc.
pub const SHORELINE_COOLDOWN: Duration = Duration::from_millis(1500);

/// Maximum number of renderer frames carried by one paced position sample.
pub const SHORELINE_FRAMES: usize = 32;

const PRESENT: u8 = 1;
const WATER: u8 = 2;
const STOP_VERTICAL: u8 = 4;

/// One stroke direction in EQ X/Y/Z order. A zero stroke releases input
/// without discarding carried velocity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShorelineStroke([i8; 3]);

impl ShorelineStroke {
    /// Quantize a normalized or shorter direction without increasing its norm.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn new(direction: [f32; 3]) -> Self {
        if !direction.iter().all(|value| value.is_finite()) {
            return Self::default();
        }
        let norm = direction[0]
            .hypot(direction[1])
            .hypot(direction[2])
            .max(1.0);
        Self(direction.map(|component| (component / norm * f32::from(i8::MAX)) as i8))
    }

    /// Whether the input has an upward component.
    #[must_use]
    pub const fn upward(self) -> bool {
        self.0[2] > 0
    }

    fn direction(self) -> [f32; 3] {
        self.0
            .map(|component| f32::from(component) / f32::from(i8::MAX))
    }
}

/// The renderer's swept frame, with durations in the native clock's milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShorelineFrame {
    millis: u8,
    flags: u8,
    stroke: ShorelineStroke,
}

impl ShorelineFrame {
    /// An unused trace slot.
    pub const EMPTY: Self = Self {
        millis: 0,
        flags: 0,
        stroke: ShorelineStroke([0; 3]),
    };

    /// A nonempty, bounded frame.
    #[must_use]
    pub fn new(
        millis: u8,
        water: bool,
        stop_vertical: bool,
        stroke: ShorelineStroke,
    ) -> Option<Self> {
        let medium = if water { WATER } else { 0 };
        let stop = if stop_vertical { STOP_VERTICAL } else { 0 };
        (millis <= 150).then_some(Self {
            millis,
            flags: PRESENT | medium | stop,
            stroke,
        })
    }

    /// Time consumed from the movement sample.
    #[must_use]
    pub const fn millis(self) -> u8 {
        self.millis
    }

    /// Frame duration for physics integration.
    #[must_use]
    pub fn seconds(self) -> f32 {
        f32::from(self.millis()) / 1000.0
    }

    /// Water at the actor origin before the frame's sweep.
    #[must_use]
    pub fn water(self) -> bool {
        self.flags & WATER != 0
    }

    /// Whether this frame contains a positive-pitch stroke.
    #[must_use]
    pub const fn upward(self) -> bool {
        self.stroke.upward()
    }

    /// Apply collision stops after consuming the frame.
    pub fn stop(self, velocity: &mut ShorelineVelocity) {
        if self.flags & STOP_VERTICAL != 0 {
            velocity.vertical = 0.0;
        }
    }
}

/// Per-sample record of the shoreline frames that survived local collision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShorelineTrace {
    /// Start one impulse from the last accepted swim sample.
    pub start: bool,
    /// End on supported land, on water reentry, or after collision stops motion.
    pub finish: bool,
    /// Nonempty slots followed by empty slots.
    pub frames: [ShorelineFrame; SHORELINE_FRAMES],
}

/// Carried velocity in Titanium's native position-update units.
#[derive(Clone, Copy, Debug)]
pub struct ShorelineVelocity {
    /// EQ X and Y.
    pub horizontal: [f32; 2],
    /// Upward positive.
    pub vertical: f32,
}

impl ShorelineVelocity {
    /// The ordinary forward/look-up shoreline impulse, eqgame 0x48a793.
    #[must_use]
    pub fn start(heading: f32) -> Self {
        let (x, y) = (heading / 512.0 * std::f32::consts::TAU).sin_cos();
        Self {
            horizontal: [0.5 * x, 0.5 * y],
            vertical: 3.0 * 2.5 * 1.35,
        }
    }

    /// Integrate one native-style update. Input uses the already admitted
    /// conservative stroke rate. Gravity is in EQ units per second squared.
    pub fn advance(&mut self, frame: ShorelineFrame, granted: f32, gravity: f32) {
        let stroke_speed = swim_speed(granted) * PROGRAM_STEP_SECONDS;
        self.vertical -= gravity * PROGRAM_STEP_SECONDS * frame.seconds();
        if frame.water() {
            // Ordinary Water (region 5), eqgame 0x4bfe0f.
            let [x, y, up] = frame.stroke.direction();
            self.horizontal[0] = self.horizontal[0] * 0.95 + x * stroke_speed * 0.025;
            self.horizontal[1] = self.horizontal[1] * 0.95 + y * stroke_speed * 0.025;
            self.vertical = self.vertical * 0.2 + up * stroke_speed * 0.8;
        } else {
            // The same routine uses the dry airborne damping at 0x64bac4.
            self.horizontal = self.horizontal.map(|component| component * 0.99);
            self.vertical *= 0.99;
        }
    }

    /// Pre-collision travel for this one frame in EQ units.
    #[must_use]
    pub fn travel(self, frame: ShorelineFrame) -> ([f32; 2], f32) {
        let steps = frame.seconds() / PROGRAM_STEP_SECONDS;
        (
            self.horizontal.map(|component| component * steps),
            self.vertical * steps,
        )
    }
}
