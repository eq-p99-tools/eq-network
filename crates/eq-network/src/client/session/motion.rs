//! The player's movement: where the player is and how that reaches the server,
//! the host's moves, jumps, calibrations and changes of stance, and the
//! server's corrections.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    command,
    message::Message,
    movement::{MotionSession, MovementRequest, PositionPacket, STATIONARY_HEARTBEAT},
    world::{PlayerState, Position, WorldEvent},
    GameDialect,
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
        stationary.encode()?;
        if let Some(motion) = self.motion.as_mut() {
            motion.correct(position, now)?;
        }
        player.position = position;
        self.stationary = stationary;
        Ok(())
    }

    /// Repeats the player's position as the server expects: the movement
    /// session's own updates, or the stationary heartbeat without one.
    fn heartbeat(&mut self, now: Instant, out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(motion) = self.motion.as_mut() {
            motion.tick(now, |packet| out.send_unreliable(packet))?;
            return Ok(());
        }
        if now.saturating_duration_since(self.last_sent) < STATIONARY_HEARTBEAT {
            return Ok(());
        }
        out.send_unreliable(&self.stationary.packet()?)?;
        self.stationary.sequence = self.stationary.sequence.wrapping_add(1);
        self.last_sent = now;
        Ok(())
    }
}

/// Takes the player's movement away until the client configures it again,
/// after a correction, a death or a transfer.
pub(super) fn withdrawn(session_id: u64) -> WorldEvent {
    WorldEvent::MotionState {
        session_id,
        units_per_second: None,
        strafe_units_per_second: None,
        walk_units_per_second: None,
        backward_units_per_second: None,
        falls: false,
    }
}

/// The player's movement and stance.
pub(super) struct Motion {
    dialect: GameDialect,
    character: String,
    /// Whether the server lets the player fall and jump.
    falls: bool,
}

impl Motion {
    pub(super) fn new(dialect: GameDialect, character: &str, falls: bool) -> Self {
        Self {
            dialect,
            character: character.into(),
            falls,
        }
    }

    /// Moves the player, standing them up first if they were sitting or
    /// crouched; a refused move undoes the host's prediction.
    fn step(request: &MovementRequest, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let Some(motion) = world.body.motion.as_mut() else {
            return out
                .log
                .diagnostic("Rejected movement: the player cannot move yet".into());
        };
        let (to, from) = (request.position, motion.position());
        if let (true, Some(player)) = (
            (to.x, to.y, to.z) != (from.x, from.y, from.z),
            &world.player,
        ) {
            world.posture.stand_to_move(player.spawn_id, out)?;
        }
        let mut transport_failed = false;
        let sink = &mut *out.sink;
        let result = motion.send_move(request, Instant::now(), |packet| {
            let sent = sink.send_unreliable(packet);
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
            Ok(()) => out.log.send(ClientEvent::World(WorldEvent::MotionState {
                session_id: world.session_id,
                units_per_second: Some(calibration.units_per_second),
                walk_units_per_second: calibration.walk.map(|value| value.units_per_second),
                strafe_units_per_second: calibration.strafe.map(|value| value.units_per_second),
                backward_units_per_second: calibration.backward.map(|value| value.units_per_second),
                falls: motion.falls(),
            })),
            Err(error) => out
                .log
                .diagnostic(format!("Rejected movement configuration: {error}")),
        }
    }

    /// Jumps where the server allows falls.
    fn jump(world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        let jump = world
            .body
            .motion
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("the player cannot move yet"))
            .and_then(MotionSession::jump);
        match jump {
            // A failed send ends the admission like any other.
            Ok(packet) => out.send(&packet),
            Err(error) => out.log.diagnostic(format!("Rejected jump: {error}")),
        }
    }

    /// Sits, stands or crouches; the server never echoes the player's own
    /// stance, so it is reported here.
    fn stance(
        &self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::SetPosture {
            spawn_id, posture, ..
        } = *command
        else {
            return Ok(());
        };
        if !world.is_player(spawn_id) {
            return out
                .log
                .diagnostic("Rejected a posture for another spawn".into());
        }
        match command::encode(self.dialect, command, &self.character) {
            Ok(packet) => {
                out.send(&packet)?;
                world.posture.sent(spawn_id, posture, out.log)
            }
            Err(error) => out
                .log
                .diagnostic(format!("Rejected invalid outbound client command: {error}")),
        }
    }
}

impl Feature for Motion {
    /// Movement starts from the moment the zone admitted the player, before
    /// the host heard of it, so a calibration the host makes on hearing it
    /// counts as fresh.
    fn admitted(&mut self, world: &mut World, _out: &mut Out<'_, '_>) -> Result<()> {
        if let (Some(player), Some(admitted)) = (&world.player, world.admitted) {
            world.body.motion = Some(
                MotionSession::new(world.session_id, player.spawn_id, player.position, admitted)?
                    .with_falls(self.falls),
            );
        }
        Ok(())
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::Move(_)
                | ClientCommand::Jump { .. }
                | ClientCommand::ConfigureMotion { .. }
                | ClientCommand::SetPosture { .. }
        )
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match command {
            ClientCommand::Move(request) => Self::step(request, world, out),
            ClientCommand::Jump { .. } => Self::jump(world, out),
            ClientCommand::ConfigureMotion { .. } => Self::calibrate(command, world, out),
            _ => self.stance(command, world, out),
        }
    }

    /// Repeats the player's position while they may move.
    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if !world.ready() || world.lifecycle.blocks_motion() {
            return Ok(());
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
            WorldEvent::Position {
                spawn_id, position, ..
            } if world.is_player(*spawn_id) => {
                world.correct_own(*position, Instant::now())?;
                out.log
                    .send(ClientEvent::World(withdrawn(world.session_id)))
            }
            WorldEvent::Posture { spawn_id, posture } if world.is_player(*spawn_id) => {
                world.posture.observed(*posture);
                Ok(())
            }
            WorldEvent::Death(death) if world.is_player(death.spawn_id) => {
                world.body.suspend();
                out.log
                    .send(ClientEvent::World(withdrawn(world.session_id)))
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::world::PostureState;
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
        let mut motion = Motion::new(GameDialect::TitaniumP99, "Tester", false);
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
        assert!(outcome.unreliable.is_empty());
        let outcome = testing::run(|out| motion.tick(now + STATIONARY_HEARTBEAT, &mut world, out));
        assert_eq!(&outcome.unreliable[0].body[2..4], &[1, 0]);
    }

    #[test]
    fn a_calibration_made_on_hearing_of_the_admission_is_fresh() {
        let now = Instant::now();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player = Some(testing::player(7));
        world.admitted = now.checked_sub(Duration::from_millis(10));
        // The host calibrates before the feature has started movement.
        let configure = ClientCommand::ConfigureMotion {
            session_id: 5,
            calibration: eq_network_game::movement::MotionCalibration {
                units_per_second: 6.0,
                velocity_scale: 0.05,
                animation: 12,
                backward: None,
                walk: None,
                strafe: None,
            },
            created: now.checked_sub(Duration::from_millis(5)).unwrap(),
        };
        let mut motion = Motion::new(GameDialect::TitaniumP99, "Tester", false);
        testing::run(|out| motion.admitted(&mut world, out))
            .result
            .unwrap();
        let outcome = testing::run(|out| motion.handle(&configure, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::MotionState {
                units_per_second: Some(_),
                ..
            })]
        ));
    }

    #[test]
    fn the_server_correcting_the_player_withdraws_movement_until_recalibrated() {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player = Some(testing::player(7));
        world.admitted = Some(Instant::now());
        let mut motion = Motion::new(GameDialect::TitaniumP99, "Tester", false);
        testing::run(|out| motion.admitted(&mut world, out))
            .result
            .unwrap();
        let correction = Message::Event(WorldEvent::Position {
            spawn_id: 7,
            position: at(3.0),
            velocity: [0.0; 3],
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
        });
        testing::run(|out| motion.observe(&other, &mut world, out))
            .result
            .unwrap();
        assert_eq!(world.body.position(), Some(at(3.0)));
    }

    #[test]
    fn the_player_stance_goes_out_and_is_reported_since_the_server_never_echoes_it() {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player = Some(testing::player(7));
        let mut motion = Motion::new(GameDialect::TitaniumP99, "Tester", false);
        let sit = |spawn_id| ClientCommand::SetPosture {
            session_id: 5,
            spawn_id,
            posture: command::Posture::Sitting,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| motion.handle(&sit(7), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [command::titanium_posture(7, command::Posture::Sitting).unwrap()]
        );
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::Posture {
                spawn_id: 7,
                posture: PostureState::Sitting
            })]
        ));
        let outcome = testing::run(|out| motion.handle(&sit(8), &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty());
    }

    #[test]
    fn the_player_dying_stops_all_movement() {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player = Some(testing::player(7));
        world.admitted = Some(Instant::now());
        let mut motion = Motion::new(GameDialect::TitaniumP99, "Tester", false);
        testing::run(|out| motion.admitted(&mut world, out))
            .result
            .unwrap();
        let death = Message::Event(WorldEvent::Death(eq_network_game::zoning::Death {
            spawn_id: 7,
            killer_id: 0,
            corpse_id: 9,
            bind_zone_id: 2,
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
        // A suspended session sends no heartbeat.
        let later = Instant::now() + STATIONARY_HEARTBEAT * 2;
        let outcome = testing::run(|out| motion.tick(later, &mut world, out));
        assert!(outcome.unreliable.is_empty());
    }
}
