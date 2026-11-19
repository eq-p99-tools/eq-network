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

/// One generation's water: the set its `WT` regions take, a zone whose
/// water differs, and a zone with no air to breathe. These are facts of
/// the generation's own region parser and zone table, so they come from
/// [`crate::GameDialect::water`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterRules {
    /// The set of a `WT` region: level 5 (parser 0x435db5, read R41 a).
    pub water: WetCoefficients,
    /// A zone whose level 5 takes another set.
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
        self.zone_water
            .filter(|(id, _)| *id == zone)
            .map_or(self.water, |(_, set)| set)
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

impl WetVelocity {
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
