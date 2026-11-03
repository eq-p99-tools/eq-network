use super::*;
use crate::client::{ClientConfig, ServerProtocol};
use eq_network_game::spell_tables::{Assignment, DecodeError, IdRole, SpellId, Table};

fn body(count: u32, words: &[u16]) -> Vec<u8> {
    let mut bytes = count.to_le_bytes().to_vec();
    bytes.extend(words.iter().flat_map(|word| word.to_le_bytes()));
    bytes
}

fn registration() -> Vec<u8> {
    body(3, &[42, 9001, 42])
}

fn replay() -> Vec<u8> {
    body(1, &[42, 9001])
}

#[test]
fn actual_attempt_reset_binds_table_run_to_the_connecting_status_identity() {
    use crate::client::ConnectionState;
    let server = super::super::servers::server_type(ServerProtocol::Project1999);
    let config = ClientConfig::new("EXAMPLE_ACCOUNT", "EXAMPLE_PASSWORD", "Test", "Tester");
    let mut received = Vec::new();
    let mut handler = |event| {
        received.push(event);
        Ok(())
    };
    let mut log = Events::new(&config, &mut handler);
    for _ in 0..2 {
        log.reset();
        log.status(ConnectionState::Connecting, 0, None).unwrap();
        receive(
            server,
            Stage::World,
            spell_tables::REGISTRATION_OPCODE,
            &registration(),
            &mut log,
        )
        .unwrap();
    }
    drop(log);
    for pair in received.as_chunks::<2>().0 {
        let [ClientEvent::Status(status), ClientEvent::World(WorldEvent::SpellTable(event))] = pair
        else {
            panic!()
        };
        assert_eq!(status.state, ConnectionState::Connecting);
        assert_eq!(status.session_id, format!("{:016x}", event.context.run_id));
        assert_eq!(event.context.revision, 1);
    }
    assert_eq!(received.len(), 4);
}

#[test]
fn p99_world_and_pre_profile_zone_data_share_one_order_across_transport_changes() {
    let server = super::super::servers::server_type(ServerProtocol::Project1999);
    let config = ClientConfig::new("EXAMPLE_ACCOUNT", "EXAMPLE_PASSWORD", "Test", "Tester");
    let mut received = Vec::new();
    let mut handler = |event| {
        received.push(event);
        Ok(())
    };
    let mut log = Events::new(&config, &mut handler);
    log.spell_tables = Sequence::new(17);
    assert!(receive(
        server,
        Stage::World,
        spell_tables::REGISTRATION_OPCODE,
        &registration(),
        &mut log
    )
    .unwrap());
    // Neither zone has a player/profile. Table delivery has no admission dependency.
    for session_id in [31, 32] {
        let world = super::super::feature::World::new(session_id);
        assert!(!world.ready());
        assert!(receive(
            server,
            Stage::Zone { session_id },
            spell_tables::REPLAY_OPCODE,
            &replay(),
            &mut log
        )
        .unwrap());
        assert!(!world.ready());
    }
    drop(log);
    let [ClientEvent::World(WorldEvent::SpellTable(first)), ClientEvent::World(WorldEvent::SpellTable(second)), ClientEvent::World(WorldEvent::SpellTable(third))] =
        received.as_slice()
    else {
        panic!("{received:?}")
    };
    assert_eq!(first.context, Context::new(17, 1, Stage::World));
    assert!(
        matches!(&first.update, Update::Table(Table::Registration(ids))
        if ids.iter().map(|id| id.get()).collect::<Vec<_>>() == [42, 9001, 42])
    );
    assert_eq!(
        second.context,
        Context::new(17, 2, Stage::Zone { session_id: 31 })
    );
    assert_eq!(
        third.context,
        Context::new(17, 3, Stage::Zone { session_id: 32 })
    );
    assert_eq!(second.update, third.update);
    assert_eq!(
        second.update,
        Update::Table(Table::Replay(vec![Assignment::new(
            SpellId::new(42).unwrap(),
            SpellId::new(9001).unwrap()
        )]))
    );
}

#[test]
fn other_policies_do_not_consume_even_valid_or_malformed_p99_tables() {
    let config = ClientConfig::new("EXAMPLE_ACCOUNT", "EXAMPLE_PASSWORD", "Test", "Tester");
    let mut received = Vec::new();
    let mut handler = |event| {
        received.push(event);
        Ok(())
    };
    let mut log = Events::new(&config, &mut handler);
    for protocol in [
        ServerProtocol::EqEmu,
        ServerProtocol::Takp,
        ServerProtocol::Quarm,
    ] {
        let server = super::super::servers::server_type(protocol);
        for (stage, opcode, bytes) in [
            (
                Stage::World,
                spell_tables::REGISTRATION_OPCODE,
                registration(),
            ),
            (
                Stage::Zone { session_id: 31 },
                spell_tables::REPLAY_OPCODE,
                replay(),
            ),
            (
                Stage::Zone { session_id: 31 },
                spell_tables::REPLAY_OPCODE,
                vec![1],
            ),
        ] {
            assert!(!receive(server, stage, opcode, &bytes, &mut log).unwrap());
        }
    }
    assert_eq!(log.spell_tables.revision, 0);
    drop(log);
    assert!(received.is_empty());
}

#[test]
fn malformed_replay_is_one_visible_unavailable_event_with_no_valid_prefix() {
    let server = super::super::servers::server_type(ServerProtocol::Project1999);
    let config = ClientConfig::new("EXAMPLE_ACCOUNT", "EXAMPLE_PASSWORD", "Test", "Tester");
    let mut received = Vec::new();
    let mut handler = |event| {
        received.push(event);
        Ok(())
    };
    let mut log = Events::new(&config, &mut handler);
    assert!(receive(
        server,
        Stage::Zone { session_id: 31 },
        spell_tables::REPLAY_OPCODE,
        &body(2, &[42, 9001, 7, 10000]),
        &mut log
    )
    .unwrap());
    drop(log);
    let [ClientEvent::World(WorldEvent::SpellTable(event))] = received.as_slice() else {
        panic!("{received:?}")
    };
    assert_eq!(event.context.revision, 1);
    assert_eq!(
        event.update,
        Update::Unavailable {
            kind: Kind::Replay,
            reason: Unavailable::Malformed(DecodeError::InvalidId {
                entry: 1,
                role: IdRole::Source,
                value: 10000
            }),
        }
    );
}

#[test]
fn wrong_stage_is_unavailable_and_an_unrelated_opcode_does_not_advance_order() {
    let server = super::super::servers::server_type(ServerProtocol::Project1999);
    let config = ClientConfig::new("EXAMPLE_ACCOUNT", "EXAMPLE_PASSWORD", "Test", "Tester");
    let mut received = Vec::new();
    let mut handler = |event| {
        received.push(event);
        Ok(())
    };
    let mut log = Events::new(&config, &mut handler);
    assert!(!receive(server, Stage::World, 0x1236, &[], &mut log).unwrap());
    assert!(receive(
        server,
        Stage::World,
        spell_tables::REPLAY_OPCODE,
        &replay(),
        &mut log
    )
    .unwrap());
    assert!(receive(
        server,
        Stage::Zone { session_id: 31 },
        spell_tables::REGISTRATION_OPCODE,
        &registration(),
        &mut log
    )
    .unwrap());
    drop(log);
    for (index, event) in received.iter().enumerate() {
        let ClientEvent::World(WorldEvent::SpellTable(event)) = event else {
            panic!()
        };
        assert_eq!(event.context.revision, u64::try_from(index).unwrap() + 1);
        assert!(matches!(
            event.update,
            Update::Unavailable {
                reason: Unavailable::UnexpectedStage,
                ..
            }
        ));
    }
    assert_eq!(received.len(), 2);
}

#[test]
fn failed_event_delivery_is_a_session_error_and_order_never_wraps() {
    let server = super::super::servers::server_type(ServerProtocol::Project1999);
    let config = ClientConfig::new("EXAMPLE_ACCOUNT", "EXAMPLE_PASSWORD", "Test", "Tester");
    let mut handler = |_| anyhow::bail!("synthetic closed event bridge");
    let mut log = Events::new(&config, &mut handler);
    let error = receive(
        server,
        Stage::World,
        spell_tables::REGISTRATION_OPCODE,
        &registration(),
        &mut log,
    )
    .unwrap_err();
    assert!(error.is::<crate::client::EventDeliveryError>());
    log.spell_tables = Sequence {
        run_id: 17,
        revision: u64::MAX,
    };
    let error = receive(
        server,
        Stage::World,
        spell_tables::REGISTRATION_OPCODE,
        &registration(),
        &mut log,
    )
    .unwrap_err();
    assert!(error.to_string().contains("revision exhausted"));
    assert_eq!(log.spell_tables.revision, u64::MAX);
}

#[test]
fn actual_zone_receive_delivers_replay_before_any_profile_or_entered_event() {
    use crate::client::{CancellationToken, ClientIdentity};
    use std::{net::UdpSocket, panic::AssertUnwindSafe, thread, time::Duration};

    let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let port = peer.local_addr().unwrap().port();
    let stop = CancellationToken::default();
    let worker_stop = stop.clone();
    let worker = thread::spawn(move || {
        let mut config = ClientConfig::for_protocol(
            ServerProtocol::Project1999,
            "EXAMPLE_ACCOUNT",
            "EXAMPLE_PASSWORD",
            "Test",
            "Tester",
        );
        config.local_only = true;
        let identity = ClientIdentity::new("example-device", "example-user");
        let credentials = super::super::login::Credentials {
            account: "EXAMPLE_ACCOUNT".into(),
            key: *b"0123456789",
        };
        let context = super::super::CharacterSession {
            config: config.clone(),
            identity: &identity,
            credentials: &credentials,
            stop: &worker_stop,
            duration: Some(Duration::from_secs(3)),
            commands: None,
            sysinfo_reported_at: std::cell::Cell::new(0),
        };
        let mut received = Vec::new();
        let mut handler = |event| {
            if matches!(&event, ClientEvent::World(WorldEvent::SpellTable(_))) {
                worker_stop.cancel();
            }
            received.push(event);
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        let result =
            super::super::zone::run(&context, &mut None, "127.0.0.1", port, vec![], &mut log);
        drop(log);
        (result, received)
    });
    let exchange = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut packet = [0; 2048];
        let (length, address) = peer.recv_from(&mut packet).unwrap();
        assert!(length >= 10);
        assert_eq!(&packet[..2], &[0, 1]);
        let mut response = vec![0, 2];
        response.extend_from_slice(&packet[6..10]);
        response.extend_from_slice(&1234u32.to_be_bytes());
        response.extend_from_slice(&[2, 0, 0]);
        response.extend_from_slice(&512u32.to_be_bytes());
        peer.send_to(&response, address).unwrap();

        // First reliable incoming application; no profile/admission follows.
        let mut replay_packet = vec![0, 9, 0, 0];
        replay_packet.extend_from_slice(&spell_tables::REPLAY_OPCODE.to_le_bytes());
        replay_packet.extend_from_slice(&replay());
        let mut crc = crc32fast::Hasher::new();
        crc.update(&1234u32.to_le_bytes());
        crc.update(&replay_packet);
        replay_packet.extend_from_slice(&crc.finalize().to_be_bytes()[2..]);
        peer.send_to(&replay_packet, address).unwrap();
    }));
    if exchange.is_err() {
        stop.cancel();
    }
    let (result, received) = worker.join().unwrap();
    if let Err(error) = exchange {
        std::panic::resume_unwind(error);
    }
    assert!(matches!(result.unwrap(), super::super::ZoneExit::Stopped));
    assert!(!received
        .iter()
        .any(|event| matches!(event, ClientEvent::World(WorldEvent::Entered { .. }))));
    let tables: Vec<_> = received
        .iter()
        .filter_map(|event| match event {
            ClientEvent::World(WorldEvent::SpellTable(table)) => Some(table),
            _ => None,
        })
        .collect();
    assert_eq!(tables.len(), 1);
    assert!(matches!(tables[0].context.stage, Stage::Zone { .. }));
    assert!(matches!(tables[0].update, Update::Table(Table::Replay(_))));
}
