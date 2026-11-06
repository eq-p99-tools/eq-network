//! Scripted zone transports exercise the handshake and feature gates without sockets.
use super::*;
use crate::client::{
    session::login::Credentials, CancellationToken, ClientConfig, ClientIdentity, ServerProtocol,
};
use eq_network_game::{
    chat::OutboundChat,
    movement::POSITION_OPCODE,
    world::WorldEvent,
    zoning::{DEPART_OPCODE, HANDOFF_OPCODE, MOVE_OPCODE},
};
use eq_network_transport::Application;
use std::{
    cell::Cell,
    collections::VecDeque,
    sync::mpsc::{self, Receiver},
};

struct Script {
    incoming: VecDeque<Application>,
    stop_when_drained: Option<CancellationToken>,
    sent: Vec<u16>,
    closed: bool,
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
            closed: false,
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
}

impl Transport for Script {
    fn send(&mut self, opcode: u16, _: &[u8]) -> Result<()> {
        assert!(!self.closed);
        self.sent.push(opcode);
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Application>> {
        if let Some(packet) = self.incoming.pop_front() {
            return Ok(Some(packet));
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
    )
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
    let mut offer = vec![0; 24];
    offer[..2].copy_from_slice(&2u16.to_le_bytes());
    first.packet(MOVE_OPCODE, offer);
    let mut address = vec![0; 130];
    address[..9].copy_from_slice(b"127.0.0.1");
    address[128..].copy_from_slice(&7000u16.to_le_bytes());
    first.packet(HANDOFF_OPCODE, address.clone());
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
