//! Zone-scoped movement submission with caller-supplied calibration.
use super::{MovementGuard, MovementMode, MovementRequest, PositionPacket};
use crate::world::Position;
use anyhow::{ensure, Result};
use std::time::{Duration, Instant};

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
    /// Signed animation/speed value for this mode.
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
    /// Negative signed 10-bit animation value observed when backing up.
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
    /// Positive signed 10-bit walking animation value.
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
    /// Nonzero signed 10-bit animation observed while strafing.
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
                && (1..=511).contains(&self.animation),
            "invalid movement calibration"
        );
        if let Some(backward) = self.backward {
            ensure!(
                backward.units_per_second.is_finite()
                    && backward.units_per_second > 0.0
                    && backward.velocity_scale.is_finite()
                    && backward.velocity_scale > 0.0
                    && (-512..=-1).contains(&backward.animation),
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
                    && (1..=511).contains(&walk.animation),
                "invalid walking calibration"
            );
        }
        if let Some(strafe) = self.strafe {
            ensure!(
                strafe.units_per_second.is_finite()
                    && strafe.units_per_second > 0.0
                    && strafe.velocity_scale.is_finite()
                    && strafe.velocity_scale > 0.0
                    && (-512..=511).contains(&strafe.animation)
                    && strafe.animation != 0,
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
    last_sample: Instant,
    moving: bool,
    suspended: bool,
    falls: bool,
}
impl MotionSession {
    /// Permits [`MovementMode::Fall`] samples. Only stock `EQEmu` sessions enable it
    /// until official-client falls are calibrated.
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

    /// Checks that a jump may be announced: jumps rise and fall under the same
    /// client-side physics as falls, so only sessions that accept falls allow them.
    ///
    /// # Errors
    /// Rejects jumps on sessions without falls.
    pub fn jump(&self) -> Result<()> {
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
            last_sample: now,
            moving: false,
            suspended: false,
            falls: false,
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
        self.last_sample = now;
        Ok(())
    }

    /// Calibrates from a queued request only if it postdates the latest reset.
    ///
    /// # Errors
    /// Rejects stale requests and invalid calibration values.
    pub fn calibrate_fresh(
        &mut self,
        value: MotionCalibration,
        created: Instant,
        now: Instant,
    ) -> Result<()> {
        ensure!(
            created >= self.guard.input_epoch
                && created <= now
                && now.duration_since(created) <= Duration::from_millis(250),
            "stale movement calibration"
        );
        self.calibrate(value, now)
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
        self.moving = false;
        self.guard.correct(self.position, now);
        self.last_sample = now;
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
        send: impl FnOnce(&[u8; 36]) -> Result<()>,
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
        let displacement = guard.accept(request, now)?;
        let elapsed = now
            .saturating_duration_since(self.last_sample)
            .as_secs_f32()
            .min(0.25);
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
        let packet = PositionPacket {
            spawn_id: self.spawn_id,
            sequence: self.sequence,
            position: request.position,
            delta,
            animation: if moving { animation } else { 0 },
            delta_heading: turn,
        }
        .encode()?;
        send(&packet)?;
        self.guard = guard;
        self.position = request.position;
        self.last_sample = now;
        self.last_sent = now;
        self.moving = moving || turn != 0;
        self.sequence = self.sequence.wrapping_add(1);
        Ok(())
    }

    /// Sends a stop after 250 ms without input, or a stationary heartbeat every 2 s.
    ///
    /// # Errors
    /// Propagates encoding and transport failures without advancing state.
    pub fn tick(
        &mut self,
        now: Instant,
        send: impl FnOnce(&[u8; 36]) -> Result<()>,
    ) -> Result<bool> {
        let interval = if self.moving {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(2)
        };
        if self.suspended || now.saturating_duration_since(self.last_sent) < interval {
            return Ok(false);
        }
        let packet = PositionPacket {
            spawn_id: self.spawn_id,
            sequence: self.sequence,
            position: self.position,
            delta: [0.0; 3],
            animation: 0,
            delta_heading: 0,
        }
        .encode()?;
        send(&packet)?;
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
        .encode()?;
        self.position = position;
        self.guard.correct(position, now);
        self.calibration = None;
        self.last_sample = now;
        // Keep the pending stop deadline if the last transmitted sample was moving.
        Ok(())
    }

    /// Stops all position packets after death or transfer; admission creates a new instance.
    pub fn suspend(&mut self) {
        self.suspended = true;
        self.calibration = None;
        self.moving = false;
    }
}
#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

/// Uses shortest-arc heading velocity; the observed steady turn field is in heading units/sec.
#[allow(clippy::cast_possible_truncation)] // Rounded and checked against the signed wire range.
fn heading_velocity(from: f32, to: f32, elapsed: f32) -> Result<i16> {
    let difference = (to - from + 256.0).rem_euclid(512.0) - 256.0;
    let rate = (difference / elapsed).round();
    ensure!(
        rate.is_finite() && (-512.0..=511.0).contains(&rate),
        "heading change exceeds wire turn rate"
    );
    Ok(rate as i16)
}
