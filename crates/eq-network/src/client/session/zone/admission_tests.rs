//! Scripted zone transports exercise the handshake and feature gates without sockets.
use super::*;
use crate::client::{
    session::login::Credentials, CancellationToken, ClientConfig, ClientIdentity, ServerProtocol,
};
use eq_network_game::{
    chat::OutboundChat,
    list_poll,
    movement::{MovementMode, MovementRequest, POSITION_OPCODE},
    world::WorldEvent,
    zoning::{DEPART_OPCODE, HANDOFF_OPCODE, MOVE_OPCODE},
};
use eq_network_transport::Application;
use std::{
    cell::Cell,
    collections::VecDeque,
    sync::mpsc::{self, Receiver},
};

mod aa_readiness_tests;

struct Script {
    incoming: VecDeque<Application>,
    stop_when_drained: Option<CancellationToken>,
    sent: Vec<u16>,
    /// Outgoing AA requests, distinct from incoming timer facts on this opcode.
    aa_actions: Vec<Vec<u8>>,
    /// Outgoing AA count immediately before each exact ownership list is read.
    aa_actions_at_selections: Vec<usize>,
    /// The bodies of the server filter packets sent.
    filters: Vec<Vec<u8>>,
    closed: bool,
    refuse_list_reply: bool,
    /// Once drained, the script stays idle until then before it stops.
    idle_until: Option<Instant>,
}

impl Script {
    fn titanium(run_speed: f32) -> Self {
        let mut profile = vec![0; 19592];
        profile[12940..12946].copy_from_slice(b"Tester");
        profile[13276..13278].copy_from_slice(&1u16.to_le_bytes());
        let mut spawn = vec![0; 385];
        spawn[7..13].copy_from_slice(b"Tester");
        spawn[233..237].copy_from_slice(&run_speed.to_le_bytes());
        spawn[340..344].copy_from_slice(&7u32.to_le_bytes());
        let mut header = vec![0; 96];
        header[64..71].copy_from_slice(b"example");
        let mut script = Self {
            incoming: VecDeque::new(),
            stop_when_drained: None,
            sent: Vec::new(),
            aa_actions: Vec::new(),
            aa_actions_at_selections: Vec::new(),
            filters: Vec::new(),
            closed: false,
            refuse_list_reply: false,
            idle_until: None,
        };
        for (opcode, body) in [
            (0x75df, profile),
            (0x7213, spawn),
            (0x254d, vec![0; 12]),
            (0x0920, header),
            (0x1580, vec![13, 0, 1, 1, 0x1c, 0x0c, 0, 0]),
            (0x0587, vec![]),
            (0x0587, vec![]),
        ] {
            script.packet(opcode, body);
        }
        script
    }

    fn packet(&mut self, opcode: u16, body: Vec<u8>) {
        self.incoming.push_back(Application { opcode, body });
    }

    fn transfer(&mut self) -> Vec<u8> {
        let mut offer = vec![0; 24];
        offer[..2].copy_from_slice(&2u16.to_le_bytes());
        self.packet(MOVE_OPCODE, offer);
        let mut address = vec![0; 130];
        address[..9].copy_from_slice(b"127.0.0.1");
        address[128..].copy_from_slice(&7000u16.to_le_bytes());
        self.packet(HANDOFF_OPCODE, address.clone());
        address
    }
}

impl Transport for Script {
    fn send(&mut self, opcode: u16, body: &[u8]) -> Result<()> {
        assert!(!self.closed);
        self.sent.push(opcode);
        if opcode == eq_network_game::alternate_advancement::ACTION_OPCODE {
            self.aa_actions.push(body.to_vec());
        }
        if opcode == eq_network_game::filters::TITANIUM_OPCODE {
            self.filters.push(body.to_vec());
        }
        if opcode == list_poll::OPCODE {
            assert_eq!(body, [3, 0, 0, 0, 0]);
            ensure!(!self.refuse_list_reply, "scripted list reply send failure");
        }
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Application>> {
        if let Some(packet) = self.incoming.pop_front() {
            if packet.opcode == eq_network_game::alternate_advancement::SELECTIONS_OPCODE {
                self.aa_actions_at_selections.push(self.aa_actions.len());
            }
            return Ok(Some(packet));
        }
        if self.idle_until.is_some_and(|until| Instant::now() < until) {
            std::thread::sleep(Duration::from_millis(5));
            return Ok(None);
        }
        // The scripted idle poll cancels once. The loop checks cancellation
        // before calling receive again, so another poll is unexpected.
        self.stop_when_drained
            .take()
            .expect("unexpected receive")
            .cancel();
        Ok(None)
    }

    fn close(&mut self) -> Result<()> {
        self.closed = true;
        Ok(())
    }

    fn last_received_seconds(&self) -> u64 {
        0
    }
}

fn config() -> ClientConfig {
    ClientConfig::for_protocol(
        ServerProtocol::EqEmu,
        "ACCOUNT",
        "PASSWORD",
        "Test Server",
        "Tester",
    )
}

fn context<'a>(
    config: &ClientConfig,
    identity: &'a ClientIdentity,
    credentials: &'a Credentials,
    stop: &'a CancellationToken,
    commands: Option<&'a Receiver<ClientCommand>>,
) -> CharacterSession<'a> {
    CharacterSession {
        config: config.clone(),
        identity,
        credentials,
        stop,
        duration: None,
        commands,
        sysinfo_reported_at: Cell::new(0),
        answer_list_polls: Cell::new(false),
        server_filters: std::cell::RefCell::default(),
        riding: std::sync::Arc::default(),
        chat: std::sync::Arc::default(),
        chat_pace: std::sync::Arc::default(),
        world_ruleset: std::cell::Cell::new(None),
    }
}

fn credentials() -> Credentials {
    Credentials {
        account: "LS#1".into(),
        key: *b"0123456789",
    }
}

fn run_script(
    context: &CharacterSession<'_>,
    script: &mut Script,
    log: &mut Events<'_>,
) -> Result<ZoneExit> {
    run_connected(
        context,
        servers::server_type(context.config.protocol),
        script,
        &mut None,
        Vec::new(),
        log,
        Instant::now,
    )
}

#[test]
fn a_failed_due_list_reply_refuses_every_move_drained_by_the_zone_loop() {
    let mut config = config();
    config.protocol = ServerProtocol::Project1999;
    let identity = ClientIdentity::new("example-host", "example-user");
    let credentials = credentials();
    let stop = CancellationToken::default();
    let (sender, commands) = mpsc::channel();
    let context = context(&config, &identity, &credentials, &stop, Some(&commands));
    let mut script = Script::titanium(1.0);
    script.refuse_list_reply = true;
    script.packet(list_poll::OPCODE, vec![2]);
    let clock = Cell::new(Instant::now());
    let mut admitted = None;
    let mut requested = Vec::new();
    let mut refused = Vec::new();
    let mut handler = |event| {
        match event {
            ClientEvent::World(WorldEvent::Entered {
                session_id, player, ..
            }) => {
                admitted = Some((session_id, player.position));
                sender.send(ClientCommand::AnswerListPolls {
                    session_id,
                    enabled: true,
                })?;
            }
            ClientEvent::Diagnostic(line) if line == "Received a /list presence poll." => {
                let (session_id, position) = admitted.expect("poll after admission");
                let created = Instant::now();
                for (session_id, created) in [
                    (session_id, created),
                    (
                        session_id.wrapping_add(1),
                        created + Duration::from_millis(1),
                    ),
                ] {
                    requested.push((session_id, created));
                    sender.send(ClientCommand::Move(MovementRequest::new(
                        MovementMode::Forward,
                        session_id,
                        position,
                        created,
                    )))?;
                }
                clock.set(clock.get() + Duration::from_secs(61));
            }
            ClientEvent::World(WorldEvent::MotionRefused {
                session_id,
                created,
                reason,
            }) => {
                assert_eq!(reason, "The zone connection ended");
                refused.push((session_id, created));
            }
            ClientEvent::World(WorldEvent::MotionSent { .. }) => {
                panic!("no command can reach the movement owner");
            }
            ClientEvent::Diagnostic(line) if line.starts_with("Sent a /list presence reply") => {
                panic!("the refused transport send cannot be reported as sent");
            }
            _ => (),
        }
        Ok(())
    };
    let result = run_connected(
        &context,
        servers::server_type(config.protocol),
        &mut script,
        &mut None,
        Vec::new(),
        &mut Events::new(&config, &mut handler),
        || clock.get(),
    );
    assert_eq!(
        result.err().unwrap().to_string(),
        "scripted list reply send failure"
    );
    assert_eq!(script.sent.last(), Some(&list_poll::OPCODE));
    assert_eq!(requested.len(), 2);
    assert_eq!(refused, requested);
    assert!(commands.try_recv().is_err());
}

#[test]
fn a_zone_transfer_keeps_the_list_choice_without_sending_the_old_reply() {
    let mut config = config();
    config.protocol = ServerProtocol::Project1999;
    let identity = ClientIdentity::new("example-host", "example-user");
    let credentials = credentials();
    let stop = CancellationToken::default();
    let (_sender, commands) = mpsc::channel();
    let mut context = context(&config, &identity, &credentials, &stop, Some(&commands));
    context.answer_list_polls.set(true);
    let mut first = Script::titanium(1.0);
    first.packet(list_poll::OPCODE, vec![2]);
    let address = first.transfer();
    let clock = Cell::new(Instant::now());
    let mut admissions = Vec::new();
    let mut polls = 0;
    let mut handler = |event| {
        match event {
            ClientEvent::World(WorldEvent::Entered { session_id, .. }) => {
                admissions.push(session_id);
            }
            ClientEvent::Diagnostic(line) if line == "Received a /list presence poll." => {
                polls += 1;
                if polls == 2 {
                    clock.set(clock.get() + Duration::from_secs(61));
                }
            }
            _ => (),
        }
        Ok(())
    };
    let server = servers::server_type(config.protocol);
    let mut log = Events::new(&config, &mut handler);
    let exit = run_connected(
        &context,
        server,
        &mut first,
        &mut None,
        Vec::new(),
        &mut log,
        || clock.get(),
    )
    .unwrap();
    assert!(matches!(&exit, ZoneExit::Direct(next) if *next == address));
    assert!(!first.sent.contains(&list_poll::OPCODE));
    context.on_zone_exit(&exit);
    clock.set(clock.get() + Duration::from_secs(61));

    let mut second = Script::titanium(1.0);
    second.packet(list_poll::OPCODE, vec![2]);
    second.stop_when_drained = Some(stop.clone());
    let exit = run_connected(
        &context,
        server,
        &mut second,
        &mut None,
        Vec::new(),
        &mut log,
        || clock.get(),
    )
    .unwrap();
    assert!(matches!(exit, ZoneExit::Stopped));
    assert_eq!(polls, 2);
    assert!(matches!(admissions.as_slice(), [old, new] if old != new));
    assert_eq!(
        second
            .sent
            .iter()
            .filter(|&&code| code == list_poll::OPCODE)
            .count(),
        1
    );
}

#[test]
fn the_connected_zone_delivers_one_result_for_each_queued_move() {
    let config = config();
    let identity = ClientIdentity::new("example-host", "example-user");
    let credentials = credentials();
    let stop = CancellationToken::default();
    let (sender, commands) = mpsc::channel();
    let context = context(&config, &identity, &credentials, &stop, Some(&commands));
    let mut script = Script::titanium(1.0);
    script.stop_when_drained = Some(stop.clone());
    let mut expected = None;
    let mut results = Vec::new();
    let mut handler = |event| {
        match event {
            ClientEvent::World(WorldEvent::Entered {
                session_id, player, ..
            }) => {
                let other = session_id.wrapping_add(1);
                let created = Instant::now();
                expected = Some([(other, created), (session_id, created)]);
                for visit in [other, session_id] {
                    sender.send(ClientCommand::Move(MovementRequest::new(
                        MovementMode::Forward,
                        visit,
                        player.position,
                        created,
                    )))?;
                }
            }
            ClientEvent::World(
                WorldEvent::MotionRefused {
                    session_id,
                    created,
                    ..
                }
                | WorldEvent::MotionSent {
                    session_id,
                    created,
                    refused: Some(_),
                    ..
                },
            ) => {
                results.push((session_id, created));
            }
            ClientEvent::World(WorldEvent::MotionSent { refused: None, .. }) => {
                panic!("the synthetic host provided no speed inputs");
            }
            _ => (),
        }
        Ok(())
    };
    assert!(matches!(
        run_script(
            &context,
            &mut script,
            &mut Events::new(&config, &mut handler)
        )
        .unwrap(),
        ZoneExit::Stopped
    ));
    assert_eq!(results, expected.unwrap());
    assert!(commands.try_recv().is_err());
}

fn assert_failed_admission(
    result: Result<ZoneExit>,
    script: &Script,
    events: &[ClientEvent],
    commands: &Receiver<ClientCommand>,
) {
    assert!(result.is_err_and(|error| error.is::<PlayerAdmissionError>()));
    assert!(script.closed);
    // Final wire answers were sent, including the client-ready sequence.
    assert!(script.sent.ends_with(&[0x6563, 0x5e20, 0x0c11]));
    assert!(matches!(
        commands.try_recv(),
        Ok(ClientCommand::SendChat(_))
    ));
    assert!(!events.iter().any(|event| matches!(
        event,
        ClientEvent::Progress(ConnectionStage::Ready | ConnectionStage::Recording)
            | ClientEvent::World(_)
    )));
    assert!(!events.iter().any(|event| matches!(
        event,
        ClientEvent::Status(status) if status.state == ConnectionState::Connected
    )));
}

#[test]
fn titanium_rejects_a_completed_handshake_with_an_unusable_player() {
    let config = config();
    let identity = ClientIdentity::new("example-host", "example-user");
    let credentials = credentials();
    let stop = CancellationToken::default();
    let (sender, commands) = mpsc::channel();
    let context = context(&config, &identity, &credentials, &stop, Some(&commands));
    // Negative and non-finite speeds both fit a structurally valid own spawn.
    for run_speed in [-1.0, f32::NAN] {
        sender
            .send(ClientCommand::SendChat(OutboundChat::Say("Queued".into())))
            .unwrap();
        let mut script = Script::titanium(run_speed);
        let mut events = Vec::new();
        let mut handler = |event| {
            events.push(event);
            Ok(())
        };
        let result = run_script(
            &context,
            &mut script,
            &mut Events::new(&config, &mut handler),
        );
        assert_failed_admission(result, &script, &events, &commands);
    }
}

#[test]
fn a_transfer_destination_requires_its_own_usable_player() {
    let config = config();
    let identity = ClientIdentity::new("example-host", "example-user");
    let credentials = credentials();
    let stop = CancellationToken::default();
    let (sender, commands) = mpsc::channel();
    let context = context(&config, &identity, &credentials, &stop, Some(&commands));
    let mut events = Vec::new();
    let mut handler = |event| {
        events.push(event);
        Ok(())
    };
    let mut log = Events::new(&config, &mut handler);
    let mut first = Script::titanium(1.0);
    let address = first.transfer();
    assert!(matches!(
        run_script(&context, &mut first, &mut log).unwrap(),
        ZoneExit::Direct(next) if next == address
    ));
    assert!(first.closed);
    sender
        .send(ClientCommand::SendChat(OutboundChat::Say(
            "Between zones".into(),
        )))
        .unwrap();
    let mut destination = Script::titanium(-1.0);
    let result = run_script(&context, &mut destination, &mut log);

    let entered = events.iter().position(|event| matches!(
        event, ClientEvent::World(WorldEvent::Entered { player, .. }) if player.spawn_id == 7
    )).unwrap();
    let staged = events
        .iter()
        .position(|event| matches!(event, ClientEvent::World(WorldEvent::TimeOfDay(_))))
        .unwrap();
    let ready = events
        .iter()
        .position(|event| matches!(event, ClientEvent::Progress(ConnectionStage::Ready)))
        .unwrap();
    let transfer = events
        .iter()
        .position(|event| {
            matches!(
                event,
                ClientEvent::Progress(ConnectionStage::ConnectingZone)
            )
        })
        .unwrap();
    assert!(entered < staged && staged < ready && ready < transfer);
    assert_failed_admission(result, &destination, &events[transfer + 1..], &commands);
}

fn record_chat_through(script: &mut Script) {
    let config = config();
    let identity = ClientIdentity::new("example-host", "example-user");
    let credentials = credentials();
    let stop = CancellationToken::default();
    let context = context(&config, &identity, &credentials, &stop, None);
    let mut events = Vec::new();
    let mut handler = |event| {
        if matches!(event, ClientEvent::Record(_)) {
            stop.cancel();
        }
        events.push(event);
        Ok(())
    };
    let mut body = vec![0; 148];
    body[64..70].copy_from_slice(b"Trader");
    body[132..136].copy_from_slice(&5u32.to_le_bytes());
    body.extend_from_slice(b"Hello from the zone\0");
    script.packet(0x1004, body);
    assert!(matches!(
        run_script(&context, script, &mut Events::new(&config, &mut handler)).unwrap(),
        ZoneExit::Stopped
    ));
    assert!(script.closed);
    assert!(!events.iter().any(|event| matches!(
        event,
        ClientEvent::Progress(ConnectionStage::Ready) | ClientEvent::World(_)
    )));
    let active: Vec<_> = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                ClientEvent::Progress(ConnectionStage::Recording) | ClientEvent::Record(_)
            ) || matches!(
                event, ClientEvent::Status(status) if status.state == ConnectionState::Connected
            )
        })
        .collect();
    assert!(matches!(
        active.as_slice(),
        [
            ClientEvent::Progress(ConnectionStage::Recording),
            ClientEvent::Status(_),
            ClientEvent::Record(record)
        ] if matches!(&record.event, RecordEvent::Chat(chat)
            if chat.message.as_ref().is_some_and(|line| line.text == "Hello from the zone"))
    ));
}

#[test]
fn the_same_incomplete_admission_keeps_record_only_chat_running() {
    let mut script = Script::titanium(-1.0);
    record_chat_through(&mut script);
    assert!(script
        .sent
        .ends_with(&[0x6563, 0x5e20, 0x0c11, POSITION_OPCODE]));
}

#[test]
fn recording_reports_repeated_zone_offers_once_without_following_them() {
    let mut offer = vec![0; 24];
    offer[..2].copy_from_slice(&2u16.to_le_bytes());
    assert_recording_offer_diagnostic(
        &offer,
        "Zone offer received; record-only mode cannot follow it",
    );
}

#[test]
fn recording_reports_unreadable_zone_offers_once_without_following_them() {
    assert_recording_offer_diagnostic(
        &[],
        "Unreadable zone offer received; record-only mode cannot follow it",
    );
}

fn assert_recording_offer_diagnostic(offer: &[u8], expected: &str) {
    let config = config();
    let identity = ClientIdentity::new("example-host", "example-user");
    let credentials = credentials();
    let stop = CancellationToken::default();
    let context = context(&config, &identity, &credentials, &stop, None);
    let mut script = Script::titanium(-1.0);
    script.stop_when_drained = Some(stop.clone());
    script.packet(MOVE_OPCODE, offer.to_vec());
    script.packet(MOVE_OPCODE, offer.to_vec());
    let mut events = Vec::new();
    let mut handler = |event| {
        events.push(event);
        Ok(())
    };

    assert!(matches!(
        run_script(
            &context,
            &mut script,
            &mut Events::new(&config, &mut handler)
        )
        .unwrap(),
        ZoneExit::Stopped
    ));
    assert!(stop.is_cancelled());
    assert!(script.incoming.is_empty());
    assert!(script.closed);
    assert!(script
        .sent
        .ends_with(&[0x6563, 0x5e20, 0x0c11, POSITION_OPCODE]));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                ClientEvent::Diagnostic(message) if message == expected
            ))
            .count(),
        1
    );
    assert!(!events.iter().any(|event| matches!(
        event,
        ClientEvent::Progress(ConnectionStage::Ready) | ClientEvent::World(_)
    )));
}

/// A valid Titanium server update away from the saved position.
fn position_update(spawn_id: u16) -> Vec<u8> {
    let mut body = vec![0; 22];
    body[..2].copy_from_slice(&spawn_id.to_le_bytes());
    body[6..10].copy_from_slice(&8u32.to_le_bytes());
    body
}

#[test]
fn own_updates_during_the_handshake_withdraw_the_recording_heartbeat() {
    let mut death = vec![0; 32];
    death[..4].copy_from_slice(&7u32.to_le_bytes());
    for (opcode, body) in [
        (POSITION_OPCODE, position_update(7)),
        (0x6160, death),
        (DEPART_OPCODE, 7u32.to_le_bytes().to_vec()),
    ] {
        let mut script = Script::titanium(-1.0);
        script
            .incoming
            .insert(script.incoming.len() - 1, Application { opcode, body });
        record_chat_through(&mut script);
        assert!(
            !script.sent.contains(&POSITION_OPCODE),
            "opcode {opcode:#06x}"
        );
    }
}

#[test]
fn recording_uses_only_own_identity_known_when_the_update_arrives() {
    // Profile, then own spawn. The same numeric ID is unresolved before the
    // spawn and identifies the player afterwards. A different ID remains remote.
    for (index, spawn_id) in [(1, 7), (2, 8)] {
        let mut script = Script::titanium(-1.0);
        script.incoming.insert(
            index,
            Application {
                opcode: POSITION_OPCODE,
                body: position_update(spawn_id),
            },
        );
        record_chat_through(&mut script);
        assert!(script.sent.contains(&POSITION_OPCODE));
    }
}

#[test]
fn a_later_profile_cannot_restore_a_superseded_recording_heartbeat() {
    let mut script = Script::titanium(-1.0);
    let profile = script.incoming.pop_front().unwrap();
    // The own spawn is already known when the server reports a correction.
    script.incoming.insert(
        1,
        Application {
            opcode: POSITION_OPCODE,
            body: position_update(7),
        },
    );
    script.incoming.insert(2, profile);
    record_chat_through(&mut script);
    assert!(!script.sent.contains(&POSITION_OPCODE));
}

/// A Titanium zone that sends no inventory message, as after a death that
/// left everything on the corpse: the admitted player holds nothing, and the host is asked for speed inputs it can supply.
/// Stock `EQEmu` sends none exactly then: its Titanium encoder drops an
/// `OP_CharInventory` with no entries. The diagnostic says so on every server.
#[test]
fn a_titanium_admission_without_an_inventory_admits_an_empty_one() {
    let config = config();
    let identity = ClientIdentity::new("example-host", "example-user");
    let credentials = credentials();
    let stop = CancellationToken::default();
    let context = context(&config, &identity, &credentials, &stop, None);
    let mut script = Script::titanium(1.0);
    script.stop_when_drained = Some(stop.clone());
    let mut inventory = Vec::new();
    let mut entered = false;
    let mut said = Vec::new();
    let mut handler = |event| {
        match event {
            ClientEvent::Diagnostic(line) if line.starts_with("Zone sent no inventory") => {
                said.push(line);
            }
            ClientEvent::World(WorldEvent::Entered { .. }) => entered = true,
            ClientEvent::World(WorldEvent::Inventory(update)) => {
                assert!(entered, "inventory reported before admission");
                inventory.push(update);
            }
            _ => (),
        }
        Ok(())
    };
    assert!(matches!(
        run_script(
            &context,
            &mut script,
            &mut Events::new(&config, &mut handler)
        )
        .unwrap(),
        ZoneExit::Stopped
    ));
    assert!(matches!(
        &inventory[..],
        [eq_network_game::inventory::InventoryUpdate::Snapshot(items)] if items.is_empty()
    ));
    assert_eq!(
        said,
        ["Zone sent no inventory message before admission; the player holds nothing"]
    );
}

#[test]
fn every_server_type_sends_the_characters_kept_filters_at_zone_in() {
    // P99 takes no change mid-zone, but the set the character list kept
    // still goes at the first zone-in (6.494e is only the mid-zone send).
    use eq_network_game::filters::{FilterCategory, FilterMode, ServerFilters};
    for protocol in [ServerProtocol::Project1999, ServerProtocol::EqEmu] {
        let mut config = config();
        config.protocol = protocol;
        let identity = ClientIdentity::new("example-host", "example-user");
        let credentials = credentials();
        let stop = CancellationToken::default();
        let context = context(&config, &identity, &credentials, &stop, None);
        let mut kept = ServerFilters::default();
        kept.set(FilterCategory::Shouts, FilterMode::Hide);
        context.server_filters.replace(kept.clone());
        let mut script = Script::titanium(1.0);
        script.stop_when_drained = Some(stop.clone());
        let mut handler = |_| Ok(());
        assert!(matches!(
            run_script(
                &context,
                &mut script,
                &mut Events::new(&config, &mut handler)
            )
            .unwrap(),
            ZoneExit::Stopped
        ));
        let expected =
            eq_network_game::filters::encode(eq_network_game::GameDialect::Titanium, &kept)
                .unwrap()
                .body;
        assert_eq!(script.filters, [expected], "{protocol:?}");
        // Shouts are word 4: hidden is 0.
        assert_eq!(script.filters[0][16..20], [0, 0, 0, 0]);
    }
}

impl Script {
    /// Stock `EQEmu`'s zone entry for a `RoF2` client: with `player`, the
    /// profile and own spawn; a creature's record, as the zone's spawns
    /// follow the own spawn; its description, unasked, then its objects
    /// (the spawn request's answer). After the entry: another player's
    /// record, a record with the player's own id, a record cut short, and
    /// a message numbered as a Titanium spawn, which `RoF2` must not read.
    /// It stays idle for `idle` before it stops.
    fn rof2(player: bool, stop: &CancellationToken, idle: Duration) -> Self {
        use eq_network_game::rof2_player::{
            samples::{
                despawn_body, profile_body, record_body, spawn_body, update_body, Packed, Record,
                Sample,
            },
            DESPAWN_OPCODE, OWN_SPAWN_OPCODE, POSITION_OPCODE, PROFILE_OPCODE, SPAWN_OPCODE,
        };
        let mut description = vec![0; 948];
        description[64..71].copy_from_slice(b"example");
        let mut script = Self {
            incoming: VecDeque::new(),
            stop_when_drained: Some(stop.clone()),
            sent: Vec::new(),
            aa_actions: Vec::new(),
            aa_actions_at_selections: Vec::new(),
            filters: Vec::new(),
            closed: false,
            refuse_list_reply: false,
            idle_until: Some(Instant::now() + idle),
        };
        if player {
            let sample = Sample::default();
            script.packet(PROFILE_OPCODE, profile_body(&sample).bytes);
            script.packet(OWN_SPAWN_OPCODE, spawn_body(&sample));
        }
        script.packet(SPAWN_OPCODE, record_body(&Record::creature()));
        script.packet(0x1795, description);
        script.packet(0x5ae2, Vec::new());
        script.packet(SPAWN_OPCODE, record_body(&Record::player()));
        script.packet(
            SPAWN_OPCODE,
            record_body(&Record {
                spawn_id: 4321,
                ..Record::creature()
            }),
        );
        let cut = record_body(&Record::creature());
        script.packet(SPAWN_OPCODE, cut[..cut.len() - 1].to_vec());
        // The other player moves and leaves, and a removal names the
        // player's own id (6.514).
        script.packet(
            POSITION_OPCODE,
            update_body(
                77,
                0,
                &Packed {
                    x: 80,
                    delta_x: 64,
                    animation: 40,
                    ..Packed::default()
                },
            ),
        );
        for (opcode, body) in rof2_looks_and_news()
            .into_iter()
            .chain(rof2_chat())
            .chain(rof2_casting())
            .chain(rof2_buffs())
            .chain(rof2_spellbook())
            .chain(rof2_pools())
            .chain(rof2_world())
            .chain(rof2_socials())
            .chain(rof2_answers())
            .chain(rof2_notices())
            .chain(rof2_loot_titles())
        {
            script.packet(opcode, body);
        }
        script.packet(DESPAWN_OPCODE, despawn_body(77));
        script.packet(DESPAWN_OPCODE, despawn_body(4321));
        script.packet(0x7213, vec![0; 385]);
        script
    }
}

/// How the other player looks and how hurt they are (6.518), then the
/// player's own news and the combat records (6.519), as `RoF2` messages.
fn rof2_looks_and_news() -> Vec<(u16, Vec<u8>)> {
    // How the other player looks and how hurt they are (6.518): they
    // sit, grow, change their chest, swing, are stunned and recover, and
    // drop to half health; the player's own hit points come too.
    let word = |value: u32| value.to_le_bytes();
    let looks: [(u16, Vec<u8>); 8] = [
        (
            0x0971,
            [&77u16.to_le_bytes()[..], &14u16.to_le_bytes(), &word(110)].concat(),
        ),
        (
            0x4707,
            [
                word(77),
                6.0f32.to_bits().to_le_bytes(),
                word(0),
                1.0f32.to_bits().to_le_bytes(),
            ]
            .concat(),
        ),
        (
            0x2828,
            [&77u16.to_le_bytes()[..], &word(90), &word(120)].concat(),
        ),
        (0x37b1, vec![77, 0, 50]),
        (
            0x7994,
            [
                &77u16.to_le_bytes()[..],
                &word(70_000),
                &[0; 16],
                &word(0),
                &[1],
            ]
            .concat(),
        ),
        (0x7177, vec![77, 0, 5, 10]),
        (0x559a, [word(77), word(32)].concat()),
        (0x2d25, [word(77), word(32)].concat()),
    ];
    // The player's own news and the combat records (6.519): the other
    // player hits them and is considered, the clock, coins, food,
    // experience, a level, a skill and the mana change come, and then
    // the player dies.
    let mut damage = [&4321u16.to_le_bytes()[..], &77u16.to_le_bytes(), &[0]].concat();
    damage.extend_from_slice(&[0; 4]);
    damage.extend_from_slice(&word(12));
    damage.extend_from_slice(&[0; 17]);
    let news: [(u16, Vec<u8>); 10] = [
        (0x6f15, damage),
        (
            0x742b,
            [word(4321), word(77), word(5), word(4), word(0)].concat(),
        ),
        (0x5070, [vec![7, 30, 2, 5], word(3100).to_vec()].concat()),
        (0x640c, [word(1), word(2), word(3), word(4)].concat()),
        (0x2a79, [word(90), word(80)].concat()),
        (0x20ed, [word(150), word(0)].concat()),
        (0x1eec, [word(12), word(11), word(200)].concat()),
        (0x004c, [word(22), word(150), [1, 80, 136, 54]].concat()),
        (
            0x5467,
            [
                word(300),
                word(200),
                word(0),
                word(1),
                (-1i32).to_le_bytes(),
            ]
            .concat(),
        ),
        (
            0x6517,
            [
                word(4321),
                word(77),
                word(4321),
                word(9),
                word(u32::MAX),
                word(3),
                word(12),
                word(0),
            ]
            .concat(),
        ),
    ];
    looks.into_iter().chain(news).collect()
}

/// Chat and the server's own lines as `RoF2` messages (6.521): a say, a
/// tell with a 56-character link, a guard's special message, a formatted
/// and a simple line.
fn rof2_chat() -> Vec<(u16, Vec<u8>)> {
    let channel = |sender: &[u8], target: &[u8], channel: u32, text: &[u8]| {
        let mut body = [sender, b"\0", target, b"\0"].concat();
        for word in [0u32, 0, channel, 0] {
            body.extend_from_slice(&word.to_le_bytes());
        }
        body.push(0);
        body.extend_from_slice(&100u32.to_le_bytes());
        body.extend_from_slice(text);
        body.push(0);
        body.extend_from_slice(&[0; 15]);
        body
    };
    let mut link = b"\x1201A2B3".to_vec();
    link.resize(57, b'0');
    link.extend_from_slice(b"Short Sword\x12");
    let mut special = vec![1, 0, 0];
    special.extend_from_slice(&256u32.to_le_bytes());
    special.extend_from_slice(&4321u32.to_le_bytes());
    special.extend_from_slice(b"Guard\0");
    special.extend_from_slice(&[0; 12]);
    special.extend_from_slice(b"Move along\0");
    let mut formatted = [0u32, 1032, 15].map(u32::to_le_bytes).concat();
    formatted.extend_from_slice(b"Guard\0\0");
    vec![
        (0x2b2d, channel(b"Other", b"", 8, b"well met")),
        (
            0x2b2d,
            channel(
                b"Other",
                b"Example",
                7,
                &[b"see ".as_slice(), &link].concat(),
            ),
        ),
        (0x0083, special),
        (0x1024, formatted),
        (0x213f, [338u32, 13, 0].map(u32::to_le_bytes).concat()),
    ]
}

/// The other player casting on the player, an interruption, a stun of the
/// player, and an illusion and a face change of the other player (6.522),
/// as `RoF2` messages.
fn rof2_casting() -> Vec<(u16, Vec<u8>)> {
    let word = |value: u32| value.to_le_bytes();
    let mut action = [
        &4321u16.to_le_bytes()[..],
        &77u16.to_le_bytes(),
        &51u16.to_le_bytes(),
    ]
    .concat();
    action.extend_from_slice(&[0; 4]);
    action.extend_from_slice(&1.5f32.to_bits().to_le_bytes());
    action.extend_from_slice(&[0; 12]);
    action.push(231);
    action.extend_from_slice(&[0; 6]);
    action.extend_from_slice(&word(1234));
    action.extend_from_slice(&[49, 4]);
    action.extend_from_slice(&[0; 17]);
    let mut interrupt = [word(77), word(439)].concat();
    interrupt.extend_from_slice(b"Other\0");
    let mut illusion = vec![0; 336];
    illusion[..4].copy_from_slice(&word(77));
    illusion[68..70].copy_from_slice(&75u16.to_le_bytes());
    illusion[88..92].copy_from_slice(&6.5f32.to_bits().to_le_bytes());
    let mut face = vec![1, 2, 3, 4, 5, 6, 7, 0];
    face.extend_from_slice(&[0; 12]);
    face.extend_from_slice(&word(77));
    vec![
        (
            0x318f,
            [&word(1234)[..], &77u16.to_le_bytes(), &word(3000)].concat(),
        ),
        (0x744c, action),
        (0x048c, interrupt),
        (0x36a4, [word(2000), [0, 163, 67, 0]].concat()),
        (0x312a, illusion),
        (0x1af3, face),
    ]
}

/// The player's buff list, a duration and a fade with the fade's own icon
/// message, the other player's buffs as a target, and a pet's window
/// (6.527), as `RoF2` messages.
fn rof2_buffs() -> Vec<(u16, Vec<u8>)> {
    let word = |value: u32| value.to_le_bytes();
    let update = |fade: u32| {
        let mut body = vec![0; 100];
        body[..4].copy_from_slice(&word(4321));
        body[4] = 2;
        body[5] = 51;
        body[8..12].copy_from_slice(&1.0f32.to_bits().to_le_bytes());
        body[12..16].copy_from_slice(&word(1234));
        body[16..20].copy_from_slice(&word(120));
        body[92..96].copy_from_slice(&word(3));
        body[96..].copy_from_slice(&word(fade));
        body
    };
    let icons = |spawn: u32, all: u8, spell: u32, window: u8| {
        let mut body = [word(spawn), word(0)].concat();
        body.push(all);
        body.extend_from_slice(&1u16.to_le_bytes());
        for value in [3, spell, 120, 0] {
            body.extend_from_slice(&word(value));
        }
        body.extend_from_slice(&[0, window]);
        body
    };
    vec![
        (0x3377, icons(4321, 1, 1234, 0)),
        (0x659c, update(2)),
        (0x659c, update(1)),
        (0x3377, icons(4321, 0, u32::MAX, 0)),
        (0x4f4b, icons(77, 1, 2345, 5)),
        (0x5882, icons(312, 1, 3456, 0)),
    ]
}

/// A memorize, a spell bar refresh, a swap and a deletion past Titanium's
/// book (6.532), as `RoF2` messages.
fn rof2_spellbook() -> Vec<(u16, Vec<u8>)> {
    let words = |values: [u32; 4]| values.map(u32::to_le_bytes).concat();
    vec![
        (0x217c, words([2, 1234, 1, 0])),
        (0x217c, words([11, 1234, 3, 1500])),
        (0x0efa, [12u32, 650].map(u32::to_le_bytes).concat()),
        (
            0x3358,
            [&650u16.to_le_bytes()[..], &[0, 0, 1, 0, 0, 0]].concat(),
        ),
    ]
}

/// The player's mana and endurance pools, and a group member's two
/// percents (6.538), as `RoF2` messages.
fn rof2_pools() -> Vec<(u16, Vec<u8>)> {
    let pool = |current: u32, maximum: u32| {
        [
            &current.to_le_bytes()[..],
            &maximum.to_le_bytes(),
            &4321u16.to_le_bytes(),
        ]
        .concat()
    };
    vec![
        (0x3791, pool(300, 1200)),
        (0x5f42, pool(200, 900)),
        (0x2404, [&77u16.to_le_bytes()[..], &[40]].concat()),
        (0x1c81, [&77u16.to_le_bytes()[..], &[60]].concat()),
    ]
}

/// A door table of one door, its move, a ground object, its click, the
/// zone-in weather and a weather change, as stock `EQEmu` writes them for
/// `RoF2` (6.539), with filler where the encoders write nothing from the
/// server.
fn rof2_world() -> Vec<(u16, Vec<u8>)> {
    let mut door = vec![0xa5; 100];
    door[..32].fill(0);
    door[..10].copy_from_slice(b"TESTDOOR01");
    for (at, value) in [(32, -12.5f32), (36, 27.25), (40, 9.0), (44, 128.0)] {
        door[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    door[48..52].copy_from_slice(&0u32.to_le_bytes());
    door[52..56].copy_from_slice(&100u32.to_le_bytes());
    door[60..64].copy_from_slice(&[7, 58, 0, 0]);
    door[64..68].copy_from_slice(&0u32.to_le_bytes());
    door[80..87].copy_from_slice(&[0, 1, 0, 1, 0, 0, 0]);
    let floats: Vec<u8> = [256.0f32, 0.5, -0.25, 1.0, -40.5, 120.25, 3.75]
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let object = [
        &4321u32.to_le_bytes()[..],
        b"IT63_ACTORDEF\0",
        &54u16.to_le_bytes(),
        &0u16.to_le_bytes(),
        &4321u32.to_le_bytes(),
        &0u32.to_le_bytes(),
        &floats,
        &1u32.to_le_bytes(),
        &[0x5a; 52],
    ]
    .concat();
    let words =
        |words: &[u32]| -> Vec<u8> { words.iter().flat_map(|word| word.to_le_bytes()).collect() };
    vec![
        (0x7291, door),
        (0x08e8, vec![7, 2]),
        (0x6fca, object),
        (0x4aa1, words(&[4321, 4321])),
        (0x661e, words(&[0, 5, 0])),
        (0x661e, words(&[1, 3])),
    ]
}

/// A roll, an assist, a yell for help, particles, a camera shake, a sound
/// file and a spell's nimbus, as stock `EQEmu` sends them to `RoF2` (6.541).
fn rof2_socials() -> Vec<(u16, Vec<u8>)> {
    let words =
        |words: &[u32]| -> Vec<u8> { words.iter().flat_map(|word| word.to_le_bytes()).collect() };
    let mut roll = words(&[1, 100, 42]);
    let mut name = vec![0; 64];
    name[..10].copy_from_slice(b"Testroller");
    roll.extend(name);
    let mut particles = vec![77];
    particles.extend([88, 1, 0]);
    particles.extend([0; 12]);
    let mut nimbus = words(&[1234, 4321, 77, 6000, 0, 3000]);
    nimbus.extend([1, 1, 0, 0]);
    vec![
        (0x106b, roll),
        (0x4478, words(&[77])),
        (0x4e56, words(&[77])),
        (0x3bc9, words(&particles)),
        (0x127f, words(&[1500, 0.25f32.to_bits()])),
        (0x5770, b"testsound.mp3\0".to_vec()),
        (0x5936, nimbus),
    ]
}

/// A target, a target's target, a charm, a cash reward, a linked reuse
/// timer, a beg's and a pick's answers and a bandage's, as stock `EQEmu`
/// sends them to `RoF2` (6.544).
fn rof2_answers() -> Vec<(u16, Vec<u8>)> {
    let words =
        |words: &[u32]| -> Vec<u8> { words.iter().flat_map(|word| word.to_le_bytes()).collect() };
    let mut pick = words(&[4321, 77, 150, 5, 0]);
    let mut item = vec![0; 64];
    item[..11].copy_from_slice(b"Test Dagger");
    pick.extend(item);
    vec![
        (0x58e2, words(&[77])),
        (0x0272, words(&[553])),
        (0x5d92, words(&[4321, 77, 1])),
        (0x5f7a, words(&[4, 3, 2, 1])),
        (0x1619, words(&[7, 5030, 5000])),
        (0x6703, words(&[0, 0, 0, 3, 7])),
        (0x39e8, pick),
        (0x0386, words(&[77, 1])),
    ]
}

/// A quest reward, a rename, the new titles notice and the banker's change,
/// as stock `EQEmu` sends them to `RoF2` (6.549).
fn rof2_notices() -> Vec<(u16, Vec<u8>)> {
    let words =
        |words: &[u32]| -> Vec<u8> { words.iter().flat_map(|word| word.to_le_bytes()).collect() };
    let mut reward = vec![77, 4321, 1500, 0, 0, 8, 7, 6, 5, 1001];
    reward.extend([u32::MAX; 7]);
    let mut rename = vec![0; 200];
    rename[..7].copy_from_slice(b"Testmob");
    rename[64..71].copy_from_slice(b"Testmob");
    rename[128..142].copy_from_slice(b"Testmob_corpse");
    rename[196..200].copy_from_slice(&1u32.to_le_bytes());
    vec![
        (0x1a30, words(&reward)),
        (0x2c57, rename),
        (0x0d32, Vec::new()),
        (0x791e, words(&[1, 2, 3, 4, 10, 20, 30, 40])),
    ]
}

/// Corpse money, the Loot All marker after it, the loot close, a surname
/// answer, a title list and a title change, as stock `EQEmu` sends them to
/// `RoF2` (6.552).
fn rof2_loot_titles() -> Vec<(u16, Vec<u8>)> {
    let words =
        |words: &[u32]| -> Vec<u8> { words.iter().flat_map(|word| word.to_le_bytes()).collect() };
    let money = |response: u8, platinum: u32| {
        let mut body = vec![response, 0x42, 0xef, 0];
        body.extend(words(&[platinum, 0, 0, 0]));
        body
    };
    let mut surname = vec![0; 100];
    surname[..10].copy_from_slice(b"Testplayer");
    surname[64..68].copy_from_slice(&1u32.to_le_bytes());
    surname[68..79].copy_from_slice(b"Testsurname");
    let mut titles = words(&[1, 3]);
    titles.extend(b"Testprefix\0\0");
    let mut title = words(&[1]);
    let mut field = vec![0; 32];
    field[..10].copy_from_slice(b"Testsuffix");
    title.extend(field);
    title.extend(words(&[77]));
    vec![
        (0x5f44, money(1, 12)),
        (0x5f44, money(6, 0)),
        (0x55c4, Vec::new()),
        (0x0423, surname),
        (0x2d08, titles),
        (0x4c21, title),
    ]
}

/// The session's idle time after the entry: longer than any position
/// update or timer a feature would wake for in the first seconds.
const IDLE: Duration = Duration::from_millis(1500);

/// Runs a `RoF2` session through its entry and idle time: what it sent, the
/// diagnostics, the events, and whether it was admitted.
fn rof2_session(
    player: bool,
    commands: Option<(&mpsc::Sender<ClientCommand>, &Receiver<ClientCommand>)>,
) -> (
    Vec<u16>,
    Vec<String>,
    Vec<WorldEvent>,
    Vec<serde_json::Value>,
) {
    let mut config = config();
    config.protocol = ServerProtocol::EqEmuRof2;
    let identity = ClientIdentity::new("example-host", "example-user");
    let credentials = credentials();
    let stop = CancellationToken::default();
    let context = context(
        &config,
        &identity,
        &credentials,
        &stop,
        commands.map(|(_, receiver)| receiver),
    );
    let mut script = Script::rof2(player, &stop, IDLE);
    let mut diagnostics = Vec::new();
    let mut events = Vec::new();
    let mut records = Vec::new();
    let mut handler = |event| {
        match event {
            ClientEvent::Diagnostic(line) => diagnostics.push(line),
            ClientEvent::Record(record) => records.push(serde_json::to_value(record).unwrap()),
            ClientEvent::World(event) => {
                if let (WorldEvent::Entered { session_id, .. }, Some((sender, _))) =
                    (&event, commands)
                {
                    // Commands that need an admission, given once there is one.
                    for command in [
                        ClientCommand::Camp {
                            session_id: *session_id,
                            created: Instant::now(),
                        },
                        ClientCommand::SetPosture {
                            session_id: *session_id,
                            spawn_id: 4321,
                            posture: eq_network_game::command::Posture::Sitting,
                            created: Instant::now(),
                        },
                    ] {
                        sender.send(command).unwrap();
                    }
                }
                events.push(event);
            }
            _ => (),
        }
        Ok(())
    };
    let mut log = Events::new(&config, &mut handler);
    let exit = run_script(&context, &mut script, &mut log).unwrap();
    drop(log);
    assert!(matches!(exit, ZoneExit::Stopped));
    assert_eq!(script.filters.len(), 0);
    (script.sent, diagnostics, events, records)
}

#[test]
fn an_admitted_rof2_session_sends_nothing_past_its_entry_and_reads_no_other_generations_layout() {
    let (sent, diagnostics, events, _) = rof2_session(true, None);
    // The entry, the spawn request and the ready, and nothing in the zone
    // through the idle time.
    assert_eq!(sent, [0x5089, 0x35fa, 0x345d]);
    // The player is admitted, and offered nothing.
    let entered = events.iter().find_map(|event| match event {
        WorldEvent::Entered {
            capabilities,
            choices,
            zone,
            ..
        } => Some((capabilities.clone(), choices.clone(), zone.clone())),
        _ => None,
    });
    assert_eq!(entered, Some((Vec::new(), Vec::new(), "example".into())));
    // A message numbered as Titanium's spawn is told by number and length,
    // not read as one, and as unlisted, since no RoF2 stage reads it.
    assert!(
        diagnostics
            .iter()
            .any(|line| line.contains("an unlisted message (0x7213, 385 bytes), left unread")),
        "{diagnostics:?}"
    );
    // What the entry reads is not told as unread (6.508).
    for opcode in ["0x6506", "0x5089", "0x1795", "0x5ae2"] {
        assert!(
            !diagnostics
                .iter()
                .any(|line| line.contains(&format!("({opcode},"))),
            "{opcode}: {diagnostics:?}"
        );
    }
    // The zone's spawns are read (6.510), and the other player's after the
    // entry reaches the host. The creature that came during the entry is
    // read too, but nothing stages it for the admission: `EqEmuRof2` offers
    // no feature yet, the spawn table among them. The player's own spawn is
    // never another spawn, at the entry or when a later record carries its
    // id.
    let spawns: Vec<Vec<u16>> = events
        .iter()
        .filter_map(|event| match event {
            WorldEvent::Spawns(spawns) => Some(spawns.iter().map(|spawn| spawn.spawn_id).collect()),
            _ => None,
        })
        .collect();
    assert_eq!(spawns, [vec![77]]);
    let length = eq_network_game::rof2_player::samples::record_body(
        &eq_network_game::rof2_player::samples::Record::creature(),
    )
    .len();
    assert!(
        diagnostics.iter().any(|line| line
            == &format!(
                "RoF2 zone sent the player's own spawn again ({length} bytes), left unread"
            )),
        "{diagnostics:?}"
    );
    // A record cut short is told by number and length, in fixed words.
    let length = length - 1;
    assert!(
        diagnostics.iter().any(|line| line
            == &format!(
                "World update rejected: truncated RoF2 spawn (opcode=0x5089, payload_len={length})"
            )),
        "{diagnostics:?}"
    );
    assert!(!diagnostics.iter().any(|line| line.contains("a_rat")));
    // The move and the removals reach the host, the player's own removal
    // as Titanium's decoder gives it, and nothing is sent in answer.
    let moved = events.iter().find_map(|event| match event {
        WorldEvent::Position {
            spawn_id: 77,
            position,
            velocity,
            forward,
        } => Some((position.x, velocity[0], *forward)),
        _ => None,
    });
    assert_eq!(moved, Some((10.0, 20.0, 1.0)));
    let removed: Vec<u16> = events
        .iter()
        .filter_map(|event| match event {
            WorldEvent::Despawn(id) => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(removed, [77, 4321]);
    for opcode in [
        "0x7dfc", "0x7280", "0x0971", "0x4707", "0x2828", "0x37b1", "0x7994", "0x7177", "0x559a",
        "0x2d25",
    ] {
        assert!(
            !diagnostics.iter().any(|line| line.contains(opcode)),
            "{opcode}: {diagnostics:?}"
        );
    }
}

#[test]
fn an_admitted_rof2_session_hands_the_host_each_look_and_health_message() {
    let (sent, _, events, _) = rof2_session(true, None);
    assert_eq!(sent, [0x5089, 0x35fa, 0x345d]);
    // Each look and health message reaches the host as one event (6.518).
    let looks: Vec<String> = events
        .iter()
        .filter_map(|event| match event {
            WorldEvent::Posture { spawn_id: 77, .. } => Some("posture".into()),
            // The illusion is the casting batch's (6.522).
            WorldEvent::Appearance {
                spawn_id: 77,
                change,
            } if !matches!(
                change,
                eq_network_game::appearance::AppearanceChange::Illusion(_)
            ) =>
            {
                Some(format!("{change:?}"))
            }
            WorldEvent::HitPoints {
                spawn_id: 77,
                current,
                maximum,
                ..
            } => Some(format!("hp {current}/{maximum}")),
            WorldEvent::HealthPercent {
                spawn_id: 77,
                percent,
            } => Some(format!("health {percent}")),
            WorldEvent::WearChange(change) if change.spawn_id == 77 => {
                Some(format!("wear {}", change.material))
            }
            WorldEvent::Animation(animation) if animation.spawn_id == 77 => {
                Some(format!("animation {}", animation.action))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        looks,
        [
            "posture",
            "Size(6.0)",
            "hp 90/120",
            "health 50",
            "wear 70000",
            "animation 5",
            "StatesAdded(SpawnStates(32))",
            "StatesRemoved(SpawnStates(32))",
        ]
    );
}

#[test]
fn an_admitted_rof2_session_hands_the_host_its_news_and_combat_and_dies_in_place() {
    let (sent, diagnostics, events, _) = rof2_session(true, None);
    // Nothing is sent in answer, the death included: no feature is offered
    // and the handoff still refuses, so no zone change starts.
    assert_eq!(sent, [0x5089, 0x35fa, 0x345d]);
    let news: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            WorldEvent::Damage(damage) if damage.target_id == 4321 => Some("damage"),
            WorldEvent::Consideration(consider) if consider.hit_points.is_none() => {
                Some("consider")
            }
            WorldEvent::TimeOfDay(_) => Some("time"),
            WorldEvent::Coins(_) => Some("coins"),
            WorldEvent::Nourishment(_) => Some("food"),
            WorldEvent::Experience(150) => Some("experience"),
            WorldEvent::Level { current: 12, .. } => Some("level"),
            WorldEvent::Skill {
                skill_id: 22,
                value: 150,
            } => Some("skill"),
            WorldEvent::Spell(eq_network_game::spells::SpellUpdate::Mana { .. }) => Some("spell"),
            WorldEvent::Resources {
                mana: 300,
                endurance: 200,
            } => Some("resources"),
            WorldEvent::Death(death) if death.spawn_id == 4321 => Some("death"),
            _ => None,
        })
        .collect();
    assert_eq!(
        news,
        [
            "damage",
            "consider",
            "time",
            "coins",
            "food",
            "experience",
            "level",
            "skill",
            "spell",
            "resources",
            "death",
        ]
    );
    for opcode in [
        "0x6f15", "0x742b", "0x5070", "0x640c", "0x2a79", "0x20ed", "0x1eec", "0x004c", "0x5467",
        "0x6517",
    ] {
        assert!(
            !diagnostics.iter().any(|line| line.contains(opcode)),
            "{opcode}: {diagnostics:?}"
        );
    }
}

#[test]
fn an_admitted_rof2_session_records_its_chat_and_the_servers_lines() {
    let (sent, diagnostics, _, records) = rof2_session(true, None);
    assert_eq!(sent, [0x5089, 0x35fa, 0x345d]);
    let chat: Vec<(String, String, String)> = records
        .iter()
        .map(|record| {
            let field = |name: &str| record[name].as_str().unwrap_or_default().to_owned();
            (field("channel_name"), field("sender"), field("text"))
        })
        .collect();
    assert_eq!(
        chat,
        [
            ("say".into(), "Other".into(), "well met".into()),
            ("tell".into(), "Other".into(), "see Short Sword".into()),
            ("system".into(), "Guard".into(), "Move along".into()),
            ("system".into(), String::new(), String::new()),
            ("system".into(), String::new(), String::new()),
        ]
    );
    assert_eq!(records[1]["item_links"][0]["item_id"], 0x1a2b3);
    assert_eq!(records[3]["string_id"], 1032);
    assert_eq!(records[4]["string_id"], 338);
    for opcode in ["0x2b2d", "0x0083", "0x1024", "0x213f"] {
        assert!(
            !diagnostics.iter().any(|line| line.contains(opcode)),
            "{opcode}: {diagnostics:?}"
        );
    }
}

#[test]
fn an_admitted_rof2_session_hands_the_host_its_casting_stun_and_illusions() {
    let (sent, diagnostics, events, _) = rof2_session(true, None);
    // The stun of the player sends nothing either.
    assert_eq!(sent, [0x5089, 0x35fa, 0x345d]);
    let casting: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            WorldEvent::Spell(eq_network_game::spells::SpellUpdate::Began {
                caster_id: 77,
                spell_id: 1234,
                duration_ms: 3000,
            }) => Some("began"),
            WorldEvent::SpellEffect(effect)
                if effect.target_id == 4321 && effect.instrument_modifier == 15 =>
            {
                Some("effect")
            }
            WorldEvent::Spell(eq_network_game::spells::SpellUpdate::Interrupted {
                caster_id: 77,
                message_id: 439,
                ..
            }) => Some("interrupted"),
            WorldEvent::Stun(Some(duration)) if duration.as_millis() == 2000 => Some("stun"),
            WorldEvent::Appearance {
                spawn_id: 77,
                change: eq_network_game::appearance::AppearanceChange::Illusion(_),
            } => Some("illusion"),
            WorldEvent::FaceChanged {
                spawn_id: Some(77),
                eye_colors: Some([3, 4]),
                ..
            } => Some("face"),
            _ => None,
        })
        .collect();
    assert_eq!(
        casting,
        ["began", "effect", "interrupted", "stun", "illusion", "face"]
    );
    for opcode in ["0x318f", "0x744c", "0x048c", "0x36a4", "0x312a", "0x1af3"] {
        assert!(
            !diagnostics.iter().any(|line| line.contains(opcode)),
            "{opcode}: {diagnostics:?}"
        );
    }
}

#[test]
fn an_admitted_rof2_session_hands_the_host_its_buffs() {
    let (sent, diagnostics, events, _) = rof2_session(true, None);
    assert_eq!(sent, [0x5089, 0x35fa, 0x345d]);
    let buffs: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            WorldEvent::BuffIcons(icons) if icons.spawn_id == 4321 && icons.all => Some("list"),
            WorldEvent::Buff(update) if update.buff.is_some() => Some("duration"),
            WorldEvent::Buff(update) if update.buff.is_none() => Some("fade"),
            WorldEvent::BuffIcons(icons)
                if icons.spawn_id == 4321 && icons.icons[0].spell_id.is_none() =>
            {
                Some("emptied")
            }
            WorldEvent::BuffIcons(icons) if icons.spawn_id == 77 => Some("target"),
            WorldEvent::PetBuffs(pet) if pet.pet == 312 => Some("pet"),
            _ => None,
        })
        .collect();
    assert_eq!(
        buffs,
        ["list", "duration", "fade", "emptied", "target", "pet"]
    );
    for opcode in ["0x659c", "0x3377", "0x4f4b", "0x5882"] {
        assert!(
            !diagnostics.iter().any(|line| line.contains(opcode)),
            "{opcode}: {diagnostics:?}"
        );
    }
}

#[test]
fn an_admitted_rof2_session_hands_the_host_its_spell_book_and_gems() {
    use eq_network_game::spells::SpellUpdate;
    let (sent, diagnostics, events, _) = rof2_session(true, None);
    assert_eq!(sent, [0x5089, 0x35fa, 0x345d]);
    let book: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            WorldEvent::Spell(SpellUpdate::Slot {
                slot: 2,
                spell_id: 1234,
                mode: 1,
            }) => Some("memorized"),
            WorldEvent::Spell(SpellUpdate::BarRefresh {
                slot: 11,
                reduction_ms: 1500,
                ..
            }) => Some("bar"),
            WorldEvent::Spell(SpellUpdate::BookSwap { from: 12, to: 650 }) => Some("swap"),
            WorldEvent::Spell(SpellUpdate::BookDeletion {
                slot: 650,
                success: true,
            }) => Some("deleted"),
            _ => None,
        })
        .collect();
    assert_eq!(book, ["memorized", "bar", "swap", "deleted"]);
    // The session's spell book is Titanium's and `EQMac`'s alone: an
    // admitted RoF2 session keeps no book and answers nothing (6.532).
    assert!(!events
        .iter()
        .any(|event| matches!(event, WorldEvent::SpellBook(_) | WorldEvent::BookAction(_))));
    for opcode in ["0x217c", "0x0efa", "0x3358"] {
        assert!(
            !diagnostics.iter().any(|line| line.contains(opcode)),
            "{opcode}: {diagnostics:?}"
        );
    }
}

#[test]
fn an_admitted_rof2_session_hands_the_host_its_mana_and_endurance_pools() {
    use eq_network_game::rof2_pools::Pool;
    let (sent, diagnostics, events, _) = rof2_session(true, None);
    assert_eq!(sent, [0x5089, 0x35fa, 0x345d]);
    let pools: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            WorldEvent::ResourcePool {
                spawn_id: 4321,
                pool: Pool::Mana,
                current: 300,
                maximum: 1200,
            } => Some("mana"),
            WorldEvent::ResourcePool {
                spawn_id: 4321,
                pool: Pool::Endurance,
                current: 200,
                maximum: 900,
            } => Some("endurance"),
            WorldEvent::ResourcePercent {
                spawn_id: 77,
                pool: Pool::Mana,
                percent: 40,
            } => Some("mana percent"),
            WorldEvent::ResourcePercent {
                spawn_id: 77,
                pool: Pool::Endurance,
                percent: 60,
            } => Some("endurance percent"),
            _ => None,
        })
        .collect();
    assert_eq!(
        pools,
        ["mana", "endurance", "mana percent", "endurance percent"]
    );
    // RoF2 gets OP_ManaChange's current pair too, for the same change, and
    // both reach the host, in order.
    let resources = events
        .iter()
        .position(|event| {
            matches!(
                event,
                WorldEvent::Resources {
                    mana: 300,
                    endurance: 200
                }
            )
        })
        .unwrap();
    let pool = events
        .iter()
        .position(|event| {
            matches!(
                event,
                WorldEvent::ResourcePool {
                    pool: Pool::Mana,
                    ..
                }
            )
        })
        .unwrap();
    assert!(resources < pool);
    for opcode in ["0x3791", "0x5f42", "0x2404", "0x1c81"] {
        assert!(
            !diagnostics.iter().any(|line| line.contains(opcode)),
            "{opcode}: {diagnostics:?}"
        );
    }
}

#[test]
fn an_admitted_rof2_session_hands_the_host_its_doors_objects_and_weather() {
    use eq_network_game::{clock::Weather, doors::DoorUpdate, objects::ObjectUpdate};
    let (sent, diagnostics, events, _) = rof2_session(true, None);
    assert_eq!(sent, [0x5089, 0x35fa, 0x345d]);
    let world: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            WorldEvent::Doors(DoorUpdate::Spawn(doors))
                if doors.len() == 1
                    && doors[0].id == 7
                    && doors[0].model == "TESTDOOR01"
                    && doors[0].size == 100
                    && doors[0].open_type == 58 =>
            {
                Some("doors")
            }
            WorldEvent::Doors(DoorUpdate::Move { id: 7, action: 2 }) => Some("move"),
            WorldEvent::Objects(ObjectUpdate::Spawn(object))
                if object.drop_id == 4321
                    && object.model == "IT63_ACTORDEF"
                    && object.object_type == 1 =>
            {
                Some("object")
            }
            WorldEvent::Objects(ObjectUpdate::Remove {
                drop_id: 4321,
                taken_by: Some(4321),
            }) => Some("click"),
            WorldEvent::Weather(Weather::Snow { intensity: 3 }) => Some("weather"),
            WorldEvent::Weather(_) => Some("other weather"),
            _ => None,
        })
        .collect();
    // The zone-in form gives nothing, as on Titanium.
    assert_eq!(world, ["doors", "move", "object", "click", "weather"]);
    for opcode in ["0x7291", "0x08e8", "0x6fca", "0x4aa1", "0x661e"] {
        assert!(
            !diagnostics.iter().any(|line| line.contains(opcode)),
            "{opcode}: {diagnostics:?}"
        );
    }
}

#[test]
fn an_admitted_rof2_session_hands_the_host_its_rolls_assists_and_effects() {
    use eq_network_game::{
        effects::{Cue, Effect},
        socials::{Assisted, Roll},
    };
    let (sent, diagnostics, events, _) = rof2_session(true, None);
    assert_eq!(sent, [0x5089, 0x35fa, 0x345d]);
    let read: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            WorldEvent::Roll(Roll {
                name,
                low: 1,
                high: 100,
                result: 42,
            }) if name == "Testroller" => Some("roll"),
            WorldEvent::Assisted(Assisted { target: Some(77) }) => Some("assist"),
            WorldEvent::YellForHelp { spawn_id: 77 } => Some("yell"),
            WorldEvent::Effect(Effect::Particles {
                spawn_id: 77,
                effects,
            }) if effects.len() == 1 && effects[0].effect == 88 => Some("particles"),
            WorldEvent::Effect(Effect::CameraShake { intensity, .. })
                if intensity.to_bits() == 0.25f32.to_bits() =>
            {
                Some("camera")
            }
            WorldEvent::Effect(Effect::Sound(Cue::File(file))) if file == "testsound.mp3" => {
                Some("sound")
            }
            WorldEvent::Effect(Effect::Nimbus {
                effect: 1234,
                caster_id: 4321,
                target_id: 77,
                ..
            }) => Some("nimbus"),
            _ => None,
        })
        .collect();
    assert_eq!(
        read,
        [
            "roll",
            "assist",
            "yell",
            "particles",
            "camera",
            "sound",
            "nimbus"
        ]
    );
    for opcode in [
        "0x106b", "0x4478", "0x4e56", "0x3bc9", "0x127f", "0x5770", "0x5936",
    ] {
        assert!(
            !diagnostics.iter().any(|line| line.contains(opcode)),
            "{opcode}: {diagnostics:?}"
        );
    }
    // The roller's name reaches the host in the event only.
    assert!(!diagnostics.iter().any(|line| line.contains("Testroller")));
}

#[test]
fn an_admitted_rof2_session_hands_the_host_its_targets_rewards_and_answers() {
    use eq_network_game::{
        abilities::{Ability, AskAnswer, AskOutcome},
        bind_wound::{BindWoundEnd, BindWoundUpdate},
    };
    let (sent, diagnostics, events, _) = rof2_session(true, None);
    assert_eq!(sent, [0x5089, 0x35fa, 0x345d]);
    let read: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            WorldEvent::Target(Some(77)) => Some("target"),
            WorldEvent::TargetOfTarget(Some(553)) => Some("target of target"),
            WorldEvent::Charm(charm) if charm.owner_id == Some(4321) && charm.pet_id == 77 => {
                Some("charm")
            }
            WorldEvent::Reward(reward) if reward.coins.platinum == 1 => Some("reward"),
            WorldEvent::LinkedReuse(reuse) if reuse.timer == 7 => Some("reuse"),
            WorldEvent::AskAnswered(AskAnswer {
                ability: Ability::Begging,
                ..
            }) => Some("beg"),
            WorldEvent::AskAnswered(AskAnswer {
                ability: Ability::PickPockets,
                outcome: AskOutcome::Item(item),
            }) if item == "Test Dagger" => Some("pick"),
            WorldEvent::BindWound(BindWoundUpdate::Ended(BindWoundEnd::Complete)) => {
                Some("bandage")
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        read,
        [
            "target",
            "target of target",
            "charm",
            "reward",
            "reuse",
            "beg",
            "pick",
            "bandage"
        ]
    );
    for opcode in [
        "0x58e2", "0x0272", "0x5d92", "0x5f7a", "0x1619", "0x6703", "0x39e8", "0x0386",
    ] {
        assert!(
            !diagnostics.iter().any(|line| line.contains(opcode)),
            "{opcode}: {diagnostics:?}"
        );
    }
    // The item's name reaches the host in the event only.
    assert!(!diagnostics.iter().any(|line| line.contains("Test Dagger")));
}

#[test]
fn an_admitted_rof2_session_hands_the_host_its_rewards_renames_titles_and_change() {
    let (sent, diagnostics, events, _) = rof2_session(true, None);
    assert_eq!(sent, [0x5089, 0x35fa, 0x345d]);
    let read: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            WorldEvent::Reward(reward)
                if reward.giver_id == Some(77)
                    && reward.experience == 1500
                    && reward.coins.platinum == 5
                    && reward.items == [1001] =>
            {
                Some("reward")
            }
            WorldEvent::Renamed {
                spawn_id: None,
                old_name,
                new_name,
            } if old_name == "Testmob" && new_name == "Testmob_corpse" => Some("rename"),
            WorldEvent::NewTitles => Some("titles"),
            WorldEvent::CoinsChanged(change)
                if change.purse.platinum == 1 && change.bank.copper == 40 =>
            {
                Some("change")
            }
            _ => None,
        })
        .collect();
    assert_eq!(read, ["reward", "rename", "titles", "change"]);
    for opcode in ["0x1a30", "0x2c57", "0x0d32", "0x791e"] {
        assert!(
            !diagnostics.iter().any(|line| line.contains(opcode)),
            "{opcode}: {diagnostics:?}"
        );
    }
    // The spawn names reach the host in the event only.
    assert!(!diagnostics.iter().any(|line| line.contains("Testmob")));
}

#[test]
fn an_admitted_rof2_session_hands_the_host_its_loot_titles_and_surname() {
    use eq_network_game::{
        loot::{LootResponse, LootUpdate},
        names::TitleChange,
    };
    let (sent, diagnostics, events, _) = rof2_session(true, None);
    assert_eq!(sent, [0x5089, 0x35fa, 0x345d]);
    let read: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            WorldEvent::Loot(LootUpdate::Opened {
                response: LootResponse::Normal,
                coins,
            }) if coins.platinum == 12 => Some("money"),
            WorldEvent::Loot(LootUpdate::Opened { .. }) => Some("another opening"),
            WorldEvent::Loot(LootUpdate::Closed) => Some("close"),
            WorldEvent::SurnameAnswer {
                accepted: true,
                last_name,
            } if last_name == "Testsurname" => Some("surname"),
            WorldEvent::TitleList(titles)
                if titles.len() == 1 && titles[0].id == 3 && titles[0].prefix == "Testprefix" =>
            {
                Some("titles")
            }
            WorldEvent::TitleChanged {
                spawn_id: 77,
                change: TitleChange::Suffix(suffix),
            } if suffix == "Testsuffix" => Some("title"),
            _ => None,
        })
        .collect();
    // The Loot All marker gives no event.
    assert_eq!(read, ["money", "close", "surname", "titles", "title"]);
    for opcode in ["0x55c4", "0x0423", "0x2d08", "0x4c21"] {
        assert!(
            !diagnostics.iter().any(|line| line.contains(opcode)),
            "{opcode}: {diagnostics:?}"
        );
    }
    // The last name and the titles reach the host in the event only.
    for text in ["Testplayer", "Testsurname", "Testprefix", "Testsuffix"] {
        assert!(
            !diagnostics.iter().any(|line| line.contains(text)),
            "{text}"
        );
    }
}

#[test]
fn a_rof2_session_without_its_profile_only_records() {
    let (sent, diagnostics, events, _) = rof2_session(false, None);
    assert_eq!(sent, [0x5089, 0x35fa, 0x345d]);
    assert!(!events
        .iter()
        .any(|event| matches!(event, WorldEvent::Entered { .. })));
    assert!(
        diagnostics.iter().any(|line| {
            line == "Player presentation unavailable: the RoF2 profile or own spawn did not come"
        }),
        "{diagnostics:?}"
    );
}

#[test]
fn a_rof2_session_with_a_player_at_the_controls_refuses_rather_than_sends() {
    let (sender, receiver) = mpsc::channel();
    sender
        .send(ClientCommand::SendChat(OutboundChat::Say("Hail".into())))
        .unwrap();
    let (sent, diagnostics, events, _) = rof2_session(true, Some((&sender, &receiver)));
    // No command was sent: only the entry went out, through the idle time.
    assert_eq!(sent, [0x5089, 0x35fa, 0x345d]);
    for command in ["SendChat", "Camp", "SetPosture"] {
        assert!(
            diagnostics.contains(&format!(
                "Rejected {command} command: Not available on this server"
            )),
            "{diagnostics:?}"
        );
    }
    assert!(events
        .iter()
        .any(|event| matches!(event, WorldEvent::Entered { .. })));
}
