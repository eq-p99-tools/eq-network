//! What the zone session's features share: the interface each one implements
//! and the state they all read.
//!
//! A feature owns one part of the game (doors, items on the ground, ...): it
//! records what the server says about it and takes the host's commands for
//! it. The zone loop only fans packets and commands out to the features.

use super::{
    lifecycle::ZoneLifecycle, posture::OwnPosture, spellbook::PendingBookAction, ClientCommand,
    Events, Session, ZoneExit,
};
use anyhow::Result;
use eq_network_game::{
    inventory::Inventory,
    movement::MotionSession,
    world::{PlayerState, Position, WorldEvent},
};
use std::time::Instant;

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
    /// The player's spawn ID, from the zone's first spawn record for them.
    pub(super) own_spawn: Option<u16>,
    /// The player's posture as last sent or reported.
    pub(super) posture: OwnPosture,
    /// Set by a feature that ends the zone session.
    pub(super) exit: Option<ZoneExit>,
    /// Death and zone transfers, which hold the player's actions.
    pub(super) lifecycle: ZoneLifecycle,
    /// The zone's ID and instance, from the player's profile.
    pub(super) zone: (u16, u16),
    /// The player's own position packet, kept current for the stationary
    /// heartbeat: spawn ID, sequence, coordinates and heading.
    pub(super) stationary: [u8; 36],
    /// The heartbeat's next sequence number.
    pub(super) sequence: u16,
    /// When the player's position was last sent.
    pub(super) last_position: Instant,
    /// A spellbook action waiting for the player to sit, which moving,
    /// casting, dying or zoning cancels.
    pub(super) book_action: Option<PendingBookAction>,
}

impl World {
    /// The shared state of a new admission.
    pub(super) fn new(session_id: u64) -> Self {
        Self {
            session_id,
            player: None,
            motion: None,
            inventory: Inventory::default(),
            own_spawn: None,
            posture: OwnPosture::default(),
            exit: None,
            lifecycle: ZoneLifecycle::default(),
            zone: (0, 0),
            stationary: [0; 36],
            sequence: 0,
            // In the past, so the first stationary heartbeat goes out at once.
            last_position: Instant::now()
                .checked_sub(eq_network_game::movement::STATIONARY_HEARTBEAT)
                .unwrap_or_else(Instant::now),
            book_action: None,
        }
    }

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

    /// Takes a packet the world decoder has no event for.
    ///
    /// # Errors
    /// Returns an error when the connection fails.
    fn receive(
        &mut self,
        _opcode: u16,
        _body: &[u8],
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        Ok(())
    }

    /// Runs the feature's timers.
    ///
    /// # Errors
    /// Returns an error when the connection fails.
    fn tick(&mut self, _now: Instant, _world: &mut World, _out: &mut Out<'_, '_>) -> Result<()> {
        Ok(())
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
