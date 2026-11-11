//! The player's movement: where the player is and how that reaches the server,
//! the host's moves, jumps, calibrations and changes of stance, and the
//! server's corrections.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    appearance::AppearanceChange,
    inventory::MAX_CARRIED_WEIGHT,
    message::Message,
    movement::{
        DiagonalRates, MotionCalibration, MotionSession, MovementRequest, PositionPacket,
        SpeedInputAcceptance, SpeedInputState, SpeedInputs, SpeedState, Support,
        STATIONARY_HEARTBEAT,
    },
    request::Request,
    world::{PlayerState, Position, PostureState, WorldEvent},
};
use std::time::{Duration, Instant};

/// How the player's position reaches the server: the movement session once the
/// zone admits the player, or else a stationary heartbeat from where the
/// profile left them.
pub(super) struct Body {
    /// The player's movement, once admitted.
    motion: Option<MotionSession>,
    /// What the stationary heartbeat repeats: the player's spawn ID, its next
    /// sequence number and position.
    stationary: PositionPacket,
    /// Whether the server supplied a position that can still be repeated.
    stationary_known: bool,
    /// When the stationary heartbeat last went out.
    last_sent: Instant,
}

impl Default for Body {
    fn default() -> Self {
        Self {
            motion: None,
            stationary: PositionPacket {
                spawn_id: 0,
                sequence: 0,
                position: Position::default(),
                delta: [0.0; 3],
                animation: 0,
                delta_heading: 0,
            },
            stationary_known: false,
            // In the past, so the first stationary heartbeat goes out at once.
            last_sent: Instant::now()
                .checked_sub(STATIONARY_HEARTBEAT)
                .unwrap_or_else(Instant::now),
        }
    }
}

impl Body {
    /// Notes where the profile left the player.
    pub(super) fn place(&mut self, position: Position) {
        self.stationary.position = position;
        self.stationary_known = true;
    }

    /// Notes the player's spawn ID.
    pub(super) fn own(&mut self, spawn_id: u16) {
        self.stationary.spawn_id = spawn_id;
    }

    /// Where the player is now, once they can move.
    pub(super) fn position(&self) -> Option<Position> {
        self.motion.as_ref().map(MotionSession::position)
    }

    /// Previous accepted point for corroborating a current pad entrance.
    pub(super) fn pad_approach(&self, created: Instant) -> Option<Position> {
        self.motion.as_ref()?.pad_approach(created)
    }

    /// Submit a verified WLD pad entry through the ordinary Titanium codec,
    /// then keep the local player record and movement owner in step.
    pub(super) fn cross_pad(
        &mut self,
        player: &mut PlayerState,
        pad: eq_network_game::zoning::LocalPad,
        created: Instant,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        use anyhow::Context;
        self.motion
            .as_mut()
            .context("pad entry before movement admission")?
            .cross_pad(pad, created, Instant::now(), |sample| {
                // Titanium's explicit entry report uses send mode 4 (0x48ab25–0x48ab37).
                out.request(&Request::Position(*sample))
            })?;
        player.position = pad.landing();
        self.place(player.position);
        Ok(())
    }

    /// The existing movement owner's stop state; None means no such owner.
    pub(super) fn stop_pending(&self) -> Option<bool> {
        self.motion.as_ref().map(MotionSession::stop_pending)
    }

    /// Classify only against the actual movement owner's committed state.
    pub(super) fn is_trailing_stop(&self, request: &MovementRequest) -> bool {
        self.motion
            .as_ref()
            .is_some_and(|motion| motion.is_trailing_stop(request))
    }

    /// Equality context from the current movement owner, never a host receipt.
    pub(super) fn stop_context(&self) -> Option<(u64, Instant)> {
        self.motion.as_ref().and_then(MotionSession::stop_context)
    }

    /// Whether the owner's normal submission accepted this trailing stop.
    pub(super) fn accepted_stop(&self, created: Instant) -> bool {
        self.motion
            .as_ref()
            .is_some_and(|motion| motion.accepted_stop(created))
    }

    /// Starts movement for the admitted player.
    #[cfg(test)]
    pub(super) fn admit(&mut self, motion: MotionSession) {
        self.motion = Some(motion);
    }

    /// The player's movement session, once admitted.
    #[cfg(test)]
    pub(super) fn motion_mut(&mut self) -> Option<&mut MotionSession> {
        self.motion.as_mut()
    }

    /// Stops all position updates, after death or while a transfer waits.
    pub(super) fn suspend(&mut self) {
        if let Some(motion) = self.motion.as_mut() {
            motion.suspend();
        }
    }

    /// The player's movement and posture departures, once admitted.
    pub(super) fn options(&self) -> Option<eq_network_game::movement::MovementOptions> {
        self.motion.as_ref().map(MotionSession::options)
    }

    /// Preserve physics and calibration while replacing paced voluntary
    /// motion with its normal stationary stop.
    pub(super) fn stop_voluntary(&mut self) {
        if let Some(motion) = self.motion.as_mut() {
            motion.stop_voluntary();
        }
    }

    /// Lets a player who stays after a refused transfer stand and move again.
    pub(super) fn resume(&mut self, now: Instant) {
        if let Some(motion) = self.motion.as_mut() {
            motion.resume_stationary(now);
        }
    }

    /// Puts the player where the server says they are, keeping the player's
    /// state, the movement session and the stationary heartbeat in step; an
    /// invalid position changes none of them.
    pub(super) fn correct(
        &mut self,
        player: &mut PlayerState,
        position: Position,
        now: Instant,
    ) -> Result<()> {
        let stationary = PositionPacket {
            spawn_id: player.spawn_id,
            position,
            ..self.stationary
        };
        stationary.check()?;
        if let Some(motion) = self.motion.as_mut() {
            motion.correct(position, now)?;
        }
        player.position = position;
        self.stationary = stationary;
        self.stationary_known = true;
        Ok(())
    }

    /// Repeats the player's position as the server expects: the movement
    /// session's own updates, or the stationary heartbeat without one.
    pub(super) fn heartbeat(&mut self, now: Instant, out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(motion) = self.motion.as_mut() {
            motion.tick_sample(now, |sample| {
                out.request_unreliable(&Request::Position(*sample))
            })?;
            return Ok(());
        }
        if !self.stationary_known
            || self.stationary.spawn_id == 0
            || now.saturating_duration_since(self.last_sent) < STATIONARY_HEARTBEAT
        {
            return Ok(());
        }
        out.request_unreliable(&Request::Position(self.stationary))?;
        self.stationary.sequence = self.stationary.sequence.wrapping_add(1);
        self.last_sent = now;
        Ok(())
    }
}

/// The server's hold on the player, as the update gate sees it: frozen by
/// the player's own posture, or stunned unless a departure lets them move.
fn hold(
    posture: Option<PostureState>,
    stunned: bool,
    motion: &MotionSession,
) -> eq_network_game::movement::Hold {
    use eq_network_game::movement::Hold;
    if posture == Some(PostureState::Frozen) {
        Hold::Frozen
    } else if stunned && !motion.options().walk_while_stunned {
        Hold::Stunned
    } else {
        Hold::None
    }
}

/// Whether the player lies where the generation's program moves nothing
/// ([`eq_network_game::feign::LyingRules::holds_still`]).
fn lies_still(world: &World, out: &Out<'_, '_>) -> bool {
    out.wire.dialect().lying().holds_still && world.posture.current() == Some(PostureState::Lying)
}

/// The player's movement at these speeds.
fn granted(
    session_id: u64,
    speeds: &MotionCalibration,
    falls: bool,
    jump_factor: f32,
) -> WorldEvent {
    WorldEvent::MotionState {
        session_id,
        units_per_second: Some(speeds.units_per_second),
        walk_units_per_second: speeds.walk.map(|value| value.units_per_second),
        strafe_units_per_second: speeds.strafe.map(|value| value.units_per_second),
        strafe_walk_units_per_second: speeds.strafe.and_then(|value| value.walk_units_per_second),
        swim_backward_units_per_second: speeds.swim_backward_units_per_second,
        backward_units_per_second: speeds.backward.map(|value| value.units_per_second),
        diagonal: speeds.diagonal,
        falls,
        jump_factor,
    }
}

/// The player rooted: granted, at no speed. Input goes on and nothing
/// carries them anywhere ([`MotionSession::root`]).
fn rooted(session_id: u64, falls: bool, jump_factor: f32) -> WorldEvent {
    WorldEvent::MotionState {
        session_id,
        units_per_second: Some(0.0),
        strafe_units_per_second: None,
        strafe_walk_units_per_second: None,
        swim_backward_units_per_second: None,
        walk_units_per_second: None,
        backward_units_per_second: None,
        diagonal: DiagonalRates::default(),
        falls,
        jump_factor,
    }
}

/// Takes the player's movement away until it is granted again, after a
/// correction, a death or a transfer.
pub(super) fn withdrawn(session_id: u64) -> WorldEvent {
    WorldEvent::MotionState {
        session_id,
        units_per_second: None,
        strafe_units_per_second: None,
        strafe_walk_units_per_second: None,
        swim_backward_units_per_second: None,
        walk_units_per_second: None,
        backward_units_per_second: None,
        diagonal: DiagonalRates::default(),
        falls: false,
        jump_factor: eq_network_game::movement::full_jump(),
    }
}

/// How long a grant kept while the host works out new inputs runs before
/// the session notes, once, that the answer is late. The grant stays: the
/// program recomputes its speed modifier on buff and effect changes and once
/// a second, and its input reads the result as it stands without waiting for
/// it (reads R7 and R18, 0x4a1d74, 0x410fdb).
const KEPT_GRANT_NOTICE: Duration = Duration::from_secs(1);

/// The player's movement and stance.
#[allow(
    clippy::struct_excessive_bools,
    reason = "Independent server permissions and observed sneak state"
)]
pub(super) struct Motion {
    /// Whether the server lets the player fall.
    falls: bool,
    /// Whether the player may also jump: the generation's notice of a jump
    /// is known, and the server takes falls.
    jumps: bool,
    swimming: bool,
    /// The host's report of what the player's speeds depend on (the buffs'
    /// movement rate, the weight carried and the capacity), once it has
    /// sent one; until then the session grants no movement itself.
    inputs: Option<SpeedInputs>,
    input_report_id: Option<u64>,
    accepted_inputs: Option<SpeedInputAcceptance>,
    revision: u64,
    fatigue: Option<u8>,
    belongings: Option<(
        u64,
        Option<eq_network_game::world::Coins>,
        eq_network_game::world::Coins,
    )>,
    /// When a change the speeds depend on (a buff, a level, a spell landing,
    /// the player's belongings, a server's correction) asked for the inputs
    /// again while the player held a grant. Until the answer lands the last
    /// inputs stay in force (health, stance and fatigue still apply at
    /// once); the answer replaces them. An answer late by
    /// [`KEPT_GRANT_NOTICE`] is noted once and the grant stays. Anything
    /// that withdraws movement ends it.
    kept_since: Option<Instant>,
    /// What the inputs in force leave unproven: the host could not work
    /// them out, so the last ones are kept, with any term it could prove
    /// slower applied (recording checklist 6.451). The first exact report
    /// ends it.
    unproven: Option<eq_network_game::movement::SpeedInputUnavailable>,
    /// Whether the player carried too much at the last grant, so the
    /// official client's line goes out once each way.
    encumbered: Option<bool>,
    /// Whether the player is sneaking, as the server last said: its
    /// appearance update to the player, or its end of their hiding and
    /// sneaking.
    sneaking: bool,
    /// The player's hit points and their maximum, as the server last
    /// reported them. `EQMac`'s servers leave out what items add from the
    /// current alone, so the player reads as more hurt than they are: the
    /// slower side.
    hit_points: Option<(i32, i32)>,
    /// The spell action flag that says a spell landed on its target, as the
    /// server type says ([`ServerType::spell_landing_flag`](super::servers::ServerType::spell_landing_flag)); none where it
    /// is unchecked, and then no action is taken as a landing.
    spell_landing: Option<u8>,
    /// Whether the host was last told a positive grant from this feature,
    /// with nothing withdrawn since. A server's move clears the movement
    /// session's calibration, so this is what says the grant goes on.
    grant_in_force: bool,
}

impl Motion {
    pub(super) const fn new(falls: bool, jumps: bool, spell_landing: Option<u8>) -> Self {
        Self {
            falls,
            jumps: falls && jumps,
            swimming: false,
            spell_landing,
            inputs: None,
            input_report_id: None,
            accepted_inputs: None,
            revision: 0,
            fatigue: None,
            belongings: None,
            kept_since: None,
            unproven: None,
            encumbered: None,
            sneaking: false,
            hit_points: None,
            grant_in_force: false,
        }
    }

    pub(super) const fn with_swimming(mut self) -> Self {
        self.swimming = true;
        self
    }

    /// Takes the host's word on how far the player's body climbs a step in
    /// this admission, for the movement guard.
    fn climb(command: &ClientCommand, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let ClientCommand::ClimbReach { per_step, .. } = *command else {
            return Ok(());
        };
        let Some(motion) = world.body.motion.as_mut() else {
            return out
                .log
                .diagnostic("Rejected a climb: the player cannot move yet".into());
        };
        if !(per_step.is_finite() && per_step > 0.0) {
            return out
                .log
                .diagnostic(format!("Rejected a climb of {per_step} a step"));
        }
        motion.set_climb(per_step);
        Ok(())
    }

    /// Grants the player the speeds the generation's client allows, once
    /// the host has reported what they depend on, and keeps them in step;
    /// the host hears of each change, and of the player starting or stopping
    /// carrying too much. Where the generation's rule is unread, only a
    /// host's calibration grants them.
    fn grant(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let granted = self.apply_grant(world, out)?;
        self.acknowledge_inputs(world, out, granted)
    }

    /// Evaluate the existing movement rule, distinguishing successful current
    /// calibration from missing, withdrawn or rejected calibration. Receipt
    /// bookkeeping must not infer success from a prior positive calibration.
    fn apply_grant(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<bool> {
        let (Some(rule), Some(inputs), Some(player)) =
            (out.wire.speeds(), self.inputs, world.player.as_ref())
        else {
            return Ok(false);
        };
        let speeds = rule(&SpeedState {
            run: player.run_speed,
            walk: player.walk_speed,
            level: player.level,
            hit_points: self.hit_points,
            rate: inputs.rate,
            weight: inputs.weight,
            capacity: if out.wire.dialect() == eq_network_game::GameDialect::EqMac {
                inputs.eqmac_capacity(self.fatigue)
            } else {
                inputs.capacity
            },
            sneaking: self.sneaking,
            zone_id: world.zone.0,
        });
        if let Some((encumbered, string_id)) = speeds.encumbrance {
            // The official client speaks as the player enters a state. Whether
            // it speaks for the state they zone in with is unread, so the
            // first grant of an admission only learns it.
            if self.encumbered.is_some_and(|was| was != encumbered) {
                out.log.send(ClientEvent::World(WorldEvent::Encumbrance {
                    session_id: world.session_id,
                    encumbered,
                    string_id,
                }))?;
            }
            self.encumbered = Some(encumbered);
        }
        let Some(motion) = world.body.motion.as_mut() else {
            return Ok(false);
        };
        // The factor moves with the speeds it also slows, so a changed
        // factor comes with a changed calibration and its grant.
        motion.set_jump_factor(speeds.jump_factor);
        let jump_factor = speeds.jump_factor;
        // A root is a grant at no speed: the program zeroes the speed and
        // the velocity once and blocks nothing else (read R18, 0x410fdb), so
        // input goes on, the player turns, a song pulse or a fade withdraws
        // nothing, and the autorun latch is the generation's
        // ([`eq_network_game::movement::RootRules`]). It is never a positive
        // grant for the host's receipt.
        if speeds.rooted {
            if !motion.rooted() {
                if let Err(error) = motion.root(Instant::now()) {
                    self.grant_in_force = false;
                    return out
                        .log
                        .diagnostic(format!("Could not root the player: {error}"))
                        .map(|()| false);
                }
                out.log.send(ClientEvent::World(rooted(
                    world.session_id,
                    motion.falls(),
                    jump_factor,
                )))?;
            }
            self.grant_in_force = true;
            return Ok(false);
        }
        if motion.calibration() == speeds.motion {
            self.grant_in_force = speeds.motion.is_some();
            return Ok(self.grant_in_force);
        }
        let Some(speeds) = speeds.motion else {
            self.withdraw_input(world, out)?;
            return Ok(false);
        };
        match motion.calibrate(speeds, Instant::now()) {
            Ok(()) => {
                self.grant_in_force = true;
                // A fresh calibration follows a withdrawal the host heard: a
                // correction, a same-zone relocation or a refused transfer's
                // rewind lose the calibration without asking for the inputs
                // again. The host's receipt died with that withdrawal, so the
                // grant carries the receipt again.
                self.accepted_inputs = None;
                out.log.send(ClientEvent::World(granted(
                    world.session_id,
                    &speeds,
                    motion.falls(),
                    jump_factor,
                )))?;
                Ok(true)
            }
            Err(error) => {
                self.grant_in_force = false;
                out.log
                    .diagnostic(format!("Rejected movement speeds: {error}"))?;
                Ok(false)
            }
        }
    }

    /// Publish exact accepted-report identity even when calibration is unchanged.
    /// A later change in its grant publishes, and so does every fresh
    /// calibration; identical ticks stay quiet.
    fn acknowledge_inputs(
        &mut self,
        world: &World,
        out: &mut Out<'_, '_>,
        granted: bool,
    ) -> Result<()> {
        let (Some(inputs), Some(report_id)) = (self.inputs, self.input_report_id) else {
            self.accepted_inputs = None;
            return Ok(());
        };
        let accepted = SpeedInputAcceptance {
            session_id: world.session_id,
            revision: self.revision,
            report_id,
            inputs,
            granted: granted && world.ready() && !world.lifecycle.blocks_motion(),
        };
        if self.accepted_inputs != Some(accepted) {
            out.log
                .send(ClientEvent::World(WorldEvent::SpeedInputsAccepted(
                    accepted,
                )))?;
            self.accepted_inputs = Some(accepted);
        }
        Ok(())
    }

    /// Takes the host's report of what the player's speeds depend on.
    fn inputs(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.refresh_belongings(world, out)?;
        let ClientCommand::SpeedInputs {
            session_id,
            revision,
            report_id,
            state,
        } = *command
        else {
            return Ok(());
        };
        if session_id != world.session_id || revision != self.revision {
            return out
                .log
                .diagnostic("Rejected obsolete movement inputs".into());
        }
        // The answer to a kept grant's request replaces it. One the session
        // cannot take leaves the last inputs in force (recording checklist
        // 6.451): the program never stops input for its speeds (read R7).
        self.kept_since = None;
        let held = self.holds_grant(world);
        let mut state = state;
        if let (SpeedInputState::Ready(inputs) | SpeedInputState::Bounded { inputs, .. }, true) =
            (state, world.inventory.ground_drop_pending())
        {
            let bound = world.coins.carried_coin_count().and_then(|coins| {
                world
                    .inventory
                    .carried_weight_tenths(eq_network_game::GameDialect::Titanium, coins)
                    .ok()
            });
            let floor = bound.map(|tenths| {
                u16::try_from((tenths / 10).min(u64::from(MAX_CARRIED_WEIGHT))).unwrap_or(u16::MAX)
            });
            if floor.is_none_or(|floor| inputs.weight < floor) {
                if floor.is_none() && !held {
                    return out.log.diagnostic(
                        "Movement needs a current carried-weight bound after the drop".into(),
                    );
                }
                // The drop's bound is a weight the player is proven to carry
                // at least, so it bounds the report. Without a bound the
                // report is still a bound, its weight no lower than in force,
                // so a root or a lower rate in it still applies.
                let in_force = self
                    .inputs
                    .filter(|_| held)
                    .map_or(0, |inputs| inputs.weight);
                state = SpeedInputState::Bounded {
                    inputs: SpeedInputs {
                        weight: floor.unwrap_or(inputs.weight.max(in_force)),
                        ..inputs
                    },
                    missing: eq_network_game::movement::SpeedInputUnavailable::Inventory,
                };
            }
        }
        match state {
            SpeedInputState::Ready(inputs) => {
                self.input_report_id = Some(report_id);
                self.inputs = Some(inputs);
                if self.unproven.take().is_some() {
                    out.log.diagnostic("Movement inputs proven again".into())?;
                }
            }
            // The host proves the terms: a bound is taken as reported, with
            // or without a grant in force (Adam's decision of 11-10; 6.451).
            // It is faster than the last one only where a proven slower term
            // ended. A root in it still withdraws.
            SpeedInputState::Bounded { inputs, missing } => {
                self.input_report_id = Some(report_id);
                self.inputs = Some(inputs);
                self.note_unproven(missing, out)?;
            }
            // Nothing the host can prove: the grant in force and its inputs
            // stay, with no time limit (Adam's decision of 11-10; 6.451).
            SpeedInputState::Unavailable(missing) if held => {
                self.input_report_id = None;
                self.accepted_inputs = None;
                self.note_unproven(missing, out)?;
            }
            // Nothing provable and no grant: wait, as at the first frames of
            // an admission.
            SpeedInputState::Unavailable(_) => {
                self.input_report_id = None;
                self.inputs = None;
                self.unproven = None;
                self.withdraw_input(world, out)?;
            }
        }
        self.grant(world, out)
    }

    /// Withdraws input while retaining bounded passive descent and heartbeats.
    fn withdraw_input(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        self.grant_in_force = false;
        self.accepted_inputs = None;
        self.kept_since = None;
        self.unproven = None;
        if let Some(motion) = world.body.motion.as_mut() {
            motion.withdraw();
        }
        let mut event = withdrawn(world.session_id);
        if let WorldEvent::MotionState { falls, .. } = &mut event {
            *falls = self.falls && !world.lifecycle.blocks_motion();
        }
        out.log.send(ClientEvent::World(event))
    }

    /// Invalidates queued reports and requests a calculation from the new state.
    fn require_inputs(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        self.kept_since = None;
        self.inputs = None;
        self.input_report_id = None;
        self.revision = self.revision.wrapping_add(1);
        self.withdraw_input(world, out)?;
        world.happened(Message::Event(WorldEvent::SpeedInputsRequired {
            session_id: world.session_id,
            revision: self.revision,
        }));
        Ok(())
    }

    /// Inventory/coin owners also publish predictions, settlements and rewards
    /// directly. Compare their state before accepting input and on every tick.
    fn refresh_belongings(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if world.body.motion.is_none() {
            return Ok(());
        }
        let current = (
            world.inventory.revision(),
            world.coins.purse,
            world.coins.cursor,
        );
        if self.belongings == Some(current) {
            return Ok(());
        }
        self.belongings = Some(current);
        self.recalculate(world, out)
    }

    /// Notes, once for each cause, that the inputs in force are kept while
    /// the host cannot prove new ones.
    fn note_unproven(
        &mut self,
        missing: eq_network_game::movement::SpeedInputUnavailable,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if self.unproven.replace(missing) == Some(missing) {
            return Ok(());
        }
        out.log.diagnostic(format!(
            "Kept the movement inputs in force: the host cannot prove new ones ({missing:?})"
        ))
    }

    /// Whether the player holds a grant from inputs the host reported.
    fn holds_grant(&self, world: &World) -> bool {
        self.inputs.is_some() && self.grant_in_force && !world.lifecycle.blocks_motion()
    }

    /// After the server moved the player (a correction to the player's own
    /// spawn, a same-zone relocation or a local pad crossing), which stopped
    /// them there: a grant in force goes on at once while the host works out
    /// new inputs, so autorun and held keys keep driving from the new place
    /// (read R18: 0x451aba and the same-zone warp 0x48aaa6 touch no input).
    /// Without a grant the inputs are asked for as before.
    fn server_moved(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if !self.holds_grant(world) {
            return self.require_inputs(world, out);
        }
        self.keep_and_ask(world);
        self.grant(world, out)
    }

    /// Asks the host for the inputs again after a change they depend on. A
    /// grant in force stays in force while the host works the new ones out,
    /// so a Bard's song pulse, a buff, an item eaten or picked up does not
    /// stop the player, drop a jump or a crouch, or end autorun: the
    /// program reads its speed modifier live, and stops input only for a
    /// stun, a freeze or a posture other than standing or ducking (read R7,
    /// 0x4a1d74). Only the inputs stay stale, until the answer lands.
    /// Without a grant, the host is asked as before.
    fn recalculate(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if !self.holds_grant(world) {
            return self.require_inputs(world, out);
        }
        self.keep_and_ask(world);
        Ok(())
    }

    fn keep_and_ask(&mut self, world: &mut World) {
        self.kept_since.get_or_insert_with(Instant::now);
        self.input_report_id = None;
        self.accepted_inputs = None;
        self.revision = self.revision.wrapping_add(1);
        world.happened(Message::Event(WorldEvent::SpeedInputsRequired {
            session_id: world.session_id,
            revision: self.revision,
        }));
    }

    /// Moves the player, standing them up first if they were sitting or
    /// crouched; a refused move undoes the host's prediction.
    fn step(request: &MovementRequest, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        Self::step_at(request, Instant::now(), world, out)
    }

    fn step_at(
        request: &MovementRequest,
        now: Instant,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let lying = lies_still(world, out);
        let Some(motion) = world.body.motion.as_mut() else {
            return actions::refuse(
                &ClientCommand::Move(request.clone()),
                "The player cannot move yet",
                out.log,
            );
        };
        let (to, from) = (request.position, motion.position());
        // A lift the client stands the player on may carry them up as far as
        // it has risen since they named it, less what they have risen with it
        // already; one that sinks closes that account.
        let lift = match request.support {
            Some(Support::Door(door))
                if !world.lifts.sinks(door, motion.sampled(), request.created) =>
            {
                let since = motion.lift_since(door);
                Some((
                    door,
                    world.lifts.carry(
                        door,
                        world.zone.0,
                        (from, motion.sampled()),
                        since,
                        request.created,
                    ),
                ))
            }
            _ => None,
        };
        motion.allow_lift_rise(lift);
        let may_walk_while_stunned =
            !request.mode.is_water_motion() && motion.options().walk_while_stunned;
        if let Some(reason) = world.control.motion_refusal(
            request,
            motion.is_passive(request),
            may_walk_while_stunned,
        ) {
            return out.log.send(ClientEvent::World(WorldEvent::MotionSent {
                session_id: world.session_id,
                created: request.created,
                position: from,
                refused: Some(reason.into()),
            }));
        }
        // The program moves nothing while the player lies and stands no one
        // to move (read R19 c), so a step that goes anywhere is refused and
        // no stand is sent. A fall, a lift's carry and a turn alone go on:
        // mouse look turns a lying player in the chase views.
        let moves = (to.x, to.y, to.z) != (from.x, from.y, from.z);
        if lying && !motion.is_passive(request) && moves {
            return out.log.send(ClientEvent::World(WorldEvent::MotionSent {
                session_id: world.session_id,
                created: request.created,
                position: from,
                refused: Some("The player lies on the ground".into()),
            }));
        }
        if let (true, Some(player)) = (
            (!world.control.stunned() || motion.options().walk_while_stunned) && moves,
            world.player.as_ref(),
        ) {
            world.posture.stand_to_move(player.spawn_id, out)?;
        }
        let mut transport_failed = false;
        let (wire, sender, sink) = (out.wire, out.sender, &mut *out.sink);
        let result = motion.send_move_sample(request, now, |sample| {
            // A sample the generation cannot carry is refused like any other;
            // only a failed send ends the admission.
            let packet = wire.encode(&Request::Position(*sample), sender)?;
            let sent = sink.send_unreliable(&packet);
            transport_failed = sent.is_err();
            sent
        });
        // A failed send ends the admission.
        if transport_failed {
            return result;
        }
        let refused = result.err().map(|error| error.to_string());
        if let Some(error) = &refused {
            out.log
                .diagnostic(format!("Rejected movement proposal: {error}"))?;
        }
        // Refused coordinates are never published.
        out.log.send(ClientEvent::World(WorldEvent::MotionSent {
            session_id: world.session_id,
            created: request.created,
            position: motion.position(),
            refused,
        }))
    }

    /// Takes the client's measured movement speeds; until then the player
    /// cannot move.
    fn calibrate(command: &ClientCommand, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if out.wire.speeds().is_some() {
            return out
                .log
                .diagnostic("Movement requires current SpeedInputs for this generation".into());
        }
        let ClientCommand::ConfigureMotion {
            calibration,
            created,
            ..
        } = command
        else {
            return Ok(());
        };
        let Some(motion) = world.body.motion.as_mut() else {
            return out
                .log
                .diagnostic("Rejected movement configuration: the player cannot move yet".into());
        };
        match motion.calibrate_fresh(*calibration, *created, Instant::now()) {
            Ok(()) => out.log.send(ClientEvent::World(granted(
                world.session_id,
                calibration,
                motion.falls(),
                eq_network_game::movement::full_jump(),
            ))),
            Err(error) => out
                .log
                .diagnostic(format!("Rejected movement configuration: {error}")),
        }
    }

    /// Jumps where the server allows falls and the generation's notice of a
    /// jump is known.
    /// The samples after it may rise as the jump does.
    fn jump(&self, created: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(reason) = world.control.input_refusal(created) {
            return out.log.diagnostic(format!("Rejected jump: {reason}"));
        }
        // Jumping waits on the can-move flag, off while the player lies
        // (read R19 c).
        if lies_still(world, out) {
            return out.log.diagnostic("Rejected jump: the player lies".into());
        }
        let jump = world
            .body
            .motion
            .as_ref()
            .filter(|_| self.jumps)
            .ok_or_else(|| anyhow::anyhow!("jumping is not enabled for this server"))
            .and_then(MotionSession::check_jump)
            .and_then(|()| out.encode(&Request::Jump));
        match jump {
            // A failed send ends the admission like any other.
            Ok(packet) => {
                out.send(&packet)?;
                if let Some(motion) = world.body.motion.as_mut() {
                    motion.jumped();
                }
                Ok(())
            }
            Err(error) => out.log.diagnostic(format!("Rejected jump: {error}")),
        }
    }
}

impl Feature for Motion {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        let mut caps = vec![Capability::Moving];
        if self.falls {
            caps.push(Capability::Falling);
        }
        if self.jumps {
            caps.push(Capability::Jumping);
        }
        if self.swimming {
            caps.push(Capability::Swimming);
        }
        caps
    }

    fn admit(&mut self, message: &Message, _world: &mut World) -> Result<()> {
        if let Message::Event(WorldEvent::Nourishment(value)) = message {
            self.fatigue = value.fatigue;
        }
        Ok(())
    }

    /// Movement starts from the moment the zone admitted the player, before
    /// the host heard of it, so a calibration the host makes on hearing it
    /// counts as fresh. Its updates keep the spacing the client generation
    /// keeps between them.
    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if let (Some(player), Some(admitted)) = (world.player.as_ref(), world.admitted) {
            world.body.motion = Some(
                MotionSession::new(world.session_id, player.spawn_id, player.position, admitted)?
                    .with_falls(self.falls)
                    .with_rooted_jumps(out.wire.dialect().root().jumps)
                    .with_heading_snapshots(out.wire.dialect().supports_heading_snapshots())
                    .with_swimming(
                        self.swimming
                            && out.wire.dialect() == eq_network_game::GameDialect::Titanium,
                    )
                    .with_gate(out.wire.update_gate())
                    .with_counter(out.wire.update_counter())
                    .with_fall_speed(out.wire.fall_speed(world.gravity))
                    .with_jump(out.wire.jump_speed().map(|speed| {
                        (
                            speed,
                            eq_network_game::movement::gravity_per_second(world.gravity),
                        )
                    })),
            );
        }
        self.belongings = Some((
            world.inventory.revision(),
            world.coins.purse,
            world.coins.cursor,
        ));
        self.require_inputs(world, out)
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::Move(_)
                | ClientCommand::Jump { .. }
                | ClientCommand::ConfigureMotion { .. }
                | ClientCommand::SpeedInputs { .. }
                | ClientCommand::ClimbReach { .. }
                | ClientCommand::SetMovementOptions { .. }
        )
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.refresh_belongings(world, out)?;
        match command {
            ClientCommand::Move(request) => Self::step(request, world, out),
            ClientCommand::Jump { created, .. } => self.jump(*created, world, out),
            ClientCommand::ConfigureMotion { .. } => Self::calibrate(command, world, out),
            ClientCommand::SpeedInputs { .. } => self.inputs(command, world, out),
            ClientCommand::ClimbReach { .. } => Self::climb(command, world, out),
            ClientCommand::SetMovementOptions {
                session_id,
                request_id,
                options,
                created,
            } if *session_id == world.session_id => {
                let result = world
                    .body
                    .motion
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("The player cannot move yet"))
                    .and_then(|motion| motion.set_options(*options, *created));
                match result {
                    Ok(()) => {
                        out.log
                            .send(ClientEvent::World(WorldEvent::MovementOptionsApplied {
                                session_id: world.session_id,
                                request_id: *request_id,
                                options: *options,
                            }))
                    }
                    Err(error) => super::actions::refuse(command, &error.to_string(), out.log),
                }
            }
            _ => Ok(()),
        }
    }

    /// Keeps the player's speeds in step, granting them again after a
    /// correction or a refused transfer took them away, and repeats the
    /// player's position while they may move.
    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if !world.ready() || world.lifecycle.blocks_motion() {
            return Ok(());
        }
        self.refresh_belongings(world, out)?;
        if self
            .kept_since
            .is_some_and(|since| now.saturating_duration_since(since) > KEPT_GRANT_NOTICE)
        {
            // No answer in time: say so once, and keep the grant until the
            // answer to the same request lands.
            self.kept_since = None;
            out.log.diagnostic(
                "Movement inputs are late; the last ones stay in force until they land".into(),
            )?;
        }
        self.grant(world, out)?;
        if let Some(motion) = world.body.motion.as_mut() {
            motion.set_hold(hold(
                world.posture.current(),
                world.control.stunned(),
                motion,
            ));
        }
        world.body.heartbeat(now, out)
    }

    /// The server's corrections and its reports of the player's stance, and
    /// the player's death, which stops all movement.
    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let Message::Event(event) = message else {
            return Ok(());
        };
        match event {
            WorldEvent::Nourishment(value) if value.fatigue != self.fatigue => {
                self.fatigue = value.fatigue;
                if out.wire.dialect() == eq_network_game::GameDialect::EqMac
                    && self
                        .inputs
                        .is_none_or(|inputs| inputs.eqmac_fatigue.is_none())
                {
                    return self.recalculate(world, out);
                }
                // Fatigue belongs to this session. The current host report
                // already carries its strength/stamina prerequisites; only
                // genuine prerequisite changes require another host revision.
                self.refresh_belongings(world, out)?;
                if world.ready() && !world.lifecycle.blocks_motion() {
                    self.grant(world, out)?;
                }
                Ok(())
            }
            WorldEvent::Level { .. } | WorldEvent::BuffSnapshot(_) => self.recalculate(world, out),
            WorldEvent::Buff(update) if world.is_player(update.entity_id) => {
                self.recalculate(world, out)
            }
            // A spell landing on the player, by the server type's flag.
            WorldEvent::SpellEffect(effect)
                if world.is_player(effect.target_id)
                    && self
                        .spell_landing
                        .is_some_and(|flag| flag == effect.effect_flag) =>
            {
                self.recalculate(world, out)
            }
            // The speeds do not depend on where the player stands: the
            // correction moves them and stops them there, and the grant in
            // force goes on at once while the host works out new inputs.
            WorldEvent::Position {
                spawn_id, position, ..
            } if world.is_player(*spawn_id) => {
                world.correct_own(*position, Instant::now())?;
                self.server_moved(world, out)
            }
            // The transfers feature has already moved the player.
            WorldEvent::SameZoneRelocated(relocation)
                if relocation.session_id == world.session_id =>
            {
                self.server_moved(world, out)
            }
            WorldEvent::LocalPadCrossed(crossing) if crossing.session_id == world.session_id => {
                self.server_moved(world, out)
            }
            // What the player's speeds depend on; the next tick grants them.
            WorldEvent::HitPoints {
                spawn_id,
                current,
                maximum,
                ..
            } if world.is_player(*spawn_id) => {
                self.hit_points = Some((*current, *maximum));
                Ok(())
            }
            WorldEvent::Appearance {
                spawn_id,
                change: AppearanceChange::Sneaking(sneaking),
            } if world.is_player(*spawn_id) => {
                self.sneaking = *sneaking;
                Ok(())
            }
            WorldEvent::HideAndSneakEnded => {
                self.sneaking = false;
                Ok(())
            }
            // The transfers feature stops the body when the player dies.
            event if world.owns_death(event) => {
                self.grant_in_force = false;
                self.kept_since = None;
                self.unproven = None;
                self.accepted_inputs = None;
                out.log
                    .send(ClientEvent::World(withdrawn(world.session_id)))
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
#[path = "motion_stun_tests.rs"]
mod stun_tests;

#[cfg(test)]
#[path = "motion_lying_tests.rs"]
mod lying_tests;

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::movement::MovementMode;
    use std::time::Duration;

    fn at(x: f32) -> Position {
        Position {
            x,
            y: 456.0,
            z: 7.0,
            heading: 256.0,
        }
    }

    #[test]
    fn the_gate_hears_a_freeze_by_posture_and_a_stun_unless_a_departure_moves() {
        use eq_network_game::movement::{Hold, MovementOptions};
        let start = Instant::now();
        let mut motion = MotionSession::new(5, 7, at(0.0), start).unwrap();
        assert_eq!(
            hold(Some(PostureState::Frozen), false, &motion),
            Hold::Frozen
        );
        assert_eq!(
            hold(Some(PostureState::Frozen), true, &motion),
            Hold::Frozen
        );
        assert_eq!(
            hold(Some(PostureState::Standing), true, &motion),
            Hold::Stunned
        );
        assert_eq!(hold(None, false, &motion), Hold::None);
        motion
            .set_options(
                MovementOptions {
                    walk_while_stunned: true,
                    ..MovementOptions::default()
                },
                start,
            )
            .unwrap();
        assert_eq!(
            hold(Some(PostureState::Standing), true, &motion),
            Hold::None
        );
    }

    #[test]
    fn p99_strokes_use_the_paced_titanium_position_stream() {
        use super::super::servers::{server_type, Setup};
        use crate::client::ServerProtocol;
        use eq_network_game::food::AutoEat;
        use eq_network_game::movement::{POSITION_OPCODE, TITANIUM_POSITION_SPACING};

        let server = server_type(ServerProtocol::Project1999);
        let setup = Setup::new("Tester", AutoEat::default());
        let mut feature = server.motion(&setup).unwrap().feature;
        let start = Instant::now();
        let mut world = World::new(5);
        world.admitted = Some(start);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        testing::run_on(server.wire(), |out| feature.admitted(&mut world, out))
            .result
            .unwrap();
        world
            .body
            .motion_mut()
            .unwrap()
            .calibrate(
                MotionCalibration {
                    swim_backward_units_per_second: None,
                    units_per_second: 6.0,
                    velocity_scale: 0.05,
                    animation: 12,
                    backward: None,
                    walk: None,
                    strafe: None,
                    diagonal: DiagonalRates::default(),
                },
                start,
            )
            .unwrap();

        let sampled = start + Duration::from_millis(50);
        let stroke = MovementRequest::new(
            MovementMode::Swim,
            5,
            Position {
                z: -0.03,
                ..Position::default()
            },
            sampled,
        );
        let held = testing::run_on(server.wire(), |out| {
            Motion::step_at(&stroke, sampled, &mut world, out)
        });
        held.result.unwrap();
        assert_eq!(held.unreliable, []);
        assert_eq!(held.sent, []);
        assert!(held.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::MotionSent { position, refused: None, .. })
                if *position == stroke.position
        )));
        assert_eq!(world.body.position(), Some(stroke.position));

        // The program sends more than 280 ms after the last update.
        let before_due = start + TITANIUM_POSITION_SPACING;
        let due = before_due + Duration::from_millis(1);
        let waiting = testing::run_on(server.wire(), |out| world.body.heartbeat(before_due, out));
        waiting.result.unwrap();
        assert_eq!(waiting.unreliable, []);
        let sent = testing::run_on(server.wire(), |out| world.body.heartbeat(due, out));
        sent.result.unwrap();
        assert_eq!(sent.sent, []);
        assert_eq!(sent.unreliable.len(), 1);
        let packet = &sent.unreliable[0];
        assert_eq!(packet.opcode, POSITION_OPCODE);
        assert_eq!(packet.body.len(), 36);
        assert_eq!(packet.body[28..32], stroke.position.z.to_le_bytes());
        assert!(f32::from_le_bytes(packet.body[8..12].try_into().unwrap()) < 0.0);
        assert!(sent.events.is_empty());
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "Exact equality verifies unchanged or explicitly assigned state"
    )]
    fn a_correction_moves_every_owner_of_the_position_or_none() {
        let mut player = testing::player(7);
        let now = Instant::now();
        let mut body = Body::default();
        body.admit(MotionSession::new(12, 7, player.position, now).unwrap());
        body.correct(&mut player, at(-123.0), now).unwrap();
        assert_eq!(player.position, at(-123.0));
        assert_eq!(body.position(), Some(at(-123.0)));
        assert_eq!(body.stationary.position, at(-123.0));
        assert_eq!(body.stationary.spawn_id, 7);
        let invalid = Position {
            x: f32::NAN,
            ..at(0.0)
        };
        assert!(body.correct(&mut player, invalid, now).is_err());
        assert_eq!(player.position, at(-123.0));
        assert_eq!(body.position(), Some(at(-123.0)));
        assert_eq!(body.stationary.position, at(-123.0));
        // Without a movement session, the stationary heartbeat follows.
        let mut body = Body::default();
        body.correct(&mut player, at(10.0), now).unwrap();
        assert_eq!(player.position, at(10.0));
        assert_eq!(body.stationary.position.x, 10.0);
    }

    #[test]
    fn without_movement_the_profile_position_repeats_each_second() {
        let mut world = World::new(5);
        world.admitted = Some(Instant::now());
        let mut motion = Motion::new(false, false, None);
        let now = Instant::now();
        for supply_position in [false, true] {
            if supply_position {
                world.body.place(at(1.0));
            }
            let outcome = testing::run(|out| motion.tick(now, &mut world, out));
            outcome.result.unwrap();
            assert_eq!(
                outcome.unreliable,
                Vec::<eq_network_game::command::EncodedCommand>::new()
            );
        }
        world.body.own(7);
        let outcome = testing::run(|out| motion.tick(now, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.unreliable.len(), 1);
        assert_eq!(
            outcome.unreliable[0],
            PositionPacket {
                spawn_id: 7,
                sequence: 0,
                position: at(1.0),
                delta: [0.0; 3],
                animation: 0,
                delta_heading: 0,
            }
            .packet()
            .unwrap()
        );
        let outcome = testing::run(|out| motion.tick(now, &mut world, out));
        assert!(
            outcome.unreliable.is_empty(),
            "expected no unreliable packet, got {:?}",
            outcome.unreliable
        );
        let outcome = testing::run(|out| motion.tick(now + STATIONARY_HEARTBEAT, &mut world, out));
        assert_eq!(&outcome.unreliable[0].body[2..4], &[1, 0]);
    }

    #[test]
    fn a_manual_calibration_cannot_bypass_authoritative_inputs() {
        let now = Instant::now();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        world.admitted = now.checked_sub(Duration::from_millis(10));
        // The host calibrates before the feature has started movement.
        let configure = ClientCommand::ConfigureMotion {
            session_id: 5,
            calibration: eq_network_game::movement::MotionCalibration {
                swim_backward_units_per_second: None,
                diagonal: DiagonalRates::default(),
                units_per_second: 6.0,
                velocity_scale: 0.05,
                animation: 12,
                backward: None,
                walk: None,
                strafe: None,
            },
            created: now.checked_sub(Duration::from_millis(5)).unwrap(),
        };
        let mut motion = Motion::new(false, false, None);
        testing::run(|out| motion.admitted(&mut world, out))
            .result
            .unwrap();
        let outcome = testing::run(|out| motion.handle(&configure, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(outcome.events[..], [ClientEvent::Diagnostic(_)]));
    }

    #[test]
    fn the_server_correcting_the_player_withdraws_movement_until_recalibrated() {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        world.admitted = Some(Instant::now());
        let mut motion = Motion::new(false, false, None);
        testing::run(|out| motion.admitted(&mut world, out))
            .result
            .unwrap();
        let correction = Message::Event(WorldEvent::Position {
            spawn_id: 7,
            position: at(3.0),
            velocity: [0.0; 3],
            forward: 0.0,
        });
        let outcome = testing::run(|out| motion.observe(&correction, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(world.body.position(), Some(at(3.0)));
        assert_eq!(world.player.as_ref().unwrap().position, at(3.0));
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::MotionState {
                units_per_second: None,
                ..
            })]
        ));
        // Another spawn's position is not the player's.
        let other = Message::Event(WorldEvent::Position {
            spawn_id: 8,
            position: at(9.0),
            velocity: [0.0; 3],
            forward: 0.0,
        });
        testing::run(|out| motion.observe(&other, &mut world, out))
            .result
            .unwrap();
        assert_eq!(world.body.position(), Some(at(3.0)));
    }

    /// The speed each step granted, or None where it withdrew movement.
    fn speeds(events: &[ClientEvent]) -> Vec<Option<f32>> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::MotionState {
                    units_per_second, ..
                }) => Some(*units_per_second),
                _ => None,
            })
            .collect()
    }

    const EQMAC: &dyn super::super::wire::Wire = &super::super::wire::EqMac;

    /// An `EQMac` player, level 50, admitted and not yet granted speeds.
    fn eqmac_admitted() -> (World, Motion) {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        (player.run_speed, player.walk_speed, player.level) = (0.7, 0.46, 50);
        world.player.admit(player);
        world.admitted = Some(Instant::now());
        let mut motion = Motion::new(true, true, None);
        motion
            .admit(
                &Message::Event(WorldEvent::Nourishment(
                    eq_network_game::food::Nourishment {
                        fatigue: Some(0),
                        ..Default::default()
                    },
                )),
                &mut world,
            )
            .unwrap();
        let outcome = testing::run_on(EQMAC, |out| motion.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(
            speeds(&outcome.events).iter().all(Option::is_none),
            "granted before the report"
        );
        (world, motion)
    }

    /// The host's report of the player's speed inputs, carrying `weight` of
    /// 100.
    fn report(rate: i16, weight: u16) -> ClientCommand {
        ClientCommand::SpeedInputs {
            session_id: 5,
            revision: 1,
            report_id: 1,
            state: SpeedInputState::Ready(SpeedInputs {
                eqmac_fatigue: Some(eq_network_game::movement::EqMacFatigueInputs {
                    strength: 100,
                    stamina: Some(100),
                }),
                rate,
                weight,
                capacity: 100,
            }),
        }
    }

    #[test]
    fn fatigue_ticks_reuse_current_prerequisites_without_withdrawal_or_new_revision() {
        let (mut world, mut motion) = eqmac_admitted();
        let command = report(0, 95);
        testing::run_on(EQMAC, |out| motion.inputs(&command, &mut world, out))
            .result
            .unwrap();
        world.take_news();
        let receipt = motion.accepted_inputs;
        let context = world.body.stop_context();
        for fatigue in [1, 2, 100] {
            let outcome = testing::run_on(EQMAC, |out| {
                motion.observe(
                    &Message::Event(WorldEvent::Nourishment(
                        eq_network_game::food::Nourishment {
                            fatigue: Some(fatigue),
                            ..Default::default()
                        },
                    )),
                    &mut world,
                    out,
                )
            });
            outcome.result.unwrap();
            assert!(outcome.events.is_empty(), "{:#?}", outcome.events);
            assert!(world.take_news().is_empty());
            assert_eq!(motion.revision, 1);
            assert_eq!(world.body.stop_context(), context);
        }
        let event = Message::Event(WorldEvent::Nourishment(
            eq_network_game::food::Nourishment {
                fatigue: Some(110),
                ..Default::default()
            },
        ));
        let outcome = testing::run_on(EQMAC, |out| motion.observe(&event, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(motion.revision, 1);
        assert!(world.take_news().is_empty());
        assert!(speeds(&outcome.events).iter().all(Option::is_some));
        assert_eq!(
            motion.accepted_inputs, receipt,
            "derived capacity is not a host report"
        );
        assert!(world.body.motion.as_ref().unwrap().calibration().is_some());
        assert!(
            !speeds(&outcome.events).is_empty(),
            "changed capacity changes speed"
        );
        let outcome = testing::run_on(EQMAC, |out| motion.observe(&event, &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.events.is_empty());
    }

    #[test]
    fn fatigue_cannot_restore_root_missing_prerequisites_or_a_dead_session() {
        for boundary in 0..3 {
            let (mut world, mut motion) = eqmac_admitted();
            let command = report(if boundary == 0 { -1000 } else { 0 }, 0);
            testing::run_on(EQMAC, |out| motion.inputs(&command, &mut world, out))
                .result
                .unwrap();
            if boundary == 1 {
                testing::run_on(EQMAC, |out| motion.require_inputs(&mut world, out))
                    .result
                    .unwrap();
            } else if boundary == 2 {
                world.died();
            }
            let outcome = testing::run_on(EQMAC, |out| {
                motion.observe(
                    &Message::Event(WorldEvent::Nourishment(
                        eq_network_game::food::Nourishment {
                            fatigue: Some(1),
                            ..Default::default()
                        },
                    )),
                    &mut world,
                    out,
                )
            });
            outcome.result.unwrap();
            assert!(speeds(&outcome.events).iter().all(Option::is_none));
            if boundary == 2 {
                assert!(world.lifecycle.blocks_motion());
            } else {
                assert!(world.body.motion.as_ref().unwrap().calibration().is_none());
            }
        }
    }

    #[test]
    fn eqmac_without_fatigue_prerequisites_keeps_legacy_withdrawal_and_capacity() {
        let (mut world, mut motion) = eqmac_admitted();
        let mut command = report(0, 0);
        if let ClientCommand::SpeedInputs {
            state: SpeedInputState::Ready(inputs),
            ..
        } = &mut command
        {
            inputs.eqmac_fatigue = None;
        }
        testing::run_on(EQMAC, |out| motion.inputs(&command, &mut world, out))
            .result
            .unwrap();
        let outcome = testing::run_on(EQMAC, |out| {
            motion.observe(
                &Message::Event(WorldEvent::Nourishment(
                    eq_network_game::food::Nourishment {
                        fatigue: Some(1),
                        ..Default::default()
                    },
                )),
                &mut world,
                out,
            )
        });
        outcome.result.unwrap();
        // Asked again, the grant in force goes on while the host answers.
        assert_eq!(motion.revision, 2);
        assert_eq!(speeds(&outcome.events), Vec::<Option<f32>>::new());
        assert!(motion.inputs.is_some());
        assert!(world.body.motion.as_ref().unwrap().calibration().is_some());
    }

    fn acceptances(events: &[ClientEvent]) -> Vec<SpeedInputAcceptance> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::SpeedInputsAccepted(accepted)) => Some(*accepted),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn accepted_reports_are_identified_even_when_the_calibration_is_unchanged() {
        use super::super::servers;
        use crate::client::ServerProtocol;

        for protocol in [
            ServerProtocol::EqEmu,
            ServerProtocol::Project1999,
            ServerProtocol::Takp,
            ServerProtocol::Quarm,
        ] {
            let wire = servers::server_type(protocol).wire();
            let (mut world, mut motion) = eqmac_admitted();
            for (report_id, weight) in [(11, 0), (12, 1), (13, 0)] {
                let inputs = SpeedInputs {
                    eqmac_fatigue: None,
                    rate: 0,
                    weight,
                    capacity: 100,
                };
                let report = ClientCommand::SpeedInputs {
                    session_id: 5,
                    revision: 1,
                    report_id,
                    state: SpeedInputState::Ready(inputs),
                };
                let outcome = testing::run_on(wire, |out| motion.handle(&report, &mut world, out));
                outcome.result.unwrap();
                assert_eq!(
                    acceptances(&outcome.events),
                    [SpeedInputAcceptance {
                        session_id: 5,
                        revision: 1,
                        report_id,
                        inputs,
                        granted: true,
                    }]
                );
                if report_id != 11 {
                    assert_eq!(speeds(&outcome.events), Vec::<Option<f32>>::new());
                }
                assert!(outcome.sent.is_empty() && outcome.unreliable.is_empty());
                let tick =
                    testing::run_on(wire, |out| motion.tick(Instant::now(), &mut world, out));
                tick.result.unwrap();
                assert_eq!(
                    acceptances(&tick.events),
                    Vec::<SpeedInputAcceptance>::new()
                );
            }
        }
    }

    #[test]
    fn unavailable_root_and_obsolete_reports_cannot_acknowledge_a_positive_grant() {
        use eq_network_game::movement::SpeedInputUnavailable;

        let (mut world, mut motion) = eqmac_admitted();
        let unavailable = ClientCommand::SpeedInputs {
            session_id: 5,
            revision: 1,
            report_id: 21,
            state: SpeedInputState::Unavailable(SpeedInputUnavailable::Effects),
        };
        let output = testing::run_on(EQMAC, |out| motion.handle(&unavailable, &mut world, out));
        output.result.unwrap();
        assert_eq!(
            acceptances(&output.events),
            Vec::<SpeedInputAcceptance>::new()
        );
        let mut rooted = report(-1000, 0);
        if let ClientCommand::SpeedInputs { report_id, .. } = &mut rooted {
            *report_id = 22;
        }
        let output = testing::run_on(EQMAC, |out| motion.handle(&rooted, &mut world, out));
        output.result.unwrap();
        assert!(matches!(
            acceptances(&output.events)[..],
            [SpeedInputAcceptance {
                report_id: 22,
                granted: false,
                ..
            }]
        ));
        for (session_id, revision) in [(4, 1), (5, 0)] {
            let stale = ClientCommand::SpeedInputs {
                session_id,
                revision,
                report_id: 23,
                state: SpeedInputState::Ready(SpeedInputs {
                    eqmac_fatigue: None,
                    rate: 0,
                    weight: 0,
                    capacity: 100,
                }),
            };
            let output = testing::run_on(EQMAC, |out| motion.handle(&stale, &mut world, out));
            output.result.unwrap();
            assert_eq!(
                acceptances(&output.events),
                Vec::<SpeedInputAcceptance>::new()
            );
            assert!(world.body.motion.as_ref().unwrap().calibration().is_none());
        }
        let output = testing::run_on(EQMAC, |out| motion.handle(&report(0, 0), &mut world, out));
        output.result.unwrap();
        assert!(matches!(
            acceptances(&output.events)[..],
            [SpeedInputAcceptance { granted: true, .. }]
        ));
    }

    #[test]
    fn an_accepted_report_receives_its_later_positive_grant_without_reposting() {
        let (mut world, mut motion) = eqmac_admitted();
        let mut player = world.player.as_ref().unwrap().clone();
        player.run_speed = f32::NAN;
        world.player.admit(player);
        let output = testing::run_on(EQMAC, |out| motion.handle(&report(0, 0), &mut world, out));
        output.result.unwrap();
        assert!(matches!(
            acceptances(&output.events)[..],
            [SpeedInputAcceptance { granted: false, .. }]
        ));
        let mut player = world.player.as_ref().unwrap().clone();
        player.run_speed = 0.7;
        world.player.admit(player);
        let output = testing::run_on(EQMAC, |out| motion.tick(Instant::now(), &mut world, out));
        output.result.unwrap();
        assert!(matches!(
            acceptances(&output.events)[..],
            [SpeedInputAcceptance {
                report_id: 1,
                granted: true,
                ..
            }]
        ));
        let output = testing::run_on(EQMAC, |out| motion.tick(Instant::now(), &mut world, out));
        output.result.unwrap();
        assert_eq!(
            acceptances(&output.events),
            Vec::<SpeedInputAcceptance>::new()
        );
    }

    /// Whether a step granted the one speed, near `expected`.
    fn granted(events: &[ClientEvent], expected: f32) -> bool {
        matches!(speeds(events)[..], [Some(speed)] if (speed - expected).abs() < 0.05)
    }

    #[test]
    fn eqmac_grants_the_programs_speeds_once_the_host_reports_their_inputs() {
        let eqmac = EQMAC;
        let (mut world, mut motion) = eqmac_admitted();
        let outcome = testing::run_on(eqmac, |out| motion.handle(&report(0, 50), &mut world, out));
        outcome.result.unwrap();
        let close = |granted: &[Option<f32>], expected: f32| matches!(granted, [Some(speed)] if (speed - expected).abs() < 0.05);
        assert!(
            close(&speeds(&outcome.events), 29.9),
            "{:?}",
            outcome.events
        );
        // The same speeds again tell the host nothing.
        let now = Instant::now();
        let outcome = testing::run_on(eqmac, |out| motion.tick(now, &mut world, out));
        assert!(
            speeds(&outcome.events).is_empty(),
            "granted {:?}",
            outcome.events
        );
        // Badly hurt, then sneaking, the player slows at the next tick.
        let hurt = Message::Event(WorldEvent::HitPoints {
            spawn_id: 7,
            current: 40,
            maximum: 500,
            items: eq_network_game::world::ItemHitPoints::LeftOutOfCurrent,
        });
        testing::run_on(eqmac, |out| motion.observe(&hurt, &mut world, out))
            .result
            .unwrap();
        let outcome = testing::run_on(eqmac, |out| motion.tick(now, &mut world, out));
        assert!(
            close(&speeds(&outcome.events), 14.9),
            "{:?}",
            outcome.events
        );
        let sneaking = Message::Event(WorldEvent::Appearance {
            spawn_id: 7,
            change: AppearanceChange::Sneaking(true),
        });
        testing::run_on(eqmac, |out| motion.observe(&sneaking, &mut world, out))
            .result
            .unwrap();
        let outcome = testing::run_on(eqmac, |out| motion.tick(now, &mut world, out));
        assert!(close(&speeds(&outcome.events), 4.9), "{:?}", outcome.events);
        let ended = Message::Event(WorldEvent::HideAndSneakEnded);
        testing::run_on(eqmac, |out| motion.observe(&ended, &mut world, out))
            .result
            .unwrap();
        let outcome = testing::run_on(eqmac, |out| motion.tick(now, &mut world, out));
        assert!(
            close(&speeds(&outcome.events), 14.9),
            "{:?}",
            outcome.events
        );
        // A correction moves the player and grants the speeds in force again
        // at once, while the host reports a fresh revision.
        let correction = Message::Event(WorldEvent::Position {
            spawn_id: 7,
            position: at(3.0),
            velocity: [0.0; 3],
            forward: 0.0,
        });
        let outcome = testing::run_on(eqmac, |out| motion.observe(&correction, &mut world, out));
        assert!(
            close(&speeds(&outcome.events), 14.9),
            "{:?}",
            outcome.events
        );
        assert!(!withdrew(&outcome.events));
        let outcome = testing::run_on(eqmac, |out| motion.tick(now, &mut world, out));
        assert_eq!(speeds(&outcome.events), Vec::<Option<f32>>::new());
        let fresh_report = |rate| ClientCommand::SpeedInputs {
            session_id: 5,
            revision: 2,
            report_id: 1,
            state: SpeedInputState::Ready(SpeedInputs {
                eqmac_fatigue: None,
                rate,
                weight: 50,
                capacity: 100,
            }),
        };
        // The answer matches the grant in force: nothing changes.
        let outcome = testing::run_on(eqmac, |out| {
            motion.handle(&fresh_report(0), &mut world, out)
        });
        assert_eq!(speeds(&outcome.events), Vec::<Option<f32>>::new());
        // A root grants no speed until it wears off, withdrawing nothing.
        let outcome = testing::run_on(eqmac, |out| {
            motion.handle(&fresh_report(-1000), &mut world, out)
        });
        assert_eq!(speeds(&outcome.events), [Some(0.0)]);
        let outcome = testing::run_on(eqmac, |out| {
            motion.handle(&fresh_report(0), &mut world, out)
        });
        assert!(
            close(&speeds(&outcome.events), 14.9),
            "{:?}",
            outcome.events
        );
    }

    #[test]
    fn eqmac_slows_a_player_carrying_too_much_and_says_so_once_each_way() {
        let (mut world, mut motion) = eqmac_admitted();
        let encumbrance = |events: &[ClientEvent]| -> Vec<(bool, u32)> {
            events
                .iter()
                .filter_map(|event| match event {
                    ClientEvent::World(WorldEvent::Encumbrance {
                        session_id: 5,
                        encumbered,
                        string_id,
                    }) => Some((*encumbered, *string_id)),
                    _ => None,
                })
                .collect()
        };
        let mut step = |command| {
            let outcome = testing::run_on(EQMAC, |out| motion.handle(&command, &mut world, out));
            outcome.result.unwrap();
            outcome.events
        };
        // The first report only learns the state.
        let events = step(report(0, 50));
        assert!(encumbrance(&events).is_empty() && granted(&events, 29.9));
        let events = step(report(0, 150));
        assert_eq!(encumbrance(&events), [(true, 0x3068)]);
        assert!(granted(&events, 29.9 * 0.75), "{events:?}");
        let events = step(report(0, 160));
        assert!(encumbrance(&events).is_empty(), "{events:?}");
        let events = step(report(0, 100));
        assert_eq!(encumbrance(&events), [(false, 0x3067)]);
        assert!(granted(&events, 29.9), "{events:?}");
    }

    #[test]
    fn the_hosts_climb_goes_to_the_guard_of_an_admitted_player_only() {
        let climb = |per_step| ClientCommand::ClimbReach {
            session_id: 5,
            per_step,
        };
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        let mut motion = Motion::new(true, false, None);
        // Before the admission there is no guard to tell.
        let outcome = testing::run(|out| motion.handle(&climb(3.15), &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(outcome.events[..], [ClientEvent::Diagnostic(_)]));
        world.admitted = Some(Instant::now());
        testing::run(|out| motion.admitted(&mut world, out))
            .result
            .unwrap();
        let outcome = testing::run(|out| motion.handle(&climb(3.15), &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.events.is_empty(), "{:?}", outcome.events);
        let outcome = testing::run(|out| motion.handle(&climb(f32::NAN), &mut world, out));
        assert!(matches!(outcome.events[..], [ClientEvent::Diagnostic(_)]));
    }

    #[test]
    fn titanium_grants_the_programs_speeds_with_its_own_wire_values() {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        (player.run_speed, player.walk_speed, player.level) = (0.7, 0.46, 50);
        world.player.admit(player);
        world.admitted = Some(Instant::now());
        let mut motion = Motion::new(false, false, None);
        let outcome = testing::run(|out| motion.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(
            speeds(&outcome.events).iter().all(Option::is_none),
            "granted before the report"
        );
        let outcome = testing::run(|out| motion.handle(&report(0, 50), &mut world, out));
        outcome.result.unwrap();
        assert!(granted(&outcome.events, 29.9), "{:?}", outcome.events);
        let calibration = world.body.motion.as_ref().unwrap().calibration().unwrap();
        // Forty times the input on the wire, cut toward zero, and the
        // velocity in units a tick.
        assert_eq!(calibration.animation, 28);
        assert_eq!(calibration.walk.unwrap().animation, 18);
        assert_eq!(calibration.backward.unwrap().animation, -18);
        assert_eq!(calibration.strafe.unwrap().animation, 0);
        assert!((calibration.velocity_scale - 0.05).abs() < f32::EPSILON);
        // A slow takes no more than 65% of the run speed off.
        let outcome = testing::run(|out| motion.handle(&report(-90, 50), &mut world, out));
        assert!(
            granted(&outcome.events, 0.245 * 42.667),
            "{:?}",
            outcome.events
        );
        let mut sentinel = report(0, u16::MAX);
        if let ClientCommand::SpeedInputs {
            state: SpeedInputState::Ready(inputs),
            ..
        } = &mut sentinel
        {
            inputs.capacity = 0;
            inputs.eqmac_fatigue = Some(eq_network_game::movement::EqMacFatigueInputs {
                strength: i64::MIN,
                stamina: None,
            });
        }
        let outcome = testing::run(|out| motion.handle(&sentinel, &mut world, out));
        outcome.result.unwrap();
        assert!(
            granted(&outcome.events, 29.9),
            "Titanium keeps its zero sentinel and ignores EQMac inputs"
        );
    }

    #[test]
    fn the_player_dying_withdraws_movement_and_stops_the_body() {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        world.admitted = Some(Instant::now());
        let mut motion = Motion::new(false, false, None);
        testing::run(|out| motion.admitted(&mut world, out))
            .result
            .unwrap();
        let death = Message::Event(WorldEvent::Death(eq_network_game::zoning::Death {
            spawn_id: 7,
            killer_id: 0,
            corpse_id: 9,
            bind_zone_id: 2,
            corpse_name: None,
        }));
        let outcome = testing::run(|out| motion.observe(&death, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::MotionState {
                units_per_second: None,
                ..
            })]
        ));
        // The transfers feature hears the death too and stops the body; a
        // suspended session sends no heartbeat.
        world.died();
        let later = Instant::now() + STATIONARY_HEARTBEAT * 2;
        let outcome = testing::run(|out| motion.tick(later, &mut world, out));
        assert!(
            outcome.unreliable.is_empty(),
            "expected no unreliable packet, got {:?}",
            outcome.unreliable
        );
    }
    /// With a grant in force, inputs the host cannot prove keep the last ones
    /// in force with no time limit (Adam's decision of 11-10, 6.451); a slower
    /// bound it proves replaces them in place; the first exact report ends
    /// both. Without a grant, a bound starts one and nothing provable waits.
    #[test]
    fn unproven_inputs_keep_a_grant_and_proven_bounds_slow_it_in_place() {
        use eq_network_game::{money::Wallet, movement::SpeedInputUnavailable, world::Coins};
        let unavailable = |revision| ClientCommand::SpeedInputs {
            session_id: 5,
            revision,
            report_id: 9,
            state: SpeedInputState::Unavailable(SpeedInputUnavailable::Inventory),
        };
        let inputs = |rate, weight| SpeedInputs {
            eqmac_fatigue: None,
            rate,
            weight,
            capacity: 100,
        };
        // Without a grant, unavailable inputs withdraw as before.
        let (mut world, mut motion) = eqmac_admitted();
        let output = testing::run_on(EQMAC, |out| motion.handle(&unavailable(1), &mut world, out));
        output.result.unwrap();
        assert!(withdrew(&output.events));
        assert_eq!(world_speed(&world), None);
        // A bound starts a grant from what the host could prove, as at a
        // zone-in with a term unproven from the start.
        let bounded = ClientCommand::SpeedInputs {
            session_id: 5,
            revision: 1,
            report_id: 10,
            state: SpeedInputState::Bounded {
                inputs: inputs(0, 0),
                missing: SpeedInputUnavailable::Effects,
            },
        };
        let output = testing::run_on(EQMAC, |out| motion.handle(&bounded, &mut world, out));
        output.result.unwrap();
        assert!(world_speed(&world).is_some());

        let (mut world, mut motion) = eqmac_admitted();
        testing::run_on(EQMAC, |out| motion.handle(&report(0, 0), &mut world, out))
            .result
            .unwrap();
        let full = world_speed(&world).unwrap();
        // With one, the grant and its inputs stay, noted once.
        let output = testing::run_on(EQMAC, |out| motion.handle(&unavailable(1), &mut world, out));
        output.result.unwrap();
        assert!(!withdrew(&output.events));
        assert_eq!(world_speed(&world), Some(full));
        assert_eq!(motion.unproven, Some(SpeedInputUnavailable::Inventory));
        // A coin change asks again; long after, the grant is still in force.
        world.coins = Wallet {
            purse: Some(Coins {
                copper: 400,
                ..Coins::default()
            }),
            ..Wallet::default()
        }
        .into();
        let later = Instant::now() + Duration::from_secs(30);
        let output = testing::run_on(EQMAC, |out| motion.tick(later, &mut world, out));
        output.result.unwrap();
        let output = testing::run_on(EQMAC, |out| motion.tick(later, &mut world, out));
        output.result.unwrap();
        assert!(!withdrew(&output.events));
        assert_eq!(motion.revision, 2);
        assert_eq!(world_speed(&world), Some(full));
        // An old report cannot answer the new revision.
        testing::run_on(EQMAC, |out| motion.handle(&report(0, 0), &mut world, out))
            .result
            .unwrap();
        assert_eq!(world_speed(&world), Some(full));
        // A proven heavier load replaces the grant in place: slower.
        let heavier = ClientCommand::SpeedInputs {
            session_id: 5,
            revision: 2,
            report_id: 11,
            state: SpeedInputState::Bounded {
                inputs: inputs(0, 150),
                missing: SpeedInputUnavailable::Effects,
            },
        };
        let output = testing::run_on(EQMAC, |out| motion.handle(&heavier, &mut world, out));
        output.result.unwrap();
        assert!(!withdrew(&output.events));
        assert_eq!(motion.unproven, Some(SpeedInputUnavailable::Effects));
        let slowed = world_speed(&world).unwrap();
        assert!(slowed < full);
        // The first exact report ends it, in place.
        let exact = ClientCommand::SpeedInputs {
            session_id: 5,
            revision: 2,
            report_id: 12,
            state: SpeedInputState::Ready(inputs(0, 10)),
        };
        let output = testing::run_on(EQMAC, |out| motion.handle(&exact, &mut world, out));
        output.result.unwrap();
        assert!(!withdrew(&output.events));
        assert_eq!(motion.unproven, None);
        assert_eq!(world_speed(&world), Some(full));
        // A root the host proves, whatever else it cannot, withdraws.
        let rooted = ClientCommand::SpeedInputs {
            session_id: 5,
            revision: 2,
            report_id: 13,
            state: SpeedInputState::Bounded {
                inputs: inputs(-1000, 10),
                missing: SpeedInputUnavailable::Effects,
            },
        };
        let output = testing::run_on(EQMAC, |out| motion.handle(&rooted, &mut world, out));
        output.result.unwrap();
        assert!(!withdrew(&output.events));
        assert!(was_rooted(&output.events));
        assert_eq!(world_speed(&world), None);
    }

    /// After a root, a bound with a term still unproven starts the grant
    /// again: a root that fades while a song runs does not leave the player
    /// frozen until the song ends. The host proves the terms, so a bound is
    /// taken as reported, slower or faster than the inputs in force.
    #[test]
    fn a_bound_restarts_after_a_root_and_is_taken_as_reported() {
        use eq_network_game::movement::SpeedInputUnavailable;
        let bounded = |report_id, rate, weight, capacity| ClientCommand::SpeedInputs {
            session_id: 5,
            revision: 1,
            report_id,
            state: SpeedInputState::Bounded {
                inputs: SpeedInputs {
                    eqmac_fatigue: None,
                    rate,
                    weight,
                    capacity,
                },
                missing: SpeedInputUnavailable::Effects,
            },
        };
        let (mut world, mut motion) = eqmac_admitted();
        testing::run_on(EQMAC, |out| motion.handle(&report(0, 50), &mut world, out))
            .result
            .unwrap();
        let full = world_speed(&world).unwrap();
        // Rooted while a song runs.
        let output = testing::run_on(EQMAC, |out| {
            motion.handle(&bounded(2, -1000, 50, 100), &mut world, out)
        });
        output.result.unwrap();
        assert!(!withdrew(&output.events));
        assert!(was_rooted(&output.events));
        assert!(world.body.motion.as_ref().unwrap().rooted());
        // The root fades; the song is still unproven.
        let output = testing::run_on(EQMAC, |out| {
            motion.handle(&bounded(3, 0, 50, 100), &mut world, out)
        });
        output.result.unwrap();
        assert!(!withdrew(&output.events));
        assert_eq!(world_speed(&world), Some(full));
        // A faster bound (a proven snare ended) is taken as reported.
        let output = testing::run_on(EQMAC, |out| {
            motion.handle(&bounded(4, 50, 0, 500), &mut world, out)
        });
        output.result.unwrap();
        assert!(world_speed(&world).unwrap() > full);
        assert_eq!(
            motion
                .inputs
                .map(|inputs| (inputs.rate, inputs.weight, inputs.capacity)),
            Some((50, 0, 500))
        );
        // A slower one applies too, and an exact report replaces it.
        testing::run_on(EQMAC, |out| {
            motion.handle(&bounded(5, -50, 50, 100), &mut world, out)
        })
        .result
        .unwrap();
        assert!(world_speed(&world).unwrap() < full);
        let exact = ClientCommand::SpeedInputs {
            session_id: 5,
            revision: 1,
            report_id: 6,
            state: SpeedInputState::Ready(SpeedInputs {
                eqmac_fatigue: None,
                rate: 0,
                weight: 50,
                capacity: 100,
            }),
        };
        testing::run_on(EQMAC, |out| motion.handle(&exact, &mut world, out))
            .result
            .unwrap();
        assert_eq!(world_speed(&world), Some(full));
        assert_eq!(motion.unproven, None);
    }

    /// The guard takes its bound from the grant in force in the same step: a
    /// sample at the old rate after a proven snare slows the grant is refused.
    #[test]
    fn a_sample_at_the_old_rate_after_a_slower_grant_is_refused() {
        use eq_network_game::movement::SpeedInputUnavailable;
        let (mut world, mut motion) = eqmac_admitted();
        testing::run_on(EQMAC, |out| motion.handle(&report(0, 0), &mut world, out))
            .result
            .unwrap();
        let full = world_speed(&world).unwrap();
        let origin = world.body.position().unwrap();
        let start = Instant::now() + Duration::from_millis(100);
        let along = |dx: f32| Position {
            x: origin.x + dx,
            ..origin
        };
        let first = MovementRequest::new(MovementMode::Forward, 5, along(0.5), start);
        let output = testing::run_on(EQMAC, |out| Motion::step_at(&first, start, &mut world, out));
        output.result.unwrap();
        assert!(!output.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::MotionSent {
                refused: Some(_),
                ..
            })
        )));
        let snared = ClientCommand::SpeedInputs {
            session_id: 5,
            revision: 1,
            report_id: 9,
            state: SpeedInputState::Bounded {
                inputs: SpeedInputs {
                    eqmac_fatigue: None,
                    rate: -50,
                    weight: 0,
                    capacity: 100,
                },
                missing: SpeedInputUnavailable::Effects,
            },
        };
        testing::run_on(EQMAC, |out| motion.handle(&snared, &mut world, out))
            .result
            .unwrap();
        let slowed = world_speed(&world).unwrap();
        assert!(slowed < full);
        // Half a second at the full rate is beyond the snared grant.
        let later = start + Duration::from_millis(500);
        let fast = MovementRequest::new(MovementMode::Forward, 5, along(0.5 + full * 0.5), later);
        let output = testing::run_on(EQMAC, |out| Motion::step_at(&fast, later, &mut world, out));
        output.result.unwrap();
        assert!(
            output.events.iter().any(|event| matches!(
                event,
                ClientEvent::World(WorldEvent::MotionSent {
                    refused: Some(_),
                    ..
                })
            )),
            "{:?}",
            output.events
        );
    }

    /// A Bard's song pulses refresh a buff on the player every few seconds.
    /// The program reads its speed modifier live and stops input only for a
    /// stun, a freeze or not standing (read R7, 0x4a1d74), so each pulse asks
    /// the host again while the grant goes on: no withdrawal, no stop.
    #[test]
    fn a_song_pulse_keeps_the_grant_and_sends_no_stop() {
        use eq_network_game::buffs::BuffUpdate;
        let (mut world, mut motion) = eqmac_admitted();
        testing::run_on(EQMAC, |out| motion.handle(&report(0, 0), &mut world, out))
            .result
            .unwrap();
        let full = world_speed(&world);
        let origin = world.body.position().unwrap();
        let moved = Position {
            x: origin.x + 1.0,
            ..origin
        };
        let now = Instant::now() + Duration::from_millis(100);
        let step = MovementRequest::new(MovementMode::Forward, 5, moved, now);
        testing::run_on(EQMAC, |out| Motion::step_at(&step, now, &mut world, out))
            .result
            .unwrap();
        for pulse in 0_u64..5 {
            let revision = motion.revision;
            let refreshed = Message::Event(WorldEvent::Buff(BuffUpdate {
                spell_id: 700,
                entity_id: 7,
                slot: 0,
                replaces_slot: true,
                buff: None,
            }));
            let output = testing::run_on(EQMAC, |out| motion.observe(&refreshed, &mut world, out));
            output.result.unwrap();
            assert_eq!(speeds(&output.events), Vec::<Option<f32>>::new(), "{pulse}");
            assert_eq!(output.unreliable, [], "{pulse}");
            assert_eq!(motion.revision, revision + 1);
            assert_eq!(world_speed(&world), full);
            // The host answers before the next pulse.
            let answer = ClientCommand::SpeedInputs {
                session_id: 5,
                revision: motion.revision,
                report_id: pulse + 2,
                state: SpeedInputState::Ready(SpeedInputs {
                    eqmac_fatigue: Some(eq_network_game::movement::EqMacFatigueInputs {
                        strength: 100,
                        stamina: Some(100),
                    }),
                    rate: 0,
                    weight: 0,
                    capacity: 100,
                }),
            };
            let output = testing::run_on(EQMAC, |out| motion.handle(&answer, &mut world, out));
            output.result.unwrap();
            assert!(!withdrew(&output.events));
            assert_eq!(world_speed(&world), full);
        }
    }

    /// A root is a grant at no speed (read R18, 0x410fdb zeroes the speed
    /// once and blocks nothing else): no withdrawal at the landing, none
    /// at each song pulse while rooted, a turn in place goes out, and the
    /// fade grants the speed again in place. Run 2026-11-11 row 3 withdrew
    /// ten times.
    #[test]
    fn a_root_grants_no_speed_and_withdraws_nothing_through_pulses_and_the_fade() {
        use eq_network_game::buffs::BuffUpdate;
        let (mut world, mut motion) = eqmac_admitted();
        testing::run_on(EQMAC, |out| motion.handle(&report(0, 0), &mut world, out))
            .result
            .unwrap();
        let full = world_speed(&world);
        let answer = |motion: &Motion, report_id: u64, rate: i16| ClientCommand::SpeedInputs {
            session_id: 5,
            revision: motion.revision,
            report_id,
            state: SpeedInputState::Ready(SpeedInputs {
                eqmac_fatigue: Some(eq_network_game::movement::EqMacFatigueInputs {
                    strength: 100,
                    stamina: Some(100),
                }),
                rate,
                weight: 0,
                capacity: 100,
            }),
        };
        let pulse = Message::Event(WorldEvent::Buff(BuffUpdate {
            spell_id: 230,
            entity_id: 7,
            slot: 0,
            replaces_slot: true,
            buff: None,
        }));
        // The root lands.
        let output = testing::run_on(EQMAC, |out| motion.observe(&pulse, &mut world, out));
        output.result.unwrap();
        assert!(!withdrew(&output.events));
        let output = testing::run_on(EQMAC, |out| {
            motion.handle(&answer(&motion, 2, -10_000), &mut world, out)
        });
        output.result.unwrap();
        assert_eq!(speeds(&output.events), [Some(0.0)]);
        assert!(world.body.motion.as_ref().unwrap().rooted());
        // Song pulses while rooted withdraw nothing and grant nothing new.
        for report_id in 3..6 {
            let output = testing::run_on(EQMAC, |out| motion.observe(&pulse, &mut world, out));
            output.result.unwrap();
            assert_eq!(speeds(&output.events), Vec::<Option<f32>>::new());
            let output = testing::run_on(EQMAC, |out| {
                motion.handle(&answer(&motion, report_id, -10_000), &mut world, out)
            });
            output.result.unwrap();
            assert_eq!(speeds(&output.events), Vec::<Option<f32>>::new());
        }
        // A turn in place goes out while rooted; a step does not.
        let origin = world.body.position().unwrap();
        let now = Instant::now() + Duration::from_millis(300);
        let turned = Position {
            heading: origin.heading + 32.0,
            ..origin
        };
        let turn = MovementRequest::new(MovementMode::Forward, 5, turned, now);
        let output = testing::run_on(EQMAC, |out| Motion::step_at(&turn, now, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.unreliable.len(), 1, "{:?}", output.events);
        assert_eq!(world.body.position().unwrap().heading, turned.heading);
        // The fade grants the speed again, in place.
        let output = testing::run_on(EQMAC, |out| motion.observe(&pulse, &mut world, out));
        output.result.unwrap();
        let output = testing::run_on(EQMAC, |out| {
            motion.handle(&answer(&motion, 9, 0), &mut world, out)
        });
        output.result.unwrap();
        assert!(!withdrew(&output.events));
        assert_eq!(speeds(&output.events), [full]);
        assert!(!world.body.motion.as_ref().unwrap().rooted());
    }

    /// The speed the session grants the player now, if any.
    /// A same-zone relocation (summon, GM move) or a local pad crossing,
    /// which the transfers feature has already made, keeps a grant in force
    /// as a correction does: no withdrawal, the calibration again at once,
    /// the inputs asked for again (read R18, the same-zone warp 0x48aaa6).
    /// Without a grant both wait for the host as before.
    #[test]
    fn a_relocation_or_pad_crossing_keeps_a_grant_in_force() {
        let landing = |world: &World| Position {
            x: world.body.position().unwrap().x + 40.0,
            ..world.body.position().unwrap()
        };
        let moves = |position: Position| {
            [
                WorldEvent::SameZoneRelocated(eq_network_game::zoning::SameZoneRelocation {
                    session_id: 5,
                    spawn_id: 7,
                    position,
                }),
                WorldEvent::LocalPadCrossed(eq_network_game::zoning::LocalPadCrossing {
                    session_id: 5,
                    spawn_id: 7,
                    entry: position,
                    landing: position,
                }),
            ]
        };
        for index in 0..2 {
            // With a grant in force.
            let (mut world, mut motion) = eqmac_admitted();
            testing::run_on(EQMAC, |out| motion.handle(&report(0, 0), &mut world, out))
                .result
                .unwrap();
            let full = world_speed(&world);
            assert!(full.is_some());
            let position = landing(&world);
            world.correct_own(position, Instant::now()).unwrap();
            let revision = motion.revision;
            let event = Message::Event(moves(position)[index].clone());
            let output = testing::run_on(EQMAC, |out| motion.observe(&event, &mut world, out));
            output.result.unwrap();
            assert!(!withdrew(&output.events), "{index}: {:?}", output.events);
            assert_eq!(world_speed(&world), full, "{index}");
            assert_eq!(motion.revision, revision + 1, "{index}");
            assert_eq!(world.body.position(), Some(position));
            // Without one: the host is asked and movement waits.
            let (mut world, mut motion) = eqmac_admitted();
            let position = landing(&world);
            world.correct_own(position, Instant::now()).unwrap();
            let revision = motion.revision;
            let event = Message::Event(moves(position)[index].clone());
            let output = testing::run_on(EQMAC, |out| motion.observe(&event, &mut world, out));
            output.result.unwrap();
            assert!(withdrew(&output.events), "{index}: {:?}", output.events);
            assert_eq!(world_speed(&world), None, "{index}");
            assert_eq!(motion.revision, revision + 1, "{index}");
        }
    }

    fn world_speed(world: &World) -> Option<f32> {
        world
            .body
            .motion
            .as_ref()
            .and_then(MotionSession::calibration)
            .map(|speeds| speeds.units_per_second)
    }

    /// Whether the events grant the player rooted: at no speed.
    fn was_rooted(events: &[ClientEvent]) -> bool {
        events.iter().any(|event| {
            matches!(
                event,
                ClientEvent::World(WorldEvent::MotionState {
                    units_per_second: Some(speed),
                    ..
                }) if *speed == 0.0
            )
        })
    }

    /// Whether the events withdraw movement, keeping falls.
    fn withdrew(events: &[ClientEvent]) -> bool {
        events.iter().any(|event| {
            matches!(
                event,
                ClientEvent::World(WorldEvent::MotionState {
                    units_per_second: None,
                    falls: true,
                    ..
                })
            )
        })
    }

    /// Eating, drinking or picking something up keeps the grant until the
    /// host's new inputs land, so a jump or autorun under way goes on; the
    /// answer replaces the grant, and a request never answered lapses.
    #[test]
    fn a_change_in_belongings_keeps_the_grant_until_the_new_inputs_land() {
        use eq_network_game::{money::Wallet, world::Coins};
        let (mut world, mut motion) = eqmac_admitted();
        testing::run_on(EQMAC, |out| motion.handle(&report(0, 0), &mut world, out))
            .result
            .unwrap();
        let coins = |copper| Wallet {
            purse: Some(Coins {
                copper,
                ..Coins::default()
            }),
            ..Wallet::default()
        };
        let before = world_speed(&world);
        assert!(before.is_some());
        let revision = motion.revision;
        world.coins = coins(400).into();
        let outcome = testing::run_on(EQMAC, |out| motion.tick(Instant::now(), &mut world, out));
        outcome.result.unwrap();
        // Asked again, nothing withdrawn, the grant exactly as it was.
        assert_eq!(motion.revision, revision + 1);
        assert_eq!(speeds(&outcome.events), Vec::<Option<f32>>::new());
        assert_eq!(world_speed(&world), before);
        assert!(motion.kept_since.is_some());
        // Health still applies at once while the weight is stale.
        let hurt = Message::Event(WorldEvent::HitPoints {
            spawn_id: 7,
            current: 40,
            maximum: 500,
            items: eq_network_game::world::ItemHitPoints::LeftOutOfCurrent,
        });
        testing::run_on(EQMAC, |out| motion.observe(&hurt, &mut world, out))
            .result
            .unwrap();
        testing::run_on(EQMAC, |out| motion.tick(Instant::now(), &mut world, out))
            .result
            .unwrap();
        let slowed = world_speed(&world).unwrap();
        assert!(slowed < before.unwrap());
        assert!(motion.kept_since.is_some());
        let healed = Message::Event(WorldEvent::HitPoints {
            spawn_id: 7,
            current: 500,
            maximum: 500,
            items: eq_network_game::world::ItemHitPoints::LeftOutOfCurrent,
        });
        testing::run_on(EQMAC, |out| motion.observe(&healed, &mut world, out))
            .result
            .unwrap();
        testing::run_on(EQMAC, |out| motion.tick(Instant::now(), &mut world, out))
            .result
            .unwrap();
        assert_eq!(world_speed(&world), before);
        // The answer replaces it: heavier, so slower.
        let answer = |revision| ClientCommand::SpeedInputs {
            session_id: 5,
            revision,
            report_id: 1,
            state: SpeedInputState::Ready(SpeedInputs {
                eqmac_fatigue: None,
                rate: 0,
                weight: 150,
                capacity: 100,
            }),
        };
        let current = answer(motion.revision);
        testing::run_on(EQMAC, |out| motion.handle(&current, &mut world, out))
            .result
            .unwrap();
        assert!(motion.kept_since.is_none());
        let after = world_speed(&world).unwrap();
        assert!(after < before.unwrap());
        // A request never answered keeps the grant past the notice, and the
        // late answer to it still lands in place.
        world.coins = coins(500).into();
        testing::run_on(EQMAC, |out| motion.tick(Instant::now(), &mut world, out))
            .result
            .unwrap();
        assert_eq!(world_speed(&world), Some(after));
        let late = Instant::now() + KEPT_GRANT_NOTICE + Duration::from_millis(10);
        let outcome = testing::run_on(EQMAC, |out| motion.tick(late, &mut world, out));
        outcome.result.unwrap();
        assert!(!withdrew(&outcome.events));
        assert_eq!(world_speed(&world), Some(after));
        let late_answer = answer(motion.revision);
        testing::run_on(EQMAC, |out| motion.handle(&late_answer, &mut world, out))
            .result
            .unwrap();
        assert_eq!(world_speed(&world), Some(after));
    }

    fn motion_with_known_inventory(
        inventory: eq_network_game::inventory::Inventory,
    ) -> (World, Motion) {
        use eq_network_game::{money::Wallet, world::Coins};
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        (player.run_speed, player.walk_speed, player.level) = (0.7, 0.46, 50);
        world.player.admit(player);
        world.admitted = Some(Instant::now());
        world.inventory = inventory.into();
        world.coins = Wallet {
            purse: Some(Coins::default()),
            bank: Some(Coins::default()),
            ..Wallet::default()
        }
        .into();
        let mut motion = Motion::new(true, true, None);
        testing::run(|out| motion.admitted(&mut world, out))
            .result
            .unwrap();
        (world, motion)
    }

    #[test]
    fn pending_drop_grants_only_with_a_current_carried_weight_bound() {
        use eq_network_game::{
            inventory::{Inventory, InventorySlot, InventoryUpdate, WeightUnavailable},
            GameDialect,
        };
        use std::num::NonZeroU32;

        let mut original = testing::item(InventorySlot::CURSOR.0);
        original.details.weight_tenths = 50;
        original.rules.ground_drop_serial = NonZeroU32::new(17);
        let mut heavier = original.clone();
        heavier.details.weight_tenths = 1_500;
        heavier.rules.ground_drop_serial = NonZeroU32::new(18);
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![original.clone()]));

        let (mut world, mut motion) = motion_with_known_inventory(inventory.clone());
        let report_at = |revision, weight| ClientCommand::SpeedInputs {
            session_id: 5,
            revision,
            report_id: revision,
            state: SpeedInputState::Ready(SpeedInputs {
                eqmac_fatigue: None,
                rate: 0,
                weight,
                capacity: 200,
            }),
        };
        let weight =
            |inventory: &Inventory| inventory.carried_weight_tenths(GameDialect::Titanium, 0);
        assert_eq!(weight(&inventory), Ok(50));
        let ready = report_at(motion.revision, 5);
        let outcome = testing::run(|out| motion.handle(&ready, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            acceptances(&outcome.events)[..],
            [SpeedInputAcceptance { granted: true, .. }]
        ));

        inventory.apply(InventoryUpdate::GroundDropPending(
            original.rules.ground_drop_serial.unwrap(),
        ));
        assert_eq!(weight(&inventory), Ok(50));
        inventory.apply(InventoryUpdate::Cursor(vec![heavier.clone()]));
        assert!(!inventory.stale());
        assert_eq!(
            inventory.items().get(&InventorySlot::CURSOR),
            Some(&original)
        );
        assert_eq!(inventory.queued().count(), 1);
        assert_eq!(weight(&inventory), Ok(1_500));
        world.inventory = inventory.clone().into();
        // The grant is kept while the host works the new weight out ...
        let outcome = testing::run(|out| motion.tick(Instant::now(), &mut world, out));
        outcome.result.unwrap();
        assert!(!withdrew(&outcome.events));
        assert!(world.body.motion.as_ref().unwrap().calibration().is_some());
        // ... and an answer under the drop's bound is taken at the bound,
        // the heaviest the player can carry: slower, never stopped.
        let underweight = report_at(motion.revision, 5);
        let outcome = testing::run(|out| motion.handle(&underweight, &mut world, out));
        outcome.result.unwrap();
        assert!(!withdrew(&outcome.events));
        assert!(world.body.motion.as_ref().unwrap().calibration().is_some());
        assert_eq!(motion.inputs.map(|inputs| inputs.weight), Some(150));
        assert!(motion.unproven.is_some());
        assert!(matches!(
            acceptances(&outcome.events)[..],
            [SpeedInputAcceptance { granted: true, .. }]
        ));
        assert!(world.inventory.ground_drop_pending());
        let none: [SpeedInputAcceptance; 0] = [];

        let bounded = report_at(motion.revision, 150);
        let outcome = testing::run(|out| motion.handle(&bounded, &mut world, out));
        outcome.result.unwrap();
        assert!(motion.unproven.is_none());
        assert!(world.body.motion.as_ref().unwrap().calibration().is_some());
        assert!(world.inventory.ground_drop_pending());

        inventory.apply(InventoryUpdate::Remove(InventorySlot::CURSOR));
        assert_eq!(weight(&inventory), Err(WeightUnavailable::Inventory));
        world.inventory = inventory.clone().into();
        let outcome = testing::run(|out| motion.tick(Instant::now(), &mut world, out));
        outcome.result.unwrap();
        // No bound to check: the report is taken as a bound, the weight no
        // lower than in force.
        let stale = report_at(motion.revision, 150);
        let outcome = testing::run(|out| motion.handle(&stale, &mut world, out));
        outcome.result.unwrap();
        assert!(!withdrew(&outcome.events));
        assert_eq!(motion.inputs.map(|inputs| inputs.weight), Some(150));
        assert!(motion.unproven.is_some());

        inventory.apply(InventoryUpdate::Snapshot(vec![heavier]));
        assert_eq!(weight(&inventory), Ok(1_500));
        world.inventory = inventory.into();
        let outcome = testing::run(|out| motion.handle(&underweight, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(acceptances(&outcome.events), none);
        let recovered = report_at(motion.revision, 150);
        let outcome = testing::run(|out| motion.handle(&recovered, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            acceptances(&outcome.events)[..],
            [SpeedInputAcceptance {
                granted: true,
                inputs: SpeedInputs { weight: 150, .. },
                ..
            }]
        ));
        assert!(!world.inventory.ground_drop_pending());
    }

    /// While a ground drop is pending and no weight bound can be worked out,
    /// a report over a grant in force is still taken as a bound: a lighter
    /// weight changes nothing, and a root in it withdraws.
    #[test]
    fn a_pending_drop_without_a_bound_never_masks_a_root() {
        use eq_network_game::inventory::{
            Inventory, InventorySlot, InventoryUpdate, WeightUnavailable,
        };
        use std::num::NonZeroU32;

        let mut held = testing::item(InventorySlot::CURSOR.0);
        held.details.weight_tenths = 50;
        held.rules.ground_drop_serial = NonZeroU32::new(17);
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![held.clone()]));
        let (mut world, mut motion) = motion_with_known_inventory(inventory.clone());
        let report_at = |revision, report_id, rate, weight| ClientCommand::SpeedInputs {
            session_id: 5,
            revision,
            report_id,
            state: SpeedInputState::Ready(SpeedInputs {
                eqmac_fatigue: None,
                rate,
                weight,
                capacity: 200,
            }),
        };
        let first = report_at(motion.revision, 1, 0, 5);
        testing::run(|out| motion.handle(&first, &mut world, out))
            .result
            .unwrap();
        let full = world.body.motion.as_ref().unwrap().calibration().is_some();
        assert!(full);

        inventory.apply(InventoryUpdate::GroundDropPending(
            held.rules.ground_drop_serial.unwrap(),
        ));
        inventory.apply(InventoryUpdate::Remove(InventorySlot::CURSOR));
        assert_eq!(
            inventory.carried_weight_tenths(eq_network_game::GameDialect::Titanium, 0),
            Err(WeightUnavailable::Inventory)
        );
        world.inventory = inventory.into();
        assert!(world.inventory.ground_drop_pending());
        let outcome = testing::run(|out| motion.tick(Instant::now(), &mut world, out));
        outcome.result.unwrap();
        assert!(!withdrew(&outcome.events));

        let lighter = report_at(motion.revision, 2, 0, 3);
        let outcome = testing::run(|out| motion.handle(&lighter, &mut world, out));
        outcome.result.unwrap();
        assert!(!withdrew(&outcome.events));
        assert!(world.body.motion.as_ref().unwrap().calibration().is_some());
        assert_eq!(motion.inputs.map(|inputs| inputs.weight), Some(5));

        let rooted = report_at(motion.revision, 3, -1000, 3);
        let outcome = testing::run(|out| motion.handle(&rooted, &mut world, out));
        outcome.result.unwrap();
        assert!(!withdrew(&outcome.events));
        assert!(was_rooted(&outcome.events));
    }

    /// A same-zone relocation or a refused transfer's rewind corrects the
    /// player without asking for the inputs again, after telling the host its
    /// movement was withdrawn. The grant that follows the fresh calibration
    /// carries its receipt again, so the host's wait for one ends; a tick
    /// that changes nothing stays quiet.
    #[test]
    fn a_fresh_calibration_after_a_correction_reissues_the_receipt() {
        let (mut world, mut motion) = eqmac_admitted();
        let receipts = |events: &[ClientEvent]| -> Vec<(bool, bool)> {
            events
                .iter()
                .filter_map(|event| match event {
                    ClientEvent::World(WorldEvent::MotionState {
                        units_per_second, ..
                    }) => Some((false, units_per_second.is_some())),
                    ClientEvent::World(WorldEvent::SpeedInputsAccepted(accepted)) => {
                        Some((true, accepted.granted))
                    }
                    _ => None,
                })
                .collect()
        };
        let outcome = testing::run_on(EQMAC, |out| motion.handle(&report(0, 50), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(receipts(&outcome.events), [(false, true), (true, true)]);
        let outcome = testing::run_on(EQMAC, |out| motion.tick(Instant::now(), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(receipts(&outcome.events), []);
        world.correct_own(at(1.0), Instant::now()).unwrap();
        assert!(world.body.motion.as_ref().unwrap().calibration().is_none());
        let outcome = testing::run_on(EQMAC, |out| motion.tick(Instant::now(), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(receipts(&outcome.events), [(false, true), (true, true)]);
        assert_eq!(motion.revision, 1);
    }

    /// A spell landing on the player asks for the speed inputs again by the
    /// server type's landing flag, not a fixed one: 4 where the server type
    /// says 4, nothing where it is unchecked.
    #[test]
    fn a_spell_landing_asks_for_inputs_again_by_the_server_types_flag() {
        use eq_network_game::buffs::SpellEffect;
        let landing = |effect_flag| {
            Message::Event(WorldEvent::SpellEffect(SpellEffect {
                target_id: 7,
                caster_id: 7,
                caster_level: 50,
                instrument_modifier: 10,
                spell_id: 278,
                spell_level: 50,
                effect_flag,
                push: None,
            }))
        };
        let granted_again = |motion: &mut Motion, world: &mut World| {
            testing::run_on(EQMAC, |out| motion.handle(&report(0, 50), world, out))
                .result
                .unwrap();
            assert!(world.body.motion.as_ref().unwrap().calibration().is_some());
        };
        let (mut world, mut motion) = eqmac_admitted();
        motion.spell_landing = Some(4);
        granted_again(&mut motion, &mut world);
        let revision = motion.revision;
        // Another flag is not a landing.
        testing::run_on(EQMAC, |out| motion.observe(&landing(0), &mut world, out))
            .result
            .unwrap();
        assert_eq!(motion.revision, revision);
        assert!(world.body.motion.as_ref().unwrap().calibration().is_some());
        // A landing asks again, and the grant goes on meanwhile.
        let output = testing::run_on(EQMAC, |out| motion.observe(&landing(4), &mut world, out));
        output.result.unwrap();
        assert_eq!(speeds(&output.events), Vec::<Option<f32>>::new());
        assert_eq!(motion.revision, revision + 1);
        assert!(world.body.motion.as_ref().unwrap().calibration().is_some());
        assert!(world.take_news().iter().any(|message| matches!(
            message,
            Message::Event(WorldEvent::SpeedInputsRequired { revision: asked, .. })
                if *asked == revision + 1
        )));
        // Unchecked for the server type: no action is a landing.
        let (mut world, mut motion) = eqmac_admitted();
        granted_again(&mut motion, &mut world);
        let revision = motion.revision;
        testing::run_on(EQMAC, |out| motion.observe(&landing(4), &mut world, out))
            .result
            .unwrap();
        assert_eq!(motion.revision, revision);
        assert!(world.body.motion.as_ref().unwrap().calibration().is_some());
    }
}
