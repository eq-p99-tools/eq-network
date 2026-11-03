//! Synthetic peers for Quarm's old stream and TAKP's rdplib startup rules.
use super::*;
use crate::client::{
    CancellationToken, ClientCommand, ClientConfig, ClientIdentity, ServerProtocol,
};
use eq_network_game::creation::{CreationOutcome, NewCharacter, EQMAC_CREATE_OPCODE};
use std::{io, net::UdpSocket, sync::mpsc, thread};

struct WorldPeer {
    protocol: ServerProtocol,
    socket: UdpSocket,
    client: Option<SocketAddr>,
    sequence: u16,
    next_arq: u16,
    client_arq: u16,
    acknowledged: Option<u16>,
    fragments: Vec<u8>,
    fragment_opcode: Option<u16>,
    sent_start: bool,
}

impl WorldPeer {
    fn new(protocol: ServerProtocol) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        Self {
            protocol,
            socket,
            client: None,
            sequence: 0,
            next_arq: if protocol == ServerProtocol::Takp {
                100
            } else {
                0
            },
            client_arq: 0,
            acknowledged: None,
            fragments: Vec::new(),
            fragment_opcode: None,
            sent_start: false,
        }
    }

    /// Quarm omits ARSP zero; TAKP accepts an arbitrary SYN/message origin.
    fn send(
        &mut self,
        reliable: bool,
        opcode: Option<u16>,
        body: &[u8],
        fragment: Option<(u16, u16)>,
    ) {
        let mut flags = (u8::from(reliable) * 2) | (u8::from(fragment.is_some()) * 8);
        if reliable && !self.sent_start && self.protocol == ServerProtocol::Takp {
            flags |= 0x20;
            self.sent_start = true;
        }
        let ack = self.client_arq != 0 || self.protocol == ServerProtocol::Takp;
        let mut wire = vec![flags, if ack { 4 } else { 0 }];
        wire.extend(self.sequence.to_be_bytes());
        self.sequence = self.sequence.wrapping_add(1);
        if ack {
            wire.extend(self.client_arq.to_be_bytes());
        }
        if reliable {
            wire.extend(self.next_arq.to_be_bytes());
            self.next_arq = self.next_arq.wrapping_add(1);
        }
        if let Some((current, total)) = fragment {
            wire.extend(0_u16.to_be_bytes());
            wire.extend(current.to_be_bytes());
            wire.extend(total.to_be_bytes());
        }
        if let Some(opcode) = opcode {
            wire.extend(opcode.to_be_bytes());
        }
        wire.extend(body);
        wire.extend(crc32fast::hash(&wire).to_be_bytes());
        self.socket.send_to(&wire, self.client.unwrap()).unwrap();
    }

    fn character_list(&mut self, name: &str) {
        let mut body = [0; 1620];
        put_string(&mut body[..64], name).unwrap();
        let mut offset = 0;
        for current in 0..4 {
            let length = if current == 0 { 510 } else { 512 }.min(body.len() - offset);
            self.send(
                true,
                (current == 0).then_some(WORLD_CHARACTER_LIST),
                &body[offset..offset + length],
                Some((current, 4)),
            );
            offset += length;
        }
    }

    /// Receive client applications, checking CRC, reliable order and ACKs.
    fn receive(&mut self) -> Option<(u16, Vec<u8>)> {
        let mut wire = [0; 2048];
        let (length, address) = match self.socket.recv_from(&mut wire) {
            Ok(received) => received,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return None
            }
            Err(error) => panic!("synthetic peer receive: {error}"),
        };
        if self.client.is_none() && self.protocol == ServerProtocol::Takp {
            // rdplib cannot create a connection without SYN/SEQStart.
            assert_ne!(wire[0] & 0x20, 0, "TAKP startup requires SYN");
        }
        assert_eq!(*self.client.get_or_insert(address), address);
        let end = length - 4;
        assert_eq!(
            crc32fast::hash(&wire[..end]).to_be_bytes(),
            wire[end..length]
        );
        assert_eq!(wire[1] & !4, 0);
        let mut position = 4;
        if wire[1] & 4 != 0 {
            self.acknowledged = Some(word(&wire, &mut position));
        }
        if wire[0] & 2 != 0 {
            let arq = word(&wire, &mut position);
            if wire[0] & 0x20 != 0 {
                // EQOldStream rejects a redundant SEQStart before processing
                // ACKs or applications. A fresh server rejects startup at one.
                if self.protocol == ServerProtocol::Quarm
                    && u32::from(self.client_arq) == u32::from(arq).wrapping_sub(1)
                {
                    return None;
                }
                // Like EQOldStream, sequence-start resets the receive origin.
                self.client_arq = arq.wrapping_sub(1);
            }
            assert_eq!(arq, self.client_arq.wrapping_add(1));
            self.client_arq = arq;
            if arq != 0 || self.protocol == ServerProtocol::Takp {
                self.send(false, None, &[], None);
            }
        }
        let fragment = if wire[0] & 8 != 0 {
            assert_eq!(word(&wire, &mut position), 0);
            Some((word(&wire, &mut position), word(&wire, &mut position)))
        } else {
            None
        };
        if wire[0] & 0x10 != 0 {
            position += if wire[0] & 2 != 0 { 2 } else { 1 };
        }
        assert_eq!(wire[0] & 0x44, 0, "world closed before handoff");
        if position == end {
            return None;
        }
        if let Some((current, total)) = fragment {
            if current == 0 {
                self.fragment_opcode = Some(word(&wire, &mut position));
                self.fragments.clear();
            }
            self.fragments.extend(&wire[position..end]);
            return (current + 1 == total).then(|| {
                (
                    self.fragment_opcode.take().unwrap(),
                    std::mem::take(&mut self.fragments),
                )
            });
        }
        Some((word(&wire, &mut position), wire[position..end].to_vec()))
    }

    fn expect(&mut self, opcode: u16, length: usize) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            assert!(
                Instant::now() < deadline,
                "missing client opcode {opcode:#06x}"
            );
            if let Some(packet) = self.receive() {
                assert_eq!((packet.0, packet.1.len()), (opcode, length));
                return;
            }
        }
    }

    /// Outlast the unchanged thirty-second reliable-send retry budget.
    fn dwell(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(31);
        let mut heartbeat = Instant::now();
        while Instant::now() < deadline {
            assert!(
                self.receive().is_none(),
                "idle selection retransmitted an application"
            );
            if Instant::now() >= heartbeat {
                self.send(true, None, &[], None);
                heartbeat = Instant::now() + Duration::from_secs(1);
            }
        }
        assert!(
            self.acknowledged.is_some_and(|ack| ack > 20),
            "idle world was not pumped"
        );
    }
}

fn word(wire: &[u8], position: &mut usize) -> u16 {
    let value = u16::from_be_bytes(wire[*position..*position + 2].try_into().unwrap());
    *position += 2;
    value
}

/// Exercise the production world loop and real UDP transport without a service.
fn selection_creation_and_handoff(protocol: ServerProtocol, camp_return: bool) {
    let mut peer = WorldPeer::new(protocol);
    let address = peer.socket.local_addr().unwrap();
    let (commands, incoming) = mpsc::channel();
    let (events, heard) = mpsc::channel();
    let stop = CancellationToken::default();
    let worker_stop = stop.clone();
    let worker = thread::spawn(move || {
        let config = ClientConfig::for_protocol(
            protocol,
            "EXAMPLE_ACCOUNT",
            "EXAMPLE_PASSWORD",
            "Synthetic World",
            "",
        );
        let credentials = Credentials {
            account: "LS#1".into(),
            key: *b"0123456789",
        };
        let identity = ClientIdentity::new("example-device", "example-user");
        let context = CharacterSession {
            config: config.clone(),
            identity: &identity,
            credentials: &credentials,
            stop: &worker_stop,
            duration: None,
            commands: Some(&incoming),
            sysinfo_reported_at: std::cell::Cell::new(0),
        };
        let mut handler = |event| {
            events.send(event).unwrap();
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        if camp_return {
            log.character = "Examplemonk".into();
        }
        world_at(&context, address, (false, false), &mut log).map(|destination| {
            destination
                .map(|destination| (destination.character, destination.host, destination.port))
        })
    });
    // Cancel on assertion failure as well, so a failing fixture leaves no worker.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        peer.expect(WORLD_LOGIN, 200);
        peer.send(true, Some(0xc341), &[0; 244], None);
        peer.send(true, Some(0x0710), &[0; 65], None);
        peer.send(true, Some(WORLD_ENTER), &[0], None);
        peer.send(true, Some(0xd841), &7_u32.to_le_bytes(), None);
        peer.character_list("Examplemonk");
        let selection = next_selection(&heard);
        // Opening/changing the creation form sends no world application; it
        // shares this selection state until the user submits a creation.
        peer.dwell();
        commands
            .send(ClientCommand::CreateCharacter {
                selection_id: selection,
                character: NewCharacter::with_points_in("Testcleric", (1, 2, 0), (212, 2), 4)
                    .unwrap(),
            })
            .unwrap();
        peer.expect(EQMAC_APPROVE_NAME_OPCODE, 76);
        peer.send(true, Some(EQMAC_APPROVE_NAME_OPCODE), &[1], None);
        peer.expect(EQMAC_CREATE_OPCODE, 8452);
        peer.character_list("Testcleric");
        let selection = next_selection(&heard);
        commands
            .send(ClientCommand::SelectCharacter {
                selection_id: selection,
                slot: 0,
            })
            .unwrap();
        peer.expect(WORLD_ENTER, 64);
        let mut destination = [0; 130];
        put_string(&mut destination[..128], "192.0.2.1").unwrap();
        destination[128..].copy_from_slice(&7000_u16.to_be_bytes());
        peer.send(true, Some(WORLD_ZONE_SERVER), &destination, None);
    }));
    if let Err(panic) = result {
        stop.cancel();
        let _ = worker.join();
        std::panic::resume_unwind(panic);
    }
    let (character, host, port) = worker.join().unwrap().unwrap().unwrap();
    assert_eq!(
        (character.as_str(), host.as_str(), port),
        ("Testcleric", "192.0.2.1", 7000)
    );
}

fn next_selection(events: &mpsc::Receiver<ClientEvent>) -> u64 {
    loop {
        match events.recv_timeout(Duration::from_secs(3)).unwrap() {
            ClientEvent::World(WorldEvent::CharacterSelection { selection_id, .. }) => {
                return selection_id
            }
            ClientEvent::World(WorldEvent::CharacterCreation { outcome, .. }) => {
                assert_eq!(outcome, CreationOutcome::Created);
            }
            _ => (),
        }
    }
}

#[test]
fn initial_selection_can_dwell_create_and_enter_without_an_extra_login_request() {
    selection_creation_and_handoff(ServerProtocol::Quarm, false);
}

#[test]
fn camp_return_selection_can_dwell_create_and_enter_without_automatic_reentry() {
    selection_creation_and_handoff(ServerProtocol::Quarm, true);
}

#[test]
fn takp_retains_syn_and_accepts_the_nonzero_origin_through_selection_and_creation() {
    selection_creation_and_handoff(ServerProtocol::Takp, false);
}
