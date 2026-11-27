//! Titanium wet movement: the per-frame step `eqgame.exe` 0x4bfe0f gives a
//! body whose origin is in liquid, the swim speed 0x4a1d74 feeds it, and
//! the steady rates a session bounds it by (reads R33, R33b and R39).
//!
//! Velocities are the program's: world units per 50 ms tick. The program's
//! time step is the frame's milliseconds x 0.02 (0x4bfe0f), so a tick is
//! 50 ms; that "now" counts milliseconds is inferred, as for the land rate
//! ([`super::PROGRAM_STEP_SECONDS`]), whose 20 ticks a second match live
//! servers. Speeds are the program's inputs, the land speed's own units:
//! on normal ground an input `s` settles at `0.8 / 0.375 x s` a tick.
//!
//! The program damps once per frame, with no time step, so its damping
//! depends on the frame rate. This client steps water at a fixed 60 frames
//! a second ([`WET_FRAME_SECONDS`]) and carries the frame phase from sample
//! to sample, so the velocity changes only at whole frames and a sample's
//! travel doesn't depend on how the time is cut up (recording checklist
//! 6.483). A jump's
//! push comes before the frame's step (0x4aba14, read R41 b), so the step
//! damps it; that a skipped step can take two pushes is not modelled.
use std::time::Duration;

/// Inert numerical criteria for a future owned Titanium wet-move step.
pub mod titanium_move;

/// One liquid's factors in the step (0x4bfe0f, read R33 section 2).
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct WetCoefficients {
    /// The share of the horizontal input added each frame (`c`).
    pub horizontal_input: f32,
    /// The share of the vertical input added each frame (`k`).
    pub vertical_input: f32,
    /// The share of horizontal velocity kept each frame (`h`).
    pub horizontal_keep: f32,
    /// The share of vertical velocity kept each frame (`v`).
    pub vertical_keep: f32,
}

/// Feet level 5, and 9 and 10 (0x4bfe0f, read R33).
pub const LIQUID_5: WetCoefficients = WetCoefficients {
    horizontal_input: 0.025,
    vertical_input: 0.8,
    horizontal_keep: 0.95,
    vertical_keep: 0.2,
};

/// Feet levels 6, 7 and 8 (0x4bfe0f, read R33).
pub const LIQUID_6_TO_8: WetCoefficients = WetCoefficients {
    horizontal_input: 0.02,
    vertical_input: 0.8,
    horizontal_keep: 0.94,
    vertical_keep: 0.15,
};

/// Feet level 5 in poearthb, zone 222 (0x4bfe0f, read R33; the zone's
/// name from the zone table, read R41 a).
pub const LIQUID_5_POEARTHB: WetCoefficients = WetCoefficients {
    horizontal_input: 0.02,
    vertical_input: 0.1,
    horizontal_keep: 0.85,
    vertical_keep: 0.1,
};

/// Which of the step's two sets a wet region takes. Titanium's region
/// classifier (0x435ec9, read R49-26) gives `WT` 5, `SL` 6, `LA` 7, `VW` 8,
/// `W2` 9 and `W3` 10; the step (0x4bfe0f, read R33) moves 5, 9 and 10 by
/// one set and 6, 7 and 8 by a slower one.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum WetKind {
    /// Kinds 5, 9 and 10: `WT`, `W2` and `W3`.
    #[default]
    Normal,
    /// Kinds 6, 7 and 8: `SL`, `LA` and `VW`.
    Slower,
}

/// One generation's water: the set its `WT` regions take, the set its
/// slower water takes, a zone whose water differs, and a zone with no air
/// to breathe. These are facts of
/// the generation's own region parser and zone table, so they come from
/// [`crate::GameDialect::water`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterRules {
    /// The set of a `WT` region: level 5 (parser 0x435db5, read R41 a).
    pub water: WetCoefficients,
    /// The set of `SL`, `LA` and `VW` regions: levels 6 to 8 (0x4bfe0f,
    /// read R33; kinds read R49-26).
    pub slower: WetCoefficients,
    /// A zone whose level 5 takes another set. Levels 6 to 8 keep theirs
    /// there (read R49-26).
    pub zone_water: Option<(u16, WetCoefficients)>,
    /// A zone whose air can't be breathed: there a head out of water
    /// breathes only by race or effect 14 (0x41a753 / 0x5f6c40, zone flag
    /// 0x4000; read R41 d).
    pub airless_zone: Option<u16>,
}

impl WaterRules {
    /// The set a `WT` region takes in `zone`.
    #[must_use]
    pub fn set(&self, zone: u16) -> WetCoefficients {
        self.set_for(zone, WetKind::Normal)
    }

    /// The set a region of this kind takes in `zone`.
    #[must_use]
    pub fn set_for(&self, zone: u16, kind: WetKind) -> WetCoefficients {
        match kind {
            WetKind::Normal => self
                .zone_water
                .filter(|(id, _)| *id == zone)
                .map_or(self.water, |(_, set)| set),
            WetKind::Slower => self.slower,
        }
    }

    /// Whether `zone` has no air to breathe.
    #[must_use]
    pub fn airless(&self, zone: u16) -> bool {
        self.airless_zone == Some(zone)
    }
}

/// Titanium's water: poearthb (222) has its own level-5 set and only
/// thegrey (171) carries the no-air flag in the zone table (0x5f7000, read
/// R41 a and d).
pub const TITANIUM_WATER: WaterRules = WaterRules {
    water: LIQUID_5,
    slower: LIQUID_6_TO_8,
    zone_water: Some((222, LIQUID_5_POEARTHB)),
    airless_zone: Some(171),
};

/// Gravity in the step, in water as on land: world units a tick, each
/// tick (`[0x923b84]` = 0.4, set at 0x4a3341; read R33). A spawn whose
/// gravity behaviour is water's skips it; a player's is not.
pub const WET_GRAVITY: f32 = 0.4;

/// What each point of the Swimming skill adds to a standing swimmer's
/// speed (0x4a1d74, `[0x640fa0]` = 0.01; read R33 section 3).
pub const SKILL_SPEED: f32 = 0.01;

/// The Swimming skill's number.
pub const SWIMMING_SKILL: usize = 50;

/// The slowest a swimmer who isn't rooted moves (0x4a1d74, read R33).
pub const SLOWEST_SWIM: f32 = 0.01;

/// What a jump in water adds to the vertical velocity each frame while it
/// lasts (0x4aba14, `[0x6410bc]` = 0.5; read R39 B).
pub const JUMP_PUSH: f32 = 0.5;

/// How long a jump in water pushes (0x4aba14, read R39 B).
pub const JUMP_PUSH_DURATION: Duration = Duration::from_millis(500);

/// This client's water frame, in seconds: the program steps once a frame,
/// here 60 frames a second (inferred, recording checklist 6.483).
pub const WET_FRAME_SECONDS: f32 = 1.0 / 60.0;

/// Ticks in one water frame.
pub const WET_FRAME_TICKS: f32 = WET_FRAME_SECONDS / super::PROGRAM_STEP_SECONDS;

const TICKS_PER_SECOND: f32 = 1.0 / super::PROGRAM_STEP_SECONDS;

/// How near a frame boundary counts as on it, in seconds.
const BOUNDARY: f32 = 1e-5;

/// Velocity components the program zeroes (0x4bfe0f, read R33 step 6).
const NEGLIGIBLE: f32 = 1.19e-7;

/// A standing swimmer's forward speed: the land speed, `base x factor +
/// modifier` ([`super::titanium_speeds`]), plus the Swimming skill, at
/// least [`SLOWEST_SWIM`]. An unknown skill adds nothing.
#[must_use]
pub fn swim_input(land_input: f32, skill: Option<u32>) -> f32 {
    #[allow(clippy::cast_precision_loss)] // Skills are small.
    let skill = skill.map_or(0.0, |skill| skill.min(1_000) as f32);
    (land_input + skill * SKILL_SPEED).max(SLOWEST_SWIM)
}

/// One frame's input, in program speed units: the horizontal input along
/// the body's heading and sideways, and the vertical input, which comes
/// from the forward speed alone, times the sine of the look pitch (R39 C).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WetInput {
    /// Horizontal input, in the caller's two horizontal axes.
    pub horizontal: [f32; 2],
    /// Vertical input, up positive.
    pub vertical: f32,
    /// Whether a jump pushes up this frame.
    pub jump: bool,
}

/// The fastest a wet body moves on each axis, in world units a second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WetCaps {
    /// Horizontal speed.
    pub horizontal: f32,
    /// Rising speed.
    pub up: f32,
    /// Sinking speed.
    pub down: f32,
}

impl WetCaps {
    /// The caps for a swimmer whose largest horizontal input is
    /// `horizontal_input` and largest vertical input `vertical_input`.
    ///
    /// The velocity only changes in whole frames, so from any start within
    /// these it stays within them, and a sample's travel stays within them
    /// times its seconds however the samples cut the time. A frame takes the
    /// push and gravity, damps, then adds the input, so each axis settles at
    /// `(keep x (push or gravity) + input scale x input) / (1 - keep)`.
    #[must_use]
    pub fn new(set: WetCoefficients, horizontal_input: f32, vertical_input: f32) -> Self {
        let keep = set.vertical_keep;
        let horizontal =
            set.horizontal_input * horizontal_input.max(0.0) / (1.0 - set.horizontal_keep);
        let stroke = set.vertical_input * vertical_input.max(0.0) / (1.0 - keep);
        let push = keep * JUMP_PUSH / (1.0 - keep);
        let gravity = keep * WET_GRAVITY * WET_FRAME_TICKS / (1.0 - keep);
        Self {
            horizontal: horizontal * TICKS_PER_SECOND,
            up: (stroke + push) * TICKS_PER_SECOND,
            down: (stroke + gravity) * TICKS_PER_SECOND,
        }
    }

    /// The larger of two caps on each axis, for a swimmer who may be in
    /// either set's water.
    #[must_use]
    pub fn covering(self, other: Self) -> Self {
        Self {
            horizontal: self.horizontal.max(other.horizontal),
            up: self.up.max(other.up),
            down: self.down.max(other.down),
        }
    }
}

/// A wet body's velocity between frames, in world units a tick, and the
/// time left until its next frame step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WetVelocity {
    horizontal: [f32; 2],
    vertical: f32,
    /// Seconds until the next frame steps the velocity; none due yet means
    /// the next call steps at once.
    due: f32,
}

/// The factors for a due water-clock frame (Titanium `0x4bfe0f`).
///
/// Air is the unsupported dry part of a surface swim: gravity and 0.99
/// damping, without new input. It does not model ordinary jumps or falls.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum WetFrameMedium {
    /// A resolved liquid's existing wet factors.
    Water(WetCoefficients),
    /// Unsupported air, with no new input or jump push.
    Air,
}

/// A read-only view of a velocity's existing carried frame phase.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WetFramePhase {
    /// Seconds left in the current frame, zero when a frame is due.
    /// The existing wet clock treats values within its rounding window as
    /// a boundary; inspect [`Self::at_boundary`] for that decision.
    pub remaining_seconds: f32,
    /// Whether the next sufficiently long piece starts a new frame.
    pub at_boundary: bool,
}

/// One predicted physical piece, ending no later than a frame boundary.
///
/// This grants no movement. The caller must give the travel to its collision
/// owner, refresh the medium only after a completed frame moves legally, and
/// discard the entire trial on an unqualified move. Clone the velocity before
/// planning; only commit the trial under the application's acceptance rules.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WetFramePiece {
    /// Time actually consumed by this piece, at most the requested time.
    pub seconds: f32,
    /// Horizontal travel in the input's axes, in world units.
    pub horizontal: [f32; 2],
    /// Vertical travel, up positive, in world units.
    pub vertical: f32,
    /// Whether this piece applied a new full frame's velocity update.
    pub starts_frame: bool,
    /// Whether its move completes a frame, using the existing clock's
    /// boundary rounding window. Only then may the caller refresh medium.
    pub completes_frame: bool,
}

impl WetVelocity {
    /// Internal domain for an already-owned native ordinary-call history.
    /// A canonical clock followed by one rounding-size tail can be overdue
    /// by at most two boundary windows. The next normal call consumes that
    /// exact debt using `advance`; nothing resets or reconstructs its phase.
    /// This predicate grants no authority to an externally supplied velocity.
    pub(crate) fn has_ordinary_call_clock(self) -> bool {
        self.due.is_finite()
            && self.due >= -2.0 * BOUNDARY
            && self.due <= WET_FRAME_SECONDS + BOUNDARY
            && self
                .horizontal
                .iter()
                .chain(std::iter::once(&self.vertical))
                .all(|value| value.is_finite())
    }

    /// Views the carried phase without reconstructing it from position or
    /// elapsed request time. A copied [`WetVelocity`] retains this phase.
    ///
    /// Returns `None` if the stored phase cannot describe one frame. In
    /// particular, repeated sub-rounding-window calls to the legacy
    /// [`Self::advance`] can leave an overdue phase. A continuation must
    /// decline that one-frame view rather than fabricate a replacement phase.
    /// An internal replay of a complete owned ordinary-call history instead
    /// retains the raw native clock and never converts its debt to this view.
    #[must_use]
    pub fn frame_phase(self) -> Option<WetFramePhase> {
        (self.due.is_finite() && self.due >= -BOUNDARY && self.due <= WET_FRAME_SECONDS + BOUNDARY)
            .then_some(WetFramePhase {
                remaining_seconds: self.due.max(0.0),
                at_boundary: self.due <= BOUNDARY,
            })
    }

    /// Plans at most one physical piece on this velocity's existing 60 Hz
    /// frame clock, across resolved water and unsupported air.
    ///
    /// A due frame updates velocity once, with full-frame gravity, damping
    /// and input. A piece ending inside it only integrates the current
    /// velocity; it does not apply a fractional update or refresh medium.
    /// Water uses the same private step as [`Self::advance`]; air uses the
    /// `0x4bfe0f` dry factors, without its unread animation clear. The fixed
    /// 60 Hz choice remains inferred, as for [`WET_FRAME_SECONDS`].
    ///
    /// The caller supplies the stored medium until `completes_frame`, then
    /// observes the collision owner's endpoint for the next frame. Changing
    /// input or medium between pieces cannot update an already-started
    /// frame. This method does not clamp, reset, collide, send, or retry.
    ///
    /// A remainder within the existing clock's rounding window integrates
    /// the current velocity without another update, exactly as
    /// [`Self::advance`] does. It does not complete a new physical frame.
    /// Returns `None`, without mutation, for invalid time or unknown phase.
    /// The caller must decline such a continuation.
    pub fn advance_piece(
        &mut self,
        medium: WetFrameMedium,
        input: WetInput,
        seconds: f32,
    ) -> Option<WetFramePiece> {
        let phase = self.frame_phase()?;
        if !(seconds.is_finite() && seconds > 0.0) {
            return None;
        }
        let starts_frame = phase.at_boundary && seconds > BOUNDARY;
        if starts_frame {
            match medium {
                WetFrameMedium::Water(set) => self.step(set, input),
                WetFrameMedium::Air => self.air_frame(),
            }
            self.due += WET_FRAME_SECONDS;
        }
        let active_frame = self.due > BOUNDARY;
        let run = if active_frame {
            self.due.min(seconds)
        } else {
            seconds
        };
        let ticks = run * TICKS_PER_SECOND;
        self.due -= run;
        Some(WetFramePiece {
            seconds: run,
            horizontal: self.horizontal.map(|value| value * ticks),
            vertical: self.vertical * ticks,
            starts_frame,
            completes_frame: active_frame && self.due <= BOUNDARY,
        })
    }

    /// A velocity given in world units a second.
    #[must_use]
    pub fn from_units_per_second(horizontal: [f32; 2], vertical: f32) -> Self {
        Self {
            horizontal: horizontal.map(|v| v / TICKS_PER_SECOND),
            vertical: vertical / TICKS_PER_SECOND,
            due: 0.0,
        }
    }

    /// The velocity in world units a second: horizontal, then vertical.
    #[must_use]
    pub fn units_per_second(self) -> ([f32; 2], f32) {
        (
            self.horizontal.map(|v| v * TICKS_PER_SECOND),
            self.vertical * TICKS_PER_SECOND,
        )
    }

    /// Native vertical velocity in world units per program tick. This is an
    /// owned-state observation, not a setter or evidence of movement authority.
    #[must_use]
    pub const fn vertical_per_tick(self) -> f32 {
        self.vertical
    }

    /// Rotates already-owned horizontal axes by one quarter turn, retaining
    /// vertical velocity and phase. This is an axis conversion, not a new seed.
    #[must_use]
    pub const fn quarter_turn(self, clockwise: bool) -> Self {
        let [x, y] = self.horizontal;
        Self {
            horizontal: if clockwise { [y, -x] } else { [-y, x] },
            ..self
        }
    }

    /// Holds each axis within `caps`: on entering water, where the program
    /// keeps the land or fall velocity and damps it (the slower choice,
    /// recording checklist 6.483), and when the caps drop.
    pub fn clamp(&mut self, caps: WetCaps) {
        let limit = caps.horizontal / TICKS_PER_SECOND;
        let length = self.horizontal[0].hypot(self.horizontal[1]);
        if length > limit {
            let scale = if length > 0.0 { limit / length } else { 0.0 };
            self.horizontal = self.horizontal.map(|v| v * scale);
        }
        self.vertical = self
            .vertical
            .clamp(-caps.down / TICKS_PER_SECOND, caps.up / TICKS_PER_SECOND);
    }

    /// What a sweep that couldn't move the body down does to its
    /// velocity: a fall faster than one unit a tick is cut to one, and
    /// nothing else changes (0x48acef; read R41 c). `achieved_down` is how
    /// far the sweep actually moved the body down.
    pub fn floor_blocked(&mut self, achieved_down: f32) {
        if achieved_down < 0.001 && self.vertical < -1.0 {
            self.vertical = -1.0;
        }
    }

    /// Zeros only native vertical velocity, retaining horizontal carry and the
    /// existing integrator phase. Used by a qualified collision owner's floor
    /// snap or saved-good revert; this does not qualify a pose or create rest.
    pub fn zero_vertical(&mut self) {
        self.vertical = 0.0;
    }

    /// The absolute-height native fast-sink clamp. Its ordinary, not-refused,
    /// nonstationary branch must be established by the collision owner first.
    /// Legacy `floor_blocked` callers retain their existing behavior.
    pub fn clamp_blocked_sink(&mut self, achieved_height: f32) {
        if titanium_move::tiny_height_fast_sink(achieved_height, self.vertical) == Some(true) {
            self.vertical = -1.0;
        }
    }

    /// Stops all motion, as a put-back or an out-of-bounds move does
    /// (0x48acef; read R41 c, which zeroes vz and keeps vx and vy on a
    /// put-back; stopping it all is this client's correction rule).
    pub fn stop(&mut self) {
        *self = Self::default();
    }

    /// Runs `seconds` of water and returns the travel in world units:
    /// horizontal, then vertical. The velocity changes only at whole
    /// [`WET_FRAME_SECONDS`] frames, the phase carried from call to call, so
    /// the result doesn't depend on how the time is cut up. Each frame takes
    /// the push, then gravity, then damps, then adds the scaled input, and
    /// the body moves at that velocity until the next frame (0x4bfe0f, R33
    /// step order).
    pub fn advance(
        &mut self,
        set: WetCoefficients,
        input: WetInput,
        seconds: f32,
    ) -> ([f32; 2], f32) {
        let mut travel = ([0.0; 2], 0.0);
        if !(seconds.is_finite() && seconds > 0.0) {
            return travel;
        }
        let mut left = seconds;
        while left > 0.0 {
            // A boundary within float rounding of the end is the next
            // call's, so cutting the same time differently steps the same
            // frames.
            if self.due <= BOUNDARY && left > BOUNDARY {
                self.step(set, input);
                self.due += WET_FRAME_SECONDS;
            }
            let run = if self.due > BOUNDARY {
                self.due.min(left)
            } else {
                left
            };
            let ticks = run * TICKS_PER_SECOND;
            travel.0[0] += self.horizontal[0] * ticks;
            travel.0[1] += self.horizontal[1] * ticks;
            travel.1 += self.vertical * ticks;
            left -= run;
            self.due -= run;
        }
        travel
    }

    /// Unsupported dry surface-swim frame (`0x4bfe0f`): no new input.
    fn air_frame(&mut self) {
        self.vertical -= WET_GRAVITY * WET_FRAME_TICKS;
        self.horizontal = self.horizontal.map(|value| value * 0.99);
        self.vertical *= 0.99;
        for value in self
            .horizontal
            .iter_mut()
            .chain(std::iter::once(&mut self.vertical))
        {
            if value.abs() < NEGLIGIBLE {
                *value = 0.0;
            }
        }
    }

    /// One whole frame of 0x4bfe0f's wet branch.
    fn step(&mut self, set: WetCoefficients, input: WetInput) {
        if input.jump {
            self.vertical += JUMP_PUSH;
        }
        self.vertical -= WET_GRAVITY * WET_FRAME_TICKS;
        self.horizontal = self.horizontal.map(|v| v * set.horizontal_keep);
        self.vertical *= set.vertical_keep;
        for value in self
            .horizontal
            .iter_mut()
            .chain(std::iter::once(&mut self.vertical))
        {
            if value.abs() < NEGLIGIBLE {
                *value = 0.0;
            }
        }
        self.horizontal[0] += set.horizontal_input * input.horizontal[0];
        self.horizontal[1] += set.horizontal_input * input.horizontal[1];
        self.vertical += set.vertical_input * input.vertical;
    }
}

#[cfg(test)]
#[path = "water/frame_pieces.rs"]
mod frame_piece_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(set: WetCoefficients, input: WetInput, seconds: f32) -> WetVelocity {
        let mut velocity = WetVelocity::default();
        velocity.advance(set, input, seconds);
        velocity
    }

    fn close(a: f32, b: f32, tolerance: f32) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn level_swimming_settles_at_the_programs_steady_rates() {
        let s = 1.7;
        let level = WetInput {
            horizontal: [s, 0.0],
            ..WetInput::default()
        };
        let ([x, _], z) = settle(LIQUID_6_TO_8, level, 10.0).units_per_second();
        // 0.02 / 0.06 x 20 = 6.67 s; the idle sink at a third of a tick.
        assert!(close(x, 6.667 * s, 0.01), "{x}");
        assert!(
            close(z, -0.4 * WET_FRAME_TICKS * 0.15 / 0.85 * 20.0, 0.001),
            "{z}"
        );
        assert!(close(z, -0.47, 0.01), "{z}");
        let ([x, _], z) = settle(LIQUID_5, level, 10.0).units_per_second();
        assert!(close(x, 10.0 * s, 0.01), "{x}");
        assert!(close(z, -0.667, 0.01), "{z}");
        // Land: 0.8 / 0.375 x 20, the session's units per input.
        assert!(close(
            super::super::speeds::EQMAC_UNITS_PER_INPUT,
            42.667,
            0.001
        ));
    }

    #[test]
    fn looking_straight_up_keeps_the_horizontal_rate_and_adds_the_rise() {
        let s = 1.7;
        let up = WetInput {
            horizontal: [s, 0.0],
            vertical: s,
            jump: false,
        };
        let ([x, _], z) = settle(LIQUID_6_TO_8, up, 10.0).units_per_second();
        assert!(close(x, 11.33, 0.01), "{x}");
        assert!(close(z, 18.82 * s - 0.47, 0.02), "{z}");
        let ([_, _], z) = settle(LIQUID_5, up, 10.0).units_per_second();
        assert!(close(z, 34.0 - 0.667, 0.02), "{z}");
    }

    #[test]
    fn a_held_jump_pushes_at_the_damped_rate() {
        let jump = WetInput {
            jump: true,
            ..WetInput::default()
        };
        let (_, z) = settle(LIQUID_6_TO_8, jump, 10.0).units_per_second();
        // 0.5 x 0.15 / 0.85 x 20 = 1.76, less the idle sink.
        assert!(close(z, 1.765 - 0.47, 0.01), "{z}");
        let (_, z) = settle(LIQUID_5, jump, 10.0).units_per_second();
        assert!(close(z, 2.5 - 0.667, 0.01), "{z}");
    }

    #[test]
    fn the_skill_adds_to_a_standing_swimmer_and_unknown_adds_nothing() {
        assert!(close(swim_input(0.7, Some(100)), 1.7, 1e-6));
        assert!(close(swim_input(0.7, None), 0.7, 1e-6));
        assert!(close(swim_input(-3.0, Some(10)), SLOWEST_SWIM, 1e-6));
        assert_eq!(TITANIUM_WATER.set(222), LIQUID_5_POEARTHB);
        assert_eq!(TITANIUM_WATER.set(1), LIQUID_5);
        assert!(TITANIUM_WATER.airless(171) && !TITANIUM_WATER.airless(1));
    }

    #[test]
    fn slower_water_keeps_its_set_in_poearthb_and_the_covering_caps_hold_both() {
        assert_eq!(TITANIUM_WATER.set_for(1, WetKind::Normal), LIQUID_5);
        assert_eq!(TITANIUM_WATER.set_for(1, WetKind::Slower), LIQUID_6_TO_8);
        assert_eq!(
            TITANIUM_WATER.set_for(222, WetKind::Normal),
            LIQUID_5_POEARTHB
        );
        // Zone 222's own set is level 5's only (read R49-26).
        assert_eq!(TITANIUM_WATER.set_for(222, WetKind::Slower), LIQUID_6_TO_8);
        let speed = 1.7;
        let level = WetInput {
            horizontal: [0.0, speed],
            ..WetInput::default()
        };
        // Slower water settles at two thirds of WT's level speed.
        let ([_, slow], _) = settle(LIQUID_6_TO_8, level, 10.0).units_per_second();
        let ([_, normal], _) = settle(LIQUID_5, level, 10.0).units_per_second();
        assert!(close(slow, 20.0 / 3.0 * speed, 0.01), "{slow}");
        assert!(close(normal, 10.0 * speed, 0.01), "{normal}");
        for normal in [LIQUID_5, LIQUID_5_POEARTHB] {
            let caps = WetCaps::new(normal, speed, speed).covering(WetCaps::new(
                LIQUID_6_TO_8,
                speed,
                speed,
            ));
            for set in [normal, LIQUID_6_TO_8] {
                let own = WetCaps::new(set, speed, speed);
                assert!(
                    own.horizontal <= caps.horizontal && own.up <= caps.up && own.down <= caps.down
                );
            }
        }
    }

    #[test]
    fn the_same_time_cut_up_differently_ends_the_same_and_stays_within_the_caps() {
        let speed = 1.7;
        let inputs = [
            ("idle", WetInput::default()),
            (
                "jump",
                WetInput {
                    jump: true,
                    ..WetInput::default()
                },
            ),
            (
                "up",
                WetInput {
                    horizontal: [speed * 0.6, speed * 0.8],
                    vertical: speed,
                    jump: true,
                },
            ),
            (
                "level",
                WetInput {
                    horizontal: [0.0, speed],
                    ..WetInput::default()
                },
            ),
            (
                "down",
                WetInput {
                    horizontal: [0.0, speed],
                    vertical: -speed,
                    jump: false,
                },
            ),
        ];
        let total = 2.0_f32;
        for set in [LIQUID_5, LIQUID_6_TO_8, LIQUID_5_POEARTHB] {
            let caps = WetCaps::new(set, speed, speed);
            for (name, input) in inputs {
                let mut whole = WetVelocity::default();
                let reference = whole.advance(set, input, total);
                for cut in [0.0042_f32, 0.0069, 1.0 / 60.0, 1.0 / 30.0, 0.04, 0.1, 0.3] {
                    let mut velocity = WetVelocity::default();
                    let (mut east, mut north, mut rise) = (0.0, 0.0, 0.0);
                    let mut left = total;
                    while left > 1e-6 {
                        let seconds = cut.min(left);
                        let (h, v) = velocity.advance(set, input, seconds);
                        assert!(
                            h[0].hypot(h[1]) <= caps.horizontal * seconds + 1e-4,
                            "{name} {cut}"
                        );
                        assert!(v <= caps.up * seconds + 1e-4, "{name} {cut} {v}");
                        assert!(-v <= caps.down * seconds + 1e-4, "{name} {cut} {v}");
                        east += h[0];
                        north += h[1];
                        rise += v;
                        left -= seconds;
                    }
                    let tolerance = 0.002 * (1.0 + reference.0[1].abs() + reference.1.abs());
                    assert!(
                        close(east, reference.0[0], tolerance),
                        "{name} {cut} {east}"
                    );
                    assert!(
                        close(north, reference.0[1], tolerance),
                        "{name} {cut} {north}"
                    );
                    assert!(close(rise, reference.1, tolerance), "{name} {cut} {rise}");
                    let (h, v) = velocity.units_per_second();
                    let (rh, rv) = whole.units_per_second();
                    assert!(
                        close(h[1], rh[1], 1e-3) && close(v, rv, 1e-3),
                        "{name} {cut} {h:?} {v} vs {rh:?} {rv}"
                    );
                }
            }
        }
    }

    #[test]
    fn entry_velocity_is_clamped_and_a_blocked_axis_keeps_what_it_achieved() {
        let caps = WetCaps::new(LIQUID_6_TO_8, 1.0, 1.0);
        let mut fall = WetVelocity::from_units_per_second([30.0, 0.0], -264.0);
        fall.clamp(caps);
        let ([x, _], z) = fall.units_per_second();
        assert!(close(x, caps.horizontal, 1e-3));
        assert!(close(z, -caps.down, 1e-3));
        // On the bottom, only a fall faster than a unit a tick is cut.
        let mut sinking = WetVelocity::from_units_per_second([6.0, 0.0], -40.0);
        sinking.floor_blocked(0.0);
        assert_eq!(sinking.units_per_second(), ([6.0, 0.0], -20.0));
        let mut slow = WetVelocity::from_units_per_second([6.0, 0.0], -0.5);
        slow.floor_blocked(0.0);
        assert_eq!(slow.units_per_second(), ([6.0, 0.0], -0.5));
    }

    #[test]
    fn standing_still_or_strafing_gives_no_vertical_input() {
        // The input itself carries none; only gravity moves the body.
        let idle = settle(LIQUID_6_TO_8, WetInput::default(), 5.0);
        let strafe = settle(
            LIQUID_6_TO_8,
            WetInput {
                horizontal: [0.0, 0.7],
                ..WetInput::default()
            },
            5.0,
        );
        assert_eq!(idle.units_per_second().1, strafe.units_per_second().1);
    }
}
