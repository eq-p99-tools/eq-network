//! What the zone session's features share: the interface each one implements
//! and the state they all read.
//!
//! A feature owns one part of the game (doors, items on the ground, ...): it
//! records what the server says about it and takes the host's commands for
//! it. The zone loop only fans packets and commands out to the features.

use super::{
    actions::Resource,
    character::PlayerRecord,
    control::OwnControl,
    entities::Spawns,
    exchange::Exchanging,
    inventory::{Carried, Ledger},
    lifecycle::ZoneLifecycle,
    motion::Body,
    objects::ZoneObjects,
    posture::OwnPosture,
    tradeskills::OpenContainer,
    ClientCommand, ClientConfig, ConnectionState, Events, ServerProtocol, ZoneExit,
};
use anyhow::{Context, Result};
use eq_network_game::GameDialect;
use eq_network_game::{
    command::EncodedCommand,
    message::Message,
    request::{Request, Sender},
    world::{PlayerState, Position},
};
use eq_network_transport::Transport;
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

    /// A peer termination whose completed application prefix may remain.
    fn termination(&self) -> Option<eq_network_transport::modern::SessionTermination> {
        None
    }
}

/// Any transport carries a feature's packets.
impl<T: Transport + ?Sized> Sink for T {
    fn termination(&self) -> Option<eq_network_transport::modern::SessionTermination> {
        Transport::termination(self)
    }
    fn send(&mut self, packet: &EncodedCommand) -> Result<()> {
        Transport::send(self, packet.opcode, &packet.body)
    }

    fn send_unreliable(&mut self, packet: &EncodedCommand) -> Result<()> {
        Transport::send_unreliable(self, packet.opcode, &packet.body)
    }

    fn last_received_seconds(&self) -> u64 {
        Transport::last_received_seconds(self)
    }
}

/// Where a feature sends packets and what it tells the host.
pub(super) struct Out<'a, 'e> {
    /// Where packets go.
    pub(super) sink: &'a mut dyn Sink,
    /// The host's events and diagnostics.
    pub(super) log: &'a mut Events<'e>,
    /// The server's client generation, which turns requests into packets.
    pub(super) wire: &'static dyn super::wire::Wire,
    /// Who the session speaks for.
    pub(super) sender: Sender<'a>,
}

impl Out<'_, '_> {
    /// The packet for a request, in the server's client generation.
    ///
    /// # Errors
    /// Refuses a request the generation cannot carry.
    pub(super) fn encode(&self, request: &Request) -> Result<EncodedCommand> {
        self.wire.encode(request, self.sender)
    }

    /// Asks the server for something, in the server's client generation.
    ///
    /// # Errors
    /// Returns an error when the generation cannot carry the request or the
    /// connection fails.
    pub(super) fn request(&mut self, request: &Request) -> Result<()> {
        let packet = self.encode(request)?;
        self.send(&packet)?;
        if matches!(request, Request::Position(_)) {
            self.log.movement_packets_sent = self.log.movement_packets_sent.saturating_add(1);
        }
        Ok(())
    }

    /// Asks the server for something that may be lost, such as a position
    /// the next one replaces.
    ///
    /// # Errors
    /// Returns an error when the generation cannot carry the request or the
    /// connection fails.
    pub(super) fn request_unreliable(&mut self, request: &Request) -> Result<()> {
        let packet = self.encode(request)?;
        self.send_unreliable(&packet)?;
        if matches!(request, Request::Position(_)) {
            self.log.movement_packets_sent = self.log.movement_packets_sent.saturating_add(1);
        }
        Ok(())
    }

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

    /// The packet for a host command that needs nothing from the session's
    /// state, in the server's client generation.
    ///
    /// # Errors
    /// Rejects a command the generation cannot represent.
    pub(super) fn encode_command(&self, command: &ClientCommand) -> Result<EncodedCommand> {
        self.encode(&Request::Command(command.clone()))
    }

    /// Sends a host command that needs nothing from the session's state; one
    /// the generation cannot represent is only noted. True when it went out.
    ///
    /// # Errors
    /// Returns an error when the connection or the host's event handler fails.
    pub(super) fn command(&mut self, command: &ClientCommand) -> Result<bool> {
        match self.encode_command(command) {
            Ok(packet) => {
                self.send(&packet)?;
                Ok(true)
            }
            Err(error) => {
                self.log
                    .diagnostic(format!("Rejected invalid outbound client command: {error}"))?;
                Ok(false)
            }
        }
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
#[allow(
    clippy::struct_excessive_bools,
    reason = "Independent facts about one admission, each its own owner's"
)]
pub(in crate::client) struct World {
    /// This admission's session, which host commands must name.
    pub(super) session_id: u64,
    /// Whether this admission is on a network-constrained local `EQEmu` connection.
    local_eqemu: bool,
    /// Installed geometry corroboration for same-zone zone lines, on the
    /// Titanium servers.
    pub(super) pad_boundaries: Option<std::sync::Arc<dyn crate::client::InstalledZoneBoundaries>>,
    /// The `EQMac` program's language minimum levels, as the host read them
    /// from the installed program; None, and on other generations, keeps
    /// `EQMac` language training shut (6.597).
    pub(super) language_levels: Option<eq_network_game::training::LanguageLevels>,
    /// The zone short name received on this admission.
    pub(super) zone_name: String,
    /// The admitted player, once the zone is ready; see [`PlayerRecord`] for
    /// who writes it.
    pub(super) player: PlayerRecord,
    /// How the player's position reaches the server.
    pub(super) body: Body,
    /// This admission's own-player stun owner; independent of lifecycle,
    /// speed prerequisites and unresolved charm/fear ownership.
    pub(super) control: OwnControl,
    /// The player's inventory, which only the inventory feature changes.
    pub(super) inventory: Carried,
    /// The player's coins, which only the inventory feature changes.
    pub(super) coins: Ledger,
    /// The player's spawn ID, from the zone's first spawn record for them.
    pub(super) own_spawn: Option<u16>,
    /// Whether this zone admission made the server's validation exchange
    /// (0x1251). Where it did, the movement history is never reported
    /// (6.369); set once, and never undone within the admission.
    pub(super) validated: bool,
    /// Whether the player is in a group, as the group feature follows it;
    /// only that feature changes it.
    pub(super) grouped: bool,
    /// The player's posture as last sent or reported.
    pub(super) posture: OwnPosture,
    /// Whether the player's melee auto attack is on, as last sent; only the
    /// combat feature changes it, and the player's death turns it off.
    pub(super) attacking: bool,
    /// The world's player-versus-player ruleset, where the world sent one
    /// (`CharacterSession::world_ruleset`): Pick Pockets on a player tests
    /// it for 4 (read R49-31 d, e).
    pub(super) ruleset: Option<u32>,
    /// How the zone session ends, once a feature has decided; see [`World::end`].
    exit: Option<ZoneExit>,
    /// Death and zone transfers, which hold the player's actions.
    pub(super) lifecycle: ZoneLifecycle,
    /// The zone's ID and instance, from the player's profile.
    pub(super) zone: (u16, u16),
    /// When the completed handshake yielded a usable player snapshot.
    pub(super) admitted: Option<Instant>,
    /// The zone's gravity from its header (`OP_NewZone` +420), once it came.
    pub(super) gravity: Option<f32>,
    /// The zone's time type from its header (`OP_NewZone` +424), which the
    /// Titanium program keeps at [0x923b88] as its zone type; 1, the
    /// program's own default (0x443764, 0x4a51ee), until a header comes.
    pub(super) time_type: u8,
    /// Application packets received in this zone session.
    pub(in crate::client) packets: u64,
    /// The zone's spawns, which only the entities feature changes.
    pub(super) spawns: Spawns,
    /// Own Titanium buff observations used to evaluate observer visibility.
    pub(super) sight: super::sight::Sight,
    /// The give or trade window asked for or open, which only the exchange
    /// feature changes.
    pub(super) exchange: Exchanging,
    /// The server's idea of the player's target, which only the targeting
    /// feature changes.
    pub(super) target: super::targeting::Target,
    /// Current own-pet identity, derived from the spawn table by Pets.
    pub(super) own_pet: Option<eq_network_game::pets::PetIdentity>,
    /// The zone's objects, which only the ground objects feature changes.
    pub(super) objects: ZoneObjects,
    /// The zone's vertical doors as Titanium steps them, which only the doors
    /// feature changes.
    pub(super) lifts: eq_network_game::doors::titanium::Lifts,
    /// Canonical received carrier reports, owned by entities.
    pub(super) carriers: eq_network_game::movement::Carriers,
    /// The world container asked for or open, which only the tradeskills
    /// feature changes.
    pub(super) container: OpenContainer,
    /// The raw name of the boat the player rides, as Titanium keeps it:
    /// the profile's at admission, then the boat underfoot each second
    /// (read R36). The riders and transfers features change it.
    pub(super) vehicle: Option<String>,
    /// An arrival aboard that holds the player's movement until the riders
    /// feature settles it.
    pub(super) arrival: Option<super::riders::Pending>,
    /// The zone's safe point from its header, as a position.
    pub(super) safe_point: Option<Position>,
    /// What the riding wire keeps across the character's zones.
    pub(super) riding: std::sync::Arc<super::riders::Riding>,
    /// Where the transfer under way began, for the save before leaving.
    pub(super) carried_from: super::riders::CarriedFrom,
    /// The potion belt as the session knows it, for the casting feature to
    /// use (6.568 B1). Only the bandolier feature writes it; it is `None`
    /// until the profile is read at this admission, or when it could not be.
    pub(super) belt: Option<
        Box<[Option<eq_network_game::bandolier::BeltItem>; eq_network_game::bandolier::BELT_SLOTS]>,
    >,
    /// What the Titanium program's event saves read (`OP_Save`; read
    /// R49-22g, checklist 6.615); see [`super::saves`].
    pub(super) saves: super::saves::Gates,
    /// What the session itself made happen, for every feature and the host
    /// to hear next; see [`World::happened`].
    news: Vec<Message>,
}

impl World {
    /// The shared state of a new admission.
    pub(in crate::client) fn new(session_id: u64) -> Self {
        Self {
            session_id,
            local_eqemu: false,
            pad_boundaries: None,
            language_levels: None,
            zone_name: String::new(),
            player: PlayerRecord::default(),
            body: Body::default(),
            control: OwnControl::default(),
            inventory: Carried::default(),
            coins: Ledger::default(),
            own_spawn: None,
            validated: false,
            grouped: false,
            posture: OwnPosture::default(),
            attacking: false,
            ruleset: None,
            exit: None,
            lifecycle: ZoneLifecycle::default(),
            zone: (0, 0),
            admitted: None,
            gravity: None,
            time_type: 1,
            packets: 0,
            spawns: Spawns::default(),
            lifts: eq_network_game::doors::titanium::Lifts::default(),
            carriers: eq_network_game::movement::Carriers::default(),
            sight: super::sight::Sight::default(),
            exchange: Exchanging::default(),
            target: super::targeting::Target::default(),
            own_pet: None,
            objects: ZoneObjects::default(),
            container: OpenContainer::default(),
            vehicle: None,
            arrival: None,
            safe_point: None,
            riding: std::sync::Arc::default(),
            carried_from: super::riders::CarriedFrom::Elsewhere,
            belt: None,
            saves: super::saves::Gates::default(),
            news: Vec::new(),
        }
    }

    /// The shared state of a production zone and its connection policy.
    pub(super) fn for_zone(session_id: u64, config: &ClientConfig) -> Self {
        Self {
            local_eqemu: config.local_only
                && match config.protocol {
                    ServerProtocol::EqEmu => true,
                    // RoF2's server type offers no local fixtures yet.
                    ServerProtocol::Project1999
                    | ServerProtocol::Quarm
                    | ServerProtocol::Takp
                    | ServerProtocol::EqEmuRof2 => false,
                },
            sight: super::sight::Sight::new(config.sight_catalog.clone().filter(|_| {
                match GameDialect::from(config.protocol) {
                    GameDialect::Titanium => true,
                    // RoF2's vision spells are not read yet.
                    GameDialect::EqMac | GameDialect::Rof2 => false,
                }
            })),
            language_levels: config.installed_language_levels.clone().filter(|_| {
                match GameDialect::from(config.protocol) {
                    GameDialect::EqMac => true,
                    // Only the `EQMac` program's table is read (6.597).
                    GameDialect::Titanium | GameDialect::Rof2 => false,
                }
            }),
            // A same-zone zone line places the player locally on the
            // Titanium servers, as 0x48aaa6 does (read same-zone zone line,
            // 6.640). The Mac programs and RoF2 are unread, and refuse it.
            pad_boundaries: match config.protocol {
                ServerProtocol::Project1999 | ServerProtocol::EqEmu => {
                    config.installed_zone_boundaries.clone()
                }
                ServerProtocol::Quarm | ServerProtocol::Takp | ServerProtocol::EqEmuRof2 => None,
            },
            saves: super::saves::Gates {
                // Stock EQEmu saves on any `OP_Save` (`Handle_OP_Save`).
                // P99's is built, but off until an official-client capture
                // there shows one of these events' saves (6.615 V2). The
                // Mac and RoF2 programs' event saves are unread.
                taken: match config.protocol {
                    ServerProtocol::EqEmu => true,
                    ServerProtocol::Project1999
                    | ServerProtocol::Quarm
                    | ServerProtocol::Takp
                    | ServerProtocol::EqEmuRof2 => false,
                },
                merchant_open: false,
            },
            ..Self::new(session_id)
        }
    }

    /// Whether the player may not move now: a death or transfer, or an
    /// arrival aboard not yet settled.
    pub(super) fn motion_held(&self) -> bool {
        self.lifecycle.blocks_motion() || self.arrival.is_some()
    }

    /// Whether the zone connection permits scoped local `EQEmu` fixture commands.
    pub(super) const fn local_eqemu(&self) -> bool {
        self.local_eqemu
    }

    /// Whether the zone yielded an admitted player.
    pub(super) const fn ready(&self) -> bool {
        self.admitted.is_some()
    }

    /// Whether a spawn is the player.
    pub(super) fn is_player(&self, spawn_id: impl Into<u32>) -> bool {
        let spawn_id = spawn_id.into();
        self.own_spawn.is_some_and(|own| u32::from(own) == spawn_id)
    }

    /// Whether a spawn is the player or one the player can see: what a
    /// target, a spell or an item's effect may be aimed at.
    pub(super) fn visible(&self, spawn_id: u16) -> bool {
        self.is_player(spawn_id) || self.visible_spawn(spawn_id).is_some()
    }

    /// The spawn record for an actor the current observer can see.
    pub(super) fn visible_spawn(&self, spawn_id: u16) -> Option<&crate::world::SpawnState> {
        self.spawns.get(spawn_id).filter(|spawn| {
            !spawn.invisible || eq_network_game::vision::can_see(spawn, self.sight.sees_invisible())
        })
    }

    /// Ends the zone session the way a feature decided: the one place the
    /// session's end is written. The first decision stands.
    pub(super) fn end(&mut self, exit: ZoneExit) {
        if self.exit.is_none() {
            self.exit = Some(exit);
        }
    }

    /// Whether a feature has ended the zone session.
    pub(super) const fn ending(&self) -> bool {
        self.exit.is_some()
    }

    /// How the zone session ends, for the loop to carry out once.
    pub(super) fn take_exit(&mut self) -> Option<ZoneExit> {
        self.exit.take()
    }

    /// Something the session itself made happen, which every feature and the
    /// host then hear as they hear the zone's messages: a death the client
    /// reports for the player is their death all the same.
    pub(super) fn happened(&mut self, message: Message) {
        self.news.push(message);
    }

    /// Own death knowledge, separate from packet details used by corpse owners.
    pub(super) fn owns_death(&self, event: &eq_network_game::world::WorldEvent) -> bool {
        use eq_network_game::world::WorldEvent;
        match event {
            WorldEvent::Death(death) => self.is_player(death.spawn_id),
            WorldEvent::DeathBind { session_id } => *session_id == self.session_id,
            _ => false,
        }
    }

    /// What the session made happen since the loop last asked, in order.
    pub(super) fn take_news(&mut self) -> Vec<Message> {
        std::mem::take(&mut self.news)
    }

    /// How the zone session ends, as decided so far.
    #[cfg(test)]
    pub(super) const fn exit(&self) -> Option<&ZoneExit> {
        self.exit.as_ref()
    }

    /// The admitted player's spawn ID and where they are now.
    pub(super) fn player_at(&self) -> Option<(u16, Position)> {
        let player = self.player.as_ref()?;
        let position = self.body.position().unwrap_or(player.position);
        Some((player.spawn_id, position))
    }

    /// The admitted player's size as the program counts it for reach (spawn
    /// +0x23c, read R16e b): their own spawn's size where above 0, which
    /// spawn and appearance updates keep; where that is 0 or not yet heard,
    /// the size `EQEmu` gives their race on entry, inferred for the program's
    /// model default (+0x54); 0 for a race it does not list, so no reach
    /// built on it ever grows past what the server told.
    pub(super) fn player_size(&self) -> Option<f32> {
        let player = self.player.as_ref()?;
        Some(
            self.spawns
                .all()
                .find(|spawn| spawn.spawn_id == player.spawn_id)
                .map(|spawn| spawn.size)
                .filter(|size| *size > 0.0)
                .unwrap_or_else(|| eq_network_game::abilities::player_size(player.race)),
        )
    }

    /// The server offered a transfer: every command waits for its answer, and
    /// the player stops where they are. Death and transfers stop and restart
    /// the player's movement here and nowhere else, so the two stay in step.
    ///
    /// # Errors
    /// Rejects an offer the lifecycle cannot take.
    pub(super) fn transfer_offered(
        &mut self,
        offer: eq_network_game::zoning::ZoneOffer,
        now: Instant,
    ) -> Result<()> {
        self.lifecycle.offer(offer, now)?;
        self.body.suspend();
        Ok(())
    }

    /// The server refused the transfer: the player stays, and moves again
    /// unless they are dead.
    ///
    /// # Errors
    /// Rejects a refusal without a transfer under way.
    pub(super) fn transfer_refused(&mut self, now: Instant) -> Result<()> {
        self.lifecycle.finish(false, now)?;
        self.carried_from = super::riders::CarriedFrom::Elsewhere;
        if !self.lifecycle.is_dead() {
            self.body.resume(now);
        }
        Ok(())
    }

    /// The player died: commands wait for the offer home, and the player
    /// stops.
    pub(super) fn died(&mut self) {
        self.lifecycle.mark_dead();
        self.body.suspend();
    }

    /// Forgets the vehicle. The z the last save carried stays: only the
    /// next save writes it (read R49-22f).
    pub(super) fn forget_vehicle(&mut self) {
        self.vehicle = None;
    }

    /// Puts the admitted player where the server says they are.
    ///
    /// # Errors
    /// Rejects an invalid position, or a correction before admission.
    pub(super) fn correct_own(&mut self, position: Position, now: Instant) -> Result<()> {
        let player = self
            .player
            .corrected()
            .context("correction without admitted player")?;
        self.body.correct(player, position, now)
    }
}

/// One part of the game in the zone session. Each step defaults to doing
/// nothing, so a feature implements only the steps it takes part in.
pub(super) trait Feature {
    /// What this feature lets the player do, for the session's report.
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        Vec::new()
    }

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

    /// Shapes the player the admission reports with what the feature staged
    /// before it.
    fn shape(&mut self, _player: &mut PlayerState) {}

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

    /// Records state changed by the command's owner before the next receive.
    /// Unlike a timer, this neither sends nor changes the shared world.
    fn after_command(&mut self, _world: &World) {}

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

    /// Hears that the zone's connection ended, which a feature may take as
    /// the session's end, as a zone closing on a player who departs.
    fn connection_ended(&mut self, _world: &mut World) {}

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
            forward: 0.0,
            walk_speed: 0.0,
            run_speed: 0.0,
            size: 6.0,
            invisible: false,
            appearance: eq_network_game::appearance::Appearance::default(),
            level: 0,
            listing: eq_network_game::listing::Listing::default(),
            name_parts: eq_network_game::names::NameParts::default(),
            body_type: None,
            pet_owner: None,
            hp_percent: None,
            posture: None,
        }
    }

    /// A plain item in this inventory slot.
    pub(in crate::client::session) fn item(slot: i32) -> eq_network_game::inventory::InventoryItem {
        eq_network_game::inventory::InventoryItem {
            activation: eq_network_game::inventory::ItemActivation::default(),
            scroll_spell: None,
            book: None,
            rules: eq_network_game::inventory::ItemPlacement::default(),
            slot: eq_network_game::inventory::InventorySlot(slot),
            stack_count: None,
            charges: 0,
            bag_slots: 0,
            augments: Default::default(),
            details: eq_network_game::items::ItemDetails {
                base_held_model: None,
                equipment: None,
                bonuses: None,
                id: 42,
                name: "Synthetic item".into(),
                lore: String::new(),
                weight_tenths: 0,
                slots: 0,
                classes: 0,
                races: 0,
                flags: vec![],
                stats: vec![],
                price: None,
                icon: None,
            },
        }
    }

    /// An admitted player with this spawn ID, standing at the origin.
    pub(in crate::client::session) fn player(spawn_id: u16) -> eq_network_game::world::PlayerState {
        let mut spawn = vec![0; 385];
        spawn[340..344].copy_from_slice(&u32::from(spawn_id).to_le_bytes());
        // Standing in the record's posture byte, as a server sends it.
        spawn[0x57] = 100;
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

    /// Runs one step of a feature with an `Out` that records, on the
    /// Titanium wire.
    pub(in crate::client::session) fn run<R>(
        step: impl FnOnce(&mut Out<'_, '_>) -> R,
    ) -> Outcome<R> {
        run_on(&super::super::wire::Titanium, step)
    }

    /// Runs one step of a feature with an `Out` that records, on `wire`.
    pub(in crate::client::session) fn run_on<R>(
        wire: &'static dyn super::super::wire::Wire,
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
            wire,
            // Every test admits the player as spawn 7 named Tester.
            sender: eq_network_game::request::Sender {
                name: "Tester",
                spawn_id: Some(7),
            },
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
#[cfg(test)]
mod movement_packet_count_tests {
    use super::*;
    use crate::client::ConnectionState;
    use eq_network_game::movement::PositionPacket;

    #[test]
    fn only_successful_position_submissions_increment_the_status_counter() {
        let output = testing::run(|out| {
            let position = Request::Position(PositionPacket {
                spawn_id: 7,
                sequence: 0,
                position: eq_network_game::world::Position::default(),
                delta: [0.; 3],
                animation: 0,
                delta_heading: 0,
            });
            out.encode(&position).unwrap();
            assert_eq!(out.log.movement_packets_sent, 0);
            out.request_unreliable(&position).unwrap();
            out.request(&position).unwrap();
            out.request(&Request::Logout).unwrap();
            assert_eq!(out.log.movement_packets_sent, 2);
            out.log.status(ConnectionState::Connected, 0, None).unwrap();
            out.log.reset();
            assert_eq!(out.log.movement_packets_sent, 0);
        });
        assert_eq!(output.sent.len(), 2);
        assert_eq!(output.unreliable.len(), 1);
        assert!(output.events.iter().any(|event| matches!(event,
            crate::client::ClientEvent::Status(status) if status.movement_packets_sent == Some(2)
        )));
        assert_eq!(
            crate::client::SessionStatus::new(ConnectionState::Connected, "display")
                .movement_packets_sent,
            None
        );
    }

    struct Broken;
    impl Sink for Broken {
        fn send(&mut self, _: &EncodedCommand) -> Result<()> {
            anyhow::bail!("synthetic send failure")
        }
        fn send_unreliable(&mut self, _: &EncodedCommand) -> Result<()> {
            anyhow::bail!("synthetic send failure")
        }
        fn last_received_seconds(&self) -> u64 {
            0
        }
    }

    #[test]
    fn failed_position_submissions_are_not_counted() {
        testing::run(|out| {
            let mut broken = Broken;
            let mut failing = Out {
                sink: &mut broken,
                log: &mut *out.log,
                wire: out.wire,
                sender: out.sender,
            };
            let position = Request::Position(PositionPacket {
                spawn_id: 7,
                sequence: 0,
                position: eq_network_game::world::Position::default(),
                delta: [0.; 3],
                animation: 0,
                delta_heading: 0,
            });
            assert!(failing.request(&position).is_err());
            assert!(failing.request_unreliable(&position).is_err());
            assert_eq!(failing.log.movement_packets_sent, 0);
        });
    }
}
