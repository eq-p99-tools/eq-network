//! What the zone session's features share: the interface each one implements
//! and the state they all read.
//!
//! A feature owns one part of the game (doors, items on the ground, ...): it
//! records what the server says about it and takes the host's commands for
//! it. The zone loop only fans packets and commands out to the features.

use super::{
    lifecycle::ZoneLifecycle, posture::OwnPosture, spellbook::PendingBookAction, ClientCommand,
    ConnectionState, Events, Session, ZoneExit,
};
use anyhow::Result;
use eq_network_game::{
    command::EncodedCommand,
    inventory::Inventory,
    movement::MotionSession,
    world::{PlayerState, Position, WorldEvent},
};
use std::time::Instant;

/// Where a feature's packets go: the zone connection, or a recording in tests.
pub(super) trait Sink {
    /// Sends a packet that must arrive.
    ///
    /// # Errors
    /// Returns an error when the connection fails.
    fn send(&mut self, packet: &EncodedCommand) -> Result<()>;

    /// Seconds since the server last sent anything.
    fn last_received_seconds(&self) -> u64;
}

impl Sink for Session {
    fn send(&mut self, packet: &EncodedCommand) -> Result<()> {
        Session::send(self, packet.opcode, &packet.body)
    }

    fn last_received_seconds(&self) -> u64 {
        Session::last_received_seconds(self)
    }
}

/// Where a feature sends packets and what it tells the host.
pub(super) struct Out<'a, 'e> {
    /// Where packets go.
    pub(super) sink: &'a mut dyn Sink,
    /// The host's events and diagnostics.
    pub(super) log: &'a mut Events<'e>,
}

impl Out<'_, '_> {
    /// Sends a packet that must arrive.
    ///
    /// # Errors
    /// Returns an error when the connection fails.
    pub(super) fn send(&mut self, packet: &EncodedCommand) -> Result<()> {
        self.sink.send(packet)
    }

    /// Tells the host the zone session's state.
    ///
    /// # Errors
    /// Returns an error when the host's event handler fails.
    pub(super) fn status(&mut self, state: ConnectionState, world: &World) -> Result<()> {
        self.log.status(
            state,
            world.packets,
            Some(self.sink.last_received_seconds()),
        )
    }
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
    /// Whether the zone has admitted the player.
    pub(super) ready: bool,
    /// Application packets received in this zone session.
    pub(super) packets: u64,
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
            ready: false,
            packets: 0,
        }
    }

    /// Whether a spawn is the player.
    pub(super) fn is_player(&self, spawn_id: impl Into<u32>) -> bool {
        let spawn_id = spawn_id.into();
        self.own_spawn.is_some_and(|own| u32::from(own) == spawn_id)
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

    /// Takes a packet before the zone decodes it, and says whether the packet
    /// was the feature's alone, so that nothing else looks at it. A feature
    /// that ends the session takes the packet that ended it.
    ///
    /// # Errors
    /// Returns an error when the connection fails or the packet breaks the
    /// feature's rules.
    fn receive(
        &mut self,
        _opcode: u16,
        _body: &[u8],
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<bool> {
        Ok(false)
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

/// Runs features in tests against a recording instead of a connection.
#[cfg(test)]
pub(super) mod testing {
    use super::{EncodedCommand, Events, Out, Result, Sink};
    use crate::client::{ClientConfig, ClientEvent};

    /// A sink that keeps what features send.
    #[derive(Default)]
    pub(in crate::client::session) struct Recorder(
        pub(in crate::client::session) Vec<EncodedCommand>,
    );

    impl Sink for Recorder {
        fn send(&mut self, packet: &EncodedCommand) -> Result<()> {
            self.0.push(packet.clone());
            Ok(())
        }

        fn last_received_seconds(&self) -> u64 {
            0
        }
    }

    /// An admitted player with this spawn ID, standing at the origin.
    pub(in crate::client::session) fn player(spawn_id: u16) -> eq_network_game::world::PlayerState {
        let mut spawn = vec![0; 385];
        spawn[340..344].copy_from_slice(&u32::from(spawn_id).to_le_bytes());
        eq_network_game::world::titanium_player(&vec![0; 19592], &spawn, 256.0)
            .expect("a zeroed profile admits a player")
    }

    /// What a step sent and told the host.
    pub(in crate::client::session) struct Outcome<R> {
        pub(in crate::client::session) result: R,
        pub(in crate::client::session) sent: Vec<EncodedCommand>,
        pub(in crate::client::session) events: Vec<ClientEvent>,
    }

    /// Runs one step of a feature with an `Out` that records.
    pub(in crate::client::session) fn run<R>(
        step: impl FnOnce(&mut Out<'_, '_>) -> R,
    ) -> Outcome<R> {
        let config = ClientConfig::new(
            "EXAMPLE_ACCOUNT",
            "EXAMPLE_PASSWORD",
            "Test Server",
            "ExampleCharacter",
        );
        let mut events = Vec::new();
        let mut handler = |event| {
            events.push(event);
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        let mut sink = Recorder::default();
        let result = step(&mut Out {
            sink: &mut sink,
            log: &mut log,
        });
        drop(log);
        Outcome {
            result,
            sent: sink.0,
            events,
        }
    }
}
