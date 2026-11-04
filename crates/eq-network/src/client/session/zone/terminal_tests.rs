//! A synthetic Titanium peer drives the actual zone loop, with no login service.
use super::*;
use crate::client::{
    session::login::Credentials, CancellationToken, ClientConfig, ClientIdentity, ServerProtocol,
};
use eq_network_game::{
    chat::OutboundChat,
    world::{CampStatus, WorldEvent},
};
use std::{
    cell::Cell,
    net::{SocketAddr, UdpSocket},
    sync::mpsc,
    thread,
};

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

struct Peer {
    socket: UdpSocket,
    client: SocketAddr,
    id: [u8; 4],
    sequence: u16,
}

impl Peer {
    fn negotiate(socket: UdpSocket) -> Self {
        let mut bytes = [0; 512];
        let (length, client) = socket.recv_from(&mut bytes).unwrap();
        assert_eq!(length, 14);
        assert_eq!(&bytes[..2], &[0, 1]);
        let id: [u8; 4] = bytes[6..10].try_into().unwrap();
        let mut response = vec![0, 2];
        response.extend_from_slice(&id);
        response.extend_from_slice(&SEED.to_be_bytes());
        response.extend_from_slice(&[2, 0, 0]);
        response.extend_from_slice(&65507u32.to_be_bytes());
        socket.send_to(&response, client).unwrap();
        Self {
            socket,
            client,
            id,
            sequence: 0,
        }
    }

    fn expect(&self, opcode: u16) {
        loop {
            let mut bytes = [0; 2048];
            let (length, sender) = self.socket.recv_from(&mut bytes).unwrap();
            assert_eq!(sender, self.client);
            if bytes[..2] != [0, 9] {
                continue;
            }
            let sequence = &bytes[2..4];
            self.socket
                .send_to(&datagram(0x15, sequence), self.client)
                .unwrap();
            assert!(length >= 8);
            if u16::from_le_bytes(bytes[4..6].try_into().unwrap()) == opcode {
                return;
            }
        }
    }

    fn application(&mut self, opcode: u16, body: &[u8]) -> Vec<u8> {
        let mut packet = vec![0, 9];
        packet.extend_from_slice(&self.sequence.to_be_bytes());
        self.sequence = self.sequence.wrapping_add(1);
        packet.extend_from_slice(&opcode.to_le_bytes());
        packet.extend_from_slice(body);
        packet
    }

    fn send(&mut self, opcode: u16, body: &[u8]) {
        let packet = self.application(opcode, body);
        self.socket
            .send_to(&datagram(9, &packet[2..]), self.client)
            .unwrap();
    }

    fn admit(&mut self) {
        self.expect(0x7213);
        self.send(0x75df, &vec![0; 19592]);
        let mut spawn = vec![0; 385];
        spawn[7..13].copy_from_slice(b"Tester");
        spawn[340..344].copy_from_slice(&7u32.to_le_bytes());
        self.send(0x7213, &spawn);
        self.send(0x254d, &[0; 12]);
        self.expect(0x5966);
        let mut zone = [0; 96];
        zone[64..71].copy_from_slice(b"example");
        self.send(0x0920, &zone);
        self.expect(0x0322);
        self.send(0x0587, &[]);
        self.expect(0x0587);
        self.send(0x0587, &[]);
        self.expect(0x61ff);
    }

    fn finish(&mut self) {
        let ordinary = self.application(0x1580, &[13, 0, 1, 1, 0x1c, 0x0c, 0, 0]);
        let logout = self.application(0x3cdc, &[]);
        let mut terminal = vec![0, 5];
        terminal.extend_from_slice(&self.id);
        let mut body = Vec::new();
        for part in [&ordinary, &logout, &terminal] {
            body.push(u8::try_from(part.len()).unwrap());
            body.extend_from_slice(part);
        }
        self.socket
            .send_to(&datagram(3, &body), self.client)
            .unwrap();
    }
}

#[test]
fn the_zone_loop_refuses_queued_work_and_drains_the_final_logout() {
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let port = server.local_addr().unwrap().port();
    let worker = thread::spawn(move || {
        let mut peer = Peer::negotiate(server);
        peer.admit();
        peer.finish();
        peer
    });
    let config = ClientConfig::for_protocol(
        ServerProtocol::EqEmu,
        "EXAMPLE",
        "EXAMPLE",
        "Test Server",
        "Tester",
    );
    let identity = ClientIdentity::new("example-device", "example-user");
    let credentials = Credentials {
        account: "LS#1".into(),
        key: *b"0123456789",
    };
    let stop = CancellationToken::default();
    let (commands, receiver) = mpsc::channel();
    let context = CharacterSession {
        config: config.clone(),
        identity: &identity,
        credentials: &credentials,
        stop: &stop,
        duration: Some(Duration::from_secs(5)),
        commands: Some(&receiver),
        sysinfo_reported_at: Cell::new(0),
        answer_list_polls: Cell::new(false),
    };
    let mut seen = Vec::new();
    let mut handler = |event| {
        match &event {
            ClientEvent::World(WorldEvent::Entered { session_id, .. }) => commands
                .send(ClientCommand::ExitAtOnce {
                    session_id: *session_id,
                    created: Instant::now(),
                })
                .unwrap(),
            ClientEvent::World(WorldEvent::TimeOfDay(_)) => commands
                .send(ClientCommand::SendChat(OutboundChat::Say(
                    "Queued after termination".into(),
                )))
                .unwrap(),
            _ => (),
        }
        seen.push(event);
        Ok(())
    };
    let mut log = Events::new(&config, &mut handler);
    let exit = run(&context, &mut None, "127.0.0.1", port, Vec::new(), &mut log).unwrap();
    assert!(matches!(exit, ZoneExit::CharacterSelect));
    drop(log);
    assert!(seen
        .iter()
        .any(|event| matches!(event, ClientEvent::World(WorldEvent::TimeOfDay(_)))));
    assert!(seen.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::Camp(CampStatus::Camped))
    )));
    assert!(seen.iter().any(|event| matches!(event, ClientEvent::Diagnostic(message) if message.contains("The zone connection ended"))));
    let peer = worker.join().unwrap();
    peer.socket.set_nonblocking(true).unwrap();
    assert!(
        peer.socket.recv(&mut [0; 512]).is_err(),
        "no outbound command, ACK or close after termination"
    );
}

struct Ended;

impl super::super::feature::Sink for Ended {
    fn send(&mut self, _packet: &eq_network_game::command::EncodedCommand) -> Result<()> {
        Err(eq_network_transport::modern::SessionTermination::OutOfSession.into())
    }

    fn send_unreliable(&mut self, packet: &eq_network_game::command::EncodedCommand) -> Result<()> {
        self.send(packet)
    }

    fn last_received_seconds(&self) -> u64 {
        0
    }

    fn termination(&self) -> Option<eq_network_transport::modern::SessionTermination> {
        Some(eq_network_transport::modern::SessionTermination::OutOfSession)
    }
}

struct Respond(Option<eq_network_transport::modern::SessionTermination>);

impl Feature for Respond {
    fn observe(
        &mut self,
        _message: &Message,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match self.0 {
            Some(reason) => Err(reason.into()),
            None => out.request(&eq_network_game::request::Request::Logout),
        }
    }
}

#[test]
fn only_the_matching_latched_send_refusal_is_deferred() {
    use super::super::{camp::Camp, feature::testing};
    use eq_network_transport::modern::SessionTermination;

    for different in [
        None,
        Some(SessionTermination::PeerDisconnected { reason: None }),
    ] {
        let config = ClientConfig::new("EXAMPLE", "EXAMPLE", "Test", "Tester");
        let mut handler = |_| Ok(());
        let mut log = Events::new(&config, &mut handler);
        let mut world = World::new(5);
        let mut camp = Camp::default();
        testing::run(|out| {
            camp.handle(
                &ClientCommand::Camp {
                    session_id: 5,
                    created: Instant::now(),
                },
                &mut world,
                out,
            )
        })
        .result
        .unwrap();
        let mut features = Features {
            provided: vec![Box::new(Respond(different)), Box::new(camp)],
            choices: Vec::new(),
            spell_landing_flag: None,
            answers_list_polls: false,
        };
        let result = features.observe(
            &Message::LoggedOut,
            &mut world,
            &mut Out {
                sink: &mut Ended,
                log: &mut log,
                wire: &super::super::wire::Titanium,
                sender: eq_network_game::request::Sender {
                    name: "Tester",
                    spawn_id: Some(7),
                },
            },
        );
        if let Some(expected) = different {
            assert_eq!(
                result.unwrap_err().downcast_ref::<SessionTermination>(),
                Some(&expected)
            );
            assert!(world.take_exit().is_none());
        } else {
            result.unwrap();
            assert!(matches!(world.take_exit(), Some(ZoneExit::CharacterSelect)));
        }
    }
}
