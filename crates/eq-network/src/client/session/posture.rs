//! The player's own stance. Titanium servers never echo it (`EQEmu`'s
//! `Mob::SetAppearance` skips the sender), so the session keeps the stance it
//! last sent or was told, reports it to the client, and stands the character up
//! before it moves, as the official client does; a server leaves a seated
//! character seated while it walks.
use super::{feature::Out, ClientEvent, Events};
use anyhow::Result;
use eq_network_game::{
    command::Posture,
    world::{PostureState, WorldEvent},
};

/// The player's stance in this zone admission; unknown counts as standing.
#[derive(Debug, Default)]
pub(super) struct OwnPosture(Option<PostureState>);

impl OwnPosture {
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
        out.send(&eq_network_game::command::titanium_posture(
            spawn_id, posture,
        )?)?;
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
    use super::*;

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
