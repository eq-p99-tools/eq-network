//! The zone session: admission, the player's commands and the zone's traffic
//! until the character leaves for another zone, the world or the character list.
use super::{
    actions, bail, book_edits, camp, casting, chat, command, cstr,
    doors::Doors,
    ensure, entities,
    feature::{Feature, Out, World},
    inventory, le32, merchant, motion, objects, put_string, scribe_consumption, servers, spellbook,
    transfers, BookActionStatus, BookIntent, CharacterSession, ClientCommand, ClientEvent,
    ConnectionStage, ConnectionState, Context, DecodeError, Duration, Events, Instant,
    MotionSession, PendingBookAction, RecordEvent, Result, Session, Shield, ZoneExit,
};

use eq_network_game::message::{Message, Part};

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
    fn new(character: &str) -> Self {
        Self(vec![
            Box::new(entities::Entities::default()),
            Box::new(camp::Camp::default()),
            Box::new(Doors::default()),
            Box::new(objects::GroundObjects::default()),
            Box::new(transfers::Transfers::new(character)),
        ])
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
    let mut inventory_actor: Option<eq_network_game::inventory::InventoryActor> = None;
    let mut admitted_book: Option<eq_network_game::spells::SpellBook> = None;
    let mut book_edits = book_edits::BookEdits::default();
    let mut cast_guard = casting::CastGuard::default();
    let mut trades = merchant::MerchantTrades::default();
    let mut settlement = inventory::Settlement::default();
    let mut scribe_consumption = scribe_consumption::ScribeConsumption::default();
    // Wear changes for the player that arrive before its state is built.
    let mut initial_own_wear = Vec::new();
    let mut features = Features::new(&config.character);
    let mut profile_data = Vec::new();
    let mut spawn_data = Vec::new();
    loop {
        if stop.is_cancelled() || duration.is_some_and(|limit| connected.elapsed() >= limit) {
            session.close()?;
            // The outer run reports Stopped after the session has closed.
            return Ok(ZoneExit::Stopped);
        }
        if cast_guard.expire(Instant::now()) {
            log.send(ClientEvent::World(crate::world::WorldEvent::CastPending {
                session_id,
                spell_id: None,
            }))?;
            log.diagnostic("Cast acknowledgement timed out; a manual retry is available".into())?;
        }
        // Servers answer only a refused move, so silence settles the rest.
        if let Some(update) = settlement.due(&world.inventory, Instant::now()) {
            world.inventory.apply(update.clone());
            if world.ready {
                log.send(ClientEvent::World(crate::world::WorldEvent::Inventory(
                    update,
                )))?;
            }
        }
        if trades.expire(Instant::now()) {
            log.send(ClientEvent::World(
                crate::world::WorldEvent::MerchantRefused {
                    session_id,
                    reason: "The merchant did not accept that offer.".into(),
                },
            ))?;
            log.diagnostic("Merchant trade was not answered; released the inventory".into())?;
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
        // Servers refuse an edit (a spell above the character's level, another
        // class's scroll) with only a chat message, so silence means refusal.
        if let Some(status) = book_edits.expire(Instant::now()) {
            log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                status,
            )))?;
        }
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
                        actions::Held::from_state(
                            &cast_guard,
                            &book_edits,
                            world.book_action.as_ref(),
                            &trades,
                            scribe_consumption.awaiting_cursor(now),
                        )
                        .conflict(&command)
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
                    if matches!(
                        &command,
                        ClientCommand::Move(_)
                            | ClientCommand::CastSpell { .. }
                            | ClientCommand::UseItem(_)
                            | ClientCommand::SetPosture { .. }
                    ) && world.book_action.take().is_some()
                    {
                        log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                            BookActionStatus::Cancelled(
                                "Movement, casting or posture changed".into(),
                            ),
                        )))?;
                    }
                    if let ClientCommand::ScribeSpell {
                        revision,
                        slot,
                        spell_id,
                        ..
                    } = &command
                    {
                        let now = Instant::now();
                        let pending = PendingBookAction {
                            started: now,
                            intent: BookIntent::Scribe {
                                revision: *revision,
                                slot: *slot,
                                spell_id: *spell_id,
                            },
                        };
                        let packet = admitted_book
                            .as_ref()
                            .filter(|_| world.book_action.is_none())
                            .context("scribing is unavailable")
                            .and_then(|book| pending.packet(book, &world.inventory));
                        match packet {
                            Ok(_) => {
                                if let Some(player) = world.player.as_ref() {
                                    let sit = command::encode(
                                        config.protocol.into(),
                                        &ClientCommand::SetPosture {
                                            session_id,
                                            spawn_id: player.spawn_id,
                                            posture: command::Posture::Sitting,
                                            created: now,
                                        },
                                        &config.character,
                                    )?;
                                    session.send(sit.opcode, &sit.body)?;
                                    world.posture.sent(
                                        player.spawn_id,
                                        command::Posture::Sitting,
                                        log,
                                    )?;
                                    world.book_action = Some(pending);
                                    log.send(ClientEvent::World(
                                        crate::world::WorldEvent::BookAction(
                                            BookActionStatus::Preparing,
                                        ),
                                    ))?;
                                    log.diagnostic(
                                        "Scribing scroll; movement or posture changes cancel it"
                                            .into(),
                                    )?;
                                }
                            }
                            Err(error) => {
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::BookAction(
                                        BookActionStatus::Rejected(error.to_string()),
                                    ),
                                ))?;
                                log.diagnostic(format!("Rejected scribing: {error}"))?;
                            }
                        }
                        continue;
                    }
                    if let Some(packet) = spellbook::edit_packet(
                        &command,
                        admitted_book.as_ref(),
                        world.book_action.is_some(),
                    ) {
                        let status = match packet {
                            Ok((opcode, body)) => {
                                session.send(opcode, &body)?;
                                book_edits.sent(&command, Instant::now());
                                BookActionStatus::AwaitingReply
                            }
                            Err(error) => BookActionStatus::Rejected(error.to_string()),
                        };
                        log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                            status,
                        )))?;
                        continue;
                    }
                    if let ClientCommand::ForgetSpell { gem, spell_id, .. } = &command {
                        let packet = world
                            .player
                            .as_ref()
                            .context("forget request is unavailable")
                            .and_then(|player| {
                                eq_network_game::spells::forget_packet(
                                    &player.memorized_spells,
                                    *gem,
                                    *spell_id,
                                )
                            });
                        match packet {
                            Ok(body) => {
                                world.book_action = None;
                                session.send(0x308e, &body)?;
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::BookAction(
                                        BookActionStatus::Submitted,
                                    ),
                                ))?;
                            }
                            Err(error) => {
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::BookAction(
                                        BookActionStatus::Rejected(error.to_string()),
                                    ),
                                ))?;
                                log.diagnostic(format!("Rejected forgetting spell: {error}"))?;
                            }
                        }
                        continue;
                    }
                    if let ClientCommand::MemorizeSpell { gem, spell_id, .. } = &command {
                        let packet = admitted_book
                            .as_ref()
                            .filter(|_| world.book_action.is_none())
                            .context("memorization is unavailable")
                            .and_then(|book| book.memorize_packet(*gem, *spell_id));
                        match packet {
                            Ok(_) => {
                                if let Some(player) = world.player.as_ref() {
                                    let sit = ClientCommand::SetPosture {
                                        session_id,
                                        spawn_id: player.spawn_id,
                                        posture: command::Posture::Sitting,
                                        created: Instant::now(),
                                    };
                                    let sit = command::encode(
                                        config.protocol.into(),
                                        &sit,
                                        &config.character,
                                    )?;
                                    session.send(sit.opcode, &sit.body)?;
                                    world.posture.sent(
                                        player.spawn_id,
                                        command::Posture::Sitting,
                                        log,
                                    )?;
                                    world.book_action = Some(PendingBookAction {
                                        started: Instant::now(),
                                        intent: BookIntent::Memorize {
                                            gem: *gem,
                                            spell_id: *spell_id,
                                        },
                                    });
                                    log.send(ClientEvent::World(
                                        crate::world::WorldEvent::BookAction(
                                            BookActionStatus::Preparing,
                                        ),
                                    ))?;
                                    log.diagnostic(
                                        "Memorizing spell; movement or posture changes cancel it"
                                            .into(),
                                    )?;
                                }
                            }
                            Err(error) => {
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::BookAction(
                                        BookActionStatus::Rejected(error.to_string()),
                                    ),
                                ))?;
                                log.diagnostic(format!("Rejected memorization: {error}"))?;
                            }
                        }
                        continue;
                    }
                    if let ClientCommand::UseItem(request) = &command {
                        let target_available = world.player.as_ref().is_some_and(|player| {
                            request.target_id == player.spawn_id
                                || world.spawns.visible(request.target_id).is_some()
                        });
                        let prepared = inventory_actor
                            .as_ref()
                            .context("Character level is unavailable")
                            .and_then(|actor| {
                                world.inventory.prepare_item_cast(
                                    request,
                                    session_id,
                                    actor.level,
                                    target_available,
                                    Instant::now(),
                                )
                            });
                        let error = match prepared {
                            Ok((spell_id, body)) => {
                                session.send(0x304b, &body)?;
                                cast_guard.submitted(spell_id, Instant::now());
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::CastPending {
                                        session_id,
                                        spell_id: Some(spell_id),
                                    },
                                ))?;
                                None
                            }
                            Err(error) => Some(error.to_string()),
                        };
                        log.send(ClientEvent::World(
                            crate::world::WorldEvent::ItemUseAction {
                                session_id: request.session_id,
                                request_id: request.request_id,
                                error,
                            },
                        ))?;
                        continue;
                    }
                    let action_valid = match &command {
                        ClientCommand::CastSpell {
                            gem,
                            spell_id,
                            target_id,
                            ..
                        } => world.player.as_ref().is_some_and(|player| {
                            player.memorized_spells.get(usize::from(*gem)) == Some(&Some(*spell_id))
                                && (*target_id == player.spawn_id
                                    || world.spawns.visible(*target_id).is_some())
                        }),
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
                        if let Some(event) = casting::rejected(
                            &command,
                            "The spell gem changed, or the target is unavailable",
                        ) {
                            log.send(ClientEvent::World(event))?;
                        }
                        log.diagnostic("Rejected an unavailable spell, posture or target".into())?;
                        continue;
                    }
                    if inventory::handle(
                        &mut world.inventory,
                        &mut settlement,
                        inventory_actor.map(|mut actor| {
                            actor.bank_access = world.player.as_ref().is_some_and(|player| {
                                let position = world
                                    .motion
                                    .as_ref()
                                    .map_or(player.position, MotionSession::position);
                                world.spawns.all().any(|spawn| {
                                    eq_network_game::inventory::banker_in_range(position, spawn)
                                })
                            });
                            actor
                        }),
                        session_id,
                        &command,
                        &mut session,
                        log,
                    )? {
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
                            trades.sent(&command, Instant::now());
                            if let ClientCommand::SetPosture {
                                spawn_id, posture, ..
                            } = &command
                            {
                                world.posture.sent(*spawn_id, *posture, log)?;
                            }
                            if let ClientCommand::CastSpell { spell_id, .. } = &command {
                                cast_guard.submitted(*spell_id, Instant::now());
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::CastPending {
                                        session_id,
                                        spell_id: Some(*spell_id),
                                    },
                                ))?;
                            }
                            if let ClientCommand::SelectTarget { spawn_id, .. } = command {
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::TargetSent(spawn_id),
                                ))?;
                            }
                        }
                        Err(error) => {
                            if let Some(event) = casting::rejected(&command, &error.to_string()) {
                                log.send(ClientEvent::World(event))?;
                            }
                            log.diagnostic(format!(
                                "Rejected invalid outbound client command: {error}"
                            ))?;
                        }
                    }
                }
            }
        }
        if !world.lifecycle.is_dead() && world.lifecycle.pending().is_none() {
            if world
                .book_action
                .as_ref()
                .is_some_and(|pending| pending.ready(Instant::now()))
            {
                if let Some(pending) = world.book_action.take() {
                    let packet = admitted_book
                        .as_ref()
                        .context("spellbook unavailable")
                        .and_then(|book| pending.packet(book, &world.inventory));
                    match packet {
                        Ok(body) => {
                            session.send(0x308e, &body)?;
                            book_edits.prepared_sent(&pending.intent, Instant::now());
                            scribe_consumption.sent(&pending.intent);
                            log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                                BookActionStatus::AwaitingReply,
                            )))?;
                        }
                        Err(error) => {
                            log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                                BookActionStatus::Cancelled(error.to_string()),
                            )))?;
                            log.diagnostic(format!("Cancelled spellbook action: {error}"))?;
                        }
                    }
                }
            }
        } else {
            spellbook::cancel_pending(
                &mut world.book_action,
                "Character died or zone transfer started",
                log,
            )?;
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
                        let spell_book =
                            eq_network_game::spells::SpellBook::titanium_profile(&profile_data)?;
                        admitted_book = Some(spell_book.clone());
                        inventory_actor = Some(eq_network_game::inventory::InventoryActor {
                            bank_access: false,
                            deity: player.deity,
                            dual_wield: player
                                .skills
                                .as_ref()
                                .and_then(|skills| skills.get(22))
                                .copied(),
                            class: player.class,
                            race: player.race,
                            level: player.level,
                        });
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
                        let admission = world.inventory.admission_updates();
                        log.send(ClientEvent::World(crate::world::WorldEvent::BuffSnapshot(
                            eq_network_game::buffs::titanium_profile(&profile_data)?,
                        )))?;
                        log.send(ClientEvent::World(crate::world::WorldEvent::SpellBook(
                            spell_book,
                        )))?;
                        log.send(ClientEvent::World(crate::world::WorldEvent::Coins(
                            crate::world::titanium_coins(&profile_data)?,
                        )))?;
                        world.inventory = eq_network_game::inventory::Inventory::default();
                        for update in admission {
                            world.inventory.apply(update.clone());
                            log.send(ClientEvent::World(crate::world::WorldEvent::Inventory(
                                update,
                            )))?;
                        }

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
        for message in eq_network_game::message::titanium(packet.opcode, &packet.body) {
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
            let event = match message {
                Message::Event(event) => event,
                Message::Unreadable {
                    part: Part::Inventory,
                    ..
                } => {
                    let update = eq_network_game::inventory::InventoryUpdate::Invalidated;
                    world.inventory.apply(update.clone());
                    if world.ready {
                        log.send(ClientEvent::World(crate::world::WorldEvent::Inventory(
                            update,
                        )))?;
                    }
                    continue;
                }
                _ => continue,
            };
            match event {
                crate::world::WorldEvent::Inventory(update) => {
                    let update = scribe_consumption.reconcile(&world.inventory, update);
                    world.inventory.apply(update.clone());
                    if world.ready {
                        log.send(ClientEvent::World(crate::world::WorldEvent::Inventory(
                            update,
                        )))?;
                    }
                }
                crate::world::WorldEvent::Spell(update) if world.ready => {
                    let book_result = book_edits.observe(&update);
                    scribe_consumption
                        .observe(&update, book_result == Some(BookActionStatus::Confirmed));
                    if let Some(player) = world.player.as_ref() {
                        let pending = cast_guard.pending();
                        cast_guard.observe(player.spawn_id, &update);
                        if pending.is_some() && cast_guard.pending().is_none() {
                            log.send(ClientEvent::World(crate::world::WorldEvent::CastPending {
                                session_id,
                                spell_id: None,
                            }))?;
                        }
                        if cast_guard.active() && world.book_action.take().is_some() {
                            log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                                BookActionStatus::Cancelled("Casting started".into()),
                            )))?;
                        }
                    }
                    if let Some(book) = admitted_book.as_mut() {
                        book.apply(&update);
                    }
                    if let Some(player) = world.player.as_mut() {
                        update.apply_gems(&mut player.memorized_spells);
                    }
                    log.send(ClientEvent::World(crate::world::WorldEvent::Spell(update)))?;
                    if let Some(status) = book_result {
                        log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                            status,
                        )))?;
                    }
                }
                event if world.ready => {
                    match &event {
                        crate::world::WorldEvent::Posture { spawn_id, posture }
                            if world.is_player(*spawn_id) =>
                        {
                            world.posture.observed(*posture);
                            if *posture != crate::world::PostureState::Sitting {
                                spellbook::cancel_pending(
                                    &mut world.book_action,
                                    "Server changed character posture",
                                    log,
                                )?;
                            }
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
                            if let Some(actor) = inventory_actor.as_mut() {
                                actor.level = *current;
                            }
                        }
                        crate::world::WorldEvent::Skill { skill_id, value } => {
                            if let Some(player) = world.player.as_mut() {
                                player.apply_skill(*skill_id, *value);
                                if let Some(actor) = inventory_actor.as_mut() {
                                    actor.dual_wield = player
                                        .skills
                                        .as_ref()
                                        .and_then(|skills| skills.get(22))
                                        .copied();
                                }
                            }
                        }
                        crate::world::WorldEvent::Death(death)
                            if world.is_player(death.spawn_id) =>
                        {
                            world.lifecycle.mark_dead();
                            spellbook::cancel_pending(
                                &mut world.book_action,
                                "Character died",
                                log,
                            )?;
                            cast_guard.clear();
                            trades.clear();
                            if let Some(motion) = world.motion.as_mut() {
                                motion.suspend();
                            }
                            log.send(ClientEvent::World(motion::withdrawn(session_id)))?;
                            log.diagnostic(
                                "Own character died; waiting for the server bind offer".into(),
                            )?;
                        }
                        // A sale's echo is the only notice that the item left.
                        crate::world::WorldEvent::Merchant(update) => {
                            if let Some(change) = trades.observe(update) {
                                world.inventory.apply(change.clone());
                                log.send(ClientEvent::World(crate::world::WorldEvent::Inventory(
                                    change,
                                )))?;
                            }
                        }
                        _ => (),
                    }
                    if let crate::world::WorldEvent::Position {
                        spawn_id, position, ..
                    } = &event
                    {
                        if world.is_player(*spawn_id) {
                            spellbook::cancel_pending(
                                &mut world.book_action,
                                "Server corrected character position",
                                log,
                            )?;
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
        let features = Features::new("Tester");
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
