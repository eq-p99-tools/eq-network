use super::*;
use crate::client::{session::Events, ClientConfig};
use eq_network_game::{command, request::Sender};
use eq_network_transport::modern::{Session, SessionTermination};
use std::{net::UdpSocket, thread};

const SEED: u32 = 0x1122_3344;

fn datagram(opcode: u16, body: &[u8]) -> Vec<u8> {
    let mut packet = opcode.to_be_bytes().to_vec();
    packet.extend_from_slice(body);
    let mut crc = crc32fast::Hasher::new();
    crc.update(&SEED.to_le_bytes());
    crc.update(&packet);
    packet.extend_from_slice(&crc.finalize().to_be_bytes()[2..]);
    packet
}

fn logout_peer(reply: bool) -> (Session, thread::JoinHandle<UdpSocket>) {
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let address = server.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let mut bytes = [0; 512];
        let (length, client) = server.recv_from(&mut bytes).unwrap();
        assert_eq!(length, 14);
        assert_eq!(&bytes[..2], &[0, 1]);
        let id = bytes[6..10].to_vec();
        let mut response = vec![0, 2];
        response.extend_from_slice(&id);
        response.extend_from_slice(&SEED.to_be_bytes());
        response.extend_from_slice(&[2, 0, 0]);
        response.extend_from_slice(&512u32.to_be_bytes());
        server.send_to(&response, client).unwrap();
        for (sequence, command) in [command::titanium_camp(), command::titanium_logout()]
            .into_iter()
            .enumerate()
        {
            let (length, sender) = server.recv_from(&mut bytes).unwrap();
            assert_eq!(sender, client);
            let mut body = u16::try_from(sequence).unwrap().to_be_bytes().to_vec();
            body.extend_from_slice(&command.opcode.to_le_bytes());
            body.extend_from_slice(&command.body);
            assert_eq!(&bytes[..length], datagram(9, &body));
        }
        // One reliable, fully ordered LogoutReply, then the matching terminal
        // control in the same datagram. This reply is the application fact;
        // transport termination alone cannot stand in for it.
        let mut terminal = vec![0, 5];
        terminal.extend_from_slice(&id);
        let mut combined = Vec::new();
        if reply {
            let application = [0, 9, 0, 0, 0xdc, 0x3c];
            combined.push(u8::try_from(application.len()).unwrap());
            combined.extend_from_slice(&application);
        }
        combined.push(u8::try_from(terminal.len()).unwrap());
        combined.extend_from_slice(&terminal);
        server.send_to(&datagram(3, &combined), client).unwrap();
        server
    });
    (Session::connect(address, false).unwrap(), worker)
}

#[test]
fn a_final_logout_reply_precedes_termination_in_the_camp_consumer() {
    let (mut connection, worker) = logout_peer(true);
    let config = ClientConfig::new("EXAMPLE", "EXAMPLE", "Test Server", "Tester");
    let mut events = Vec::new();
    let mut handler = |event| {
        events.push(event);
        Ok(())
    };
    let mut log = Events::new(&config, &mut handler);
    let mut camp = Camp::default();
    let mut world = World::new(5);
    let sender = Sender {
        name: "Tester",
        spawn_id: Some(7),
    };
    {
        let mut out = Out {
            sink: &mut connection,
            log: &mut log,
            wire: &super::super::wire::Titanium,
            sender,
        };
        camp.handle(
            &ClientCommand::Camp {
                session_id: 5,
                created: Instant::now(),
            },
            &mut world,
            &mut out,
        )
        .unwrap();
        camp.tick(Instant::now() + CAMP_DURATION, &mut world, &mut out)
            .unwrap();
    }
    let packet = connection
        .receive()
        .unwrap()
        .expect("the final logout reply");
    let messages = eq_network_game::message::titanium(packet.opcode, &packet.body);
    assert!(matches!(messages.as_slice(), [Message::LoggedOut]));
    for message in messages {
        camp.observe(
            &message,
            &mut world,
            &mut Out {
                sink: &mut connection,
                log: &mut log,
                wire: &super::super::wire::Titanium,
                sender,
            },
        )
        .unwrap();
    }
    assert!(matches!(world.take_exit(), Some(ZoneExit::CharacterSelect)));
    let error = connection.receive().unwrap_err();
    assert_eq!(
        error.downcast_ref::<SessionTermination>(),
        Some(&SessionTermination::PeerDisconnected { reason: None })
    );
    connection.close().unwrap();
    drop(log);
    assert!(matches!(
        events.last(),
        Some(ClientEvent::World(WorldEvent::Camp(CampStatus::Camped)))
    ));
    let server = worker.join().unwrap();
    server.set_nonblocking(true).unwrap();
    assert!(
        server.recv(&mut [0; 512]).is_err(),
        "no terminal reply or ACK"
    );
}

#[test]
fn termination_without_a_reply_does_not_confirm_a_camp() {
    let (mut connection, worker) = logout_peer(false);
    for packet in [command::titanium_camp(), command::titanium_logout()] {
        connection.send(packet.opcode, &packet.body).unwrap();
    }
    let mut camp = Camp::default();
    camp.logout_sent(Instant::now());
    let mut world = World::new(5);
    let error = connection.receive().unwrap_err();
    assert_eq!(
        error.downcast_ref::<SessionTermination>(),
        Some(&SessionTermination::PeerDisconnected { reason: None })
    );
    camp.connection_ended(&mut world);
    assert!(
        world.take_exit().is_none(),
        "the receive error must propagate"
    );
    assert!(camp.logging_out());
    worker.join().unwrap();
}
