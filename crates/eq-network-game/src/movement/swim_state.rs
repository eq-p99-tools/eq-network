//! Inert, reset-anchored ordinary horizontal swim validation.
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
    /// Start of the complete interval the next observation must describe.
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
}

impl SwimControls {
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
            ensure!(piece.until > previous, "invalid applied swim boundary");
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
        })
    }

    /// Captures all physical calls that produced one ordinary controller step.
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
        for piece in &pieces {
            let seconds = piece
                .applied_seconds
                .ok_or_else(|| anyhow::anyhow!("missing applied swim duration"))?;
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
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct Owner {
    revision: u64,
    known: Option<(water::WetVelocity, Instant)>,
    reset: bool,
}

impl Owner {
    pub fn new(now: Instant) -> Self {
        Self {
            revision: 0,
            known: Some((water::WetVelocity::default(), now)),
            reset: true,
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
        self.reset = false;
        self.revision = self.revision.saturating_add(1);
    }

    pub fn recalibrate(&mut self) {
        if !self.reset {
            self.invalidate();
        }
    }

    pub fn accepted(&mut self, velocity: Option<water::WetVelocity>, now: Instant) {
        self.reset = false;
        if let Some(next) = self.revision.checked_add(1) {
            self.revision = next;
            self.known = velocity.map(|velocity| (velocity, now));
        } else {
            self.known = None;
        }
    }

    pub fn corrected(&mut self, now: Instant) {
        self.accepted(Some(water::WetVelocity::default()), now);
        self.reset = self.known.is_some();
    }

    #[allow(
        clippy::float_cmp,
        reason = "An observed final heading must match exactly"
    )]
    pub fn trial(
        &self,
        session: u64,
        controls: &SwimControls,
        calibration: MotionCalibration,
        heading: f32,
        created: Instant,
        granted_from: Instant,
    ) -> Result<(water::WetVelocity, [f32; 2])> {
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
        ensure!(velocity.frame_phase().is_some(), "unknown water phase");
        velocity.clamp(swim.caps());
        let mut previous = Duration::ZERO;
        let mut total = [0.0; 2];
        for piece in &controls.pieces {
            ensure!(velocity.frame_phase().is_some(), "unknown water phase");
            // No non-idle physical call may predate its calibrated grant.
            // Actual applied pauses contain no fabricated physical calls.
            ensure!(
                piece.input == SwimInput::Idle
                    || physical_start
                        .checked_add(previous)
                        .is_some_and(|at| at >= granted_from),
                "swim input predates the calibrated interval"
            );
            let input = water::WetInput {
                horizontal: piece.horizontal(calibration)?,
                vertical: 0.0,
                jump: false,
            };
            let (moved, _) = velocity.advance(
                swim.set_for(piece.kind),
                input,
                piece.applied_seconds.unwrap_or_else(|| {
                    piece
                        .until
                        .checked_sub(previous)
                        .expect("checked ordered controls")
                        .as_secs_f32()
                }),
            );
            ensure!(velocity.frame_phase().is_some(), "unknown water phase");
            total[0] += moved[0];
            total[1] += moved[1];
            previous = piece.until;
        }
        Ok((velocity, total))
    }
}
