//! Zone-scoped movement submission with caller-supplied calibration.
use super::{MovementGuard, MovementMode, MovementRequest, PositionPacket, JUMP_OPCODE};
use crate::{command::EncodedCommand, world::Position};
use anyhow::{ensure, Result};
use std::time::{Duration, Instant};

/// Titanium's notice that the character jumped, already checked.
#[must_use]
pub fn titanium_jump() -> EncodedCommand {
    EncodedCommand {
        opcode: JUMP_OPCODE,
        body: Vec::new(),
    }
}

/// Independently measured motion values for the current character and effects.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct MotionCalibration {
    /// Optional sideways movement measured in both directions; omission disables strafing.
    #[serde(default)]
    pub strafe: Option<StrafeCalibration>,
    /// Optional independently measured walking mode; omission disables the walk toggle.
    #[serde(default)]
    pub walk: Option<WalkCalibration>,
    /// Optional independently measured backward mode; omission disables backing up.
    #[serde(default)]
    pub backward: Option<BackwardCalibration>,
    /// Maximum horizontal world units per second.
    pub units_per_second: f32,
    /// Converts world velocity per second to wire velocity.
    pub velocity_scale: f32,
    /// Signed animation/speed value for this mode; 0 where the generation
    /// sends none for a very slow move.
    pub animation: i16,
}

/// Measured backward motion, kept separate from forward speed and animation.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackwardCalibration {
    /// Positive maximum horizontal world units per second.
    pub units_per_second: f32,
    /// Converts horizontal world velocity per second to wire velocity.
    pub velocity_scale: f32,
    /// Signed 10-bit animation value observed when backing up: negative, or
    /// 0 for a very slow move.
    pub animation: i16,
}

/// Measured forward walking values, independent of running and backing up.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct WalkCalibration {
    /// Positive maximum horizontal world units per second.
    pub units_per_second: f32,
    /// Converts horizontal world velocity per second to wire velocity.
    pub velocity_scale: f32,
    /// Signed 10-bit walking animation value: positive, or 0 for a very
    /// slow move.
    pub animation: i16,
}

/// Measured sideways motion, independent of the forward and backward modes.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct StrafeCalibration {
    /// Positive maximum horizontal world units per second.
    pub units_per_second: f32,
    /// Converts horizontal world velocity per second to wire velocity.
    pub velocity_scale: f32,
    /// Signed 10-bit animation observed while strafing; `EQMac` sends 0.
    pub animation: i16,
}

impl MotionCalibration {
    /// Validates representation, without claiming the values are measured.
    ///
    /// # Errors
    /// Rejects non-finite, non-positive, or out-of-range values.
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.units_per_second.is_finite()
                && self.units_per_second > 0.0
                && self.velocity_scale.is_finite()
                && self.velocity_scale > 0.0
                && (0..=511).contains(&self.animation),
            "invalid movement calibration"
        );
        if let Some(backward) = self.backward {
            ensure!(
                backward.units_per_second.is_finite()
                    && backward.units_per_second > 0.0
                    && backward.velocity_scale.is_finite()
                    && backward.velocity_scale > 0.0
                    && (-512..=0).contains(&backward.animation),
                "invalid backward movement calibration"
            );
        }
        if let Some(walk) = self.walk {
            ensure!(
                walk.units_per_second.is_finite()
                    && walk.units_per_second > 0.0
                    && walk.units_per_second <= self.units_per_second
                    && walk.velocity_scale.is_finite()
                    && walk.velocity_scale > 0.0
                    && (0..=511).contains(&walk.animation),
                "invalid walking calibration"
            );
        }
        if let Some(strafe) = self.strafe {
            ensure!(
                strafe.units_per_second.is_finite()
                    && strafe.units_per_second > 0.0
                    && strafe.velocity_scale.is_finite()
                    && strafe.velocity_scale > 0.0
                    && (-512..=511).contains(&strafe.animation),
                "invalid strafing calibration"
            );
        }
        Ok(())
    }
}

/// Position, packet counter, and input expiry for one zone admission.
#[derive(Debug)]
pub struct MotionSession {
    guard: MovementGuard,
    spawn_id: u16,
    sequence: u16,
    position: Position,
    calibration: Option<MotionCalibration>,
    last_sent: Instant,
    moving: bool,
    suspended: bool,
    falls: bool,
    /// The least time between two of the player's own updates, where the
    /// client generation keeps one; see [`Self::with_spacing`].
    spacing: Option<Duration>,
    /// The latest accepted sample the spacing has not let out yet.
    held: Option<PositionPacket>,
}
impl MotionSession {
    /// Sends the player's own position at most once per `spacing`: a sample
    /// accepted sooner is held, and a later one replaces it, so the latest
    /// goes out once the spacing has passed ([`Self::tick`]). Accepting a
    /// sample is unchanged, so the host's own steps keep their pace.
    #[must_use]
    pub const fn with_spacing(mut self, spacing: Option<Duration>) -> Self {
        self.spacing = spacing;
        self
    }

    /// Lets fall samples descend as fast as the client generation's falls
    /// go, where it says; otherwise at most [`super::MAX_FALL_SPEED`].
    #[must_use]
    pub fn with_fall_speed(mut self, fall_speed: Option<f32>) -> Self {
        if let Some(fall_speed) = fall_speed {
            self.guard.set_fall_speed(fall_speed);
        }
        self
    }

    /// Lets samples after a jump rise as the client generation's jump does,
    /// where it is read: its starting speed and the zone's gravity, in units
    /// a second and a second squared ([`MovementGuard::set_jump`]).
    #[must_use]
    pub fn with_jump(mut self, jump: Option<(f32, f32)>) -> Self {
        if let Some((speed, gravity)) = jump {
            self.guard.set_jump(speed, gravity);
        }
        self
    }

    /// The session announced a jump: the samples after it may rise as it
    /// does ([`MovementGuard::jumped`]).
    pub fn jumped(&mut self) {
        self.guard.jumped();
    }

    /// Permits [`MovementMode::Fall`] samples, where the server type takes
    /// falls from the client.
    #[must_use]
    pub const fn with_falls(mut self, allowed: bool) -> Self {
        self.falls = allowed;
        self
    }

    /// Whether [`MovementMode::Fall`] samples are accepted.
    #[must_use]
    pub const fn falls(&self) -> bool {
        self.falls
    }

    /// The notice that the character jumped. Jumps rise and fall under the same
    /// client-side physics as falls, so only sessions that accept falls allow them.
    ///
    /// # Errors
    /// Rejects jumps on sessions without falls.
    pub fn jump(&self) -> Result<EncodedCommand> {
        self.check_jump()?;
        Ok(titanium_jump())
    }

    /// Checks that the character may jump: only sessions that accept falls
    /// allow it.
    ///
    /// # Errors
    /// Rejects jumps on sessions without falls.
    pub fn check_jump(&self) -> Result<()> {
        ensure!(self.falls, "jumping is not enabled for this server");
        Ok(())
    }

    /// Starts stationary, without assuming any effective movement speed.
    ///
    /// # Errors
    /// Rejects invalid spawn IDs or positions.
    pub fn new(session_id: u64, spawn_id: u16, position: Position, now: Instant) -> Result<Self> {
        PositionPacket {
            spawn_id,
            sequence: 0,
            position,
            delta: [0.0; 3],
            animation: 0,
            delta_heading: 0,
        }
        .encode()?;
        Ok(Self {
            guard: MovementGuard::new(session_id, position, now),
            spawn_id,
            sequence: 0,
            position,
            calibration: None,
            last_sent: now,
            moving: false,
            suspended: false,
            falls: false,
            spacing: None,
            held: None,
        })
    }

    /// Enables an independently measured movement mode for this admission.
    ///
    /// # Errors
    /// Rejects invalid conversions and resuming a suspended session.
    pub fn calibrate(&mut self, value: MotionCalibration, now: Instant) -> Result<()> {
        ensure!(!self.suspended, "movement session is suspended");
        value.validate()?;
        self.guard.set_speed(Some(value.units_per_second), now);
        self.calibration = Some(value);
        Ok(())
    }

    /// Calibrates from a queued request only if it postdates the latest reset;
    /// the zone session has already refused one made too long ago.
    ///
    /// # Errors
    /// Rejects requests from before the latest reset and invalid calibration values.
    pub fn calibrate_fresh(
        &mut self,
        value: MotionCalibration,
        created: Instant,
        now: Instant,
    ) -> Result<()> {
        ensure!(
            created >= self.guard.input_epoch,
            "stale movement calibration"
        );
        self.calibrate(value, now)
    }

    /// The speeds movement is granted at, if it is.
    #[must_use]
    pub const fn calibration(&self) -> Option<MotionCalibration> {
        self.calibration
    }

    /// Takes the player's speeds away, as when they are rooted, until the
    /// next calibration.
    pub fn withdraw(&mut self) {
        self.calibration = None;
        self.held = None;
    }

    /// Latest accepted world position, without implying a server acknowledgment.
    #[must_use]
    pub const fn position(&self) -> Position {
        self.position
    }

    /// Resumes stationary heartbeats after a denied transfer; walking stays disabled.
    pub fn resume_stationary(&mut self, now: Instant) {
        self.suspended = false;
        self.calibration = None;
        self.held = None;
        self.moving = false;
        self.guard.correct(self.position, now);
    }

    /// Submits motion and commits position/counters only after transport success.
    ///
    /// # Errors
    /// Rejects stale, uncalibrated, suspended, excessive or invalid motion.
    /// Transport failures preserve timing, position and sequence for a retry.
    pub fn send_move(
        &mut self,
        request: &MovementRequest,
        now: Instant,
        send: impl FnOnce(&EncodedCommand) -> Result<()>,
    ) -> Result<()> {
        self.send_move_sample(request, now, |sample| send(&sample.packet()?))
    }

    /// As [`Self::send_move`], handing the checked sample to `send` for a
    /// client generation to encode; state commits only after `send` succeeds.
    ///
    /// # Errors
    /// Rejects stale, uncalibrated, suspended, excessive or invalid motion,
    /// and propagates whatever `send` returns.
    pub fn send_move_sample(
        &mut self,
        request: &MovementRequest,
        now: Instant,
        send: impl FnOnce(&PositionPacket) -> Result<()>,
    ) -> Result<()> {
        ensure!(!self.suspended, "movement session is suspended");
        ensure!(
            request.mode != MovementMode::Fall || self.falls,
            "falling is not enabled for this server"
        );
        let calibration = self
            .calibration
            .ok_or_else(|| anyhow::anyhow!("movement is not calibrated"))?;
        let mut guard = self.guard.clone();
        let (speed, velocity_scale, animation) = match request.mode {
            MovementMode::Strafe => {
                let strafe = calibration
                    .strafe
                    .ok_or_else(|| anyhow::anyhow!("strafing is not calibrated"))?;
                (
                    strafe.units_per_second,
                    strafe.velocity_scale,
                    strafe.animation,
                )
            }
            MovementMode::Walk => {
                let walk = calibration
                    .walk
                    .ok_or_else(|| anyhow::anyhow!("walking is not calibrated"))?;
                (walk.units_per_second, walk.velocity_scale, walk.animation)
            }
            MovementMode::Forward | MovementMode::Fall => (
                calibration.units_per_second,
                calibration.velocity_scale,
                calibration.animation,
            ),
            MovementMode::Backward => {
                let backward = calibration
                    .backward
                    .ok_or_else(|| anyhow::anyhow!("backward movement is not calibrated"))?;
                (
                    backward.units_per_second,
                    backward.velocity_scale,
                    backward.animation,
                )
            }
        };
        // Switching mode changes the per-sample limit without resetting input expiry
        // or granting a new time budget. The clone is committed only after send succeeds.
        guard.speed = Some(speed);
        // The wire's velocity and turn rate cover the same interval the guard
        // budgets: from the last accepted request's stamp to this one's.
        let elapsed = guard.interval(request.created);
        let displacement = guard.accept(request, now)?;
        // Following a slope changes position Z without creating the airborne
        // velocity seen during a fall. Falls also leave that field zero: its wire
        // scale is unmeasured, so position Z alone carries the descent.
        let delta = [
            displacement[0] / elapsed * velocity_scale,
            displacement[1] / elapsed * velocity_scale,
            0.0,
        ];
        let moving = displacement[..2].iter().any(|d| d.abs() > f32::EPSILON);
        let turn = heading_velocity(self.position.heading, request.position.heading, elapsed)?;
        let sample = PositionPacket {
            spawn_id: self.spawn_id,
            sequence: self.sequence,
            position: request.position,
            delta,
            animation: if moving { animation } else { 0 },
            delta_heading: turn,
        };
        sample.check()?;
        let spaced = self
            .spacing
            .is_some_and(|spacing| now.saturating_duration_since(self.last_sent) < spacing);
        if spaced {
            self.held = Some(sample);
        } else {
            send(&sample)?;
            self.held = None;
            self.last_sent = now;
            self.sequence = self.sequence.wrapping_add(1);
        }
        self.guard = guard;
        self.position = request.position;
        self.moving = moving || turn != 0;
        Ok(())
    }

    /// Sends a held sample once the spacing has passed, a stop after 250 ms
    /// without input (or the spacing, if longer), or a stationary heartbeat
    /// every [`super::STATIONARY_HEARTBEAT`].
    ///
    /// # Errors
    /// Propagates encoding and transport failures without advancing state.
    pub fn tick(
        &mut self,
        now: Instant,
        send: impl FnOnce(&EncodedCommand) -> Result<()>,
    ) -> Result<bool> {
        self.tick_sample(now, |sample| send(&sample.packet()?))
    }

    /// As [`Self::tick`], handing the checked sample to `send` for a client
    /// generation to encode.
    ///
    /// # Errors
    /// Propagates check and send failures without advancing state.
    pub fn tick_sample(
        &mut self,
        now: Instant,
        send: impl FnOnce(&PositionPacket) -> Result<()>,
    ) -> Result<bool> {
        let spacing = self.spacing.unwrap_or_default();
        let since = now.saturating_duration_since(self.last_sent);
        if self.suspended {
            return Ok(false);
        }
        if let Some(held) = self.held {
            if since < spacing {
                return Ok(false);
            }
            let sample = PositionPacket {
                sequence: self.sequence,
                ..held
            };
            send(&sample)?;
            self.held = None;
            self.sequence = self.sequence.wrapping_add(1);
            self.last_sent = now;
            return Ok(true);
        }
        let interval = if self.moving {
            Duration::from_millis(250).max(spacing)
        } else {
            super::STATIONARY_HEARTBEAT
        };
        if since < interval {
            return Ok(false);
        }
        let sample = PositionPacket {
            spawn_id: self.spawn_id,
            sequence: self.sequence,
            position: self.position,
            delta: [0.0; 3],
            animation: 0,
            delta_heading: 0,
        };
        sample.check()?;
        send(&sample)?;
        self.sequence = self.sequence.wrapping_add(1);
        self.last_sent = now;
        self.moving = false;
        Ok(true)
    }

    /// Applies a server correction and revokes calibration and queued motion.
    ///
    /// # Errors
    /// Rejects invalid positions without changing accepted state.
    pub fn correct(&mut self, position: Position, now: Instant) -> Result<()> {
        PositionPacket {
            spawn_id: self.spawn_id,
            sequence: self.sequence,
            position,
            delta: [0.0; 3],
            animation: 0,
            delta_heading: 0,
        }
        .check()?;
        self.position = position;
        self.guard.correct(position, now);
        self.calibration = None;
        self.held = None;
        // Keep the pending stop deadline if the last transmitted sample was moving.
        Ok(())
    }

    /// Stops all position packets after death or transfer; admission creates a new instance.
    pub fn suspend(&mut self) {
        self.suspended = true;
        self.calibration = None;
        self.held = None;
        self.moving = false;
    }
}
#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

/// The shortest-arc turn rate in heading units a second, cut toward zero.
/// Titanium's program sends its turn rate per 50 ms tick times 20, cut to
/// a whole number whatever the mode, so a full keyboard turn of 12 a tick
/// sends 240 (the Titanium client update read of 10-31, and sitting 7); the
/// cut is read from its conversion routine's shape only. Its mouse-turn path
/// is unread, so a turn faster than the 10-bit field holds is refused, as
/// before.
#[allow(clippy::cast_possible_truncation)] // Cut and checked against the signed wire range.
fn heading_velocity(from: f32, to: f32, elapsed: f32) -> Result<i16> {
    let difference = (to - from + 256.0).rem_euclid(512.0) - 256.0;
    let rate = (difference / elapsed).trunc();
    ensure!(
        rate.is_finite() && (-512.0..=511.0).contains(&rate),
        "heading change exceeds wire turn rate"
    );
    Ok(rate as i16)
}
