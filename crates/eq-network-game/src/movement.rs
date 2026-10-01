//! Titanium movement encoding and session-scoped displacement validation.

mod session;
pub use session::{
    BackwardCalibration, MotionCalibration, MotionSession, StrafeCalibration, WalkCalibration,
};

use crate::world::Position;
use anyhow::{ensure, Result};
use std::time::{Duration, Instant};

/// Maximum grounded stair adjustment per sample, in world units.
/// This accommodates two-unit classic zone risers; it is a client collision policy,
/// not a measured server limit or permission to encode falling as grounded motion.
pub const MAX_GROUNDED_STEP: f32 = 2.0;

/// Fastest descent a [`MovementMode::Fall`] sample may claim, in world units per
/// second. This is the client's provisional terminal speed, not a measured Titanium
/// or server limit.
pub const MAX_FALL_SPEED: f32 = 40.0;

/// How often a standing character repeats its position. The official P99 client
/// sends one about every 1.1 s (median of its recorded stationary updates).
pub const STATIONARY_HEARTBEAT: Duration = Duration::from_secs(1);

/// `OP_Jump` (Titanium): an empty notice that the character jumped. `EQEmu` only
/// charges endurance for it; the arc itself travels in position updates.
pub const JUMP_OPCODE: u16 = 0x0797;

/// Locomotion mode used to choose measured motion parameters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MovementMode {
    /// Forward or camera-directed grounded motion.
    #[default]
    Forward,
    /// Backing up while preserving the character's facing.
    Backward,
    /// Walking with its own measured speed and animation.
    Walk,
    /// Sideways movement preserving character facing.
    Strafe,
    /// Airborne descent after leaving a ledge: forward speed and animation, while
    /// height falls under gravity instead of following the ground.
    Fall,
}

/// A graphical client's proposed position, tied to its current zone admission.
#[derive(Clone, Debug, PartialEq)]
pub struct MovementRequest {
    /// Selects an independently calibrated speed and animation.
    pub mode: MovementMode,
    /// Connection identifier from `WorldEvent::Entered`.
    pub session_id: u64,
    /// Position after local collision resolution.
    pub position: Position,
    /// Local creation time; queued input expires instead of replaying after a stall.
    pub created: Instant,
}

/// One client-to-server motion sample, independent of transport sequencing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionPacket {
    /// Active character's own zone spawn ID.
    pub spawn_id: u16,
    /// Wrapping packet counter, owned by the zone session.
    pub sequence: u16,
    /// Server coordinates and canonical 0..512 heading.
    pub position: Position,
    /// Wire delta fields in server X/Y/Z order. Their time scale must be
    /// validated before translating displacement into an outgoing motion sample.
    pub delta: [f32; 3],
    /// Signed 10-bit protocol animation/speed field.
    pub animation: i16,
    /// Signed 10-bit heading velocity, separate from animation.
    pub delta_heading: i16,
}

impl PositionPacket {
    /// Encodes the 36-byte Titanium layout without native bitfields or padding.
    ///
    /// # Errors
    /// Rejects invalid IDs, non-finite coordinates, and out-of-range animation.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Explicit masks and bounded heading.
    pub fn encode(self) -> Result<[u8; 36]> {
        ensure!(self.spawn_id != 0, "movement requires an own-spawn ID");
        ensure!(
            valid_position(self.position) && self.delta.iter().all(|v| v.is_finite()),
            "non-finite movement"
        );
        ensure!(
            (-512..=511).contains(&self.animation),
            "movement animation exceeds signed 10-bit range"
        );
        ensure!(
            (-512..=511).contains(&self.delta_heading),
            "turn rate exceeds signed 10-bit range"
        );
        let mut body = [0; 36];
        body[..2].copy_from_slice(&self.spawn_id.to_le_bytes());
        body[2..4].copy_from_slice(&self.sequence.to_le_bytes());
        for (offset, value) in [
            (4, self.position.y),
            (8, self.delta[2]),
            (12, self.delta[0]),
            (16, self.delta[1]),
            (24, self.position.x),
            (28, self.position.z),
        ] {
            body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        // Fixed upper bits match the recorded official Titanium/P99 client.
        let animation = (i32::from(self.animation) as u32 & 0x3ff)
            | ((i32::from(self.delta_heading) as u32 & 0x3ff) << 10)
            | 0x0020_0000;
        body[20..24].copy_from_slice(&animation.to_le_bytes());
        let heading = (self.position.heading.rem_euclid(512.0) * 4.0) as u16;
        body[32..34].copy_from_slice(&(heading & 0x0fff).to_le_bytes());
        Ok(body)
    }
}

/// Fixed server-coordinate limits for a controlled movement session.
/// These never follow the player or move after a server correction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovementBounds {
    minimum: [f32; 3],
    maximum: [f32; 3],
}
impl MovementBounds {
    /// Constructs explicit X/Y/Z limits; both faces of every axis are inclusive.
    ///
    /// # Errors
    /// Rejects non-finite or reversed limits.
    pub fn new(minimum: [f32; 3], maximum: [f32; 3]) -> Result<Self> {
        ensure!(
            minimum
                .iter()
                .zip(maximum)
                .all(|(min, max)| min.is_finite() && max.is_finite() && *min <= max),
            "invalid movement bounds"
        );
        Ok(Self { minimum, maximum })
    }
    /// Whether a finite position lies inside the originally configured volume.
    #[must_use]
    pub fn contains(self, position: Position) -> bool {
        valid_position(position)
            && [position.x, position.y, position.z]
                .iter()
                .zip(self.minimum.iter().zip(self.maximum))
                .all(|(value, (min, max))| *value >= *min && *value <= max)
    }
}

/// Validates proposed motion without trusting render-frame timing or accumulating idle credit.
#[derive(Clone, Debug)]
pub struct MovementGuard {
    session_id: u64,
    position: Position,
    sampled: Instant,
    input_epoch: Instant,
    speed: Option<f32>,
    bounds: Option<MovementBounds>,
}

impl MovementGuard {
    /// Starts disabled until the caller has established an effective movement limit.
    #[must_use]
    pub fn new(session_id: u64, position: Position, now: Instant) -> Self {
        Self {
            session_id,
            position,
            sampled: now,
            input_epoch: now,
            speed: None,
            bounds: None,
        }
    }

    /// Pins the safe volume once for this session. Reconnecting requires new bounds.
    ///
    /// # Errors
    /// Rejects replacing bounds or admitting an initial position outside them.
    pub fn set_bounds(&mut self, bounds: MovementBounds) -> Result<()> {
        ensure!(
            self.bounds.is_none(),
            "movement bounds are already fixed for this session"
        );
        ensure!(
            bounds.contains(self.position),
            "initial position lies outside movement bounds"
        );
        self.bounds = Some(bounds);
        Ok(())
    }

    /// Sets a verified effective speed in EQ world units per second.
    /// None disables movement (for unknown state, roots, stuns, disconnects, etc.).
    pub fn set_speed(&mut self, speed: Option<f32>, now: Instant) {
        self.speed = speed.filter(|speed| speed.is_finite() && *speed > 0.0);
        self.sampled = now;
        self.input_epoch = now;
    }

    /// Resets both position and pending motion after a server correction.
    pub fn correct(&mut self, position: Position, now: Instant) {
        self.position = position;
        self.sampled = now;
        self.input_epoch = now;
        self.speed = None;
    }

    /// Validates one fresh proposal. Failure never changes the accepted position.
    ///
    /// # Errors
    /// Rejects disabled, stale, foreign-session, non-finite, or excessive motion.
    pub fn accept(&mut self, request: &MovementRequest, now: Instant) -> Result<[f32; 3]> {
        ensure!(
            request.session_id == self.session_id,
            "movement belongs to an old zone session"
        );
        ensure!(
            request.created >= self.input_epoch
                && request.created <= now
                && now.duration_since(request.created) <= Duration::from_millis(250),
            "stale movement request"
        );
        ensure!(
            valid_position(request.position),
            "non-finite movement position"
        );
        if let Some(bounds) = self.bounds {
            ensure!(
                bounds.contains(self.position) && bounds.contains(request.position),
                "movement leaves the fixed safe volume"
            );
        }
        let speed = self
            .speed
            .ok_or_else(|| anyhow::anyhow!("effective movement speed is not established"))?;
        let elapsed = now
            .saturating_duration_since(self.sampled)
            .as_secs_f32()
            .min(0.25);
        ensure!(elapsed >= 0.05, "movement samples arrive too frequently");
        let delta = [
            request.position.x - self.position.x,
            request.position.y - self.position.y,
            request.position.z - self.position.z,
        ];
        ensure!(
            delta[0].hypot(delta[1]) <= speed * elapsed + 0.001,
            "movement exceeds elapsed-time budget"
        );
        // A fall may only descend, at most at terminal speed; grounded modes follow
        // stairs and slopes one riser at a time.
        let vertical = if request.mode == MovementMode::Fall {
            delta[2] <= MAX_GROUNDED_STEP
                && -delta[2] <= MAX_FALL_SPEED * elapsed + MAX_GROUNDED_STEP
        } else {
            delta[2].abs() <= MAX_GROUNDED_STEP
        };
        ensure!(
            vertical,
            "vertical motion requires a supported movement mode"
        );
        self.position = request.position;
        self.sampled = now;
        Ok(delta)
    }
}

fn valid_position(p: Position) -> bool {
    [p.x, p.y, p.z, p.heading].iter().all(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grounded_stairs_allow_two_units_but_reject_larger_vertical_changes() {
        let start = Instant::now();
        let now = start + Duration::from_millis(100);
        let mut guard = MovementGuard::new(7, Position::default(), start);
        guard.set_speed(Some(30.0), start);
        let mut request = MovementRequest {
            mode: MovementMode::Forward,
            session_id: 7,
            position: Position {
                x: 1.0,
                z: 2.01,
                ..Position::default()
            },
            created: now,
        };
        assert!(guard.accept(&request, now).is_err());
        request.position.z = 2.0;
        assert!(guard.accept(&request, now).is_ok());
    }

    #[test]
    fn falls_descend_at_most_at_terminal_speed_and_never_rise() {
        let start = Instant::now();
        let mut guard = MovementGuard::new(7, Position::default(), start);
        guard.set_speed(Some(30.0), start);
        let now = start + Duration::from_millis(100);
        let mut request = MovementRequest {
            mode: MovementMode::Forward,
            session_id: 7,
            position: Position {
                x: 1.0,
                z: -5.0,
                ..Position::default()
            },
            created: now,
        };
        // Grounded modes still refuse the drop.
        assert!(guard.accept(&request, now).is_err());
        // 100 ms allows 4 units at terminal speed plus one grounded step.
        request.mode = MovementMode::Fall;
        request.position.z = -6.1;
        assert!(guard.accept(&request, now).is_err());
        request.position.z = -5.0;
        assert!(guard.accept(&request, now).is_ok());
        let later = now + Duration::from_millis(100);
        request.created = later;
        request.position.z = -2.9;
        assert!(guard.accept(&request, later).is_err());
    }
    #[test]
    fn layout_keeps_axis_order_heading_and_signed_animation() {
        let packet = PositionPacket {
            spawn_id: 7,
            sequence: 9,
            position: Position {
                x: 12.0,
                y: -8.0,
                z: 3.0,
                heading: 256.0,
            },
            delta: [1.0, 2.0, 3.0],
            animation: -12,
            delta_heading: -240,
        }
        .encode()
        .unwrap();
        assert_eq!(&packet[4..8], &(-8f32).to_le_bytes());
        assert_eq!(&packet[12..16], &1f32.to_le_bytes());
        assert_eq!(&packet[16..20], &2f32.to_le_bytes());
        assert_eq!(&packet[24..28], &12f32.to_le_bytes());
        assert_eq!(&packet[32..34], &1024u16.to_le_bytes());
        assert_eq!(
            u32::from_le_bytes(packet[20..24].try_into().unwrap()) & 0x3ff,
            1012
        );
        let bits = u32::from_le_bytes(packet[20..24].try_into().unwrap());
        assert_eq!((bits >> 10) & 0x3ff, 784);
        assert_eq!(bits >> 20, 2);
    }
    #[test]
    fn optional_bounds_do_not_disable_speed_validation() {
        let start = Instant::now();
        let mut guard = MovementGuard::new(3, Position::default(), start);
        guard.set_speed(Some(6.0), start);
        let now = start + Duration::from_millis(100);
        let mut request = MovementRequest {
            mode: MovementMode::Forward,
            session_id: 3,
            position: Position {
                x: 1.0,
                ..Position::default()
            },
            created: now,
        };
        assert!(guard.accept(&request, now).is_err());
        request.position.x = 0.5;
        assert!(guard.accept(&request, now).is_ok());
    }

    #[test]
    fn rejects_reconnect_replay_stalls_and_excessive_diagonal_distance() {
        let start = Instant::now();
        let mut guard = MovementGuard::new(3, Position::default(), start);
        let mut request = MovementRequest {
            mode: MovementMode::Forward,
            session_id: 3,
            position: Position {
                x: 1.0,
                y: 1.0,
                ..Position::default()
            },
            created: start,
        };
        assert!(guard.accept(&request, start).is_err());
        guard
            .set_bounds(MovementBounds::new([-10.0; 3], [10.0; 3]).unwrap())
            .unwrap();
        guard.set_speed(Some(6.0), start);
        let now = start + Duration::from_millis(200);
        request.created = now;
        assert!(guard.accept(&request, now).is_err()); // diagonal exceeds 1.2 units
        request.position.y = 0.0;
        assert!(guard.accept(&request, now).is_ok());
        request.session_id = 2;
        assert!(guard.accept(&request, now).is_err());
        request.session_id = 3;
        assert!(guard
            .accept(&request, now + Duration::from_secs(1))
            .is_err());
        guard.correct(Position::default(), now);
        assert!(guard.accept(&request, now).is_err());
    }
    #[test]
    fn safe_bounds_cannot_drift_and_server_corrections_do_not_reanchor_them() {
        let start = Instant::now();
        let mut guard = MovementGuard::new(1, Position::default(), start);
        let bounds = MovementBounds::new([-0.5, -0.5, -0.1], [0.5, 0.5, 0.1]).unwrap();
        guard.set_bounds(bounds).unwrap();
        assert!(guard.set_bounds(bounds).is_err());
        guard.set_speed(Some(6.0), start);
        let now = start + Duration::from_millis(200);
        let mut request = MovementRequest {
            mode: MovementMode::Forward,
            session_id: 1,
            position: Position {
                x: 0.51,
                ..Position::default()
            },
            created: now,
        };
        assert!(guard.accept(&request, now).is_err());
        request.position.x = 0.5;
        assert!(guard.accept(&request, now).is_ok());
        assert!(guard.accept(&request, now).is_err());
        guard.correct(
            Position {
                x: 1.0,
                ..Position::default()
            },
            now,
        );
        guard.set_speed(Some(6.0), now);
        request.created = now + Duration::from_millis(200);
        assert!(guard.accept(&request, request.created).is_err());
        assert!(MovementBounds::new([f32::NAN; 3], [1.0; 3]).is_err());
        assert!(MovementBounds::new([2.0; 3], [1.0; 3]).is_err());
    }

    #[test]
    fn idle_time_cannot_be_spent_on_a_teleport() {
        let start = Instant::now();
        let mut guard = MovementGuard::new(1, Position::default(), start);
        guard
            .set_bounds(MovementBounds::new([-10.0; 3], [10.0; 3]).unwrap())
            .unwrap();
        guard.set_speed(Some(6.0), start);
        let now = start + Duration::from_secs(60);
        assert!(guard
            .accept(
                &MovementRequest {
                    mode: MovementMode::Forward,
                    session_id: 1,
                    position: Position {
                        x: 10.0,
                        ..Position::default()
                    },
                    created: now
                },
                now
            )
            .is_err());
    }
}
