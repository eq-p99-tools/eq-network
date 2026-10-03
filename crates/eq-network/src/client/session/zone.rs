//! The zone session: admission, the player's commands and the zone's traffic
//! until the character leaves for another zone, the world or the character list.
use super::{
    actions, ensure,
    feature::{Feature, Out, World},
    servers, CharacterSession, ClientCommand, ClientEvent, ConnectionStage, ConnectionState,
    DecodeError, Duration, Events, Instant, RecordEvent, Result, Shield, ZoneExit,
};

use super::admission::{Handshake, Zone};
use eq_network_game::message::Message;
use eq_network_transport::Transport;

/// The zone's features, each offered every command, packet, timer and event.
struct Features(Vec<Box<dyn Feature>>);

impl Features {
    /// The features the server type provides.
    fn new(
        server: &dyn servers::ServerType,
        name: &str,
        auto_eat: eq_network_game::food::AutoEat,
    ) -> Self {
        Self(server.features(&servers::Setup::new(name, auto_eat)))
    }

    /// What the features let the player do, each once.
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        let mut capabilities: Vec<_> = self
            .0
            .iter()
            .flat_map(|feature| feature.capabilities())
            .collect();
        capabilities.sort_unstable();
        capabilities.dedup();
        capabilities
    }

    /// Lets every feature shape the player the admission reports.
    fn shape(&mut self, player: &mut crate::world::PlayerState) {
        for feature in &mut self.0 {
            feature.shape(player);
        }
    }

    /// Lets every feature explain a message its own action caused.
    fn explain(&mut self, message: &mut Message, world: &World) {
        for feature in &mut self.0 {
            feature.explain(message, world);
        }
    }

    /// Lets every feature record a message from before the zone admitted the
    /// player.
    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        for feature in &mut self.0 {
            feature.admit(message, world)?;
        }
        Ok(())
    }

    /// Has every feature tell the host what it staged before admission.
    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        for feature in &mut self.0 {
            feature.admitted(world, out)?;
        }
        Ok(())
    }

    /// What every feature's actions in flight hold.
    fn holds(&self, world: &World, now: Instant) -> Vec<(actions::Resource, &'static str)> {
        self.0
            .iter()
            .flat_map(|feature| feature.holds(world, now))
            .collect()
    }

    /// Lets every feature hear a command, then has its owner carry it out;
    /// true when a feature owns it.
    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<bool> {
        for feature in &mut self.0 {
            feature.notice(command, world, out)?;
        }
        let Some(owner) = self.0.iter_mut().find(|feature| feature.owns(command)) else {
            return Ok(false);
        };
        owner.handle(command, world, out)?;
        Ok(true)
    }

    /// Runs the features' timers until one ends the session.
    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        for feature in &mut self.0 {
            feature.tick(now, world, out)?;
            if world.ending() {
                break;
            }
        }
        Ok(())
    }

    /// Lets every feature hear a message once the zone has admitted the
    /// player, until one ends the session.
    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        for feature in &mut self.0 {
            feature.observe(message, world, out)?;
            if world.ending() {
                break;
            }
        }
        Ok(())
    }
}

/// Enter the zone, keep the character stationary, and collect communications.
// The linear handshake keeps packet ordering and state transitions together.
#[allow(clippy::too_many_lines)]
pub(super) fn run(
    context: &CharacterSession<'_>,
    shield: &mut Option<Box<dyn Shield>>,
    host: &str,
    port: u16,
    mut checksums: Vec<u8>,
    log: &mut Events<'_>,
) -> Result<ZoneExit> {
    let config = &context.config;
    let credentials = context.credentials;
    let stop = context.stop;
    let duration = context.duration;
    // Each generation connects to its zones its own way; the loop needs only
    // the interface every generation's transport offers.
    let server = servers::server_type(config.protocol);
    let mut session = server.wire().connect_zone(
        crate::client::endpoint(host, port, config.local_only)?,
        stop.flag(),
    )?;
    let mut admission = server.wire().admit(
        &config.character,
        server.profile_turn(),
        &mut Handshake {
            session: &mut *session,
            shield,
            key: &credentials.key,
            checksums: &mut checksums,
            log: &mut *log,
        },
    )?;
    let connected = Instant::now();
    let mut progress = Instant::now();
    let session_id = rand::random();
    let mut world = World::new(session_id);
    let mut features = Features::new(server, &config.character, config.auto_eat);
    let follows_zones = features
        .capabilities()
        .contains(&crate::world::Capability::Zoning);
    loop {
        if stop.is_cancelled() || duration.is_some_and(|limit| connected.elapsed() >= limit) {
            session.close()?;
            // The outer run reports Stopped after the session has closed.
            return Ok(ZoneExit::Stopped);
        }
        if progress.elapsed() >= Duration::from_secs(30) {
            log.status(
                if world.ready()
                    && world.lifecycle.pending().is_none()
                    && session.last_received_seconds() < 60
                {
                    ConnectionState::Connected
                } else {
                    ConnectionState::Zoning
                },
                world.packets,
                Some(session.last_received_seconds()),
            )?;
            log.diagnostic(format!(
                "Zone session: {} application packets, {} communication records",
                world.packets, log.messages
            ))?;
            progress = Instant::now();
        }
        ensure!(
            world.ready() || connected.elapsed() < Duration::from_secs(60),
            "zone admission timed out while {}",
            admission.stage()
        );
        let speaker = sender(&config.character, &world);
        features.tick(
            Instant::now(),
            &mut world,
            &mut Out {
                sink: &mut session,
                log: &mut *log,
                wire: server.wire(),
                sender: speaker,
            },
        )?;
        if let Some(exit) = world.take_exit() {
            session.close()?;
            return Ok(exit);
        }
        if world.lifecycle.blocks_motion() {
            if let Some(commands) = context.commands {
                // Death and zone transfers hold every command; each is refused,
                // so that whoever waits on one hears why.
                let reason = if world.lifecycle.is_dead() {
                    "The player is dead"
                } else {
                    "The player is between zones"
                };
                for command in commands.try_iter().take(64) {
                    actions::refuse(&command, reason, log)?;
                }
            }
        }
        if world.ready() && !world.lifecycle.is_dead() && world.lifecycle.pending().is_none() {
            if let Some(commands) = context.commands {
                // Bound each pass so continuous producers cannot starve receive/ACK work.
                for command in commands.try_iter().take(64) {
                    // Stale commands, and commands that need what an action in
                    // flight holds, are refused here and nowhere else.
                    let now = Instant::now();
                    let refusal = actions::stale(&command, session_id, now).or_else(|| {
                        actions::Held::new(features.holds(&world, now)).conflict(&command)
                    });
                    if let Some(reason) = refusal {
                        actions::refuse(&command, reason, log)?;
                        continue;
                    }
                    let speaker = sender(&config.character, &world);
                    let handled = features.handle(
                        &command,
                        &mut world,
                        &mut Out {
                            sink: &mut session,
                            log: &mut *log,
                            wire: server.wire(),
                            sender: speaker,
                        },
                    )?;
                    if let Some(exit) = world.take_exit() {
                        session.close()?;
                        return Ok(exit);
                    }
                    if !handled {
                        unoffered(&command, log)?;
                    }
                    // Commands wait while the player is dead or zoning.
                    if world.lifecycle.is_dead() || world.lifecycle.pending().is_some() {
                        break;
                    }
                }
            }
        }
        let Some(mut packet) = session.receive()? else {
            continue;
        };
        world.packets += 1;
        if let Some(shield) = shield.as_ref() {
            shield.spawns(packet.opcode, &mut packet.body, &credentials.key)?;
        }
        // What the generation's client answers by itself, at any time.
        if let Some(reply) = server.wire().answer(packet.opcode, &packet.body) {
            session.send(reply.opcode, &reply.body)?;
        }
        if !world.ready() {
            log.diagnostic(format!(
                "Zone received 0x{:04x} ({} bytes)",
                packet.opcode,
                packet.body.len()
            ))?;
        }
        if let Some((player, zone)) = admission.read(
            &mut packet,
            &mut world,
            &mut Handshake {
                session: &mut *session,
                shield,
                key: &credentials.key,
                checksums: &mut checksums,
                log: &mut *log,
            },
        )? {
            let speaker = sender(&config.character, &world);
            admit(
                player,
                &zone,
                &mut features,
                &mut world,
                &mut Out {
                    sink: &mut session,
                    log: &mut *log,
                    wire: server.wire(),
                    sender: speaker,
                },
            )?;
        }
        // Everything else is read once, in the server's client generation, the
        // same way before and after admission, and heard by every feature.
        for mut message in server.wire().messages(packet.opcode, &packet.body) {
            features.explain(&mut message, &world);
            if let Message::Unreadable { part, error } = &message {
                log.diagnostic(format!("{part} rejected: {error}"))?;
            }
            if world.ready() {
                let speaker = sender(&config.character, &world);
                features.observe(
                    &message,
                    &mut world,
                    &mut Out {
                        sink: &mut session,
                        log: &mut *log,
                        wire: server.wire(),
                        sender: speaker,
                    },
                )?;
            } else {
                features.admit(&message, &mut world)?;
            }
            if let Some(exit) = world.take_exit() {
                session.close()?;
                return Ok(exit);
            }
            ensure!(
                !matches!(message, Message::LoggedOut),
                "server logged the character out"
            );
            // The server moving the player where no feature follows ends the
            // session.
            ensure!(
                follows_zones || !matches!(message, Message::ZoneOffer(_)),
                "server requested a new zone, which this server type cannot follow yet"
            );
            // Before the admission, the features staged what they need of it.
            if let (Message::Event(event), true) = (message, world.ready()) {
                log.send(ClientEvent::World(event))?;
            }
        }
        let zone = log.zone.clone();
        match server
            .wire()
            .chat(packet.opcode, &packet.body, config.include_raw)
        {
            Ok(Some(event)) => log.record(&zone, RecordEvent::Chat(event))?,
            Ok(None) => (),
            Err(error) => log.record(
                &zone,
                RecordEvent::DecodeError(DecodeError {
                    kind: "decode_error",
                    opcode: packet.opcode,
                    payload_hex: hex::encode(&packet.body),
                    error: error.to_string(),
                }),
            )?,
        }
    }
}

/// Who the session speaks for: the character it logged in as, and their
/// spawn once the zone has admitted them.
fn sender<'a>(name: &'a str, world: &World) -> eq_network_game::request::Sender<'a> {
    eq_network_game::request::Sender {
        name,
        spawn_id: world.player.as_ref().map(|player| player.spawn_id),
    }
}

/// Why a command is refused when no feature of the server type takes it:
/// the words the client greys out such a control with.
const UNOFFERED: &str = "Not available on this server";

/// Refuses a command no feature of the server type takes, through the event
/// its caller waits for, so the player hears why instead of nothing.
fn unoffered(command: &ClientCommand, log: &mut Events<'_>) -> Result<()> {
    actions::refuse(command, UNOFFERED, log)
}

/// Tells the host the zone admitted the player, as the features shape them,
/// and lets every feature tell what it staged.
fn admit(
    player: Result<crate::world::PlayerState>,
    zone: &Zone,
    features: &mut Features,
    world: &mut World,
    out: &mut Out<'_, '_>,
) -> Result<()> {
    match player {
        Ok(mut player) => {
            features.shape(&mut player);
            out.sender.spawn_id = Some(player.spawn_id);
            world.player.admit(player.clone());
            out.log
                .send(ClientEvent::World(crate::world::WorldEvent::Entered {
                    capabilities: features.capabilities(),
                    session_id: world.session_id,
                    zone: zone.name.clone(),
                    player: Box::new(player),
                    far_clip: zone.far_clip,
                }))?;
            if let Some(sky) = zone.sky {
                out.log
                    .send(ClientEvent::World(crate::world::WorldEvent::Sky(sky)))?;
            }
            features.admitted(world, out)?;
        }
        Err(error) => out
            .log
            .diagnostic(format!("Player presentation unavailable: {error}"))?,
    }
    out.log
        .send(ClientEvent::Progress(ConnectionStage::Ready))?;
    out.status(ConnectionState::Connected, world)?;
    out.log.diagnostic(format!(
        "Zone login sequence complete for {}; waiting for ongoing server traffic",
        zone.name
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eq_network_game::{
        chat::OutboundChat,
        inventory::{InventoryMove, InventorySlot, ItemUse, MoveQuantity},
        movement::{MotionCalibration, MovementMode, MovementRequest},
    };

    /// How many kinds of command there are.
    const KINDS: usize = 62;

    /// Which kind of command this is. A new command is a compile error here
    /// until it has a number, and then a test failure until the list below
    /// has one of it, so no command can go without an owner unnoticed.
    fn kind(command: &ClientCommand) -> usize {
        match command {
            // The world server's, before any zone session.
            ClientCommand::SelectCharacter { .. } => 0,
            ClientCommand::CreateCharacter { .. } => 1,
            ClientCommand::SwapSpell { .. } => 2,
            ClientCommand::UseItem(_) => 3,
            ClientCommand::ClickDoor { .. } => 4,
            ClientCommand::PickUp { .. } => 5,
            ClientCommand::CrossZoneLine { .. } => 6,
            ClientCommand::ScribeSpell { .. } => 7,
            ClientCommand::DeleteSpell { .. } => 8,
            ClientCommand::ForgetSpell { .. } => 9,
            ClientCommand::MemorizeSpell { .. } => 10,
            ClientCommand::CastSpell { .. } => 11,
            ClientCommand::SetPosture { .. } => 12,
            ClientCommand::MoveInventory(_) => 13,
            ClientCommand::SendChat(_) => 14,
            ClientCommand::InspectItem { .. } => 15,
            ClientCommand::Consider { .. } => 16,
            ClientCommand::Camp { .. } => 17,
            ClientCommand::Loot { .. } => 18,
            ClientCommand::LootItem { .. } => 19,
            ClientCommand::EndLoot { .. } => 20,
            ClientCommand::Shop { .. } => 21,
            ClientCommand::Buy { .. } => 22,
            ClientCommand::Sell { .. } => 23,
            ClientCommand::Jump { .. } => 24,
            ClientCommand::AutoAttack { .. } => 25,
            ClientCommand::SelectTarget { .. } => 26,
            ClientCommand::ConfigureMotion { .. } => 27,
            ClientCommand::Move(_) => 28,
            ClientCommand::OfferTrade { .. } => 29,
            ClientCommand::AcceptTrade { .. } => 30,
            ClientCommand::CancelTrade { .. } => 31,
            ClientCommand::MoveCoins { .. } => 32,
            ClientCommand::UseAbility { .. } => 33,
            ClientCommand::Consume { .. } => 34,
            ClientCommand::WhoAll { .. } => 35,
            ClientCommand::Consent { .. } => 36,
            ClientCommand::SummonCorpse { .. } => 37,
            ClientCommand::DragCorpse { .. } => 38,
            ClientCommand::DropCorpse { .. } => 39,
            ClientCommand::Pet { .. } => 40,
            ClientCommand::AutoEat { .. } => 41,
            ClientCommand::Training { .. } => 42,
            ClientCommand::AnswerResurrection { .. } => 43,
            ClientCommand::ReadItem { .. } => 44,
            ClientCommand::Combine { .. } => 45,
            ClientCommand::OpenContainer { .. } => 46,
            ClientCommand::CloseContainer { .. } => 47,
            ClientCommand::InviteToGroup { .. } => 48,
            ClientCommand::FollowGroup { .. } => 49,
            ClientCommand::DeclineGroup { .. } => 50,
            ClientCommand::Disband { .. } => 51,
            ClientCommand::ToggleAway { .. } => 52,
            ClientCommand::ToggleAnonymous { .. } => 53,
            ClientCommand::ToggleRoleplay { .. } => 54,
            ClientCommand::Random { .. } => 55,
            ClientCommand::Emote { .. } => 56,
            ClientCommand::Assist { .. } => 57,
            ClientCommand::RaidInvite { .. } => 58,
            ClientCommand::RaidAccept { .. } => 59,
            ClientCommand::RaidDecline { .. } => 60,
            ClientCommand::RaidLeave { .. } => 61,
        }
    }

    /// One of every command a zone session takes.
    #[allow(
        clippy::too_many_lines,
        reason = "one literal for each kind of command"
    )]
    fn zone_commands() -> Vec<ClientCommand> {
        let (session_id, created) = (1, Instant::now());
        let position = crate::world::Position::default();
        vec![
            ClientCommand::SwapSpell {
                session_id,
                from: 0,
                to: 1,
                from_spell: 202,
                to_spell: None,
                created,
            },
            ClientCommand::UseItem(ItemUse {
                request_id: 1,
                session_id,
                revision: 1,
                slot: InventorySlot(22),
                target_id: 7,
                created,
            }),
            ClientCommand::ClickDoor {
                session_id,
                door_id: 1,
                created,
            },
            ClientCommand::PickUp {
                session_id,
                drop_id: 1,
                created,
            },
            ClientCommand::CrossZoneLine {
                session_id,
                destination: eq_network_game::zoning::ZoneLineDestination::Reference(1),
                position,
                created,
            },
            ClientCommand::ScribeSpell {
                session_id,
                revision: 1,
                slot: 0,
                spell_id: 202,
                created,
            },
            ClientCommand::DeleteSpell {
                session_id,
                slot: 0,
                spell_id: 202,
                created,
            },
            ClientCommand::ForgetSpell {
                session_id,
                gem: 0,
                spell_id: 202,
                created,
            },
            ClientCommand::MemorizeSpell {
                session_id,
                gem: 0,
                spell_id: 202,
                created,
            },
            ClientCommand::CastSpell {
                session_id,
                gem: 0,
                spell_id: 202,
                target_id: 7,
                created,
            },
            ClientCommand::SetPosture {
                session_id,
                spawn_id: 7,
                posture: eq_network_game::command::Posture::Sitting,
                created,
            },
            ClientCommand::MoveInventory(InventoryMove {
                session_id,
                revision: 1,
                from: InventorySlot(22),
                to: InventorySlot(30),
                quantity: MoveQuantity::Whole,
                created,
            }),
            ClientCommand::SendChat(OutboundChat::Say("Hail".into())),
            ClientCommand::InspectItem {
                session_id,
                link_body: String::new(),
            },
            ClientCommand::Consider {
                session_id,
                own_id: 7,
                target_id: 8,
                created,
            },
            ClientCommand::Camp {
                session_id,
                created,
            },
            ClientCommand::Loot {
                session_id,
                corpse_id: 8,
                created,
            },
            ClientCommand::LootItem {
                session_id,
                corpse_id: 8,
                own_id: 7,
                place: 0,
                auto: true,
                created,
            },
            ClientCommand::EndLoot {
                session_id,
                corpse_id: 8,
            },
            ClientCommand::Shop {
                session_id,
                merchant_id: 9,
                own_id: 7,
                open: true,
                created,
            },
            ClientCommand::Buy {
                session_id,
                merchant_id: 9,
                own_id: 7,
                slot: 0,
                quantity: 1,
                created,
            },
            ClientCommand::Sell {
                session_id,
                merchant_id: 9,
                slot: 22,
                quantity: 1,
                created,
            },
            ClientCommand::Jump {
                session_id,
                created,
            },
            ClientCommand::AutoAttack {
                session_id,
                enabled: true,
                created,
            },
            ClientCommand::SelectTarget {
                session_id,
                spawn_id: None,
            },
            ClientCommand::ConfigureMotion {
                session_id,
                calibration: MotionCalibration {
                    units_per_second: 6.0,
                    velocity_scale: 0.05,
                    animation: 12,
                    backward: None,
                    walk: None,
                    strafe: None,
                },
                created,
            },
            ClientCommand::Move(MovementRequest {
                mode: MovementMode::Forward,
                session_id,
                position,
                created,
            }),
            ClientCommand::OfferTrade {
                session_id,
                with_id: 9,
                created,
            },
            ClientCommand::AcceptTrade {
                session_id,
                created,
            },
            ClientCommand::CancelTrade { session_id },
            ClientCommand::MoveCoins {
                session_id,
                transfer: eq_network_game::money::CoinTransfer {
                    from: eq_network_game::money::CoinPlace::Purse,
                    to: eq_network_game::money::CoinPlace::Cursor,
                    coin: eq_network_game::money::Coin::Gold,
                    into: eq_network_game::money::Coin::Gold,
                    amount: 2,
                },
                created,
            },
            ClientCommand::UseAbility {
                session_id,
                ability: eq_network_game::abilities::Ability::Kick,
                created,
            },
            ClientCommand::Consume {
                session_id,
                slot: InventorySlot(22),
                created,
            },
            ClientCommand::AutoEat {
                session_id,
                auto_eat: eq_network_game::food::AutoEat::Anything,
            },
            ClientCommand::WhoAll {
                session_id,
                filter: eq_network_game::who::WhoFilter::default(),
            },
            ClientCommand::Consent {
                session_id,
                name: "Helper".into(),
                given: true,
            },
            ClientCommand::SummonCorpse {
                session_id,
                spawn_id: 4,
            },
            ClientCommand::DragCorpse {
                session_id,
                spawn_id: 4,
            },
            ClientCommand::DropCorpse {
                session_id,
                spawn_id: None,
            },
            ClientCommand::Pet {
                session_id,
                command: eq_network_game::pets::PetCommand::Follow,
                target: None,
            },
            ClientCommand::Training {
                session_id,
                request: eq_network_game::training::TrainingRequest::End,
                created,
            },
            ClientCommand::AnswerResurrection {
                session_id,
                accept: true,
            },
            ClientCommand::InviteToGroup {
                session_id,
                name: "Friend".into(),
            },
            ClientCommand::FollowGroup { session_id },
            ClientCommand::DeclineGroup { session_id },
            ClientCommand::Disband { session_id },
            ClientCommand::ToggleAway { session_id },
            ClientCommand::ToggleAnonymous { session_id },
            ClientCommand::ToggleRoleplay { session_id },
            ClientCommand::Random {
                session_id,
                low: 1,
                high: 6,
            },
            ClientCommand::Emote {
                session_id,
                text: "waves.".into(),
            },
            ClientCommand::Assist {
                session_id,
                spawn_id: 300,
            },
            ClientCommand::RaidInvite {
                session_id,
                name: "Friend".into(),
            },
            ClientCommand::RaidAccept { session_id },
            ClientCommand::RaidDecline { session_id },
            ClientCommand::RaidLeave { session_id },
            ClientCommand::ReadItem {
                session_id,
                slot: InventorySlot(23),
            },
            ClientCommand::Combine {
                session_id,
                container: InventorySlot(23),
                created,
            },
            ClientCommand::OpenContainer {
                session_id,
                drop_id: 9,
                created,
            },
            ClientCommand::CloseContainer { session_id },
        ]
    }

    #[test]
    fn the_list_has_one_of_every_command_a_zone_takes() {
        let commands = zone_commands();
        let kinds: std::collections::BTreeSet<_> = commands.iter().map(kind).collect();
        assert_eq!(kinds.len(), commands.len(), "one of each");
        assert_eq!(kinds, (2..KINDS).collect());
    }

    #[test]
    fn exactly_one_feature_owns_each_command_a_zone_takes() {
        // The server type with every feature, so that each owner can offer
        // what its commands need.
        let features = Features::new(
            servers::server_type(crate::client::ServerProtocol::EqEmu),
            "Tester",
            eq_network_game::food::AutoEat::default(),
        );
        for command in zone_commands() {
            let owners: Vec<_> = features
                .0
                .iter()
                .filter(|feature| feature.owns(&command))
                .collect();
            assert_eq!(owners.len(), 1, "{command:?}");
            let needed = command
                .capability()
                .expect("a zone command needs a capability");
            assert!(
                owners[0].capabilities().contains(&needed),
                "{command:?} needs {needed:?}, which its owner does not offer"
            );
        }
        let selection = ClientCommand::SelectCharacter {
            selection_id: 1,
            slot: 0,
        };
        assert!(!features.0.iter().any(|feature| feature.owns(&selection)));
    }

    #[test]
    fn each_zone_hands_the_wire_the_players_new_spawn() {
        use crate::client::session::feature::testing;
        use eq_network_game::{
            command::{titanium_posture, Posture},
            request::Request,
        };
        let zone = super::super::admission::Zone {
            name: "qeynos".into(),
            far_clip: None,
            sky: None,
        };
        // The player zones twice, and each zone gives them a different spawn.
        for spawn_id in [7u16, 9] {
            let mut world = World::new(5);
            let mut features = Features::new(
                servers::server_type(crate::client::ServerProtocol::EqEmu),
                "Tester",
                eq_network_game::food::AutoEat::default(),
            );
            let outcome = testing::run(|out| {
                // Before admission the session speaks for no spawn.
                out.sender = sender("Tester", &world);
                assert_eq!(out.sender.spawn_id, None);
                admit(
                    Ok(testing::player(spawn_id)),
                    &zone,
                    &mut features,
                    &mut world,
                    out,
                )?;
                // The admission turn itself, and every turn built after it,
                // speak for the spawn this zone gave the player.
                assert_eq!(out.sender.spawn_id, Some(spawn_id));
                assert_eq!(sender("Tester", &world).spawn_id, Some(spawn_id));
                out.request(&Request::Posture(Posture::Sitting))
            });
            outcome.result.unwrap();
            assert_eq!(
                outcome.sent.last(),
                Some(&titanium_posture(spawn_id, Posture::Sitting).unwrap())
            );
        }
    }

    #[test]
    fn a_command_no_feature_takes_is_refused_as_unavailable() {
        use crate::client::session::feature::testing;
        // Quarm opens no doors yet.
        let mut features = Features::new(
            servers::server_type(crate::client::ServerProtocol::Quarm),
            "Tester",
            eq_network_game::food::AutoEat::default(),
        );
        let click = ClientCommand::ClickDoor {
            session_id: 5,
            door_id: 3,
            created: Instant::now(),
        };
        let mut world = World::new(5);
        let outcome = testing::run(|out| {
            let handled = features.handle(&click, &mut world, out)?;
            assert!(!handled);
            unoffered(&click, out.log)
        });
        outcome.result.unwrap();
        assert!(matches!(
            &outcome.events[..],
            [ClientEvent::World(crate::world::WorldEvent::DoorAction {
                session_id: 5,
                door_id: 3,
                error: Some(reason),
            }), ..] if reason == UNOFFERED
        ));
    }

    #[test]
    fn a_titanium_zone_reports_everything_its_features_offer() {
        use crate::world::Capability;
        let features = |protocol| {
            Features::new(
                servers::server_type(protocol),
                "Tester",
                eq_network_game::food::AutoEat::default(),
            )
            .capabilities()
        };
        let p99 = features(crate::client::ServerProtocol::Project1999);
        for capability in [
            Capability::Casting,
            Capability::Spellbook,
            Capability::Inventory,
            Capability::Trading,
            Capability::Giving,
            Capability::Moving,
            Capability::Targeting,
            Capability::Combat,
            Capability::Looting,
            Capability::Talking,
            Capability::Camping,
            Capability::Doors,
            Capability::GroundItems,
            Capability::Zoning,
            Capability::Abilities,
            Capability::Who,
            Capability::Corpses,
            Capability::Pets,
        ] {
            assert!(p99.contains(&capability), "{capability:?}");
        }
        assert!(!p99.contains(&Capability::Falling));
        assert!(!p99.contains(&Capability::Training));
        let eqemu = features(crate::client::ServerProtocol::EqEmu);
        assert!(eqemu.contains(&Capability::Falling));
        assert!(eqemu.contains(&Capability::Training));
        assert!(eqemu.contains(&Capability::Resurrection));
        assert!(eqemu.contains(&Capability::Reading));
        assert!(eqemu.contains(&Capability::Tradeskills));
        assert!(eqemu.contains(&Capability::Map));
        assert!(eqemu.contains(&Capability::DeletingSpells));
        assert!(eqemu.contains(&Capability::MovingSpells));
        assert!(!p99.contains(&Capability::DeletingSpells));
        assert!(!p99.contains(&Capability::MovingSpells));
        assert!(eqemu.contains(&Capability::MerchantOffers));
        assert!(!p99.contains(&Capability::MerchantOffers));
        assert!(eqemu.contains(&Capability::Grouping));
        assert!(!p99.contains(&Capability::Grouping));
        assert!(eqemu.contains(&Capability::Listing));
        assert!(!p99.contains(&Capability::Listing));
        for capability in [
            Capability::Rolling,
            Capability::Emoting,
            Capability::Assisting,
            Capability::Raiding,
        ] {
            assert!(eqemu.contains(&capability));
            assert!(!p99.contains(&capability));
        }
        assert_eq!(eqemu.len(), p99.len() + 15);
        // EQMac servers talk, TAKP camps too, and neither follows a zone
        // change yet.
        assert_eq!(
            features(crate::client::ServerProtocol::Quarm),
            [Capability::Talking]
        );
        assert_eq!(
            features(crate::client::ServerProtocol::Takp),
            [Capability::Moving, Capability::Talking, Capability::Camping]
        );
    }
}

#[cfg(test)]
mod eqmac_tests;
