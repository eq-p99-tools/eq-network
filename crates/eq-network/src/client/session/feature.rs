//! What the zone session's features share: the interface each one implements
//! and the state they all read.
//!
//! A feature owns one part of the game (doors, items on the ground, ...): it
//! records what the server says about it and takes the host's commands for
//! it. The zone loop only fans packets and commands out to the features.

use super::{ClientCommand, Events, Session};
use anyhow::Result;
use eq_network_game::{
    inventory::Inventory,
    movement::MotionSession,
    world::{PlayerState, Position, WorldEvent},
};

/// Where a feature sends packets and what it tells the host.
pub(super) struct Out<'a, 'e> {
    /// The zone connection.
    pub(super) session: &'a mut Session,
    /// The host's events and diagnostics.
    pub(super) log: &'a mut Events<'e>,
}

/// What the zone's features share.
pub(super) struct World {
    /// This admission's session, which host commands must name.
    pub(super) session_id: u64,
    /// The admitted player, once the zone is ready.
    pub(super) player: Option<PlayerState>,
    /// The player's movement, once admitted.
    pub(super) motion: Option<MotionSession>,
    /// The player's inventory.
    pub(super) inventory: Inventory,
}

impl World {
    /// The admitted player's spawn ID and where they are now.
    pub(super) fn player_at(&self) -> Option<(u16, Position)> {
        let player = self.player.as_ref()?;
        let position = self
            .motion
            .as_ref()
            .map_or(player.position, MotionSession::position);
        Some((player.spawn_id, position))
    }
}

/// One part of the game in the zone session. Each step defaults to doing
/// nothing, so a feature implements only the steps it takes part in.
pub(super) trait Feature {
    /// Records a server event that arrives before the zone is ready.
    fn admit(&mut self, _event: &WorldEvent) {}

    /// What the feature reports to the host when the zone becomes ready.
    fn admission(&self) -> Option<WorldEvent> {
        None
    }

    /// Takes a host command; true when this feature handled it.
    ///
    /// # Errors
    /// Returns an error when the connection fails.
    fn handle(
        &mut self,
        _command: &ClientCommand,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<bool> {
        Ok(false)
    }

    /// Records a server event once the zone is ready.
    ///
    /// # Errors
    /// Returns an error when the connection fails.
    fn observe(
        &mut self,
        _event: &WorldEvent,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        Ok(())
    }
}
