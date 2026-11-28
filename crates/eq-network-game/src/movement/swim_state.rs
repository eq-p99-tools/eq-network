//! Reset-anchored validation of the actual ordinary water integrator calls.
use super::{water, MotionCalibration, Pace, StrafeSide};
use anyhow::{ensure, Result};
use std::time::{Duration, Instant};

/// An opaque accepted-state receipt, issued only by the motion owner.
///
/// A client must independently match the real admission/server reset before
/// using the first receipt, then retain every ordinary control piece. Merely
/// obtaining a receipt is not evidence that the client matched that reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwimBasis {
    session: u64,
    revision: u64,
    started: Instant,
    reset: bool,
}

impl SwimBasis {
    /// Timestamp of the accepted/reset basis. Applied observations retain
    /// actual model consumption; they never invent calls during a wall gap.
    #[must_use]
    pub const fn started_at(self) -> Instant {
        self.started
    }

    /// Whether this unconsumed receipt comes from a real admission/correction.
    /// It is never inferred from a position, grant, local pad or input report.
    #[must_use]
    pub const fn is_reset(self) -> bool {
        self.reset
    }
}

/// Normal horizontal controls. Rates always come from the motion owner's grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SwimInput {
    /// Released horizontal controls; only earned momentum remains.
    Idle,
    /// Forward at the selected ordinary pace.
    Forward(Pace),
    /// Backward at the separately calibrated backward rate, including zero.
    Backward,
    /// A pure sidestep at its existing selected rate.
    Strafe(StrafeSide, Pace),
    /// Normalized forward and sideways water input, using the forward rate.
    ForwardStrafe(StrafeSide, Pace),
}

/// One observed control piece, ending at a time relative to its basis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwimControl {
    until: Duration,
    heading: f32,
    kind: water::WetKind,
    input: SwimInput,
    applied_seconds: Option<f32>,
    pitch: f32,
    observed_at: Option<Instant>,
    stall: Option<(Instant, Instant)>,
}

impl SwimControl {
    /// Observes an actual ordinary digital control piece. Horizontal rates
    /// come entirely from calibration and the named input/pace. Native look
    /// pitch adds vertical input without reducing this horizontal input.
    ///
    /// # Errors
    /// Rejects noncanonical headings.
    pub fn new(
        until: Duration,
        heading: f32,
        kind: water::WetKind,
        input: SwimInput,
    ) -> Result<Self> {
        ensure!(
            (0.0..512.0).contains(&heading),
            "invalid swim control heading"
        );
        Ok(Self {
            until,
            heading,
            kind,
            input,
            applied_seconds: None,
            pitch: 0.0,
            observed_at: None,
            stall: None,
        })
    }

    /// Captures one actual call to the ordinary water integrator, retaining its
    /// exact f32 duration. `until` is the cumulative consumed model duration,
    /// not a fitted request-wall-time interval. No state or rate is supplied.
    ///
    /// # Errors
    /// Rejects invalid duration or heading. The containing observation also
    /// checks that all cumulative boundaries match the actual call durations.
    pub fn applied(
        until: Duration,
        heading: f32,
        kind: water::WetKind,
        input: SwimInput,
        seconds: f32,
    ) -> Result<Self> {
        ensure!(
            seconds.is_finite() && seconds > 0.0 && seconds <= 0.25,
            "invalid applied swim duration"
        );
        let mut piece = Self::new(until, heading, kind, input)?;
        piece.applied_seconds = Some(seconds);
        Ok(piece)
    }

    /// Records one actual completed physical frame, including its own facing
    /// and forward look pitch. Pure strafe/idle/backward add no pitch input.
    /// # Errors
    /// Rejects unobserved, overlong or invalid controls.
    pub fn physical(
        until: Duration,
        heading: f32,
        kind: water::WetKind,
        input: SwimInput,
        seconds: f32,
        pitch: f32,
        observed_at: Instant,
    ) -> Result<Self> {
        ensure!(
            seconds > 1e-5
                && seconds <= 0.15
                && pitch.is_finite()
                && pitch.abs() <= std::f32::consts::FRAC_PI_2,
            "invalid physical swim frame"
        );
        let mut piece = Self::applied(until, heading, kind, input, seconds)?;
        piece.pitch = pitch;
        piece.observed_at = Some(observed_at);
        Ok(piece)
    }

    /// Marks the first physical call after a render stall during which the
    /// producer held its scene state and applied no motion. Only the call
    /// between `resumed_at` and the observed end contributes to travel.
    ///
    /// # Errors
    /// Rejects a missing physical clock or a duration different from the
    /// observed call. Replay checks the stall against the preceding frame.
    pub fn after_stall(mut self, stopped_at: Instant, resumed_at: Instant) -> Result<Self> {
        ensure!(
            resumed_at
                .saturating_duration_since(stopped_at)
                .as_secs_f32()
                > 0.15,
            "physical swim stall must span an overlong frame"
        );
        let duration = self
            .observed_at
            .and_then(|end| end.checked_duration_since(resumed_at));
        ensure!(
            duration.is_some_and(
                |d| self.applied_seconds.map(f32::to_bits) == Some(d.as_secs_f32().to_bits())
            ),
            "invalid resumed physical swim clock"
        );
        self.stall = Some((stopped_at, resumed_at));
        Ok(self)
    }

    fn native_input(self, calibration: MotionCalibration) -> Result<water::WetInput> {
        let swim = calibration
            .swim
            .ok_or_else(|| anyhow::anyhow!("missing swim calibration"))?;
        let forward = match self.input {
            SwimInput::Forward(Pace::Run) | SwimInput::ForwardStrafe(_, Pace::Run) => swim.forward,
            SwimInput::Forward(Pace::Walk) | SwimInput::ForwardStrafe(_, Pace::Walk) => swim.walk,
            SwimInput::Idle | SwimInput::Backward | SwimInput::Strafe(..) => 0.0,
        };
        Ok(water::WetInput {
            horizontal: self.horizontal(calibration)?,
            vertical: forward * self.pitch.sin(),
            jump: false,
        })
    }

    fn horizontal(self, calibration: MotionCalibration) -> Result<[f32; 2]> {
        let swim = calibration
            .swim
            .ok_or_else(|| anyhow::anyhow!("water is not calibrated"))?;
        let forward = |pace| match pace {
            Pace::Run => swim.forward,
            Pace::Walk => swim.walk,
        };
        let side = |side| match side {
            StrafeSide::Left => 1.0,
            StrafeSide::Right => -1.0,
        };
        let (across, along) = match self.input {
            SwimInput::Idle => (0.0, 0.0),
            SwimInput::Forward(pace) => (0.0, forward(pace)),
            SwimInput::Backward => (0.0, -swim.backward),
            SwimInput::Strafe(which, pace) => {
                let strafe = calibration
                    .strafe
                    .ok_or_else(|| anyhow::anyhow!("strafe is not calibrated"))?;
                let selected = match pace {
                    Pace::Run => strafe.units_per_second,
                    Pace::Walk => strafe
                        .walk_units_per_second
                        .unwrap_or(strafe.units_per_second),
                };
                (
                    side(which) * swim.strafe * selected
                        / strafe.units_per_second.max(f32::EPSILON),
                    0.0,
                )
            }
            SwimInput::ForwardStrafe(which, pace) => {
                ensure!(calibration.strafe.is_some(), "strafe is not calibrated");
                let rate = forward(pace) * std::f32::consts::FRAC_1_SQRT_2;
                (side(which) * rate, rate)
            }
        };
        let (sin, cos) = (self.heading / 512.0 * std::f32::consts::TAU).sin_cos();
        Ok([sin * along + cos * across, cos * along - sin * across])
    }
}

/// Complete observed controls from one accepted basis, not a fitted path.
/// No entry seed, phase, reset flag, rate or ending velocity can be supplied.
#[derive(Clone, Debug, PartialEq)]
pub struct SwimControls {
    basis: SwimBasis,
    pieces: Vec<SwimControl>,
    pose: f32,
    applied: bool,
    scene: Option<super::SwimSceneHandle>,
}

impl SwimControls {
    /// Whether this is a complete physical-frame scene replay (diagnostic only).
    #[must_use]
    pub fn uses_scene(&self) -> bool {
        self.scene.is_some()
    }

    /// Retains up to 64 ordered positive pieces within the existing 250 ms
    /// maximum interval. More complex/uncertain input must use the old path.
    ///
    /// # Errors
    /// Rejects empty, unordered or overlong observations.
    pub fn new(basis: SwimBasis, pieces: Vec<SwimControl>) -> Result<Self> {
        ensure!(
            (1..=64).contains(&pieces.len()),
            "invalid swim control count"
        );
        let mut previous = Duration::ZERO;
        for piece in &pieces {
            ensure!(
                piece.until > previous && piece.until <= Duration::from_millis(250),
                "invalid swim control time"
            );
            previous = piece.until;
        }
        let pose = pieces
            .last()
            .ok_or_else(|| anyhow::anyhow!("empty swim controls"))?
            .heading;
        Ok(Self {
            basis,
            pieces,
            pose,
            applied: false,
            scene: None,
        })
    }

    /// Captures the full fixed slices and final remainder of one actual
    /// ordinary controller step, in the order the producer called them.
    /// The model may have paused while waiting; no unobserved idle is inserted.
    /// The independent final pose may follow a paced turn after those calls.
    /// Only a producer observing the actual calls may construct this ledger.
    ///
    /// # Errors
    /// Rejects an invalid final heading, missing durations or a partial/mixed
    /// model clock. Existing request wall-time and pose checks still apply.
    pub fn from_applied(basis: SwimBasis, pieces: Vec<SwimControl>, pose: f32) -> Result<Self> {
        ensure!((0.0..512.0).contains(&pose), "invalid applied swim pose");
        ensure!(
            (1..=64).contains(&pieces.len()),
            "invalid swim control count"
        );
        let mut previous = Duration::ZERO;
        for (index, piece) in pieces.iter().enumerate() {
            let seconds = piece
                .applied_seconds
                .ok_or_else(|| anyhow::anyhow!("missing applied swim duration"))?;
            // The actual ordinary producer calls full fixed slices followed
            // by at most one remainder. An arbitrary tiny-call tape is not
            // an observation of that producer and cannot establish its clock.
            ensure!(
                seconds <= water::WET_FRAME_SECONDS
                    && (index + 1 == pieces.len()
                        || seconds.to_bits() == water::WET_FRAME_SECONDS.to_bits()),
                "applied swim calls do not match the ordinary producer"
            );
            ensure!(
                piece.until.checked_sub(previous) == Some(Duration::from_secs_f32(seconds)),
                "incomplete applied swim clock"
            );
            previous = piece.until;
        }
        // Each f32 call is independently rounded to nanoseconds for the
        // cursor. At most 64 calls can contribute half a nanosecond each.
        ensure!(
            previous <= Duration::from_millis(250) + Duration::from_nanos(32),
            "applied swim interval exceeds cap"
        );
        Ok(Self {
            basis,
            pieces,
            pose,
            applied: true,
            scene: None,
        })
    }
    /// Retains complete observed physical frames and the actual immutable
    /// collision/body snapshot. Backend rates and state are replayed separately.
    /// No caller-supplied velocity, floor height or resolved delta is accepted.
    /// # Errors
    /// Rejects partial clocks, invalid durations and more than250ms of motion.
    pub fn from_physical_frames(
        basis: SwimBasis,
        pieces: Vec<SwimControl>,
        pose: f32,
        scene: super::SwimSceneHandle,
    ) -> Result<Self> {
        ensure!((0.0..512.0).contains(&pose), "invalid physical swim pose");
        let mut controls = Self::new(basis, pieces)?;
        let mut previous = Duration::ZERO;
        for piece in &controls.pieces {
            let seconds = piece
                .applied_seconds
                .ok_or_else(|| anyhow::anyhow!("missing physical frame clock"))?;
            ensure!(
                piece.observed_at.is_some()
                    && seconds > 1e-5
                    && seconds <= 0.15
                    && piece.until.checked_sub(previous) == Some(Duration::from_secs_f32(seconds)),
                "incomplete physical swim frames"
            );
            previous = piece.until;
        }
        controls.pose = pose;
        controls.applied = true;
        controls.scene = Some(scene);
        Ok(controls)
    }
}

#[derive(Clone, Debug)]
pub(super) struct Owner {
    revision: u64,
    known: Option<(water::WetVelocity, Instant)>,
    reset: bool,
    scene: Option<super::SwimSceneMotion>,
}

impl Owner {
    pub fn new(now: Instant) -> Self {
        Self {
            revision: 0,
            known: Some((water::WetVelocity::default(), now)),
            reset: true,
            scene: None,
        }
    }

    pub fn basis(&self, session: u64) -> Option<SwimBasis> {
        self.known.map(|(_, started)| SwimBasis {
            session,
            revision: self.revision,
            started,
            reset: self.reset,
        })
    }

    pub const fn invalidate(&mut self) {
        self.known = None;
        // With no known state there is no basis, so a retained immutable scene
        // is unreachable. A real correction/acceptance replaces it at runtime.
        self.reset = false;
        self.revision = self.revision.saturating_add(1);
    }

    pub fn recalibrate(&mut self) {
        if !self.reset {
            self.invalidate();
        }
    }

    pub fn accepted(&mut self, trial: Option<super::swim_scene::Trial>, now: Instant) {
        self.reset = false;
        self.scene = trial.as_ref().and_then(|trial| trial.scene.clone());
        if let Some(next) = self.revision.checked_add(1) {
            self.revision = next;
            self.known = trial.map(|trial| (trial.velocity, now));
        } else {
            self.known = None;
        }
    }

    pub fn corrected(&mut self, now: Instant) {
        self.accepted(Some(water::WetVelocity::default().into()), now);
        self.reset = self.known.is_some();
    }

    #[allow(
        clippy::float_cmp,
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "One replay transaction checks exact headings, timing, calibration and scene state"
    )]
    pub fn trial(
        &self,
        session: u64,
        controls: &SwimControls,
        calibration: MotionCalibration,
        heading: f32,
        created: Instant,
        granted_from: Instant,
        origin: [f32; 3],
    ) -> Result<(super::swim_scene::Trial, [f32; 3])> {
        ensure!(
            self.basis(session) == Some(controls.basis),
            "stale or unavailable swim basis"
        );
        let (mut velocity, started) = self
            .known
            .ok_or_else(|| anyhow::anyhow!("no owned wet state"))?;
        let seconds = controls
            .pieces
            .last()
            .expect("checked nonempty controls")
            .until
            .as_secs_f32();
        ensure!(created >= started, "swim controls predate their basis");
        let available = created.duration_since(started).as_secs_f32();
        ensure!(
            if controls.applied {
                seconds <= available.min(0.25) + 1e-5
            } else {
                (available - seconds).abs() <= 1e-5
            },
            "swim controls do not cover the complete applied interval"
        );
        ensure!(
            controls.pose == heading,
            "swim control endpoint heading differs"
        );
        // The applied ledger covers actual model consumption, not time the
        // controller spent waiting. The unchanged wall budget still caps it.
        let physical_start = if controls.applied {
            created
                .checked_sub(controls.pieces.last().expect("checked controls").until)
                .ok_or_else(|| anyhow::anyhow!("invalid applied swim clock"))?
                // Independent f32-to-nanosecond cursor rounding must not
                // place a call before the actual accepted/reset basis.
                .max(started)
        } else {
            started
        };
        let swim = calibration
            .swim
            .ok_or_else(|| anyhow::anyhow!("water is not calibrated"))?;
        let known_clock = |velocity: water::WetVelocity| {
            if controls.applied {
                velocity.has_ordinary_call_clock()
            } else {
                velocity.frame_phase().is_some()
            }
        };
        ensure!(known_clock(velocity), "unknown water phase");
        velocity.clamp(swim.caps());
        let mut scene = match controls.scene.as_ref() {
            None => {
                ensure!(
                    self.scene.is_none(),
                    "scene replay cannot silently withdraw geometry"
                );
                None
            }
            Some(handle) => Some(if let Some(known) = &self.scene {
                ensure!(
                    known.scene() == handle && known.origin() == origin,
                    "changed swim scene or origin"
                );
                known.clone()
            } else {
                ensure!(
                    self.reset,
                    "scene replay needs independently matched reset continuity"
                );
                super::SwimSceneMotion::at_reset(handle.clone(), origin)?
            }),
        };
        let mut observed = self.scene.as_ref().map(|_| started);
        let mut path = super::swim_scene::PathBudget::new();
        let mut previous = Duration::ZERO;
        let mut total = [0.0; 3];
        for (index, piece) in controls.pieces.iter().enumerate() {
            ensure!(known_clock(velocity), "unknown water phase");
            let seconds = piece.applied_seconds.unwrap_or_else(|| {
                piece
                    .until
                    .checked_sub(previous)
                    .expect("checked ordered controls")
                    .as_secs_f32()
            });
            let input_start = if controls.scene.is_some() {
                let at = piece
                    .observed_at
                    .ok_or_else(|| anyhow::anyhow!("missing physical observation time"))?;
                ensure!(at <= created, "physical swim frame is in the future");
                let begin = if let Some((stopped, resumed)) = piece.stall {
                    ensure!(
                        observed.map_or(stopped >= started, |previous| stopped == previous),
                        "physical swim stall does not follow the observed frame"
                    );
                    resumed
                } else if let Some(before) = observed {
                    ensure!(
                        at > before
                            && at.duration_since(before).as_secs_f32().to_bits()
                                == seconds.to_bits(),
                        "physical swim observation has a gap or overlap"
                    );
                    before
                } else {
                    let begin = at
                        .checked_sub(Duration::from_secs_f32(seconds))
                        .ok_or_else(|| anyhow::anyhow!("invalid first physical swim clock"))?;
                    // A <=150ms f32 duration loses at most8ns when converted
                    // back to integer nanoseconds. This is not an input gap.
                    ensure!(
                        begin >= started
                            || started.saturating_duration_since(begin) <= Duration::from_nanos(8),
                        "physical swim frame predates reset"
                    );
                    begin.max(started)
                };
                observed = Some(at);
                Some(begin)
            } else {
                physical_start.checked_add(previous)
            };
            // Input, timing and calibration use each actual physical slice.
            ensure!(
                piece.input == SwimInput::Idle || input_start.is_some_and(|at| at >= granted_from),
                "swim input predates the calibrated interval"
            );
            let input = piece.native_input(calibration)?;
            let moved = if let Some(known) = scene.as_mut() {
                ensure!(
                    known.kind()? == piece.kind,
                    "swim medium chronology differs"
                );
                let before = known.origin();
                let (next, frame) = known.frame(swim.set_for(piece.kind), input, seconds)?;
                path.add(before, frame.origin, frame.raw);
                let moved = std::array::from_fn(|axis| frame.origin[axis] - before[axis]);
                velocity = next.velocity();
                *known = next;
                moved
            } else {
                let (horizontal, vertical) =
                    velocity.advance(swim.set_for(piece.kind), input, seconds);
                [horizontal[0], horizontal[1], vertical]
            };
            // Only the final remainder may leave the bounded native debt.
            // Every intervening full caller slice must have a canonical view.
            ensure!(
                if controls.scene.is_some()
                    || (controls.applied && index + 1 == controls.pieces.len())
                {
                    velocity.has_ordinary_call_clock()
                } else {
                    velocity.frame_phase().is_some()
                },
                "unknown water phase"
            );
            total[0] += moved[0];
            total[1] += moved[1];
            total[2] += moved[2];
            previous = piece.until;
        }
        if scene.is_some() {
            ensure!(
                observed == Some(created),
                "physical swim endpoint is not its actual completed frame"
            );
            path.check(swim.caps(), seconds)?;
        }
        Ok((super::swim_scene::Trial { velocity, scene }, total))
    }
}
