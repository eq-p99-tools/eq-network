//! The player's movement: where the player is and how that reaches the server,
//! the host's moves, jumps, calibrations and changes of stance, and the
//! server's corrections.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    appearance::AppearanceChange,
    message::Message,
    movement::{
        DiagonalRates, MotionCalibration, MotionSession, MovementRequest, PositionPacket,
        SpeedInputAcceptance, SpeedInputState, SpeedInputs, SpeedState, STATIONARY_HEARTBEAT,
    },
    request::Request,
    world::{PlayerState, Position, WorldEvent},
};
use std::time::Instant;

/// How the player's position reaches the server: the movement session once the
/// zone admits the player, or else a stationary heartbeat from where the
/// profile left them.
pub(super) struct Body {
    /// The player's movement, once admitted.
    motion: Option<MotionSession>,
    /// What the stationary heartbeat repeats: the player's spawn ID, its next
    /// sequence number and position.
    stationary: PositionPacket,
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
    }

    /// Notes the player's spawn ID.
    pub(super) fn own(&mut self, spawn_id: u16) {
        self.stationary.spawn_id = spawn_id;
    }

    /// Where the player is now, once they can move.
    pub(super) fn position(&self) -> Option<Position> {
        self.motion.as_ref().map(MotionSession::position)
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
        if now.saturating_duration_since(self.last_sent) < STATIONARY_HEARTBEAT {
            return Ok(());
        }
        out.request_unreliable(&Request::Position(self.stationary))?;
        self.stationary.sequence = self.stationary.sequence.wrapping_add(1);
        self.last_sent = now;
        Ok(())
    }
}

/// The player's movement at these speeds.
fn granted(session_id: u64, speeds: &MotionCalibration, falls: bool) -> WorldEvent {
    WorldEvent::MotionState {
        session_id,
        units_per_second: Some(speeds.units_per_second),
        walk_units_per_second: speeds.walk.map(|value| value.units_per_second),
        strafe_units_per_second: speeds.strafe.map(|value| value.units_per_second),
        backward_units_per_second: speeds.backward.map(|value| value.units_per_second),
        diagonal: speeds.diagonal,
        falls,
    }
}

/// Takes the player's movement away until it is granted again, after a
/// correction, a death or a transfer.
pub(super) fn withdrawn(session_id: u64) -> WorldEvent {
    WorldEvent::MotionState {
        session_id,
        units_per_second: None,
        strafe_units_per_second: None,
        walk_units_per_second: None,
        backward_units_per_second: None,
        diagonal: DiagonalRates::default(),
        falls: false,
    }
}

/// The player's movement and stance.
pub(super) struct Motion {
    /// Whether the server lets the player fall.
    falls: bool,
    /// Whether the player may also jump: the generation's notice of a jump
    /// is known, and the server takes falls.
    jumps: bool,
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
}

impl Motion {
    pub(super) const fn new(falls: bool, jumps: bool, spell_landing: Option<u8>) -> Self {
        Self {
            falls,
            jumps: falls && jumps,
            spell_landing,
            inputs: None,
            input_report_id: None,
            accepted_inputs: None,
            revision: 0,
            fatigue: None,
            belongings: None,
            encumbered: None,
            sneaking: false,
            hit_points: None,
        }
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
        if motion.calibration() == speeds.motion {
            return Ok(speeds.motion.is_some());
        }
        let Some(speeds) = speeds.motion else {
            self.withdraw_input(world, out)?;
            return Ok(false);
        };
        match motion.calibrate(speeds, Instant::now()) {
            Ok(()) => {
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
                )))?;
                Ok(true)
            }
            Err(error) => {
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
        self.input_report_id = match state {
            SpeedInputState::Ready(_) => Some(report_id),
            SpeedInputState::Unavailable(_) => None,
        };
        self.inputs = match state {
            SpeedInputState::Ready(inputs) => Some(inputs),
            SpeedInputState::Unavailable(_) => {
                self.withdraw_input(world, out)?;
                None
            }
        };
        self.grant(world, out)
    }

    /// Withdraws input while retaining bounded passive descent and heartbeats.
    fn withdraw_input(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        self.accepted_inputs = None;
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
        self.require_inputs(world, out)
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
        let Some(motion) = world.body.motion.as_mut() else {
            return out
                .log
                .diagnostic("Rejected movement: the player cannot move yet".into());
        };
        let (to, from) = (request.position, motion.position());
        if let Some(reason) = world.control.motion_refusal(
            request,
            motion.is_passive(request),
            motion.options().walk_while_stunned,
        ) {
            return out.log.send(ClientEvent::World(WorldEvent::MotionSent {
                session_id: world.session_id,
                position: from,
                refused: Some(reason.into()),
            }));
        }
        if let (true, Some(player)) = (
            (!world.control.stunned() || motion.options().walk_while_stunned)
                && (to.x, to.y, to.z) != (from.x, from.y, from.z),
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
        match (self.falls, self.jumps) {
            (true, true) => vec![Capability::Moving, Capability::Falling, Capability::Jumping],
            (true, false) => vec![Capability::Moving, Capability::Falling],
            _ => vec![Capability::Moving],
        }
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
                    .with_gate(out.wire.update_gate())
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
        self.grant(world, out)?;
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
                    return self.require_inputs(world, out);
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
            WorldEvent::Level { .. } | WorldEvent::BuffSnapshot(_) => {
                self.require_inputs(world, out)
            }
            WorldEvent::Buff(update) if world.is_player(update.entity_id) => {
                self.require_inputs(world, out)
            }
            // A spell landing on the player, by the server type's flag.
            WorldEvent::SpellEffect(effect)
                if world.is_player(effect.target_id)
                    && self
                        .spell_landing
                        .is_some_and(|flag| flag == effect.effect_flag) =>
            {
                self.require_inputs(world, out)
            }
            WorldEvent::Position {
                spawn_id, position, ..
            } if world.is_player(*spawn_id) => {
                world.correct_own(*position, Instant::now())?;
                self.require_inputs(world, out)
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
mod tests {
    use super::super::feature::testing;
    use super::*;
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
        world.body.own(7);
        world.body.place(at(1.0));
        let mut motion = Motion::new(false, false, None);
        let now = Instant::now();
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
        assert_eq!(motion.revision, 2);
        assert_eq!(speeds(&outcome.events), [None]);
        assert!(motion.inputs.is_none());
        assert!(world.body.motion.as_ref().unwrap().calibration().is_none());
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
        // A correction takes movement away until a fresh revision is reported.
        let correction = Message::Event(WorldEvent::Position {
            spawn_id: 7,
            position: at(3.0),
            velocity: [0.0; 3],
            forward: 0.0,
        });
        let outcome = testing::run_on(eqmac, |out| motion.observe(&correction, &mut world, out));
        assert_eq!(speeds(&outcome.events), [None]);
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
        let outcome = testing::run_on(eqmac, |out| {
            motion.handle(&fresh_report(0), &mut world, out)
        });
        assert!(close(&speeds(&outcome.events), 14.9));
        // A root takes it away until it wears off.
        let outcome = testing::run_on(eqmac, |out| {
            motion.handle(&fresh_report(-1000), &mut world, out)
        });
        assert_eq!(speeds(&outcome.events), [None]);
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
    #[test]
    fn input_withdrawal_revision_correction_and_coin_recovery() {
        use eq_network_game::{money::Wallet, movement::SpeedInputUnavailable, world::Coins};
        let (mut world, mut motion) = eqmac_admitted();
        testing::run_on(EQMAC, |out| motion.handle(&report(0, 0), &mut world, out))
            .result
            .unwrap();
        assert!(world.body.motion.as_ref().unwrap().calibration().is_some());
        let unavailable = ClientCommand::SpeedInputs {
            session_id: 5,
            revision: 1,
            report_id: 1,
            state: SpeedInputState::Unavailable(SpeedInputUnavailable::Inventory),
        };
        let output = testing::run_on(EQMAC, |out| motion.handle(&unavailable, &mut world, out));
        output.result.unwrap();
        assert!(output.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::MotionState {
                units_per_second: None,
                falls: true,
                ..
            })
        )));
        testing::run_on(EQMAC, |out| motion.tick(Instant::now(), &mut world, out))
            .result
            .unwrap();
        assert!(world.body.motion.as_ref().unwrap().calibration().is_none());
        world.coins = Wallet {
            purse: Some(Coins {
                copper: 400,
                ..Coins::default()
            }),
            ..Wallet::default()
        }
        .into();
        // An old ready report cannot revive the old grant after the coin change.
        testing::run_on(EQMAC, |out| motion.handle(&report(0, 0), &mut world, out))
            .result
            .unwrap();
        assert_eq!(motion.revision, 2);
        assert!(world.body.motion.as_ref().unwrap().calibration().is_none());
        let current = ClientCommand::SpeedInputs {
            session_id: 5,
            revision: 2,
            report_id: 1,
            state: SpeedInputState::Ready(SpeedInputs {
                eqmac_fatigue: None,
                rate: 0,
                weight: 10,
                capacity: 100,
            }),
        };
        testing::run_on(EQMAC, |out| motion.handle(&current, &mut world, out))
            .result
            .unwrap();
        assert!(world.body.motion.as_ref().unwrap().calibration().is_some());
        let correction = Message::Event(WorldEvent::Position {
            spawn_id: 7,
            position: at(1.0),
            velocity: [0.0; 3],
            forward: 0.0,
        });
        testing::run_on(EQMAC, |out| motion.observe(&correction, &mut world, out))
            .result
            .unwrap();
        testing::run_on(EQMAC, |out| motion.handle(&current, &mut world, out))
            .result
            .unwrap();
        assert!(world.body.motion.as_ref().unwrap().calibration().is_none());
        assert_eq!(motion.revision, 3);
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
        let output = testing::run_on(EQMAC, |out| motion.observe(&landing(4), &mut world, out));
        output.result.unwrap();
        assert_eq!(speeds(&output.events), [None]);
        assert_eq!(motion.revision, revision + 1);
        assert!(world.body.motion.as_ref().unwrap().calibration().is_none());
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
