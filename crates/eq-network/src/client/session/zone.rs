//! The zone session: admission, the player's commands and the zone's traffic
//! until the character leaves for another zone, the world or the character list.
use super::{
    actions, bail, camp, casting, character, chat, combat, cstr,
    doors::Doors,
    ensure, entities,
    feature::{Encoder, Feature, Out, World},
    inventory, le32, looting, motion, objects, put_string, servers, spellbook, talk, targeting,
    transfers, CharacterSession, ClientCommand, ClientEvent, ConnectionStage, ConnectionState,
    Context, DecodeError, Duration, Events, Instant, RecordEvent, Result, Session, Shield,
    ZoneExit,
};

use eq_network_game::message::Message;
use eq_network_transport::Transport;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ZoneOpcode {
    PlayerProfile,
    Weather,
    PlayerSpawn,
    ZoneDescription,
    ExperienceUpdate,
    ValidationRejected,
    Unknown(u16),
}

impl From<u16> for ZoneOpcode {
    fn from(value: u16) -> Self {
        match value {
            0x75df => Self::PlayerProfile,
            0x254d => Self::Weather,
            0x7213 => Self::PlayerSpawn,
            0x0920 => Self::ZoneDescription,
            0x0587 => Self::ExperienceUpdate,
            0x1252 => Self::ValidationRejected,
            value => Self::Unknown(value),
        }
    }
}

/// The zone's features, each offered every command, packet, timer and event.
struct Features(Vec<Box<dyn Feature>>);

impl Features {
    /// Every feature a Titanium zone session has.
    fn new(dialect: eq_network_game::GameDialect, name: &str, falls: bool) -> Self {
        let encoder = Encoder::new(dialect, name);
        Self(vec![
            Box::new(casting::Casting::new(encoder.clone())),
            Box::new(spellbook::Spellbook::default()),
            Box::new(inventory::Belongings::new(encoder.clone())),
            Box::new(motion::Motion::new(encoder.clone(), falls)),
            Box::new(character::Character),
            Box::new(entities::Entities::default()),
            Box::new(targeting::Targeting::new(encoder.clone())),
            Box::new(combat::Combat::new(encoder.clone())),
            Box::new(looting::Looting::new(encoder.clone())),
            Box::new(talk::Talk::new(encoder)),
            Box::new(camp::Camp::default()),
            Box::new(Doors::default()),
            Box::new(objects::GroundObjects::default()),
            Box::new(transfers::Transfers::new(name)),
        ])
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
            if world.exit.is_some() {
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
            if world.exit.is_some() {
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
    // Titanium zones speak the modern transport; the loop needs only the
    // interface every generation's transport offers.
    let mut session: Box<dyn Transport> = Box::new(Session::connect_cancellable(
        crate::client::endpoint(host, port, config.local_only)?,
        true,
        stop.flag(),
    )?);
    session.send(0x7752, &0u32.to_le_bytes())?;
    let mut entry = vec![0; 68];
    put_string(&mut entry[4..], &config.character)?;
    let server = servers::server_type(config.protocol);
    let revolution = server.profile_turn();
    if let Some(shield) = shield.as_mut() {
        shield.zone_entry(&entry)?;
    }
    session.send(0x7213, &entry)?;
    log.send(ClientEvent::Progress(ConnectionStage::LoadingCharacter))?;
    let connected = Instant::now();
    let mut saw_spawn = false;
    let mut saw_profile = false;
    let mut saw_weather = false;
    let mut requested = false;
    let mut got_zone = false;
    let mut replied_experience = false;
    let mut zone_name = String::new();
    let mut far_clip = None;
    let mut progress = Instant::now();
    let session_id = rand::random();
    let mut world = World::new(session_id);
    let mut initial_experience = None;
    let mut initial_level = None;
    let mut initial_skills = std::collections::BTreeMap::new();
    // Wear changes for the player that arrive before its state is built.
    let mut initial_own_wear = Vec::new();
    let mut features = Features::new(config.protocol.into(), &config.character, server.falls());
    let mut profile_data = Vec::new();
    let mut spawn_data = Vec::new();
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
            "zone admission timed out"
        );
        features.tick(
            Instant::now(),
            &mut world,
            &mut Out {
                sink: &mut session,
                log: &mut *log,
            },
        )?;
        if let Some(exit) = world.exit.take() {
            session.close()?;
            return Ok(exit);
        }
        if world.lifecycle.blocks_motion() {
            if let Some(commands) = context.commands {
                for _ in commands.try_iter().take(64) {}
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
                    let handled = features.handle(
                        &command,
                        &mut world,
                        &mut Out {
                            sink: &mut session,
                            log: &mut *log,
                        },
                    )?;
                    if let Some(exit) = world.exit.take() {
                        session.close()?;
                        return Ok(exit);
                    }
                    if !handled {
                        log.diagnostic(
                            "Rejected a command this zone session does not take".into(),
                        )?;
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
        if !world.ready() {
            log.diagnostic(format!(
                "Zone received 0x{:04x} ({} bytes)",
                packet.opcode,
                packet.body.len()
            ))?;
        }
        match ZoneOpcode::from(packet.opcode) {
            ZoneOpcode::PlayerProfile => {
                ensure!(
                    packet.body.len() == 19592,
                    "unexpected Titanium player profile size"
                );
                for offset in [13116, 13120, 13124, 13128] {
                    ensure!(
                        f32::from_le_bytes(packet.body[offset..offset + 4].try_into().unwrap())
                            .is_finite(),
                        "invalid player position"
                    );
                }
                // The player stands where the server saved them until they
                // can move.
                let float = |offset: usize| {
                    f32::from_le_bytes(packet.body[offset..offset + 4].try_into().unwrap())
                };
                world.body.place(crate::world::Position {
                    x: float(13116),
                    y: float(13120),
                    z: float(13124),
                    heading: crate::world::profile_heading(float(13128), revolution),
                });
                profile_data.clone_from(&packet.body);
                initial_skills.clear();
                initial_level = None;
                world.zone = (
                    u16::from_le_bytes(packet.body[13276..13278].try_into()?),
                    u16::from_le_bytes(packet.body[13278..13280].try_into()?),
                );
                saw_profile = true;
            }
            ZoneOpcode::Weather => saw_weather = true,
            ZoneOpcode::PlayerSpawn if !saw_spawn => {
                if let Some(shield) = shield.as_mut() {
                    shield.player_spawn(&mut packet.body, &credentials.key)?;
                }
                ensure!(
                    packet.body.len() == 385
                        && cstr(&packet.body[7..71])
                            .eq_ignore_ascii_case(config.character.as_bytes()),
                    "zone returned a different character spawn"
                );
                let id = u16::try_from(le32(&packet.body[340..344]))
                    .context("spawn ID exceeds Titanium position field")?;
                world.body.own(id);
                world.own_spawn = Some(id);
                spawn_data.clone_from(&packet.body);
                saw_spawn = true;
            }
            ZoneOpcode::ZoneDescription if !world.ready() => {
                ensure!(packet.body.len() >= 96, "truncated zone description");
                zone_name = String::from_utf8_lossy(cstr(&packet.body[64..96])).into_owned();
                far_clip = crate::world::titanium_far_clip(&packet.body);
                log.zone.clone_from(&zone_name);
                log.status(
                    ConnectionState::Zoning,
                    world.packets,
                    Some(session.last_received_seconds()),
                )?;
                got_zone = true;
                session.send(0x067a, &0u32.to_le_bytes())?;
                session.send(0x5e3a, &0u32.to_le_bytes())?;
                session.send(0x7752, &0u32.to_le_bytes())?;
                session.send(0x0322, &[])?;
            }
            ZoneOpcode::ExperienceUpdate if got_zone && !world.ready() && !replied_experience => {
                session.send(0x0587, &[])?;
                replied_experience = true;
            }
            ZoneOpcode::ExperienceUpdate if got_zone && !world.ready() && replied_experience => {
                session.send(0x6563, &chat::server_filters())?;
                session.send(0x5e20, &[])?;
                session.send(0x0c11, &1u32.to_le_bytes())?;
                world.admitted = Some(Instant::now());
                match crate::world::titanium_player(&profile_data, &spawn_data, revolution) {
                    Ok(mut player) => {
                        if let Some(level) = initial_level.take() {
                            player.level = level;
                        }
                        for change in std::mem::take(&mut initial_own_wear) {
                            player.appearance.apply(&change);
                        }
                        for (skill, value) in std::mem::take(&mut initial_skills) {
                            player.apply_skill(skill, value);
                        }
                        world.player = Some(player.clone());
                        log.send(ClientEvent::World(crate::world::WorldEvent::Entered {
                            session_id,
                            zone: zone_name.clone(),
                            player: Box::new(player),
                            far_clip,
                        }))?;
                        features.admitted(
                            &mut world,
                            &mut Out {
                                sink: &mut session,
                                log: &mut *log,
                            },
                        )?;
                        log.send(ClientEvent::World(crate::world::WorldEvent::BuffSnapshot(
                            eq_network_game::buffs::titanium_profile(&profile_data)?,
                        )))?;
                        log.send(ClientEvent::World(crate::world::WorldEvent::Coins(
                            crate::world::titanium_coins(&profile_data)?,
                        )))?;
                        if let Some(value) = initial_experience.take() {
                            log.send(ClientEvent::World(crate::world::WorldEvent::Experience(
                                value,
                            )))?;
                        }
                    }
                    Err(error) => {
                        log.diagnostic(format!("Player presentation unavailable: {error}"))?;
                    }
                }
                profile_data.clear();
                spawn_data.clear();
                log.send(ClientEvent::Progress(ConnectionStage::Ready))?;
                log.status(
                    ConnectionState::Connected,
                    world.packets,
                    Some(session.last_received_seconds()),
                )?;
                log.diagnostic(format!("Zone login sequence complete for {zone_name}; waiting for ongoing server traffic"))?;
            }
            ZoneOpcode::ValidationRejected => bail!("server rejected zone validation"),
            _ => (),
        }
        if saw_spawn && saw_profile && saw_weather && !requested {
            if let Some(shield) = shield.as_ref() {
                shield.answer(&mut checksums)?;
                session.send(0x1251, &checksums)?;
            }
            session.send(0x7ac5, &[])?;
            session.send(0x367d, &[])?;
            session.send(0x5966, &[])?;
            requested = true;
            log.send(ClientEvent::Progress(ConnectionStage::EnteringWorld))?;
        }
        // Everything else is read once, the same way before and after
        // admission, and heard by every feature.
        for mut message in eq_network_game::message::titanium(packet.opcode, &packet.body) {
            features.explain(&mut message, &world);
            if let Message::Unreadable { part, error } = &message {
                log.diagnostic(format!("{part} rejected: {error}"))?;
            }
            if world.ready() {
                features.observe(
                    &message,
                    &mut world,
                    &mut Out {
                        sink: &mut session,
                        log: &mut *log,
                    },
                )?;
            } else {
                features.admit(&message, &mut world)?;
            }
            if let Some(exit) = world.exit.take() {
                session.close()?;
                return Ok(exit);
            }
            ensure!(
                !matches!(message, Message::LoggedOut),
                "server logged the character out"
            );
            let Message::Event(event) = message else {
                continue;
            };
            match event {
                event if world.ready() => log.send(ClientEvent::World(event))?,
                // Before admission, the player's and the zone's state is
                // staged for the admission report.
                crate::world::WorldEvent::Skill { skill_id, value } if skill_id < 100 => {
                    initial_skills.insert(skill_id, value);
                }
                crate::world::WorldEvent::Level {
                    current,
                    experience,
                    ..
                } => {
                    initial_level = Some(current);
                    initial_experience = Some(experience);
                }
                crate::world::WorldEvent::Experience(value) => {
                    initial_experience = Some(value);
                }
                crate::world::WorldEvent::WearChange(change)
                    if world.is_player(change.spawn_id) =>
                {
                    initial_own_wear.push(change);
                }
                _ => (),
            }
        }
        match chat::parse(packet.opcode, &packet.body, config.include_raw) {
            Ok(Some(event)) => log.record(&zone_name, RecordEvent::Chat(event))?,
            Ok(None) => (),
            Err(error) => log.record(
                &zone_name,
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

#[cfg(test)]
mod tests {
    use super::*;
    use eq_network_game::{
        chat::OutboundChat,
        inventory::{InventoryMove, InventorySlot, ItemUse, MoveQuantity},
        movement::{MotionCalibration, MovementMode, MovementRequest},
    };

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
                slot: 0,
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
        ]
    }

    #[test]
    fn exactly_one_feature_owns_each_command_a_zone_takes() {
        let features = Features::new(eq_network_game::GameDialect::TitaniumP99, "Tester", false);
        for command in zone_commands() {
            let owning = features
                .0
                .iter()
                .filter(|feature| feature.owns(&command))
                .count();
            assert_eq!(owning, 1, "{command:?}");
        }
        let selection = ClientCommand::SelectCharacter {
            selection_id: 1,
            slot: 0,
        };
        assert!(!features.0.iter().any(|feature| feature.owns(&selection)));
    }
}
