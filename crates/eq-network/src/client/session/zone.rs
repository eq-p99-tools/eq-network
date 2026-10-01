//! The zone session: admission, the player's commands and the zone's traffic
//! until the character leaves for another zone, the world or the character list.
use super::{
    actions, bail, camp, casting, chat, command, cstr,
    doors::Doors,
    ensure, entities,
    feature::{Feature, Out, World},
    inventory, le32, motion, objects, put_string, servers, spellbook, transfers, CharacterSession,
    ClientCommand, ClientEvent, ConnectionStage, ConnectionState, Context, DecodeError, Duration,
    Events, Instant, MotionSession, RecordEvent, Result, Session, Shield, ZoneExit,
};

use eq_network_game::message::Message;

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
    fn new(dialect: eq_network_game::GameDialect, character: &str) -> Self {
        Self(vec![
            Box::new(casting::Casting::new(dialect, character)),
            Box::new(spellbook::Spellbook::default()),
            Box::new(inventory::Belongings::new(dialect, character)),
            Box::new(entities::Entities::default()),
            Box::new(camp::Camp::default()),
            Box::new(Doors::default()),
            Box::new(objects::GroundObjects::default()),
            Box::new(transfers::Transfers::new(character)),
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
    let mut session = Session::connect_cancellable(
        crate::client::endpoint(host, port, config.local_only)?,
        true,
        stop.flag(),
    )?;
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
    let mut features = Features::new(config.protocol.into(), &config.character);
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
                if world.ready
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
            world.ready || connected.elapsed() < Duration::from_secs(60),
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
        if world.ready && !world.lifecycle.blocks_motion() {
            if let Some(motion) = world.motion.as_mut() {
                motion.tick(Instant::now(), |body| session.send_unreliable(0x14cb, body))?;
            } else if world.last_position.elapsed()
                >= eq_network_game::movement::STATIONARY_HEARTBEAT
            {
                world.stationary[2..4].copy_from_slice(&world.sequence.to_le_bytes());
                session.send_unreliable(0x14cb, &world.stationary)?;
                world.sequence = world.sequence.wrapping_add(1);
                world.last_position = Instant::now();
            }
        }
        if world.lifecycle.blocks_motion() {
            if let Some(commands) = context.commands {
                for _ in commands.try_iter().take(64) {}
            }
        }
        if world.ready && !world.lifecycle.is_dead() && world.lifecycle.pending().is_none() {
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
                    if handled {
                        // Commands wait while the player is dead or zoning.
                        if world.lifecycle.is_dead() || world.lifecycle.pending().is_some() {
                            break;
                        }
                        continue;
                    }
                    let action_valid = match &command {
                        ClientCommand::SetPosture { spawn_id, .. }
                        | ClientCommand::Consider {
                            own_id: spawn_id, ..
                        } => {
                            world
                                .player
                                .as_ref()
                                .is_some_and(|player| player.spawn_id == *spawn_id)
                                && match &command {
                                    ClientCommand::Consider { target_id, .. } => {
                                        world.spawns.visible(*target_id).is_some()
                                    }
                                    _ => true,
                                }
                        }
                        ClientCommand::Loot {
                            corpse_id: entity, ..
                        }
                        | ClientCommand::Shop {
                            merchant_id: entity,
                            ..
                        } => world.spawns.visible(*entity).is_some_and(|spawn| {
                            if matches!(command, ClientCommand::Loot { .. }) {
                                matches!(
                                    spawn.kind,
                                    crate::world::SpawnKind::NpcCorpse
                                        | crate::world::SpawnKind::PlayerCorpse
                                )
                            } else {
                                spawn.kind == crate::world::SpawnKind::Npc
                            }
                        }),
                        _ => true,
                    };
                    if !action_valid {
                        log.diagnostic("Rejected an unavailable posture or target".into())?;
                        continue;
                    }
                    if let Some(motion) = world.motion.as_mut() {
                        let own = (
                            &mut world.posture,
                            world.player.as_ref().map(|player| player.spawn_id),
                        );
                        if motion::handle(motion, own, session_id, &command, &mut session, log)? {
                            continue;
                        }
                    }
                    if let ClientCommand::SelectTarget {
                        session_id: requested_session,
                        spawn_id,
                    } = &command
                    {
                        if spawn_id.is_some_and(|id| {
                            world
                                .player
                                .as_ref()
                                .is_none_or(|player| player.spawn_id != id)
                                && world.spawns.visible(id).is_none()
                        }) {
                            log.send(ClientEvent::World(
                                crate::world::WorldEvent::TargetRejected {
                                    session_id: *requested_session,
                                    spawn_id: *spawn_id,
                                    reason: "Target is invisible or unavailable".into(),
                                },
                            ))?;
                            log.diagnostic("Rejected an unavailable target".into())?;
                            continue;
                        }
                    }
                    match command::encode(config.protocol.into(), &command, &config.character) {
                        Ok(packet) => {
                            session.send(packet.opcode, &packet.body)?;
                            if let ClientCommand::SetPosture {
                                spawn_id, posture, ..
                            } = &command
                            {
                                world.posture.sent(*spawn_id, *posture, log)?;
                            }
                            if let ClientCommand::SelectTarget { spawn_id, .. } = command {
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::TargetSent(spawn_id),
                                ))?;
                            }
                        }
                        Err(error) => {
                            log.diagnostic(format!(
                                "Rejected invalid outbound client command: {error}"
                            ))?;
                        }
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
        if !world.ready {
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
                // Preserve the server's saved coordinates. Velocity and
                // animation stay zero; this collector never navigates.
                world.stationary[4..8].copy_from_slice(&packet.body[13120..13124]);
                world.stationary[24..28].copy_from_slice(&packet.body[13116..13120]);
                world.stationary[28..32].copy_from_slice(&packet.body[13124..13128]);
                let heading = f32::from_le_bytes(packet.body[13128..13132].try_into().unwrap());
                let heading = crate::world::profile_heading(heading, revolution) * 8.0;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let heading = heading as u16;
                world.stationary[32..34].copy_from_slice(&(heading & 0x0fff).to_le_bytes());
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
                world.stationary[..2].copy_from_slice(&id.to_le_bytes());
                world.own_spawn = Some(id);
                spawn_data.clone_from(&packet.body);
                saw_spawn = true;
            }
            ZoneOpcode::ZoneDescription if !world.ready => {
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
            ZoneOpcode::ExperienceUpdate if got_zone && !world.ready && !replied_experience => {
                session.send(0x0587, &[])?;
                replied_experience = true;
            }
            ZoneOpcode::ExperienceUpdate if got_zone && !world.ready && replied_experience => {
                session.send(0x6563, &chat::server_filters())?;
                session.send(0x5e20, &[])?;
                session.send(0x0c11, &1u32.to_le_bytes())?;
                world.ready = true;
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
                        world.motion = Some(
                            MotionSession::new(
                                session_id,
                                player.spawn_id,
                                player.position,
                                Instant::now(),
                            )?
                            .with_falls(server.falls()),
                        );
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
            if world.ready {
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
                event if world.ready => {
                    match &event {
                        crate::world::WorldEvent::Posture { spawn_id, posture }
                            if world.is_player(*spawn_id) =>
                        {
                            world.posture.observed(*posture);
                        }
                        crate::world::WorldEvent::WearChange(change) => {
                            if let Some(player) = world
                                .player
                                .as_mut()
                                .filter(|player| player.spawn_id == change.spawn_id)
                            {
                                player.appearance.apply(change);
                            }
                        }
                        crate::world::WorldEvent::Level { current, .. } => {
                            if let Some(player) = world.player.as_mut() {
                                player.level = *current;
                            }
                        }
                        crate::world::WorldEvent::Skill { skill_id, value } => {
                            if let Some(player) = world.player.as_mut() {
                                player.apply_skill(*skill_id, *value);
                            }
                        }
                        crate::world::WorldEvent::Death(death)
                            if world.is_player(death.spawn_id) =>
                        {
                            world.lifecycle.mark_dead();
                            if let Some(motion) = world.motion.as_mut() {
                                motion.suspend();
                            }
                            log.send(ClientEvent::World(motion::withdrawn(session_id)))?;
                            log.diagnostic(
                                "Own character died; waiting for the server bind offer".into(),
                            )?;
                        }
                        _ => (),
                    }
                    if let crate::world::WorldEvent::Position {
                        spawn_id, position, ..
                    } = &event
                    {
                        if world.is_player(*spawn_id) {
                            let player = world
                                .player
                                .as_mut()
                                .context("correction without admitted player")?;
                            motion::correct_own(
                                player,
                                world.motion.as_mut(),
                                &mut world.stationary,
                                world.sequence,
                                *position,
                                Instant::now(),
                            )?;
                            log.send(ClientEvent::World(motion::withdrawn(session_id)))?;
                        }
                    }
                    log.send(ClientEvent::World(event))?;
                }
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

    #[test]
    fn one_feature_owns_each_command_a_feature_takes() {
        let features = Features::new(eq_network_game::GameDialect::TitaniumP99, "Tester");
        let created = Instant::now();
        let session_id = 1;
        for (command, owners) in [
            (
                ClientCommand::Camp {
                    session_id,
                    created,
                },
                1,
            ),
            (
                ClientCommand::ClickDoor {
                    session_id,
                    door_id: 1,
                    created,
                },
                1,
            ),
            (
                ClientCommand::PickUp {
                    session_id,
                    drop_id: 1,
                    created,
                },
                1,
            ),
            (
                ClientCommand::CrossZoneLine {
                    session_id,
                    destination: eq_network_game::zoning::ZoneLineDestination::Reference(1),
                    position: crate::world::Position::default(),
                    created,
                },
                1,
            ),
            (
                ClientCommand::CastSpell {
                    session_id,
                    gem: 0,
                    spell_id: 202,
                    target_id: 7,
                    created,
                },
                1,
            ),
            (
                ClientCommand::MemorizeSpell {
                    session_id,
                    gem: 0,
                    spell_id: 202,
                    created,
                },
                1,
            ),
            (
                ClientCommand::SwapSpell {
                    session_id,
                    from: 0,
                    to: 1,
                    from_spell: 202,
                    to_spell: None,
                    created,
                },
                1,
            ),
            (
                ClientCommand::SelectTarget {
                    session_id,
                    spawn_id: None,
                },
                0,
            ),
        ] {
            let owning = features
                .0
                .iter()
                .filter(|feature| feature.owns(&command))
                .count();
            assert_eq!(owning, owners, "{command:?}");
        }
    }
}
