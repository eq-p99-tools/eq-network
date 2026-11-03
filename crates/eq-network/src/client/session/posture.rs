//! The player's own stance. Titanium servers never echo it (`EQEmu`'s
//! `Mob::SetAppearance` skips the sender), so the session keeps the stance it
//! last sent or was told, reports it to the client, and stands the character up
//! before it moves, as the official client does; a server leaves a seated
//! character seated while it walks.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent, Events,
};
use anyhow::Result;
use eq_network_game::{
    command::Posture,
    message::Message,
    request::Request,
    world::{PostureState, WorldEvent},
};

/// Explicit stance commands, independent of locomotion. Camping clients can
/// sit and stand without being granted walking, falling or jumping.
pub(super) struct Stances;

impl Feature for Stances {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Posture]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::SetPosture { .. })
    }

    fn handle(
        &mut self,
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
        world.posture.set(spawn_id, posture, out)
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::Event(WorldEvent::Posture { spawn_id, posture }) = message {
            if world.is_player(*spawn_id) {
                world.posture.observed(*posture);
            }
        }
        Ok(())
    }
}

/// The player's stance in this zone admission; unknown counts as standing.
#[derive(Debug, Default)]
pub(super) struct OwnPosture(Option<PostureState>);

impl OwnPosture {
    /// Reports the requested lying posture, without asserting feign success.
    /// `EQMac` accepts the stance but excludes the sender from its broadcast.
    /// The `EQMac` program's posture setter (0x50be3c, from the feign-death
    /// case of its skill dispatcher 0x4bdf2f) returns at once when the
    /// player already lies, so nothing is sent then.
    pub(super) fn lie_for_feign(&mut self, spawn_id: u16, out: &mut Out<'_, '_>) -> Result<()> {
        if self.0 == Some(PostureState::Lying) {
            return Ok(());
        }
        out.request(&Request::FeignDeathPosture)?;
        self.0 = Some(PostureState::Lying);
        out.log.send(ClientEvent::World(WorldEvent::Posture {
            spawn_id,
            posture: PostureState::Lying,
        }))
    }

    /// The last requested or server-reported stance.
    pub(super) fn current(&self) -> Option<PostureState> {
        self.0
    }
    /// Notes a stance the server reported for the player.
    pub(super) fn observed(&mut self, posture: PostureState) {
        self.0 = Some(posture);
    }

    /// Sits, stands or crouches the player: sends the stance, notes it and
    /// reports it, since the server will not. Every feature that changes the
    /// player's stance does it here.
    ///
    /// # Errors
    /// Returns an error when the connection or the host's event handler fails.
    pub(super) fn set(
        &mut self,
        spawn_id: u16,
        posture: Posture,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        out.request(&Request::Posture(posture))?;
        self.sent(spawn_id, posture, out.log)
    }

    /// Notes a stance this session sent, and reports it since the server will not.
    fn sent(&mut self, spawn_id: u16, posture: Posture, log: &mut Events<'_>) -> Result<()> {
        let posture = match posture {
            Posture::Standing => PostureState::Standing,
            Posture::Sitting => PostureState::Sitting,
            Posture::Ducking => PostureState::Ducking,
        };
        self.0 = Some(posture);
        log.send(ClientEvent::World(WorldEvent::Posture {
            spawn_id,
            posture,
        }))
    }

    /// Whether the character must stand up before it can move.
    fn grounded(&self) -> bool {
        matches!(
            self.0,
            Some(PostureState::Sitting | PostureState::Ducking | PostureState::Lying)
        )
    }

    /// Stands a seated, crouched or prone character up before it moves.
    pub(super) fn stand_to_move(&mut self, spawn_id: u16, out: &mut Out<'_, '_>) -> Result<()> {
        if !self.grounded() {
            return Ok(());
        }
        self.set(spawn_id, Posture::Standing, out)
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::command;
    use std::time::Instant;
    #[test]
    fn the_player_stance_goes_out_and_is_reported_since_the_server_never_echoes_it() {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        let mut stances = Stances;
        let sit = |spawn_id| ClientCommand::SetPosture {
            session_id: 5,
            spawn_id,
            posture: command::Posture::Sitting,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| stances.handle(&sit(7), &mut world, out));
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
        let outcome = testing::run(|out| stances.handle(&sit(8), &mut world, out));
        outcome.result.unwrap();
        assert!(
            outcome.sent.is_empty(),
            "expected nothing sent, got {:?}",
            outcome.sent
        );
    }

    /// The lying appearance goes out before a feign unless the player
    /// already lies, when the program's posture setter sends nothing.
    #[test]
    fn the_feign_posture_goes_out_once_until_the_player_stands_again() {
        const EQMAC: &dyn super::super::wire::Wire = &super::super::wire::EqMac;
        let mut posture = OwnPosture::default();
        let outcome = testing::run_on(EQMAC, |out| posture.lie_for_feign(7, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [eq_network_game::abilities::eqmac::feign_posture(7).unwrap()]
        );
        assert_eq!(posture.current(), Some(PostureState::Lying));
        let outcome = testing::run_on(EQMAC, |out| posture.lie_for_feign(7, out));
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty() && outcome.events.is_empty());
        posture.observed(PostureState::Standing);
        let outcome = testing::run_on(EQMAC, |out| posture.lie_for_feign(7, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
    }

    #[test]
    fn only_seated_crouched_or_prone_characters_stand_to_move() {
        let mut posture = OwnPosture::default();
        assert!(!posture.grounded());
        for (state, grounded) in [
            (PostureState::Sitting, true),
            (PostureState::Ducking, true),
            (PostureState::Lying, true),
            (PostureState::Standing, false),
            (PostureState::Looting, false),
            (PostureState::Frozen, false),
        ] {
            posture.observed(state);
            assert_eq!(posture.grounded(), grounded, "{state:?}");
        }
    }
}
