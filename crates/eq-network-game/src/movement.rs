//! Titanium movement encoding and session-scoped displacement validation.

mod carry;
mod counter;
mod diagonal;
mod gate;
pub mod history;
mod inputs;
pub use inputs::{
    EqMacFatigueInputs, SpeedInputAcceptance, SpeedInputState, SpeedInputUnavailable, SpeedInputs,
};
mod session;
mod shoreline;
mod speeds;
pub use carry::{between_reports, carry, Carried, ZBetween};
pub use counter::{UpdateCounter, TITANIUM_HISTORY_FRAMES_PER_SECOND, TITANIUM_UPDATE_COUNTER};
pub use diagonal::{DiagonalAlong, DiagonalRates, GroundedDiagonal, StrafeSide};
pub use gate::{
    FlagRule, Hold, UpdateGate, EQMAC_FLAGS, EQMAC_HEADING_STEP, EQMAC_REPEATS, EQMAC_SPEED_STEP,
    EQMAC_UPDATE_SPACING, EQMAC_UPDATE_TICK, TITANIUM_FLAGS, TITANIUM_FROZEN_SPACING,
    TITANIUM_HEADING_STEP, TITANIUM_SPEED_STEP, TITANIUM_UPDATE_TICK,
};
pub use session::{
    titanium_jump, BackwardCalibration, MotionCalibration, MotionSession, StrafeCalibration,
    WalkCalibration,
};
pub use shoreline::{
    ShorelineFrame, ShorelineStroke, ShorelineTrace, ShorelineVelocity, SHORELINE_COOLDOWN,
    SHORELINE_FRAMES, SHORELINE_FRAME_MILLIS,
};
pub use speeds::{eqmac as eqmac_speeds, titanium as titanium_speeds, SpeedState, Speeds};

use crate::{command::EncodedCommand, world::Position};
use anyhow::{ensure, Result};
use std::time::{Duration, Instant};

/// Maximum grounded stair adjustment per sample, in world units, where the
/// session has no climb of the player's own ([`MovementGuard::set_climb`]),
/// and the most a grounded sample may descend. This accommodates two-unit
/// classic zone risers; it is a client collision policy, not a measured
/// server limit or permission to encode falling as grounded motion.
pub const MAX_GROUNDED_STEP: f32 = 2.0;

/// The program steps a player once per 50 ms update (R-s; the gravity is
/// "a 50 ms step", [`DEFAULT_GRAVITY`]).
pub const PROGRAM_STEP_SECONDS: f32 = 0.05;

/// How many program steps `seconds` spans, at least one: the count of a
/// climb's budget ([`climb_reach`]). A sample of exactly n steps counts n, not n + 1 from `f32`
/// rounding in the seconds.
#[must_use]
pub fn program_steps(seconds: f32) -> u32 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 1;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Clamped.
    let steps = (seconds / PROGRAM_STEP_SECONDS - 1e-3)
        .ceil()
        .clamp(1.0, 1e6) as u32;
    steps
}

/// How far a grounded walk may rise over `steps` program steps, climbing
/// at most `per_step` in each ([`MovementGuard::set_climb`]). The guard and
/// eq-client's walk count the same budget.
#[must_use]
pub fn climb_reach(per_step: f32, steps: u32) -> f32 {
    #[allow(clippy::cast_precision_loss)] // At most a few steps.
    let steps = steps.max(1) as f32;
    per_step * steps
}

/// Fastest descent a [`MovementMode::Fall`] sample may claim, in world units per
/// second, until the session sets the generation's own ([`fall_speed`]): a
/// provisional default, not a measured limit.
pub const MAX_FALL_SPEED: f32 = 40.0;

/// The gravity both programs use until a zone's header gives its own: the
/// fall speed added each 50 ms step (TAKP Batch 1 R3; Titanium sitting 8g,
/// the header's +420 field, default 0.4).
pub const DEFAULT_GRAVITY: f32 = 0.4;

/// The fastest a fall goes at the default gravity, before a zone's header
/// gives its own: [`fall_speed`] of none, 792 units a second.
pub const EQMAC_FALL_SPEED: f32 = 99.0 * DEFAULT_GRAVITY * 20.0;

/// The fastest a fall goes under a zone's gravity, in world units per
/// second: the guard's limit for a fall sample on both client generations.
///
/// Both programs add the gravity (the fall speed gained each 50 ms step) and
/// keep 0.99 of the vertical speed after each update (TAKP Batch 1 R3,
/// checklist 6.161; Titanium sitting 8i), and neither caps a fall. At one
/// step an update the speed tends to 99 × gravity a step, 20 steps a
/// second: 792 at the default 0.4. This is derived, not read: how many steps
/// an update covers depends on the update rate the client emulates
/// (checklist 6.294).
///
/// The gravity is the zone header's +420 field as decoded, with no default of
/// its own; `None` (no header yet) gives the program's default 0.4. A header
/// of 0 is used as read, so a fall cannot speed up (Titanium sitting 8g:
/// header 0 kept; the TAKP read R3 is silent on a zone's value). A negative or
/// non-number header is unread on both programs and counts as the default.
#[must_use]
pub fn fall_speed(gravity: Option<f32>) -> f32 {
    let gravity = gravity
        .filter(|gravity| gravity.is_finite() && *gravity >= 0.0)
        .unwrap_or(DEFAULT_GRAVITY);
    99.0 * gravity * 20.0
}

/// The speed a jump starts at, in world units per second, on both client
/// generations: the program adds 1.65 × the character's jump value (1.35)
/// to the rising speed a 50 ms step (Titanium sittings 8h and 8i; TAKP
/// sitting 2, M1), 44.55 units a second.
pub const JUMP_SPEED: f32 = 1.65 * 1.35 * 20.0;

/// A jump's whole starting speed: the factor of a player whose health and
/// weight take nothing from it.
#[must_use]
pub const fn full_jump() -> f32 {
    1.0
}

/// A zone's gravity in world units per second squared: the header's +420
/// field is the fall speed added each 50 ms step, so 160 at the default 0.4.
/// A header the program would read as the default counts as it, as in
/// [`fall_speed`].
#[must_use]
pub fn gravity_per_second(gravity: Option<f32>) -> f32 {
    let gravity = gravity
        .filter(|gravity| gravity.is_finite() && *gravity >= 0.0)
        .unwrap_or(DEFAULT_GRAVITY);
    gravity * 20.0 * 20.0
}

/// How often a standing character repeats its position. The official P99 client
/// sends one about every 1.1 s (median of its recorded stationary updates).
pub const STATIONARY_HEARTBEAT: Duration = Duration::from_secs(1);

/// The least time between two of the player's own position updates in the
/// Titanium program: it sends one only after more than 280 ms since the last
/// (from the program, Titanium sitting 6, T1; checklist 1.6; 0x4ac50b-0x4ac549,
/// read R13 a), and only once its update flag is set ([`TITANIUM_FLAGS`]).
pub const TITANIUM_POSITION_SPACING: Duration = Duration::from_millis(280);

/// `OP_Jump` (Titanium): an empty notice that the character jumped. `EQEmu` only
/// charges endurance for it; the arc itself travels in position updates.
pub const JUMP_OPCODE: u16 = 0x0797;

/// `OP_ClientUpdate` (Titanium): the player's position, sent unreliably since
/// the next one replaces it.
pub const POSITION_OPCODE: u16 = 0x14cb;

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
    /// Concurrent longitudinal and lateral movement along one ground heading.
    GroundedDiagonal(GroundedDiagonal),
    /// Airborne descent after leaving a ledge: forward speed and animation, while
    /// height falls under gravity instead of following the ground.
    Fall,
    /// Locally collision-checked water motion. Never serialized as a mode.
    Swim,
    /// One bounded Titanium shoreline arc, using ordinary position updates.
    Shoreline(ShorelineTrace),
}

impl MovementMode {
    /// Voluntary water motion never receives the walk-while-stunned allowance.
    #[must_use]
    pub const fn is_water_motion(self) -> bool {
        matches!(self, Self::Swim | Self::Shoreline(_))
    }
}

/// Conservative three-dimensional swim budget for a Titanium session.
///
/// The slower native wet horizontal branch settles at `0.02 / (1 - 0.94)`
/// input units per tick (`eqgame.exe` 0x4c052d and 0x4c0534), versus
/// `0.8 / (1 - 0.625)` on land (0x4c04c1 and 0x4c04cd). Use this
/// fraction of the current unboosted grant on every axis. This is a bound
/// for this client, not a claim about the native skill, race or buff formula;
/// the shared three-axis budget is inferred (recording checklist 6.390).
#[must_use]
pub fn swim_speed(granted_units_per_second: f32) -> f32 {
    granted_units_per_second * 0.156_25
}

/// Checks that a swim stroke's displacement `delta` goes no further than
/// `limit` behind `heading`, the heading the stroke ends facing, on the
/// ground plane.
fn swim_backing(heading: f32, delta: [f32; 3], limit: f32) -> Result<()> {
    let (sin, cos) = (heading / 512.0 * std::f32::consts::TAU).sin_cos();
    let behind = -(delta[0] * sin + delta[1] * cos);
    // The tolerance covers f32 rounding, as the other budgets'.
    ensure!(
        behind <= limit + 0.01,
        "backing up in water exceeds its rate"
    );
    Ok(())
}

/// How the heading in a movement request was produced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum HeadingUpdate {
    /// The heading follows a paced turn. The session derives a checked
    /// velocity from the heading change and elapsed sample time.
    #[default]
    Rate,
    /// The heading is the endpoint of locally applied mouse input.
    /// Titanium `eqgame.exe` packs absolute heading independently of the
    /// current yaw rate (0x46529f–0x4652c4 and 0x4652e0–0x4652fe).
    /// A snapshot uses that heading field and supplies zero continuing yaw;
    /// that a mouse-completed facing goes out with zero yaw is inferred
    /// (recording checklist 6.388).
    Snapshot,
}

/// A graphical client's proposed position, tied to its current zone admission.
#[derive(Clone, Debug, PartialEq)]
pub struct MovementRequest {
    /// Selects the calibrated movement limits and animation.
    pub mode: MovementMode,
    /// Connection identifier from `WorldEvent::Entered`.
    pub session_id: u64,
    /// Position after local collision resolution.
    pub position: Position,
    /// Whether the proposed heading includes a paced turn or a completed mouse turn.
    pub heading: HeadingUpdate,
    /// Local creation time; queued input expires instead of replaying after a stall.
    pub created: Instant,
    /// What the client's floor sweep stands the player on, if it moves: a
    /// rising lift may carry the player up with it
    /// ([`MovementGuard::allow_lift_rise`]).
    pub support: Option<Support>,
}

/// A moving thing the player stands on, which may carry them.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    /// A door by its id: a Titanium lift or other vertical door.
    Door(u8),
}

impl MovementRequest {
    /// Proposes a position with the ordinary checked turn-rate intent.
    #[must_use]
    pub const fn new(
        mode: MovementMode,
        session_id: u64,
        position: Position,
        created: Instant,
    ) -> Self {
        Self {
            mode,
            session_id,
            position,
            heading: HeadingUpdate::Rate,
            created,
            support: None,
        }
    }

    /// The same request, standing on `support`.
    #[must_use]
    pub const fn supported_by(mut self, support: Support) -> Self {
        self.support = Some(support);
        self
    }
}

/// One client-to-server motion sample, independent of transport sequencing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionPacket {
    /// Active character's own zone spawn ID.
    pub spawn_id: u16,
    /// Wrapping counter, owned by the zone session: Titanium's
    /// movement-history entry count, `EQMac`'s count of updates
    /// ([`UpdateCounter`]).
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
        self.check()?;
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

    /// Checks that the sample can be sent: an own spawn, finite values, and
    /// animation and turn rate within their signed 10-bit fields.
    ///
    /// # Errors
    /// Rejects invalid IDs, non-finite coordinates, and out-of-range animation.
    pub fn check(self) -> Result<()> {
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
        Ok(())
    }

    /// The position update carrying this sample.
    ///
    /// # Errors
    /// Rejects what [`Self::encode`] rejects.
    pub fn packet(self) -> Result<EncodedCommand> {
        Ok(EncodedCommand {
            opcode: POSITION_OPCODE,
            body: self.encode()?.to_vec(),
        })
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

/// A selected forward running speed in hundredths of the calibrated speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(transparent)]
pub struct RunSpeedMultiplier(u16);

impl RunSpeedMultiplier {
    /// Ordinary calibrated speed.
    pub const NORMAL: Self = Self(100);
    /// Highest multiplier offered by the client.
    pub const MAX: Self = Self(140);

    /// A valid selection from 100 through 140 percent.
    #[must_use]
    pub const fn from_percent(percent: u16) -> Option<Self> {
        if percent >= Self::NORMAL.0 && percent <= Self::MAX.0 {
            Some(Self(percent))
        } else {
            None
        }
    }

    /// The multiplier as a whole percentage.
    #[must_use]
    pub const fn percent(self) -> u16 {
        self.0
    }

    /// The factor for scaling horizontal forward and fall speeds.
    #[must_use]
    pub fn factor(self) -> f32 {
        f32::from(self.0) / 100.0
    }
}

impl Default for RunSpeedMultiplier {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// Explicit movement and posture departures, all disabled by default. These are local
/// policy choices, not native movement rules or detection guarantees.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct MovementOptions {
    /// Permit fresh voluntary motion while stunned. Other stun gates remain.
    pub walk_while_stunned: bool,
    /// Scale the forward running component. A forward step that leaves ground
    /// keeps this horizontal budget as a fall sample.
    pub run_speed_multiplier: RunSpeedMultiplier,
    /// Permit vertical travel within the current mode's elapsed speed budget.
    pub fly: bool,
    /// Departures for a player who lies on the ground.
    pub lying: LyingOptions,
}

/// Departures for a player who lies on the ground, as a feign leaves them,
/// both off by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct LyingOptions {
    /// Permit walking, at walk speed only and without jumping, where the
    /// program moves nothing ([`crate::feign::LyingRules::holds_still`]). No
    /// stand is sent, so a server keeps a feign it granted. Recording
    /// checklist 6.454.
    pub walk: bool,
    /// Stand the player up when a hit lands, as the program does only for a
    /// sitter or a looter ([`crate::feign::LyingRules::hit_stands`]). The
    /// stand ends a feign on the server. Recording checklist 6.453.
    pub stand_when_hit: bool,
}

impl MovementOptions {
    /// Whether two choices allow the same motion modes, regardless of run speed.
    #[must_use]
    pub const fn same_permissions(self, other: Self) -> bool {
        self.walk_while_stunned == other.walk_while_stunned
            && self.fly == other.fly
            && self.lying.walk == other.lying.walk
    }
}

/// Validates proposed motion without trusting render-frame timing or accumulating idle credit.
#[derive(Clone, Debug)]
pub struct MovementGuard {
    session_id: u64,
    position: Position,
    /// The host's stamp on the last accepted request (or when speed was set
    /// or corrected): a sample's budget runs from here to its own stamp, so
    /// the time a sample waits before it is processed neither adds to nor
    /// takes from what it may cover.
    sampled: Instant,
    input_epoch: Instant,
    speed: Option<f32>,
    flying: bool,
    bounds: Option<MovementBounds>,
    /// The fastest descent a fall sample may claim, in world units per second.
    fall_speed: f32,
    /// How far a grounded sample may rise in each program step it spans,
    /// in world units: the player's own climb once the session sets it
    /// ([`Self::set_climb`]), else [`MAX_GROUNDED_STEP`].
    climb: Option<f32>,
    /// How a jump rises, where the client generation's jump is read: its
    /// starting speed and the zone's gravity, in units a second and a second
    /// squared.
    jump: Option<(f32, f32)>,
    /// The share of the jump's starting speed the player has now: the
    /// program's health and weight factor ([`Self::set_jump_factor`]).
    jump_factor: f32,
    /// The share the latest announced jump started with.
    launch_factor: f32,
    /// When the latest jump the session announced started, until gravity
    /// has spent its speed.
    jumped_at: Option<JumpStart>,
    /// How far the next sample may rise with the lift its request names
    /// ([`Self::allow_lift_rise`]).
    lift_rise: f32,
    /// The lift the rider has named since `since`, and how far they have
    /// risen while naming it.
    lift: Option<LiftCredit>,
    last_shoreline: Option<Instant>,
    shoreline: Option<ShorelineArc>,
    shoreline_velocity: Option<[f32; 3]>,
    previous_swim: bool,
}

#[derive(Clone, Copy, Debug)]
struct ShorelineArc {
    start: Instant,
    spent: u32,
    dry: bool,
    velocity: ShorelineVelocity,
}

/// A rider's running account with one lift: they may rise in all as far as
/// the lift has risen since they named it, and never more.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LiftCredit {
    door: u8,
    since: Instant,
    spent: f32,
}

/// When an announced jump started, for [`MovementGuard`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JumpStart {
    /// With the interval of the next sample: the client's step that
    /// announces a jump already rises over its whole interval.
    Next,
    /// At this instant.
    At(Instant),
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
            flying: false,
            bounds: None,
            fall_speed: MAX_FALL_SPEED,
            climb: None,
            jump: None,
            jump_factor: 1.0,
            launch_factor: 1.0,
            jumped_at: None,
            lift_rise: 0.0,
            lift: None,
            last_shoreline: None,
            shoreline: None,
            shoreline_velocity: None,
            previous_swim: false,
        }
    }

    /// Lets a sample rise as the client generation's jump does after the
    /// session announces one ([`Self::jumped`]): starting at `speed` and
    /// slowed by `gravity`, in units a second and a second squared (the
    /// jump read in Titanium sittings 8h and 8i and TAKP sitting 2).
    pub fn set_jump(&mut self, speed: f32, gravity: f32) {
        self.jump = Some((speed, gravity)).filter(|(speed, gravity)| {
            speed.is_finite() && *speed > 0.0 && gravity.is_finite() && *gravity >= 0.0
        });
    }

    /// Sets the share of the jump's starting speed the player has: the
    /// Titanium program launches a jump at its health and weight factor
    /// times the jump speed (0x4193c5, read at 0x4aca9f and 0x4acc18), the
    /// factor that also slows running ([`Speeds::jump_factor`]).
    /// Held to 0..1; anything else counts as 1.
    ///
    /// When health or weight changes, a frontend may launch one more jump at
    /// the old factor before the new `MotionState` reaches it. A jump
    /// announced after this call is bounded by the new factor, so at most
    /// that one sample is refused and sent again; nothing desyncs.
    pub fn set_jump_factor(&mut self, factor: f32) {
        self.jump_factor = if factor.is_finite() {
            factor.clamp(0.0, 1.0)
        } else {
            1.0
        };
    }

    /// The session announced a jump: the next sample may rise as the jump
    /// does over its whole interval, and the ones after it on, until gravity
    /// has spent its speed. The jump keeps the factor it started with.
    pub fn jumped(&mut self) {
        if self.jump.is_some() {
            self.jumped_at = Some(JumpStart::Next);
            self.launch_factor = self.jump_factor;
        }
    }

    /// How far the announced jump rises between `from` and `to` seconds
    /// after it started: its speed less what gravity takes, and nothing
    /// past the top.
    fn jump_rise(&self, from: f32, to: f32) -> f32 {
        let Some((speed, gravity)) = self.jump else {
            return 0.0;
        };
        let speed = speed * self.launch_factor;
        let height = |t: f32| {
            let t = if gravity > 0.0 {
                t.min(speed / gravity)
            } else {
                t
            };
            speed * t - gravity * t * t / 2.0
        };
        (height(to) - height(from)).max(0.0)
    }

    /// Lets a grounded sample rise `per_step` in each 50 ms program step it
    /// spans, the player's own climb: in both programs a step stands the
    /// feet on whatever floor the sweep from the spawn's origin finds, a
    /// stand height above them (Titanium 0x48a6f2 / 0x48a0fc, TAKP
    /// 0x4b3a84; read 6.344), so a climb's pace does not depend on how long a
    /// sample is. Until it is set, a sample rises one riser,
    /// [`MAX_GROUNDED_STEP`], however long it is. A value that is not a
    /// positive number is ignored.
    pub fn set_climb(&mut self, per_step: f32) {
        if per_step.is_finite() && per_step > 0.0 {
            self.climb = Some(per_step);
        }
    }

    /// When the rider's account with lift `door` began: the start of the
    /// lift's rise they may spend. A lift other than the one last named
    /// starts a new account at the last accepted sample.
    #[must_use]
    pub fn lift_since(&self, door: u8) -> Instant {
        self.lift
            .filter(|credit| credit.door == door)
            .map_or(self.sampled, |credit| credit.since)
    }

    /// Lets the next sample also rise, with rounding, on top of its own
    /// climb or jump, what is left of `risen`: how far lift `door` has risen
    /// since [`Self::lift_since`], which the owner of the zone's doors works
    /// out ([`crate::doors::titanium::Lifts::carry`]). The rise the rider
    /// has already spent with that lift is taken off, so a client whose
    /// lift runs behind the session's may spend late what it did not spend
    /// early, but never more than the lift rose. The program's floor sweep
    /// stands a rider on a rising lift and sends nothing of its own (read
    /// R1). `None`, or a lift that sank, closes the account.
    pub fn allow_lift_rise(&mut self, lift: Option<(u8, f32)>) {
        let Some((door, risen)) = lift.filter(|(_, risen)| risen.is_finite()) else {
            self.lift = None;
            self.lift_rise = 0.0;
            return;
        };
        let since = self.lift_since(door);
        let spent = self
            .lift
            .filter(|credit| credit.door == door)
            .map_or(0.0, |credit| credit.spent);
        self.lift = Some(LiftCredit { door, since, spent });
        let unspent = risen - spent;
        self.lift_rise = if unspent > 0.0 { unspent + 0.01 } else { 0.0 };
    }

    /// The rise [`Self::allow_lift_rise`] allows `request`: none unless it
    /// names what carries it.
    #[must_use]
    pub fn lift_rise_for(&self, request: &MovementRequest) -> f32 {
        if request.support.is_some() {
            self.lift_rise
        } else {
            0.0
        }
    }

    /// The rise [`Self::allow_lift_rise`] allows the next sample.
    #[must_use]
    pub const fn lift_rise(&self) -> f32 {
        self.lift_rise
    }

    /// When the last accepted sample was made: the start of the next one's
    /// interval.
    #[must_use]
    pub const fn sampled(&self) -> Instant {
        self.sampled
    }

    /// Lets a fall sample descend as fast as the client generation's own
    /// falls go, in world units per second, in place of [`MAX_FALL_SPEED`].
    pub fn set_fall_speed(&mut self, fall_speed: f32) {
        self.fall_speed = fall_speed;
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
        self.clear_shoreline();
    }

    /// Resets both position and pending motion after a server correction.
    pub fn correct(&mut self, position: Position, now: Instant) {
        self.position = position;
        self.sampled = now;
        self.input_epoch = now;
        self.speed = None;
        self.jumped_at = None;
        self.clear_shoreline();
    }

    fn clear_shoreline(&mut self) {
        self.shoreline = None;
        self.shoreline_velocity = None;
        self.previous_swim = false;
    }

    fn check_shoreline(
        &self,
        request: &MovementRequest,
        trace: ShorelineTrace,
        granted: f32,
        displacement: [f32; 3],
    ) -> Result<(ShorelineArc, [f32; 3])> {
        let Some((_, gravity)) = self.jump else {
            anyhow::bail!("shoreline gravity is not calibrated");
        };
        let mut arc = if trace.start {
            ensure!(
                self.previous_swim
                    && self.shoreline.is_none()
                    && trace.frames[0].upward()
                    && trace.frames[0].water()
                    && trace.frames[1] == ShorelineFrame::EMPTY
                    && self.last_shoreline.is_none_or(|last| {
                        request.created.saturating_duration_since(last) >= SHORELINE_COOLDOWN
                    }),
                "shoreline impulse is not ready"
            );
            ShorelineArc {
                start: request.created,
                spent: 0,
                dry: false,
                velocity: ShorelineVelocity::start(self.position.heading),
            }
        } else {
            self.shoreline
                .ok_or_else(|| anyhow::anyhow!("shoreline continuation has no impulse"))?
        };
        ensure!(
            request.created.saturating_duration_since(arc.start) < SHORELINE_COOLDOWN,
            "shoreline input expired"
        );
        let mut millis = 0;
        let mut horizontal = 0.0;
        let mut lower = [0.0; 3];
        let mut upper = [0.0; 3];
        let mut empty = false;
        for frame in trace.frames {
            if frame == ShorelineFrame::EMPTY {
                empty = true;
                continue;
            }
            ensure!(
                !empty && (!arc.dry || !frame.water()),
                "invalid shoreline frame order"
            );
            arc.dry |= !frame.water();
            millis += u32::from(frame.millis());
            arc.velocity.advance(frame, granted, gravity);
            let ([x, y], vertical) = arc.velocity.travel(frame);
            horizontal += x.hypot(y);
            for (axis, step) in [x, y, vertical].into_iter().enumerate() {
                lower[axis] += step.min(0.0);
                upper[axis] += step.max(0.0);
            }
            frame.stop(&mut arc.velocity);
        }
        arc.spent += millis;
        let available = request
            .created
            .saturating_duration_since(self.sampled)
            .as_millis()
            .min(250);
        ensure!(
            millis > 0
                && u128::from(millis) <= available
                && u128::from(arc.spent) <= SHORELINE_COOLDOWN.as_millis()
                && displacement[0].hypot(displacement[1]) <= horizontal + 0.01
                && displacement.iter().enumerate().all(|(axis, value)| {
                    (lower[axis] - 0.01..=upper[axis] + 0.01).contains(value)
                }),
            "shoreline proposal exceeds its frame budget"
        );
        let wire = if trace.finish {
            [0.0; 3]
        } else {
            [
                arc.velocity.horizontal[0],
                arc.velocity.horizontal[1],
                arc.velocity.vertical,
            ]
        };
        Ok((arc, wire))
    }

    /// Validates one fresh proposal. Failure never changes the accepted position.
    ///
    /// # Errors
    /// Rejects disabled, stale, foreign-session, non-finite, or excessive motion.
    pub fn accept(&mut self, request: &MovementRequest, now: Instant) -> Result<[f32; 3]> {
        self.accept_with_modes(
            request,
            now,
            RunSpeedMultiplier::NORMAL,
            None,
            (false, None),
        )
    }

    /// `swimming` says whether the session swims, and `swim_backward` bounds
    /// a swim stroke's backward component, in units a second, where the
    /// generation reads one ([`MotionCalibration::swim_backward_units_per_second`]).
    fn accept_with_modes(
        &mut self,
        request: &MovementRequest,
        now: Instant,
        run_speed_multiplier: RunSpeedMultiplier,
        lateral_rate: Option<f32>,
        (swimming, swim_backward): (bool, Option<f32>),
    ) -> Result<[f32; 3]> {
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
        let horizontal_speed = speed * run_speed_multiplier.factor();
        let elapsed = self.interval(request.created);
        ensure!(elapsed >= 0.05, "movement samples arrive too frequently");
        let delta = [
            request.position.x - self.position.x,
            request.position.y - self.position.y,
            request.position.z - self.position.z,
        ];
        if let MovementMode::Shoreline(trace) = request.mode {
            return self.accept_shoreline(request, trace, speed, delta, swimming);
        }
        if request.mode == MovementMode::Swim {
            ensure!(swimming && !self.flying, "swimming is not available");
            ensure!(
                delta[0].hypot(delta[1]).hypot(delta[2]) <= speed * elapsed + 0.01,
                "swimming exceeds its three-dimensional time budget"
            );
            if let Some(backward) = swim_backward {
                swim_backing(request.position.heading, delta, backward * elapsed)?;
            }
        } else if let MovementMode::GroundedDiagonal(diagonal) = request.mode {
            // A known jump excludes fresh grounded diagonal input. The
            // ascent tracker does not establish where the player lands.
            ensure!(
                self.jumped_at.is_none(),
                "grounded diagonal movement is unavailable during a tracked jump"
            );
            ensure!(
                !self.flying
                    && lateral_rate.is_some_and(|lateral| {
                        diagonal.admits_with_heading(
                            self.position,
                            request.position,
                            request.heading,
                            horizontal_speed,
                            lateral,
                            elapsed,
                        )
                    }),
                "diagonal movement exceeds its grounded component budgets"
            );
        } else {
            // The tolerance covers f32 rounding in the distance and the
            // stamps' conversion to seconds; it grants no extra time.
            ensure!(
                delta[0].hypot(delta[1]) <= horizontal_speed * elapsed + 0.01,
                "movement exceeds elapsed-time budget"
            );
        }
        // A fall may only descend, at most at terminal speed; grounded modes climb
        // the player's own reach a program step ([`Self::set_climb`]) and come
        // down at most a riser, past which the client's walk falls. After a
        // jump, any sample may also rise as far as the jump does over the
        // sample's own interval.
        let since_jump = self.jumped_at.map(|start| match start {
            JumpStart::Next => (0.0, elapsed),
            JumpStart::At(at) => {
                let to = request.created.saturating_duration_since(at).as_secs_f32();
                ((to - elapsed).max(0.0), to)
            }
        });
        let rise = since_jump.map_or(0.0, |(from, to)| self.jump_rise(from, to))
            + self.lift_rise_for(request);
        let vertical = if request.mode.is_water_motion() {
            true // The single 3D budget already checks vertical movement.
        } else if self.flying {
            // Flight uses the unscaled mode rate for ascent and descent.
            delta[2].abs() <= speed * elapsed + 0.01
        } else if request.mode == MovementMode::Fall {
            delta[2] <= MAX_GROUNDED_STEP + rise
                && -delta[2] <= self.fall_speed * elapsed + MAX_GROUNDED_STEP
        } else {
            let climb = self.climb.map_or(MAX_GROUNDED_STEP, |per_step| {
                climb_reach(per_step, program_steps(elapsed))
            });
            delta[2] <= climb + rise && -delta[2] <= MAX_GROUNDED_STEP
        };
        ensure!(
            vertical,
            "vertical motion requires a supported movement mode"
        );
        self.record_move(request);
        self.update_jump_after_move(request.mode, request.created, since_jump.map(|(_, to)| to));
        Ok(delta)
    }

    fn accept_shoreline(
        &mut self,
        request: &MovementRequest,
        trace: ShorelineTrace,
        speed: f32,
        delta: [f32; 3],
        swimming: bool,
    ) -> Result<[f32; 3]> {
        ensure!(
            swimming && !self.flying,
            "shoreline swimming is not available"
        );
        let (arc, wire) = self.check_shoreline(request, trace, speed, delta)?;
        self.record_move(request);
        if trace.start {
            self.last_shoreline = Some(arc.start);
        }
        self.shoreline = (!trace.finish).then_some(arc);
        self.shoreline_velocity = Some(wire);
        self.jumped_at = None;
        Ok(delta)
    }

    fn record_move(&mut self, request: &MovementRequest) {
        // Any rise while a lift is named counts against its account.
        if let Some(credit) = self.lift.as_mut() {
            if request.support.is_some() {
                let risen = (request.position.z - self.position.z).max(0.0);
                credit.spent += risen.min((self.lift_rise - 0.01).max(0.0));
            } else {
                self.lift = None;
            }
        }
        self.lift_rise = 0.0;
        self.position = request.position;
        self.sampled = request.created;
        self.clear_shoreline();
        self.previous_swim = request.mode == MovementMode::Swim;
    }

    /// Ends jump tracking on water entry or once its upward allowance is spent.
    fn update_jump_after_move(
        &mut self,
        mode: MovementMode,
        created: Instant,
        since_jump: Option<f32>,
    ) {
        if mode.is_water_motion() {
            self.jumped_at = None;
        } else if let (Some((speed, gravity)), Some(to)) = (self.jump, since_jump) {
            self.jumped_at = if gravity > 0.0 && to >= speed / gravity {
                None
            } else {
                created
                    .checked_sub(Duration::from_secs_f32(to))
                    .map(JumpStart::At)
            };
        }
    }

    /// The seconds a request stamped `created` covers since the last
    /// accepted one, on the host's stamps, at most 0.25.
    fn interval(&self, created: Instant) -> f32 {
        created
            .saturating_duration_since(self.sampled)
            .as_secs_f32()
            .min(0.25)
    }
}

fn valid_position(p: Position) -> bool {
    [p.x, p.y, p.z, p.heading].iter().all(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swim_backing_bounds_only_the_share_behind_the_ending_heading() {
        // Heading 128 faces +x and 384 faces -x; a stroke the other way backs up.
        for (heading, forward) in [(128.0, 1.0), (384.0, -1.0)] {
            assert!(swim_backing(heading, [-2.0 * forward, 0.0, 0.0], 1.0).is_err());
            assert!(swim_backing(heading, [-0.5 * forward, 0.0, 0.0], 1.0).is_ok());
            assert!(swim_backing(heading, [2.0 * forward, 0.0, 0.0], 0.0).is_ok());
            // Sideways or up and down is not behind.
            assert!(swim_backing(heading, [0.0, 3.0, -3.0], 0.0).is_ok());
        }
    }

    #[test]
    fn a_fall_tends_to_99_steps_of_the_zones_gravity() {
        assert!((fall_speed(None) - 792.0).abs() < 0.01);
        assert!((EQMAC_FALL_SPEED - 792.0).abs() < 0.01);
        assert!((fall_speed(Some(0.8)) - 1584.0).abs() < 0.01);
        // A header of 0 pulls nothing; a broken one counts as the default.
        assert!(fall_speed(Some(0.0)).abs() < f32::EPSILON);
        assert!((fall_speed(Some(-1.0)) - 792.0).abs() < 0.01);
        assert!((fall_speed(Some(f32::NAN)) - 792.0).abs() < 0.01);
    }
    #[test]
    fn grounded_stairs_allow_two_units_but_reject_larger_vertical_changes() {
        let start = Instant::now();
        let now = start + Duration::from_millis(100);
        let mut guard = MovementGuard::new(7, Position::default(), start);
        guard.set_speed(Some(30.0), start);
        let mut request = MovementRequest {
            mode: MovementMode::Forward,
            heading: HeadingUpdate::Rate,
            session_id: 7,
            position: Position {
                x: 1.0,
                z: 2.01,
                ..Position::default()
            },
            created: now,
            support: None,
        };
        assert!(guard.accept(&request, now).is_err());
        request.position.z = 2.0;
        assert!(guard.accept(&request, now).is_ok());
    }

    #[test]
    fn a_grounded_climb_gets_the_players_reach_a_program_step() {
        let start = Instant::now();
        let now = start + Duration::from_millis(250);
        let mut guard = MovementGuard::new(7, Position::default(), start);
        guard.set_speed(Some(30.0), start);
        let mut request = MovementRequest {
            mode: MovementMode::Forward,
            heading: HeadingUpdate::Rate,
            session_id: 7,
            position: Position {
                x: 1.0,
                z: 2.1,
                ..Position::default()
            },
            created: now,
            support: None,
        };
        // Without the player's climb, one riser a sample however long.
        assert!(guard.accept(&request, now).is_err());
        // A human's Titanium reach, 3.15 a step: a quarter second is five
        // steps, so up to 15.75 units, and no more.
        guard.set_climb(3.15);
        request.position.z = 15.8;
        assert!(guard.accept(&request, now).is_err());
        request.position.z = 15.75;
        assert!(guard.accept(&request, now).is_ok());
        // Coming down stays one riser; past it the client's walk falls.
        let later = now + Duration::from_millis(250);
        request.created = later;
        request.position.z = 13.7;
        assert!(guard.accept(&request, later).is_err());
        request.position.z = 13.75;
        assert!(guard.accept(&request, later).is_ok());
        // A climb that is not a positive number is ignored.
        guard.set_climb(f32::NAN);
        guard.set_climb(0.0);
        assert_eq!(guard.climb, Some(3.15));
    }

    #[test]
    fn whole_steps_count_once_and_a_part_step_counts_whole() {
        assert_eq!(program_steps(0.05), 1);
        assert_eq!(program_steps(0.1), 2);
        assert_eq!(program_steps(0.15), 3);
        assert_eq!(program_steps(0.25), 5);
        assert_eq!(program_steps(0.051), 2);
        assert_eq!(program_steps(0.0), 1);
        assert_eq!(program_steps(f32::NAN), 1);
        assert!((climb_reach(3.0, 3) - 9.0).abs() < f32::EPSILON);
        assert!((climb_reach(2.5, 0) - 2.5).abs() < f32::EPSILON);
    }

    #[test]
    fn falls_descend_at_most_at_terminal_speed_and_never_rise() {
        let start = Instant::now();
        let mut guard = MovementGuard::new(7, Position::default(), start);
        guard.set_speed(Some(30.0), start);
        let now = start + Duration::from_millis(100);
        let mut request = MovementRequest {
            mode: MovementMode::Forward,
            heading: HeadingUpdate::Rate,
            session_id: 7,
            position: Position {
                x: 1.0,
                z: -5.0,
                ..Position::default()
            },
            created: now,
            support: None,
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
    fn rising(z: f32, created: Instant) -> MovementRequest {
        MovementRequest {
            mode: MovementMode::Fall,
            heading: HeadingUpdate::Rate,
            session_id: 7,
            position: Position {
                z,
                ..Position::default()
            },
            created,
            support: None,
        }
    }

    fn jumping_guard(start: Instant) -> MovementGuard {
        let mut guard = MovementGuard::new(7, Position::default(), start);
        guard.set_speed(Some(30.0), start);
        guard.set_jump(JUMP_SPEED, gravity_per_second(None));
        guard
    }

    #[test]
    fn a_jump_rises_as_its_speed_less_gravity_over_each_interval() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut guard = jumping_guard(start);
        // No jump announced: one step up only.
        assert!(guard.accept(&rising(2.1, at(100)), at(100)).is_err());
        // 0.1 s of a 44.55 u/s jump against 160 u/s² rises 3.655 units, and
        // one step more is allowed.
        guard.jumped();
        assert!(guard.accept(&rising(5.7, at(100)), at(100)).is_err());
        assert!(guard.accept(&rising(5.6, at(100)), at(100)).is_ok());
        // From 0.1 s to 0.2 s it rises 2.055 more.
        assert!(guard.accept(&rising(9.7, at(200)), at(200)).is_err());
        assert!(guard.accept(&rising(9.6, at(200)), at(200)).is_ok());
        // The top comes at 0.278 s: 0.2 s to 0.3 s rises 0.495 more.
        assert!(guard.accept(&rising(12.2, at(300)), at(300)).is_err());
        assert!(guard.accept(&rising(12.0, at(300)), at(300)).is_ok());
        // Past the top, one step only.
        assert!(guard.accept(&rising(14.1, at(400)), at(400)).is_err());
        assert!(guard.accept(&rising(14.0, at(400)), at(400)).is_ok());
    }

    #[test]
    fn a_hurt_or_burdened_jump_rises_from_its_share_of_the_speed() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut guard = jumping_guard(start);
        // At half the speed, 0.1 s rises 2.2275 - 0.8 = 1.4275 units, and
        // one step more is allowed.
        guard.set_jump_factor(0.5);
        guard.jumped();
        // A factor that changes in the air leaves the jump as it started.
        guard.set_jump_factor(1.0);
        assert!(guard.accept(&rising(3.5, at(100)), at(100)).is_err());
        assert!(guard.accept(&rising(3.3, at(100)), at(100)).is_ok());
        // Out of range, the factor is held to 0..1 or counts as 1.
        guard.set_jump_factor(f32::NAN);
        assert!((guard.jump_factor - 1.0).abs() < f32::EPSILON);
        guard.set_jump_factor(2.0);
        assert!((guard.jump_factor - 1.0).abs() < f32::EPSILON);
        guard.set_jump_factor(-1.0);
        assert!(guard.jump_factor.abs() < f32::EPSILON);
    }

    #[test]
    fn a_correction_ends_the_jumps_rise() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut guard = jumping_guard(start);
        guard.jumped();
        guard.correct(Position::default(), at(10));
        guard.set_speed(Some(30.0), at(10));
        assert!(guard.accept(&rising(3.0, at(110)), at(110)).is_err());
        assert!(guard.accept(&rising(2.0, at(110)), at(110)).is_ok());
    }

    #[test]
    fn without_a_read_jump_an_announcement_admits_no_rise() {
        let start = Instant::now();
        let now = start + Duration::from_millis(100);
        let mut guard = MovementGuard::new(7, Position::default(), start);
        guard.set_speed(Some(30.0), start);
        guard.set_jump(f32::NAN, 160.0);
        guard.jumped();
        assert!(guard.accept(&rising(2.1, now), now).is_err());
        assert!((gravity_per_second(None) - 160.0).abs() < 0.01);
        assert!((JUMP_SPEED - 44.55).abs() < 0.01);
    }

    fn forward(x: f32, created: Instant) -> MovementRequest {
        MovementRequest {
            mode: MovementMode::Forward,
            heading: HeadingUpdate::Rate,
            session_id: 7,
            position: Position {
                x,
                ..Position::default()
            },
            created,
            support: None,
        }
    }

    #[test]
    fn a_sample_processed_late_is_budgeted_on_its_stamps() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut guard = MovementGuard::new(7, Position::default(), start);
        guard.set_speed(Some(30.0), start);
        // Stamped 100 ms after the last, processed 30 ms late: 3 units fit.
        assert!(guard.accept(&forward(3.0, at(100)), at(130)).is_ok());
        // The next is stamped 100 ms later but processed only 75 ms after the
        // first; its stamps still cover the 3 units it moves.
        assert!(guard.accept(&forward(6.0, at(200)), at(205)).is_ok());
    }

    #[test]
    fn a_stamp_interval_too_short_for_the_distance_is_refused() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut guard = MovementGuard::new(7, Position::default(), start);
        guard.set_speed(Some(30.0), start);
        assert!(guard.accept(&forward(3.0, at(100)), at(100)).is_ok());
        // Processed 100 ms later, but stamped only 60 ms after the last: 1.8 units.
        assert!(guard.accept(&forward(6.0, at(160)), at(200)).is_err());
        assert!(guard.accept(&forward(4.8, at(160)), at(200)).is_ok());
    }

    #[test]
    fn the_floor_and_cap_apply_to_the_stamp_interval() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut guard = MovementGuard::new(7, Position::default(), start);
        guard.set_speed(Some(30.0), start);
        assert!(guard.accept(&forward(1.5, at(50)), at(50)).is_ok());
        // Stamped 40 ms after the last, though processed 100 ms after it.
        assert!(guard.accept(&forward(2.0, at(90)), at(150)).is_err());
        // Stamped 400 ms after the last: the budget stops at 0.25 s, 7.5 units.
        assert!(guard.accept(&forward(9.1, at(450)), at(450)).is_err());
        assert!(guard.accept(&forward(9.0, at(450)), at(450)).is_ok());
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
            heading: HeadingUpdate::Rate,
            session_id: 3,
            position: Position {
                x: 1.0,
                ..Position::default()
            },
            created: now,
            support: None,
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
            heading: HeadingUpdate::Rate,
            session_id: 3,
            position: Position {
                x: 1.0,
                y: 1.0,
                ..Position::default()
            },
            created: start,
            support: None,
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
            heading: HeadingUpdate::Rate,
            session_id: 1,
            position: Position {
                x: 0.51,
                ..Position::default()
            },
            created: now,
            support: None,
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
                    heading: HeadingUpdate::Rate,
                    session_id: 1,
                    position: Position {
                        x: 10.0,
                        ..Position::default()
                    },
                    created: now,
                    support: None,
                },
                now
            )
            .is_err());
    }
}
