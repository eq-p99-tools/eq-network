//! Zone-scoped movement submission with caller-supplied calibration.
use super::{
    gate::{Flags, UpdateGate},
    DiagonalRates, HeadingUpdate, MovementGuard, MovementMode, MovementRequest, PositionPacket,
    JUMP_OPCODE,
};
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
    /// Lateral calibration while also holding a longitudinal input.
    #[serde(default, skip_serializing_if = "DiagonalRates::is_unavailable")]
    pub diagonal: DiagonalRates,
    /// Forward world units per second before movement options apply.
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
        ensure!(
            [self.diagonal.run, self.diagonal.walk]
                .into_iter()
                .flatten()
                .all(|rate| rate.is_finite() && rate > 0.0),
            "invalid diagonal movement calibration"
        );
        Ok(())
    }
}

/// Position, packet counter, and input expiry for one zone admission.
#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)] // Independent session permissions and motion state.
pub struct MotionSession {
    guard: MovementGuard,
    spawn_id: u16,
    sequence: u16,
    position: Position,
    calibration: Option<MotionCalibration>,
    options: super::MovementOptions,
    last_sent: Instant,
    moving: bool,
    suspended: bool,
    falls: bool,
    heading_snapshots: bool,
    swimming: bool,
    /// How the client generation gates its own update, where it is read;
    /// see [`Self::with_gate`].
    gate: Option<UpdateGate>,
    /// The latest accepted sample the gate has not let out yet.
    held: Option<PositionPacket>,
    /// The last update sent, without its sequence: Titanium's gate sends
    /// only after a change.
    last_update: Option<PositionPacket>,
    /// The `EQMac` gate's pending flag and what it compares against.
    flags: Flags,
    /// When the host's latest sample was accepted.
    last_input: Instant,
}

/// What the gate does with an accepted sample.
#[derive(Clone, Copy)]
enum Release {
    Send,
    Hold,
    Unchanged,
}

/// One locally checked candidate, tied to the only movement owner until it
/// is submitted or dropped. Its sample and release decision cannot be replaced
/// by a later request, nor committed against a different admission.
struct PreparedMove<'a> {
    owner: &'a mut MotionSession,
    guard: MovementGuard,
    flags: Flags,
    sample: PositionPacket,
    release: Release,
    now: Instant,
    moving: bool,
}

impl PreparedMove<'_> {
    /// A checked candidate is accepted only here: held/unchanged candidates
    /// commit locally, and an immediate send commits after its callback succeeds.
    /// External callback side effects are never rolled back by this owner.
    fn submit(self, send: impl FnOnce(&PositionPacket) -> Result<()>) -> Result<()> {
        let Self {
            owner,
            guard,
            flags,
            sample,
            release,
            now,
            moving,
        } = self;
        match release {
            Release::Send => {
                send(&sample)?;
                owner.held = None;
                owner.last_sent = now;
                owner.sequence = owner.sequence.wrapping_add(1);
                owner.last_update = Some(PositionPacket {
                    sequence: 0,
                    ..sample
                });
            }
            Release::Hold => owner.held = Some(sample),
            Release::Unchanged => owner.held = None,
        }
        owner.flags = flags;
        owner.guard = guard;
        owner.position = sample.position;
        owner.moving = moving;
        owner.last_input = now;
        Ok(())
    }
}

impl MotionSession {
    /// Allows bounded swimming on a supported Titanium position stream.
    #[must_use]
    pub const fn with_swimming(mut self, enabled: bool) -> Self {
        self.swimming = enabled;
        self
    }

    /// Allows absolute mouse heading snapshots for a Titanium position stream.
    #[must_use]
    pub const fn with_heading_snapshots(mut self, enabled: bool) -> Self {
        self.heading_snapshots = enabled;
        self
    }

    /// Whether movement or its real stop still awaits local submission.
    /// Stationary heartbeat repeats do not hold an action behind a stop.
    /// This only observes this movement owner; it never forces an update or
    /// establishes a server acknowledgement.
    #[must_use]
    pub fn stop_pending(&self) -> bool {
        self.moving
            || self.held.as_ref().is_some_and(Self::moving_update)
            || self.last_update.as_ref().is_some_and(Self::moving_update)
    }

    /// Whether a command can finish this owner's pending stop: the same
    /// admission, a newer host stamp, and exactly its current point/heading.
    /// Normal submission still validates it; a refusal is not a sent stop.
    #[must_use]
    pub fn is_trailing_stop(&self, request: &MovementRequest) -> bool {
        self.stop_pending()
            && !self.suspended
            && request.session_id == self.guard.session_id
            && request.created > self.guard.sampled
            && request.position == self.position
    }

    /// Opaque equality context for a pending stop, invalidated by calibration,
    /// correction, withdrawal or a new owner. It is not a wire or server token.
    #[must_use]
    pub const fn stop_context(&self) -> Option<(u64, Instant)> {
        if self.suspended || self.calibration.is_none() {
            None
        } else {
            Some((self.guard.session_id, self.guard.input_epoch))
        }
    }

    /// Whether the expected trailing stop was accepted by this owner. A held
    /// accepted stop is distinct from its successful transport; callers still
    /// wait until [`Self::stop_pending`] is false.
    #[must_use]
    pub fn accepted_stop(&self, created: Instant) -> bool {
        self.guard.sampled == created
            && !self.moving
            && self
                .held
                .as_ref()
                .is_none_or(|sample| !Self::moving_update(sample))
    }

    #[allow(
        clippy::float_cmp,
        reason = "A submitted stop has exact zero wire fields, not an epsilon distance"
    )]
    fn moving_update(sample: &PositionPacket) -> bool {
        sample.animation != 0
            || sample.delta_heading != 0
            || sample.delta.iter().any(|value| *value != 0.0)
    }

    /// Lets the player's own update out only as the client generation's
    /// program would ([`UpdateGate`]): a sample the gate holds back is kept,
    /// and a later one replaces it, so the latest goes out once the gate
    /// opens ([`Self::tick`]). Accepting a sample is unchanged, so the
    /// host's own steps keep their pace.
    #[must_use]
    pub fn with_gate(mut self, gate: Option<UpdateGate>) -> Self {
        // The program's resets clear the last update's time, so the first
        // update after zoning in isn't held back (TAKP gate read Q1 6e).
        if matches!(gate, Some(UpdateGate::Flagged { .. })) {
            self.last_sent = self
                .last_sent
                .checked_sub(super::EQMAC_UPDATE_TICK)
                .unwrap_or(self.last_sent);
        }
        self.gate = gate;
        self
    }

    /// The least time between two updates under a spacing gate.
    fn spacing(&self) -> Duration {
        match self.gate {
            Some(UpdateGate::Changed { spacing }) => spacing,
            _ => Duration::ZERO,
        }
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

    /// Lets a grounded sample climb `per_step` in each program step it spans,
    /// the player's own climb ([`MovementGuard::set_climb`]).
    pub fn set_climb(&mut self, per_step: f32) {
        self.guard.set_climb(per_step);
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
        // The `EQMac` program's jump marks an update due (Q1 6c).
        if matches!(self.gate, Some(UpdateGate::Flagged { .. })) {
            self.flags.raise();
        }
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
        ensure!(
            !self.suspended && self.calibration.is_some(),
            "player input is withdrawn"
        );
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
            options: super::MovementOptions::default(),
            last_sent: now,
            moving: false,
            suspended: false,
            falls: false,
            heading_snapshots: false,
            swimming: false,
            gate: None,
            held: None,
            last_update: None,
            flags: Flags::default(),
            last_input: now,
        })
    }

    /// Current explicit movement choices for this admission.
    #[must_use]
    pub const fn options(&self) -> super::MovementOptions {
        self.options
    }

    /// Applies caller-stamped choices without granting speed or reusing queued
    /// input. The accepted position, sample time, tracked jump and fixed
    /// bounds remain.
    ///
    /// # Errors
    /// Refuses choices made before the latest reset or accepted sample. The
    /// caller also checks the command's future/expiry and session identity.
    pub fn set_options(&mut self, options: super::MovementOptions, created: Instant) -> Result<()> {
        ensure!(
            created >= self.guard.input_epoch && created >= self.guard.sampled,
            "movement options predate the current motion state"
        );
        if self.options == options {
            return Ok(());
        }
        self.stop_voluntary();
        self.options = options;
        self.guard.flying = options.fly;
        self.guard.input_epoch = created;
        Ok(())
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
        self.guard.correct(self.position, Instant::now());
    }

    /// Latest accepted world position, without implying a server acknowledgment.
    #[must_use]
    pub const fn position(&self) -> Position {
        self.position
    }

    /// Stops voluntary motion at the current owner point without revoking
    /// speed prerequisites, a prior jump, gravity or server corrections.
    /// Replace any paced moving update with its stationary stop; the normal
    /// update gate still decides when that existing position sample is sent.
    pub fn stop_voluntary(&mut self) {
        if self.stop_pending() {
            self.held = Some(PositionPacket {
                spawn_id: self.spawn_id,
                sequence: self.sequence,
                position: self.position,
                delta: [0.0; 3],
                animation: 0,
                delta_heading: 0,
            });
        }
        self.moving = false;
    }

    /// Whether a proposal contains only passive vertical motion. A rise
    /// must fit a jump already announced by this owner; no new climb or
    /// jump allowance is created. Normal sample checks still apply.
    #[must_use]
    #[allow(
        clippy::float_cmp,
        reason = "Passive movement preserves exact horizontal coordinates and heading"
    )]
    pub fn is_passive(&self, request: &MovementRequest) -> bool {
        if request.position.x != self.position.x
            || request.position.y != self.position.y
            || request.position.heading != self.position.heading
        {
            return false;
        }
        if request.mode == MovementMode::Swim {
            return request.position.z == self.position.z;
        }
        let elapsed = self.guard.interval(request.created);
        let rise = self.guard.jumped_at.map_or(0.0, |start| {
            let (from, to) = match start {
                super::JumpStart::Next => (0.0, elapsed),
                super::JumpStart::At(at) => {
                    let to = request.created.saturating_duration_since(at).as_secs_f32();
                    ((to - elapsed).max(0.0), to)
                }
            };
            self.guard.jump_rise(from, to)
        });
        request.position.z <= self.position.z + rise
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
        self.prepare_move_sample(request, now)?.submit(send)
    }

    /// Materialize and check exactly one immutable sample and release decision.
    /// No owner state changes before submission. The exclusive owner borrow
    /// prevents re-entrant motion changes during the existing send callback.
    /// Body/scene authority is not owned or certified by this local plan.
    #[allow(
        clippy::float_cmp,
        clippy::too_many_lines,
        reason = "One prepare/validate/encode transaction, with exact withdrawn coordinates"
    )]
    fn prepare_move_sample(
        &mut self,
        request: &MovementRequest,
        now: Instant,
    ) -> Result<PreparedMove<'_>> {
        ensure!(!self.suspended, "movement session is suspended");
        ensure!(
            request.mode != MovementMode::Fall || self.falls,
            "falling is not enabled for this server"
        );
        // Without player input, only an in-place descent is allowed. No fake
        // calibration, turn, climb, or horizontal rounding allowance is granted.
        if self.calibration.is_none() {
            ensure!(
                request.mode == MovementMode::Fall
                    && request.position.x == self.position.x
                    && request.position.y == self.position.y
                    && request.position.heading == self.position.heading
                    && request.position.z <= self.position.z,
                "player input is withdrawn; only vertical descent is allowed"
            );
        }
        let mut guard = self.guard.clone();
        let diagonal = match request.mode {
            MovementMode::GroundedDiagonal(value) => Some(value),
            _ => None,
        };
        let snapshot = request.heading == HeadingUpdate::Snapshot;
        ensure!(
            !snapshot || self.heading_snapshots,
            "absolute heading snapshots require Titanium movement"
        );
        ensure!(
            !snapshot || (0.0..512.0).contains(&request.position.heading),
            "heading snapshot is outside the canonical turn"
        );
        let along = diagonal.map_or(request.mode, |diagonal| diagonal.along.mode());
        let lateral_rate = if let Some(diagonal) = diagonal {
            ensure!(
                !self.options.fly,
                "diagonal movement requires a grounded mode"
            );
            Some(
                self.calibration
                    .and_then(|value| value.diagonal.for_mode(diagonal))
                    .ok_or_else(|| anyhow::anyhow!("diagonal movement is not calibrated"))?,
            )
        } else {
            None
        };
        let (speed, velocity_scale, animation) = if let Some(calibration) = self.calibration {
            match along {
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
                MovementMode::Swim => (
                    super::swim_speed(calibration.units_per_second),
                    calibration.velocity_scale,
                    0, // Swim animation/skill calibration is unverified.
                ),
                MovementMode::GroundedDiagonal(_) => unreachable!("longitudinal mode is cardinal"),
            }
        } else {
            (0.0, 0.0, 0)
        };
        // Switching mode changes the per-sample limit without resetting input expiry
        // or granting a new time budget. The clone is committed only after send succeeds.
        // The host can integrate a boosted forward step before collision
        // reclassifies it as Fall. Keep the same horizontal budget for the
        // transition and its bounded drift.
        let speed = if self.options.run_boost
            && matches!(along, MovementMode::Forward | MovementMode::Fall)
        {
            speed * super::MovementOptions::RUN_BOOST
        } else {
            speed
        };
        guard.speed = Some(speed);
        // The wire's velocity and turn rate cover the same interval the guard
        // budgets: from the last accepted request's stamp to this one's.
        let elapsed = guard.interval(request.created);
        let swimming = request.mode == MovementMode::Swim;
        let displacement = guard.accept_with_modes(request, now, lateral_rate, self.swimming)?;
        // Following a slope has no vertical stroke. Ordinary falls still
        // carry their descent in absolute Z. For a swim stroke Titanium
        // copies all three tick velocities into its existing update fields.
        let delta = [
            displacement[0] / elapsed * velocity_scale,
            displacement[1] / elapsed * velocity_scale,
            if swimming {
                displacement[2] / elapsed * velocity_scale
            } else {
                0.0
            },
        ];
        let moving = displacement[..if swimming { 3 } else { 2 }]
            .iter()
            .any(|d| d.abs() > f32::EPSILON);
        // Absolute heading and continuing yaw are independent fields.
        let turn = if snapshot {
            0
        } else {
            heading_velocity(self.position.heading, request.position.heading, elapsed)?
        };
        let sample = PositionPacket {
            spawn_id: self.spawn_id,
            sequence: self.sequence,
            position: request.position,
            delta,
            animation: if moving { animation } else { 0 },
            delta_heading: turn,
        };
        sample.check()?;
        let mut flags = self.flags.clone();
        let release = self.release(&sample, &mut flags, now)?;
        Ok(PreparedMove {
            owner: self,
            guard,
            flags,
            sample,
            release,
            now,
            moving: moving || turn != 0,
        })
    }

    /// What the generation's gate does with an accepted sample at `now`;
    /// `flags` is the `EQMac` gate's state, committed by the caller.
    fn release(&self, sample: &PositionPacket, flags: &mut Flags, now: Instant) -> Result<Release> {
        let since = now.saturating_duration_since(self.last_sent);
        Ok(match self.gate {
            None => Release::Send,
            Some(UpdateGate::Changed { spacing }) => {
                let unsequenced = PositionPacket {
                    sequence: 0,
                    ..*sample
                };
                if self.last_update == Some(unsequenced) {
                    Release::Unchanged
                } else if since < spacing {
                    Release::Hold
                } else {
                    Release::Send
                }
            }
            Some(UpdateGate::Flagged { pack }) => {
                flags.observe(sample, now);
                if flags.release(sample, since, pack)? {
                    Release::Send
                } else {
                    Release::Hold
                }
            }
        })
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
        let spacing = self.spacing();
        let since = now.saturating_duration_since(self.last_sent);
        if self.suspended {
            return Ok(false);
        }
        if let Some(UpdateGate::Flagged { pack }) = self.gate {
            return self.tick_flagged(now, since, pack, send);
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
            self.last_update = Some(PositionPacket {
                sequence: 0,
                ..sample
            });
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
        self.last_update = Some(PositionPacket {
            sequence: 0,
            ..sample
        });
        Ok(true)
    }

    /// The `EQMac` gate's pass: the timer and the stop mark an update due,
    /// and the latest state goes out once the gate lets it.
    fn tick_flagged(
        &mut self,
        now: Instant,
        since: Duration,
        pack: fn(&PositionPacket) -> Result<EncodedCommand>,
        send: impl FnOnce(&PositionPacket) -> Result<()>,
    ) -> Result<bool> {
        // A move without input for 250 ms has stopped: its speed falls to 0,
        // a change the program flags at once.
        let stopped = self.moving
            && now.saturating_duration_since(self.last_input) >= Duration::from_millis(250);
        let still = PositionPacket {
            spawn_id: self.spawn_id,
            sequence: self.sequence,
            position: self.position,
            delta: [0.0; 3],
            animation: 0,
            delta_heading: 0,
        };
        let candidate = match self.held {
            _ if stopped => still,
            Some(held) => PositionPacket {
                sequence: self.sequence,
                ..held
            },
            // The latest move went out and the host's next has not come.
            None if self.moving => return Ok(false),
            None => still,
        };
        candidate.check()?;
        let mut flags = self.flags.clone();
        flags.observe(&candidate, now);
        let released = flags.release(&candidate, since, pack)?;
        if released {
            send(&candidate)?;
            self.held = None;
            self.sequence = self.sequence.wrapping_add(1);
            self.last_sent = now;
            self.last_update = Some(PositionPacket {
                sequence: 0,
                ..candidate
            });
        }
        if stopped {
            self.moving = false;
            self.held = None;
        }
        self.flags = flags;
        Ok(released)
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
/// Titanium `eqgame.exe` sends current turn rate per 50 ms tick times 20,
/// cut to a whole number (0x4652e0–0x4652fe); a full keyboard turn of 12
/// a tick sends 240. The paced-turn path derives its own rate from the
/// samples. A completed snapshot supplies zero continuing yaw.
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
