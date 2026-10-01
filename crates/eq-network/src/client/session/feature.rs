//! What the zone session's features share: the interface each one implements
//! and the state they all read.
//!
//! A feature owns one part of the game (doors, items on the ground, ...): it
//! records what the server says about it and takes the host's commands for
//! it. The zone loop only fans packets and commands out to the features.

use super::{
    actions::Resource, entities::Spawns, lifecycle::ZoneLifecycle, motion::Body,
    posture::OwnPosture, ClientCommand, ConnectionState, Events, Session, ZoneExit,
};
use anyhow::{Context, Result};
use eq_network_game::{
    command::{self, EncodedCommand},
    inventory::Inventory,
    message::Message,
    world::{PlayerState, Position},
    GameDialect,
};
use std::time::Instant;

/// Where a feature's packets go: the zone connection, or a recording in tests.
pub(super) trait Sink {
    /// Sends a packet that must arrive.
    ///
    /// # Errors
    /// Returns an error when the connection fails.
    fn send(&mut self, packet: &EncodedCommand) -> Result<()>;

    /// Sends a packet that may be lost, such as a position the next one
    /// replaces.
    ///
    /// # Errors
    /// Returns an error when the connection fails.
    fn send_unreliable(&mut self, packet: &EncodedCommand) -> Result<()>;

    /// Seconds since the server last sent anything.
    fn last_received_seconds(&self) -> u64;
}

impl Sink for Session {
    fn send(&mut self, packet: &EncodedCommand) -> Result<()> {
        Session::send(self, packet.opcode, &packet.body)
    }

    fn send_unreliable(&mut self, packet: &EncodedCommand) -> Result<()> {
        Session::send_unreliable(self, packet.opcode, &packet.body)
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

    /// Sends a packet that may be lost.
    ///
    /// # Errors
    /// Returns an error when the connection fails.
    pub(super) fn send_unreliable(&mut self, packet: &EncodedCommand) -> Result<()> {
        self.sink.send_unreliable(packet)
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

/// Encodes host commands in the server's dialect, for the commands that need
/// nothing from the session's state.
#[derive(Clone)]
pub(super) struct Encoder {
    dialect: GameDialect,
    /// The player's name, which some packets repeat.
    character: String,
}

impl Encoder {
    pub(super) fn new(dialect: GameDialect, character: &str) -> Self {
        Self {
            dialect,
            character: character.into(),
        }
    }

    /// The packet for a command.
    ///
    /// # Errors
    /// Rejects a command the dialect cannot represent.
    pub(super) fn encode(&self, command: &ClientCommand) -> Result<EncodedCommand> {
        command::encode(self.dialect, command, &self.character)
    }

    /// Sends a command; one the dialect cannot represent is only noted. True
    /// when it went out.
    ///
    /// # Errors
    /// Returns an error when the connection or the host's event handler fails.
    pub(super) fn send(&self, command: &ClientCommand, out: &mut Out<'_, '_>) -> Result<bool> {
        match self.encode(command) {
            Ok(packet) => {
                out.send(&packet)?;
                Ok(true)
            }
            Err(error) => {
                out.log
                    .diagnostic(format!("Rejected invalid outbound client command: {error}"))?;
                Ok(false)
            }
        }
    }
}

/// What the zone's features share.
pub(super) struct World {
    /// This admission's session, which host commands must name.
    pub(super) session_id: u64,
    /// The admitted player, once the zone is ready.
    pub(super) player: Option<PlayerState>,
    /// How the player's position reaches the server.
    pub(super) body: Body,
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
    /// When the zone admitted the player, once it has.
    pub(super) admitted: Option<Instant>,
    /// Application packets received in this zone session.
    pub(super) packets: u64,
    /// The zone's spawns, which only the entities feature changes.
    pub(super) spawns: Spawns,
}

impl World {
    /// The shared state of a new admission.
    pub(super) fn new(session_id: u64) -> Self {
        Self {
            session_id,
            player: None,
            body: Body::default(),
            inventory: Inventory::default(),
            own_spawn: None,
            posture: OwnPosture::default(),
            exit: None,
            lifecycle: ZoneLifecycle::default(),
            zone: (0, 0),
            admitted: None,
            packets: 0,
            spawns: Spawns::default(),
        }
    }

    /// Whether the zone has admitted the player.
    pub(super) const fn ready(&self) -> bool {
        self.admitted.is_some()
    }

    /// Whether a spawn is the player.
    pub(super) fn is_player(&self, spawn_id: impl Into<u32>) -> bool {
        let spawn_id = spawn_id.into();
        self.own_spawn.is_some_and(|own| u32::from(own) == spawn_id)
    }

    /// The admitted player's spawn ID and where they are now.
    pub(super) fn player_at(&self) -> Option<(u16, Position)> {
        let player = self.player.as_ref()?;
        let position = self.body.position().unwrap_or(player.position);
        Some((player.spawn_id, position))
    }

    /// Puts the admitted player where the server says they are.
    ///
    /// # Errors
    /// Rejects an invalid position, or a correction before admission.
    pub(super) fn correct_own(&mut self, position: Position, now: Instant) -> Result<()> {
        let player = self
            .player
            .as_mut()
            .context("correction without admitted player")?;
        self.body.correct(player, position, now)
    }
}

/// One part of the game in the zone session. Each step defaults to doing
/// nothing, so a feature implements only the steps it takes part in.
pub(super) trait Feature {
    /// Explains a message that the feature's own action caused, before any
    /// feature hears it: the cursor scroll a scribe used up arrives as an
    /// item removed.
    fn explain(&mut self, _message: &mut Message, _world: &World) {}

    /// Records a message that arrives before the zone admits the player; the
    /// host hears nothing until the admission.
    ///
    /// # Errors
    /// Returns an error when the message breaks the feature's rules.
    fn admit(&mut self, _message: &Message, _world: &mut World) -> Result<()> {
        Ok(())
    }

    /// Tells the host what the feature staged, once the zone has admitted the
    /// player.
    ///
    /// # Errors
    /// Returns an error when the host's event handler fails.
    fn admitted(&mut self, _world: &mut World, _out: &mut Out<'_, '_>) -> Result<()> {
        Ok(())
    }

    /// What the feature's actions in flight hold, each with the reason a
    /// command that needs it must wait.
    fn holds(&self, _world: &World, _now: Instant) -> Vec<(Resource, &'static str)> {
        Vec::new()
    }

    /// Hears every host command before its owner carries it out, so that a
    /// feature can react to what the player does: moving abandons a camp.
    ///
    /// # Errors
    /// Returns an error when the connection fails.
    fn notice(
        &mut self,
        _command: &ClientCommand,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        Ok(())
    }

    /// Whether this feature carries out a command. One feature owns each kind
    /// of command.
    fn owns(&self, _command: &ClientCommand) -> bool {
        false
    }

    /// Carries out a command this feature owns.
    ///
    /// # Errors
    /// Returns an error when the connection fails.
    fn handle(
        &mut self,
        _command: &ClientCommand,
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

    /// Hears a message once the zone has admitted the player.
    ///
    /// # Errors
    /// Returns an error when the connection fails or the message breaks the
    /// feature's rules.
    fn observe(
        &mut self,
        _message: &Message,
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

    /// A sink that keeps what features send, reliably or not.
    #[derive(Default)]
    struct Recorder {
        sent: Vec<EncodedCommand>,
        unreliable: Vec<EncodedCommand>,
    }

    impl Sink for Recorder {
        fn send(&mut self, packet: &EncodedCommand) -> Result<()> {
            self.sent.push(packet.clone());
            Ok(())
        }

        fn send_unreliable(&mut self, packet: &EncodedCommand) -> Result<()> {
            self.unreliable.push(packet.clone());
            Ok(())
        }

        fn last_received_seconds(&self) -> u64 {
            0
        }
    }

    /// A visible spawn of this kind, standing at the origin.
    pub(in crate::client::session) fn spawn(
        spawn_id: u16,
        kind: eq_network_game::world::SpawnKind,
    ) -> eq_network_game::world::SpawnState {
        eq_network_game::world::SpawnState {
            class: None,
            spawn_id,
            name: format!("Spawn {spawn_id}"),
            kind,
            race: 1,
            gender: 0,
            position: eq_network_game::world::Position::default(),
            velocity: [0.0; 3],
            size: 6.0,
            invisible: false,
            appearance: eq_network_game::appearance::Appearance::default(),
        }
    }

    /// An admitted player with this spawn ID, standing at the origin.
    pub(in crate::client::session) fn player(spawn_id: u16) -> eq_network_game::world::PlayerState {
        let mut spawn = vec![0; 385];
        spawn[340..344].copy_from_slice(&u32::from(spawn_id).to_le_bytes());
        eq_network_game::world::titanium_player(&vec![0; 19592], &spawn, 256.0)
            .expect("a zeroed profile admits a player")
    }

    /// What a step sent and told the host, and when the host heard each event.
    pub(in crate::client::session) struct Outcome<R> {
        pub(in crate::client::session) result: R,
        pub(in crate::client::session) sent: Vec<EncodedCommand>,
        pub(in crate::client::session) unreliable: Vec<EncodedCommand>,
        pub(in crate::client::session) events: Vec<ClientEvent>,
        pub(in crate::client::session) heard: Vec<std::time::Instant>,
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
        let (mut events, mut heard) = (Vec::new(), Vec::new());
        let mut handler = |event| {
            events.push(event);
            heard.push(std::time::Instant::now());
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
            sent: sink.sent,
            unreliable: sink.unreliable,
            events,
            heard,
        }
    }
}
