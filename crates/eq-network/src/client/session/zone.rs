//! The zone session: admission, the player's commands and the zone's traffic
//! until the character leaves for another zone, the world or the character list.
use super::{
    actions, bail, book_edits, camp, casting, chat, command, cstr, ensure, inventory, le32,
    merchant, motion, objects, posture, put_string, scribe_consumption, servers, spellbook, zoning,
    BTreeMap, BookActionStatus, BookIntent, CharacterSession, ClientCommand, ClientEvent,
    ConnectionStage, ConnectionState, Context, DecodeError, Duration, Events, Instant,
    MotionSession, PendingBookAction, RecordEvent, Result, Session, Shield, ZoneExit,
    ZoneLifecycle,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ZoneOpcode {
    PlayerProfile,
    Weather,
    PlayerSpawn,
    ZoneDescription,
    ExperienceUpdate,
    ValidationRejected,
    LoggedOut,
    ZoneHandoff,
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
            0x3cdc => Self::LoggedOut,
            0x61b6 => Self::ZoneHandoff,
            value => Self::Unknown(value),
        }
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
    let mut ready = false;
    let mut lifecycle = ZoneLifecycle::default();
    let mut motion: Option<MotionSession> = None;
    let mut saw_spawn = false;
    let mut saw_profile = false;
    let mut saw_weather = false;
    let mut requested = false;
    let mut got_zone = false;
    let mut replied_experience = false;
    let mut zone_name = String::new();
    let mut far_clip = None;
    let mut progress = Instant::now();
    let mut packets = 0u64;
    let mut stationary = [0u8; 36];
    let session_id = rand::random();
    let mut initial_experience = None;
    let mut initial_level = None;
    let mut initial_skills = std::collections::BTreeMap::new();
    let mut inventory = eq_network_game::inventory::Inventory::default();
    let mut inventory_actor: Option<eq_network_game::inventory::InventoryActor> = None;
    let mut admitted_player: Option<crate::world::PlayerState> = None;
    let mut admitted_book: Option<eq_network_game::spells::SpellBook> = None;
    let mut book_edits = book_edits::BookEdits::default();
    let mut cast_guard = casting::CastGuard::default();
    let mut trades = merchant::MerchantTrades::default();
    let mut settlement = inventory::Settlement::default();
    let mut camp = camp::Camp::default();
    let mut own_posture = posture::OwnPosture::default();
    let mut zone_points = zoning::ZonePoints::default();
    let mut current_zone = (0u16, 0u16);
    let mut pending_memorization: Option<PendingBookAction> = None;
    let mut scribe_consumption = scribe_consumption::ScribeConsumption::default();
    let mut initial_spawns = BTreeMap::new();
    let mut initial_postures = BTreeMap::new();
    // Wear changes for the player that arrive before its state is built.
    let mut initial_own_wear = Vec::new();
    let mut doors = eq_network_game::doors::Doors::default();
    let mut doors_changed_at = Instant::now();
    let mut ground = objects::GroundObjects::default();
    let mut profile_data = Vec::new();
    let mut spawn_data = Vec::new();
    let mut position_sequence = 0u16;
    // In the past, so the first stationary heartbeat goes out at once.
    let mut last_position = Instant::now()
        .checked_sub(eq_network_game::movement::STATIONARY_HEARTBEAT)
        .unwrap_or_else(Instant::now);
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
        if let Some(update) = settlement.due(&inventory, Instant::now()) {
            inventory.apply(update.clone());
            if ready {
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
                if ready && lifecycle.pending().is_none() && session.last_received_seconds() < 60 {
                    ConnectionState::Connected
                } else {
                    ConnectionState::Zoning
                },
                packets,
                Some(session.last_received_seconds()),
            )?;
            log.diagnostic(format!(
                "Zone session: {packets} application packets, {} communication records",
                log.messages
            ))?;
            progress = Instant::now();
        }
        ensure!(
            ready || connected.elapsed() < Duration::from_secs(60),
            "zone admission timed out"
        );
        ensure!(
            !lifecycle.expired(Instant::now()),
            "zone transfer approval timed out"
        );
        // Servers refuse an edit (a spell above the character's level, another
        // class's scroll) with only a chat message, so silence means refusal.
        if let Some(status) = book_edits.expire(Instant::now()) {
            log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                status,
            )))?;
        }
        if camp.logout_due(Instant::now()) {
            session.send(camp::LOGOUT_OPCODE, &[])?;
            camp.logout_sent(Instant::now());
            log.send(ClientEvent::World(crate::world::WorldEvent::Camp(
                crate::world::CampStatus::LoggingOut,
            )))?;
        }
        if camp.reply_overdue(Instant::now()) {
            log.send(ClientEvent::World(crate::world::WorldEvent::Camp(
                crate::world::CampStatus::Camped,
            )))?;
            session.close()?;
            return Ok(ZoneExit::CharacterSelect);
        }
        if ready && !lifecycle.blocks_motion() {
            if let Some(motion) = motion.as_mut() {
                motion.tick(Instant::now(), |body| session.send_unreliable(0x14cb, body))?;
            } else if last_position.elapsed() >= eq_network_game::movement::STATIONARY_HEARTBEAT {
                stationary[2..4].copy_from_slice(&position_sequence.to_le_bytes());
                session.send_unreliable(0x14cb, &stationary)?;
                position_sequence = position_sequence.wrapping_add(1);
                last_position = Instant::now();
            }
        }
        if lifecycle.blocks_motion() {
            if let Some(commands) = context.commands {
                for _ in commands.try_iter().take(64) {}
            }
        }
        if ready && !lifecycle.is_dead() && lifecycle.pending().is_none() {
            if let Some(commands) = context.commands {
                // Bound each pass so continuous producers cannot starve receive/ACK work.
                for command in commands.try_iter().take(64) {
                    // One table decides which in-flight actions a command must wait for.
                    if let Some(reason) = actions::Held::from_state(
                        &cast_guard,
                        &book_edits,
                        pending_memorization.as_ref(),
                        &trades,
                        scribe_consumption.awaiting_cursor(Instant::now()),
                    )
                    .conflict(&command)
                    {
                        actions::refuse(&command, reason, log)?;
                        continue;
                    }
                    let own_spawn = admitted_player.as_ref().map(|player| player.spawn_id);
                    if camp::handle(
                        (&mut camp, &mut own_posture),
                        session_id,
                        own_spawn,
                        &command,
                        &mut session,
                        log,
                    )? {
                        continue;
                    }
                    if let ClientCommand::ClickDoor {
                        session_id: requested,
                        door_id,
                        created,
                    } = &command
                    {
                        let result = (|| -> Result<[u8; 16]> {
                            let now = Instant::now();
                            ensure!(
                                *requested == session_id
                                    && *created >= doors_changed_at
                                    && *created <= now
                                    && now.duration_since(*created) < Duration::from_secs(1),
                                "stale door request"
                            );
                            let player = admitted_player
                                .as_ref()
                                .ok_or_else(|| anyhow::anyhow!("player is unavailable"))?;
                            let position = motion
                                .as_ref()
                                .map_or(player.position, MotionSession::position);
                            doors.click_packet(*door_id, player.spawn_id, position)
                        })();
                        let error = match result {
                            Ok(body) => {
                                session.send(0x043b, &body)?;
                                None
                            }
                            Err(error) => Some(error.to_string()),
                        };
                        log.send(ClientEvent::World(crate::world::WorldEvent::DoorAction {
                            session_id,
                            door_id: *door_id,
                            error,
                        }))?;
                        continue;
                    }
                    let player = admitted_player.as_ref().map(|player| {
                        let position = motion
                            .as_ref()
                            .map_or(player.position, MotionSession::position);
                        (player.spawn_id, position)
                    });
                    if ground.handle(&command, session_id, player, &inventory, &mut session, log)? {
                        continue;
                    }
                    if let ClientCommand::CrossZoneLine {
                        session_id: requested,
                        destination,
                        position,
                        created,
                    } = &command
                    {
                        let now = Instant::now();
                        let request = (|| -> Result<_> {
                            ensure!(
                                *requested == session_id
                                    && *created <= now
                                    && now.duration_since(*created) < Duration::from_millis(250),
                                "stale zone-line request"
                            );
                            ensure!(
                                motion.as_ref().is_some_and(|m| m.position() == *position),
                                "zone-line position is no longer current"
                            );
                            ensure!(current_zone.0 != 0, "zone identity is unavailable");
                            zone_points.request(
                                destination,
                                *position,
                                current_zone.0,
                                current_zone.1,
                            )
                        })();
                        match request {
                            Ok(offer) => {
                                session.send(0x5dd8, &offer.response(&config.character)?)?;
                                lifecycle.offer(offer.clone(), now)?;
                                pending_memorization = None;
                                if let Some(motion) = motion.as_mut() {
                                    motion.suspend();
                                }
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::ZoneTransfer(offer),
                                ))?;
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::MotionState {
                                        session_id,
                                        units_per_second: None,
                                        strafe_units_per_second: None,
                                        walk_units_per_second: None,
                                        backward_units_per_second: None,
                                        falls: false,
                                    },
                                ))?;
                                log.status(
                                    ConnectionState::Zoning,
                                    packets,
                                    Some(session.last_received_seconds()),
                                )?;
                                break;
                            }
                            Err(error) => {
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::ZoneLineRejected {
                                        session_id: *requested,
                                        reason: error.to_string(),
                                    },
                                ))?;
                            }
                        }
                        continue;
                    }
                    if matches!(
                        &command,
                        ClientCommand::Move(_)
                            | ClientCommand::CastSpell { .. }
                            | ClientCommand::UseItem(_)
                            | ClientCommand::SetPosture { .. }
                    ) && pending_memorization.take().is_some()
                    {
                        log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                            BookActionStatus::Cancelled(
                                "Movement, casting or posture changed".into(),
                            ),
                        )))?;
                    }
                    if let ClientCommand::ScribeSpell {
                        session_id: requested,
                        revision,
                        slot,
                        spell_id,
                        created,
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
                            .filter(|_| {
                                *requested == session_id
                                    && *created <= now
                                    && now.duration_since(*created) < Duration::from_secs(1)
                                    && pending_memorization.is_none()
                            })
                            .context("scribing is unavailable or stale")
                            .and_then(|book| pending.packet(book, &inventory));
                        match packet {
                            Ok(_) => {
                                if let Some(player) = admitted_player.as_ref() {
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
                                    own_posture.sent(
                                        player.spawn_id,
                                        command::Posture::Sitting,
                                        log,
                                    )?;
                                    pending_memorization = Some(pending);
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
                        session_id,
                        pending_memorization.is_some(),
                        Instant::now(),
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
                    if let ClientCommand::ForgetSpell {
                        session_id: requested,
                        gem,
                        spell_id,
                        created,
                    } = &command
                    {
                        let now = Instant::now();
                        let packet = admitted_player
                            .as_ref()
                            .filter(|_| {
                                *requested == session_id
                                    && *created <= now
                                    && now.duration_since(*created) < Duration::from_secs(1)
                            })
                            .context("forget request is unavailable or stale")
                            .and_then(|player| {
                                eq_network_game::spells::forget_packet(
                                    &player.memorized_spells,
                                    *gem,
                                    *spell_id,
                                )
                            });
                        match packet {
                            Ok(body) => {
                                pending_memorization = None;
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
                    if let ClientCommand::MemorizeSpell {
                        session_id: requested,
                        gem,
                        spell_id,
                        created,
                    } = &command
                    {
                        let valid = *requested == session_id
                            && *created <= Instant::now()
                            && created.elapsed() < Duration::from_secs(1)
                            && pending_memorization.is_none();
                        let packet = admitted_book
                            .as_ref()
                            .filter(|_| valid)
                            .context("memorization is unavailable or stale")
                            .and_then(|book| book.memorize_packet(*gem, *spell_id));
                        match packet {
                            Ok(_) => {
                                if let Some(player) = admitted_player.as_ref() {
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
                                    own_posture.sent(
                                        player.spawn_id,
                                        command::Posture::Sitting,
                                        log,
                                    )?;
                                    pending_memorization = Some(PendingBookAction {
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
                        let target_available = admitted_player.as_ref().is_some_and(|player| {
                            request.target_id == player.spawn_id
                                || initial_spawns.get(&request.target_id).is_some_and(
                                    |spawn: &crate::world::SpawnState| !spawn.invisible,
                                )
                        });
                        let prepared = inventory_actor
                            .as_ref()
                            .context("Character level is unavailable")
                            .and_then(|actor| {
                                inventory.prepare_item_cast(
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
                            session_id: requested,
                            gem,
                            spell_id,
                            target_id,
                            created,
                        } => {
                            *requested == session_id
                                && created.elapsed() < Duration::from_secs(1)
                                && *created <= Instant::now()
                                && admitted_player.as_ref().is_some_and(|player| {
                                    player.memorized_spells.get(usize::from(*gem))
                                        == Some(&Some(*spell_id))
                                        && (*target_id == player.spawn_id
                                            || initial_spawns.get(target_id).is_some_and(
                                                |spawn: &crate::world::SpawnState| !spawn.invisible,
                                            ))
                                })
                        }
                        ClientCommand::SetPosture {
                            session_id: requested,
                            spawn_id,
                            created,
                            ..
                        }
                        | ClientCommand::Consider {
                            session_id: requested,
                            own_id: spawn_id,
                            created,
                            ..
                        } => {
                            *requested == session_id
                                && created.elapsed() < Duration::from_secs(1)
                                && *created <= Instant::now()
                                && admitted_player
                                    .as_ref()
                                    .is_some_and(|player| player.spawn_id == *spawn_id)
                                && match &command {
                                    ClientCommand::Consider { target_id, .. } => {
                                        initial_spawns.get(target_id).is_some_and(
                                            |spawn: &crate::world::SpawnState| !spawn.invisible,
                                        )
                                    }
                                    _ => true,
                                }
                        }
                        ClientCommand::AutoAttack {
                            session_id: requested,
                            created,
                            ..
                        }
                        | ClientCommand::LootItem {
                            session_id: requested,
                            created,
                            ..
                        }
                        | ClientCommand::Buy {
                            session_id: requested,
                            created,
                            ..
                        }
                        | ClientCommand::Sell {
                            session_id: requested,
                            created,
                            ..
                        } => {
                            *requested == session_id
                                && created.elapsed() < Duration::from_secs(1)
                                && *created <= Instant::now()
                        }
                        ClientCommand::EndLoot {
                            session_id: requested,
                            ..
                        } => *requested == session_id,
                        ClientCommand::Loot {
                            session_id: requested,
                            corpse_id: entity,
                            created,
                        }
                        | ClientCommand::Shop {
                            session_id: requested,
                            merchant_id: entity,
                            created,
                            ..
                        } => {
                            *requested == session_id
                                && created.elapsed() < Duration::from_secs(1)
                                && *created <= Instant::now()
                                && initial_spawns.get(entity).is_some_and(
                                    |spawn: &crate::world::SpawnState| {
                                        !spawn.invisible
                                            && if matches!(command, ClientCommand::Loot { .. }) {
                                                matches!(
                                                    spawn.kind,
                                                    crate::world::SpawnKind::NpcCorpse
                                                        | crate::world::SpawnKind::PlayerCorpse
                                                )
                                            } else {
                                                spawn.kind == crate::world::SpawnKind::Npc
                                            }
                                    },
                                )
                        }
                        _ => true,
                    };
                    if !action_valid {
                        if let Some(event) = casting::rejected(
                            &command,
                            "Request expired, spell gem changed, or target is unavailable",
                        ) {
                            log.send(ClientEvent::World(event))?;
                        }
                        log.diagnostic(
                            "Rejected stale or unavailable spell/posture action".into(),
                        )?;
                        continue;
                    }
                    if inventory::handle(
                        &mut inventory,
                        &mut settlement,
                        inventory_actor.map(|mut actor| {
                            actor.bank_access = admitted_player.as_ref().is_some_and(|player| {
                                let position = motion
                                    .as_ref()
                                    .map_or(player.position, MotionSession::position);
                                initial_spawns.values().any(|spawn| {
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
                    if let Some(motion) = motion.as_mut() {
                        let own = (
                            &mut own_posture,
                            admitted_player.as_ref().map(|player| player.spawn_id),
                        );
                        if motion::handle(motion, own, session_id, &command, &mut session, log)? {
                            continue;
                        }
                    }
                    if matches!(&command, ClientCommand::InspectItem { session_id: requested, .. } if *requested != session_id)
                    {
                        log.diagnostic("Rejected item inspection from an old session".into())?;
                        continue;
                    }
                    if let ClientCommand::SelectTarget {
                        session_id: requested_session,
                        spawn_id,
                    } = &command
                    {
                        if *requested_session != session_id
                            || spawn_id.is_some_and(|id| {
                                admitted_player
                                    .as_ref()
                                    .is_none_or(|player| player.spawn_id != id)
                                    && initial_spawns.get(&id).is_none_or(
                                        |spawn: &crate::world::SpawnState| spawn.invisible,
                                    )
                            })
                        {
                            log.send(ClientEvent::World(
                                crate::world::WorldEvent::TargetRejected {
                                    session_id: *requested_session,
                                    spawn_id: *spawn_id,
                                    reason: "Target is stale, invisible, or unavailable".into(),
                                },
                            ))?;
                            log.diagnostic("Rejected stale or unavailable target".into())?;
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
                                own_posture.sent(*spawn_id, *posture, log)?;
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
        if !lifecycle.is_dead() && lifecycle.pending().is_none() {
            if pending_memorization
                .as_ref()
                .is_some_and(|pending| pending.ready(Instant::now()))
            {
                if let Some(pending) = pending_memorization.take() {
                    let packet = admitted_book
                        .as_ref()
                        .context("spellbook unavailable")
                        .and_then(|book| pending.packet(book, &inventory));
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
                &mut pending_memorization,
                "Character died or zone transfer started",
                log,
            )?;
        }
        let Some(mut packet) = session.receive()? else {
            continue;
        };
        packets += 1;
        if let Some(shield) = shield.as_ref() {
            shield.spawns(packet.opcode, &mut packet.body, &credentials.key)?;
        }
        if !ready {
            log.diagnostic(format!(
                "Zone received 0x{:04x} ({} bytes)",
                packet.opcode,
                packet.body.len()
            ))?;
        }
        if ready {
            match eq_network_game::spells::decode(packet.opcode, &packet.body) {
                Ok(Some(update)) => {
                    let book_result = book_edits.observe(&update);
                    scribe_consumption
                        .observe(&update, book_result == Some(BookActionStatus::Confirmed));
                    if let Some(player) = admitted_player.as_ref() {
                        let pending = cast_guard.pending();
                        cast_guard.observe(player.spawn_id, &update);
                        if pending.is_some() && cast_guard.pending().is_none() {
                            log.send(ClientEvent::World(crate::world::WorldEvent::CastPending {
                                session_id,
                                spell_id: None,
                            }))?;
                        }
                        if cast_guard.active() && pending_memorization.take().is_some() {
                            log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                                BookActionStatus::Cancelled("Casting started".into()),
                            )))?;
                        }
                    }
                    if let Some(book) = admitted_book.as_mut() {
                        book.apply(&update);
                    }
                    if let Some(player) = admitted_player.as_mut() {
                        update.apply_gems(&mut player.memorized_spells);
                    }
                    log.send(ClientEvent::World(crate::world::WorldEvent::Spell(update)))?;
                    if let Some(status) = book_result {
                        log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                            status,
                        )))?;
                    }
                }
                Ok(None) => (),
                Err(error) => log.diagnostic(format!("Invalid spell notification: {error}"))?,
            }
        }
        if matches!(packet.opcode, 0x5394 | 0x3397 | 0x420f | 0x4d81 | 0x1c4a) {
            match eq_network_game::inventory::decode(packet.opcode, &packet.body) {
                Ok(Some(update)) => {
                    let update = scribe_consumption.reconcile(&inventory, update);
                    inventory.apply(update.clone());
                    if ready {
                        log.send(ClientEvent::World(crate::world::WorldEvent::Inventory(
                            update,
                        )))?;
                    }
                    continue;
                }
                Err(error) => {
                    log.diagnostic(format!("Inventory decode rejected: {error}"))?;
                    let update = eq_network_game::inventory::InventoryUpdate::Invalidated;
                    inventory.apply(update.clone());
                    if ready {
                        log.send(ClientEvent::World(crate::world::WorldEvent::Inventory(
                            update,
                        )))?;
                    }
                    continue;
                }
                Ok(None) => (),
            }
        }
        if packet.opcode == 0x3eba {
            match zoning::ZonePoints::decode(&packet.body) {
                Ok(points) => zone_points = points,
                Err(error) => {
                    zone_points = zoning::ZonePoints::default();
                    log.diagnostic(format!("Zone-point table rejected: {error}"))?;
                }
            }
            continue;
        }
        if ready && matches!(packet.opcode, 0x385e | 0x7834) {
            let offer = zoning::offer(packet.opcode, &packet.body)?;
            if let Some(position) = offer.local_position(current_zone) {
                ensure!(
                    !lifecycle.blocks_motion(),
                    "same-zone relocation conflicts with death or transfer"
                );
                spellbook::cancel_pending(
                    &mut pending_memorization,
                    "Server relocated character",
                    log,
                )?;
                let player = admitted_player
                    .as_mut()
                    .context("relocation without admitted player")?;
                motion::correct_own(
                    player,
                    motion.as_mut(),
                    &mut stationary,
                    position_sequence,
                    position,
                    Instant::now(),
                )?;
                log.send(ClientEvent::World(crate::world::WorldEvent::MotionState {
                    session_id,
                    units_per_second: None,
                    strafe_units_per_second: None,
                    walk_units_per_second: None,
                    backward_units_per_second: None,
                    falls: false,
                }))?;
                log.send(ClientEvent::World(crate::world::WorldEvent::Position {
                    spawn_id: player.spawn_id,
                    position,
                    velocity: [0.0; 3],
                }))?;
                continue;
            }
            if let Some(pending) = lifecycle.pending() {
                ensure!(pending == &offer, "conflicting zone transfer offer");
            } else {
                session.send(0x5dd8, &offer.response(&config.character)?)?;
                spellbook::cancel_pending(&mut pending_memorization, "Zone transfer started", log)?;
                log.send(ClientEvent::World(crate::world::WorldEvent::ZoneTransfer(
                    offer.clone(),
                )))?;
                log.status(
                    ConnectionState::Zoning,
                    packets,
                    Some(session.last_received_seconds()),
                )?;
                lifecycle.offer(offer, Instant::now())?;
                if let Some(motion) = motion.as_mut() {
                    motion.suspend();
                }
                log.send(ClientEvent::World(crate::world::WorldEvent::MotionState {
                    session_id,
                    units_per_second: None,
                    strafe_units_per_second: None,
                    walk_units_per_second: None,
                    backward_units_per_second: None,
                    falls: false,
                }))?;
            }
            continue;
        }
        if camp.logging_out() && packet.opcode == camp::LOGOUT_REPLY_OPCODE {
            log.send(ClientEvent::World(crate::world::WorldEvent::Camp(
                crate::world::CampStatus::Camped,
            )))?;
            session.close()?;
            return Ok(ZoneExit::CharacterSelect);
        }
        if ready && packet.opcode == 0x5dd8 {
            let pending = lifecycle
                .pending()
                .context("zone approval without a pending server offer")?;
            let heading = motion
                .as_ref()
                .map_or(0.0, |motion| motion.position().heading);
            let reply = zoning::reply(
                &packet.body,
                &config.character,
                pending,
                current_zone,
                heading,
            )?;
            if reply == zoning::ZoneReply::Approved {
                lifecycle.finish(true)?;
                log.send(ClientEvent::Progress(ConnectionStage::ConnectingWorld))?;
                session.close()?;
                return Ok(ZoneExit::World);
            }
            lifecycle.finish(false)?;
            let reason = match &reply {
                zoning::ZoneReply::Denied(reason) => *reason,
                zoning::ZoneReply::Rewind(_) => zoning::ZoneRejection::Cancelled,
                zoning::ZoneReply::Approved => unreachable!("approved transfer returned above"),
            };
            if let zoning::ZoneReply::Rewind(position) = reply {
                let player = admitted_player
                    .as_mut()
                    .context("rewind without admitted player")?;
                motion::correct_own(
                    player,
                    motion.as_mut(),
                    &mut stationary,
                    position_sequence,
                    position,
                    Instant::now(),
                )?;
                log.send(ClientEvent::World(crate::world::WorldEvent::Position {
                    spawn_id: u16::from_le_bytes([stationary[0], stationary[1]]),
                    position,
                    velocity: [0.0; 3],
                }))?;
            }
            log.send(ClientEvent::World(
                crate::world::WorldEvent::ZoneTransferRejected { session_id, reason },
            ))?;
            if !lifecycle.is_dead() {
                if let Some(motion) = motion.as_mut() {
                    motion.resume_stationary(Instant::now());
                }
                log.status(
                    ConnectionState::Connected,
                    packets,
                    Some(session.last_received_seconds()),
                )?;
            }
            continue;
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
                stationary[4..8].copy_from_slice(&packet.body[13120..13124]);
                stationary[24..28].copy_from_slice(&packet.body[13116..13120]);
                stationary[28..32].copy_from_slice(&packet.body[13124..13128]);
                let heading = f32::from_le_bytes(packet.body[13128..13132].try_into().unwrap());
                let heading = crate::world::profile_heading(heading, revolution) * 8.0;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let heading = heading as u16;
                stationary[32..34].copy_from_slice(&(heading & 0x0fff).to_le_bytes());
                profile_data.clone_from(&packet.body);
                initial_skills.clear();
                initial_level = None;
                current_zone = (
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
                stationary[..2].copy_from_slice(&id.to_le_bytes());
                spawn_data.clone_from(&packet.body);
                saw_spawn = true;
            }
            ZoneOpcode::ZoneDescription if !ready => {
                ensure!(packet.body.len() >= 96, "truncated zone description");
                zone_name = String::from_utf8_lossy(cstr(&packet.body[64..96])).into_owned();
                far_clip = crate::world::titanium_far_clip(&packet.body);
                log.zone.clone_from(&zone_name);
                log.status(
                    ConnectionState::Zoning,
                    packets,
                    Some(session.last_received_seconds()),
                )?;
                got_zone = true;
                session.send(0x067a, &0u32.to_le_bytes())?;
                session.send(0x5e3a, &0u32.to_le_bytes())?;
                session.send(0x7752, &0u32.to_le_bytes())?;
                session.send(0x0322, &[])?;
            }
            ZoneOpcode::ExperienceUpdate if got_zone && !ready && !replied_experience => {
                session.send(0x0587, &[])?;
                replied_experience = true;
            }
            ZoneOpcode::ExperienceUpdate if got_zone && !ready && replied_experience => {
                session.send(0x6563, &chat::server_filters())?;
                session.send(0x5e20, &[])?;
                session.send(0x0c11, &1u32.to_le_bytes())?;
                ready = true;
                match crate::world::titanium_player(&profile_data, &spawn_data, revolution) {
                    Ok(mut player) => {
                        if let Some(level) = initial_level.take() {
                            player.level = level;
                        }
                        player.appearance.show_helm |= !server.helm_choice();
                        for change in std::mem::take(&mut initial_own_wear) {
                            player.appearance.apply(&change);
                        }
                        for (skill, value) in std::mem::take(&mut initial_skills) {
                            player.apply_skill(skill, value);
                        }
                        admitted_player = Some(player.clone());
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
                        motion = Some(
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
                        log.send(ClientEvent::World(crate::world::WorldEvent::Spawns(
                            initial_spawns.values().cloned().collect(),
                        )))?;
                        own_posture = posture::OwnPosture::default();
                        for (spawn_id, posture) in std::mem::take(&mut initial_postures) {
                            if spawn_id == u16::from_le_bytes([stationary[0], stationary[1]]) {
                                own_posture.observed(posture);
                            }
                            log.send(ClientEvent::World(crate::world::WorldEvent::Posture {
                                spawn_id,
                                posture,
                            }))?;
                        }
                        log.send(ClientEvent::World(crate::world::WorldEvent::Doors(
                            doors.admission(),
                        )))?;
                        log.send(ClientEvent::World(crate::world::WorldEvent::Objects(
                            ground.admission(),
                        )))?;
                        let admission = inventory.admission_updates();
                        log.send(ClientEvent::World(crate::world::WorldEvent::BuffSnapshot(
                            eq_network_game::buffs::titanium_profile(&profile_data)?,
                        )))?;
                        log.send(ClientEvent::World(crate::world::WorldEvent::SpellBook(
                            spell_book,
                        )))?;
                        log.send(ClientEvent::World(crate::world::WorldEvent::Coins(
                            crate::world::titanium_coins(&profile_data)?,
                        )))?;
                        inventory = eq_network_game::inventory::Inventory::default();
                        for update in admission {
                            inventory.apply(update.clone());
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
                    packets,
                    Some(session.last_received_seconds()),
                )?;
                log.diagnostic(format!("Zone login sequence complete for {zone_name}; waiting for ongoing server traffic"))?;
            }
            ZoneOpcode::ValidationRejected => bail!("server rejected zone validation"),
            ZoneOpcode::LoggedOut => bail!("server logged the character out"),
            ZoneOpcode::ZoneHandoff => {
                ensure!(
                    ready && lifecycle.pending().is_some(),
                    "zone handoff without a pending transfer"
                );
                log.send(ClientEvent::Progress(ConnectionStage::ConnectingZone))?;
                session.close()?;
                return Ok(ZoneExit::Direct(packet.body));
            }
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
        if !ready && matches!(packet.opcode, 0x4c24 | 0x700d | 0x77d0) {
            match eq_network_game::doors::decode(packet.opcode, &packet.body) {
                Ok(Some(update)) => doors.apply(&update),
                Err(error) => log.diagnostic(format!("Initial door update rejected: {error}"))?,
                Ok(None) => (),
            }
        }
        if !ready
            && matches!(
                packet.opcode,
                eq_network_game::objects::SPAWN_OPCODE | eq_network_game::objects::CLICK_OPCODE
            )
        {
            match eq_network_game::objects::decode(packet.opcode, &packet.body) {
                Ok(Some(update)) => ground.apply(&update),
                Err(error) => log.diagnostic(format!("Initial ground object rejected: {error}"))?,
                Ok(None) => (),
            }
        }
        if !ready && packet.opcode == 0x6a93 {
            match crate::world::titanium_update(packet.opcode, &packet.body) {
                Ok(Some(crate::world::WorldEvent::Skill { skill_id, value })) if skill_id < 100 => {
                    initial_skills.insert(skill_id, value);
                }
                Err(error) => log.diagnostic(format!("Initial skill update rejected: {error}"))?,
                _ => (),
            }
        }
        if !ready && matches!(packet.opcode, 0x5ecd | 0x6d44) {
            match crate::world::titanium_update(packet.opcode, &packet.body) {
                Ok(Some(crate::world::WorldEvent::Level {
                    current,
                    experience,
                    ..
                })) => {
                    initial_level = Some(current);
                    initial_experience = Some(experience);
                }
                Ok(Some(crate::world::WorldEvent::Experience(value))) => {
                    initial_experience = Some(value);
                }
                Err(error) => log.diagnostic(format!("Initial experience rejected: {error}"))?,
                _ => (),
            }
        }
        if !ready
            && matches!(
                packet.opcode,
                0x2e78
                    | 0x1860
                    | 0x55bc
                    | 0x14cb
                    | 0x7c32
                    | eq_network_game::appearance::WEAR_CHANGE_OPCODE
            )
        {
            let update = crate::world::titanium_update(packet.opcode, &packet.body);
            match update.map(|event| {
                event.map(|mut event| {
                    servers::show_helms(server, &mut event);
                    event
                })
            }) {
                Ok(Some(crate::world::WorldEvent::WearChange(change))) => {
                    if let Some(spawn) = initial_spawns.get_mut(&change.spawn_id) {
                        spawn.appearance.apply(&change);
                    }
                    if change.spawn_id == u16::from_le_bytes([stationary[0], stationary[1]]) {
                        initial_own_wear.push(change);
                    }
                }
                Ok(Some(crate::world::WorldEvent::Spawns(spawns))) => {
                    for spawn in spawns {
                        initial_spawns.insert(spawn.spawn_id, spawn);
                    }
                    ensure!(initial_spawns.len() <= 65535, "zone entity limit exceeded");
                }
                Ok(Some(crate::world::WorldEvent::Despawn(id))) => {
                    initial_spawns.remove(&id);
                    initial_postures.remove(&id);
                }
                Ok(Some(crate::world::WorldEvent::Posture { spawn_id, posture })) => {
                    initial_postures.insert(spawn_id, posture);
                }
                Ok(Some(crate::world::WorldEvent::Visibility {
                    spawn_id,
                    invisible,
                })) => {
                    if let Some(spawn) = initial_spawns.get_mut(&spawn_id) {
                        spawn.invisible = invisible;
                    }
                }
                Ok(Some(crate::world::WorldEvent::Position {
                    spawn_id,
                    position,
                    velocity,
                })) => {
                    if let Some(spawn) = initial_spawns.get_mut(&spawn_id) {
                        spawn.position = position;
                        spawn.velocity = velocity;
                    }
                }
                Err(error) => log.diagnostic(format!("Initial entity decode rejected: {error}"))?,
                _ => (),
            }
        }
        if ready {
            let update = crate::world::titanium_update(packet.opcode, &packet.body);
            match update.map(|event| {
                event.map(|mut event| {
                    servers::show_helms(server, &mut event);
                    event
                })
            }) {
                Ok(Some(event)) => {
                    match &event {
                        crate::world::WorldEvent::Posture { spawn_id, posture }
                            if *spawn_id == u16::from_le_bytes([stationary[0], stationary[1]]) =>
                        {
                            own_posture.observed(*posture);
                            if *posture != crate::world::PostureState::Sitting {
                                spellbook::cancel_pending(
                                    &mut pending_memorization,
                                    "Server changed character posture",
                                    log,
                                )?;
                            }
                        }
                        crate::world::WorldEvent::Visibility {
                            spawn_id,
                            invisible,
                        } => {
                            if let Some(spawn) = initial_spawns.get_mut(spawn_id) {
                                spawn.invisible = *invisible;
                            }
                        }
                        crate::world::WorldEvent::WearChange(change) => {
                            if let Some(spawn) = initial_spawns.get_mut(&change.spawn_id) {
                                spawn.appearance.apply(change);
                            }
                            if let Some(player) = admitted_player
                                .as_mut()
                                .filter(|player| player.spawn_id == change.spawn_id)
                            {
                                player.appearance.apply(change);
                            }
                        }
                        crate::world::WorldEvent::Doors(update) => {
                            if matches!(
                                update,
                                eq_network_game::doors::DoorUpdate::Spawn(_)
                                    | eq_network_game::doors::DoorUpdate::RemoveAll
                            ) {
                                doors_changed_at = Instant::now();
                            }
                            doors.apply(update);
                        }
                        crate::world::WorldEvent::Objects(update) => {
                            let own = admitted_player.as_ref().map(|player| player.spawn_id);
                            ground.observe(update, own, &mut session, log)?;
                        }
                        crate::world::WorldEvent::Level { current, .. } => {
                            if let Some(player) = admitted_player.as_mut() {
                                player.level = *current;
                            }
                            if let Some(actor) = inventory_actor.as_mut() {
                                actor.level = *current;
                            }
                        }
                        crate::world::WorldEvent::Skill { skill_id, value } => {
                            if let Some(player) = admitted_player.as_mut() {
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
                            if death.spawn_id
                                == u32::from(u16::from_le_bytes([
                                    stationary[0],
                                    stationary[1],
                                ])) =>
                        {
                            lifecycle.mark_dead();
                            if camp.cancel() {
                                log.send(ClientEvent::World(crate::world::WorldEvent::Camp(
                                    crate::world::CampStatus::Abandoned,
                                )))?;
                            }
                            spellbook::cancel_pending(
                                &mut pending_memorization,
                                "Character died",
                                log,
                            )?;
                            cast_guard.clear();
                            trades.clear();
                            if let Some(motion) = motion.as_mut() {
                                motion.suspend();
                            }
                            log.send(ClientEvent::World(crate::world::WorldEvent::MotionState {
                                session_id,
                                units_per_second: None,
                                strafe_units_per_second: None,
                                walk_units_per_second: None,
                                backward_units_per_second: None,
                                falls: false,
                            }))?;
                            log.diagnostic(
                                "Own character died; waiting for the server bind offer".into(),
                            )?;
                        }
                        crate::world::WorldEvent::Spawns(spawns) => {
                            for spawn in spawns {
                                initial_spawns.insert(spawn.spawn_id, spawn.clone());
                            }
                        }
                        crate::world::WorldEvent::Despawn(id) => {
                            initial_spawns.remove(id);
                        }
                        // Another entity died: its corpse keeps the spawn ID.
                        crate::world::WorldEvent::Death(death) => {
                            if let Some(spawn) = u16::try_from(death.spawn_id)
                                .ok()
                                .and_then(|id| initial_spawns.get_mut(&id))
                            {
                                spawn.kind = spawn.kind.corpse();
                            }
                        }
                        // A sale's echo is the only notice that the item left.
                        crate::world::WorldEvent::Merchant(update) => {
                            if let Some(change) = trades.observe(update) {
                                inventory.apply(change.clone());
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
                        if *spawn_id == u16::from_le_bytes([stationary[0], stationary[1]]) {
                            spellbook::cancel_pending(
                                &mut pending_memorization,
                                "Server corrected character position",
                                log,
                            )?;
                            let player = admitted_player
                                .as_mut()
                                .context("correction without admitted player")?;
                            motion::correct_own(
                                player,
                                motion.as_mut(),
                                &mut stationary,
                                position_sequence,
                                *position,
                                Instant::now(),
                            )?;
                            log.send(ClientEvent::World(crate::world::WorldEvent::MotionState {
                                session_id,
                                units_per_second: None,
                                strafe_units_per_second: None,
                                walk_units_per_second: None,
                                backward_units_per_second: None,
                                falls: false,
                            }))?;
                        }
                    }
                    log.send(ClientEvent::World(event))?;
                }
                Ok(None) => (),
                Err(error) => log.diagnostic(format!("World-state decode rejected: {error}"))?,
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
