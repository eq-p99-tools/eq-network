//! Synthetic loopback peers exercise the public API without accounts or captures.
use anyhow::{bail, Result};
use eq_network::chat::OutboundChat;
use eq_network::client::{
    CancellationToken, Client, ClientCommand, ClientConfig, ClientEvent, ClientIdentity,
    ConnectionStage, ConnectionState, LoginError, RunOptions,
};
use std::{
    net::{SocketAddr, UdpSocket},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn config(port: u16) -> ClientConfig {
    let mut config = ClientConfig::new(
        "EXAMPLE_ACCOUNT",
        "EXAMPLE_PASSWORD",
        "Test Server",
        "ExampleCharacter",
    );
    config.host = "127.0.0.1".into();
    config.port = port;
    config
}

fn client(config: ClientConfig) -> Client {
    Client::new(config, ClientIdentity::new("test-device", "test-user")).unwrap()
}

fn peer() -> UdpSocket {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    socket
}

fn receive(socket: &UdpSocket) -> (Vec<u8>, SocketAddr) {
    let mut buffer = [0; 2048];
    let (length, address) = socket.recv_from(&mut buffer).unwrap();
    (buffer[..length].to_vec(), address)
}

fn negotiate(socket: &UdpSocket) -> (SocketAddr, Vec<u8>) {
    let (request, address) = receive(socket);
    assert_eq!(&request[..6], &[0, 1, 0, 0, 0, 2]);
    let id = request[6..10].to_vec();
    let mut response = vec![0, 2];
    response.extend(&id);
    response.extend(1234u32.to_be_bytes());
    response.extend([2, 0, 0]);
    response.extend(512u32.to_be_bytes());
    socket.send_to(&response, address).unwrap();
    let (ready, sender) = receive(socket);
    assert_eq!(sender, address);
    assert_eq!(&ready[..6], &[0, 9, 0, 0, 1, 0]);
    (address, id)
}

fn closed_packet(id: &[u8]) -> Vec<u8> {
    let mut packet = vec![0, 5];
    packet.extend(id);
    let mut crc = crc32fast::Hasher::new();
    crc.update(&1234u32.to_le_bytes());
    crc.update(&packet);
    packet.extend(&crc.finalize().to_be_bytes()[2..]);
    packet
}

#[test]
fn cancellation_before_start_never_resolves_or_connects() {
    let mut settings = config(1);
    settings.host = "this host cannot resolve".into();
    let cancel = CancellationToken::default();
    cancel.cancel();
    let mut states = Vec::new();
    client(settings)
        .run(&cancel, RunOptions::default(), |event| {
            if let ClientEvent::Status(status) = event {
                states.push(status.state);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(states, [ConnectionState::Stopped]);
}

#[test]
fn command_enabled_run_can_stop_before_connecting() {
    let mut settings = config(1);
    settings.host = "this host cannot resolve".into();
    let cancel = CancellationToken::default();
    cancel.cancel();
    let (sender, commands) = mpsc::sync_channel(1);
    sender
        .try_send(ClientCommand::SendChat(OutboundChat::Say("ok".into())))
        .unwrap();
    let mut states = Vec::new();
    client(settings)
        .run_with_commands(&cancel, RunOptions::default(), &commands, |event| {
            if let ClientEvent::Status(status) = event {
                states.push(status.state);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(states, [ConnectionState::Stopped]);
}

#[test]
fn handler_failure_is_returned_without_reconnecting_or_calling_it_again() {
    let mut settings = config(1);
    settings.host = "this host cannot resolve".into();
    let mut calls = 0;
    let error = client(settings)
        .run(&CancellationToken::default(), RunOptions::default(), |_| {
            calls += 1;
            bail!("UI event receiver closed")
        })
        .unwrap_err();
    assert_eq!(calls, 1);
    assert!(error.to_string().contains("UI event receiver closed"));
}

#[test]
fn cancellation_interrupts_udp_negotiation() {
    let socket = peer();
    let engine = client(config(socket.local_addr().unwrap().port()));
    let cancel = CancellationToken::default();
    let worker_cancel = cancel.clone();
    let (done, result) = mpsc::channel();
    let worker = thread::spawn(move || {
        done.send(engine.run(&worker_cancel, RunOptions::default(), |_| Ok(())))
            .unwrap();
    });
    let (request, _) = receive(&socket);
    assert_eq!(&request[..2], &[0, 1]);
    cancel.cancel();
    result
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
}

#[test]
fn cancellation_during_login_closes_the_session_and_stops_without_retry() {
    let socket = peer();
    let engine = client(config(socket.local_addr().unwrap().port()));
    let cancel = CancellationToken::default();
    let worker_cancel = cancel.clone();
    let (done, result) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut states = Vec::new();
        let outcome = engine.run(&worker_cancel, RunOptions::default(), |event| {
            if let ClientEvent::Status(status) = event {
                states.push(status.state);
            }
            Ok(())
        });
        done.send((outcome, states)).unwrap();
    });
    let (address, id) = negotiate(&socket);
    cancel.cancel();
    let (packet, sender) = receive(&socket);
    assert_eq!(sender, address);
    assert_eq!(packet, closed_packet(&id));
    let (outcome, states) = result.recv_timeout(Duration::from_secs(2)).unwrap();
    outcome.unwrap();
    assert_eq!(
        states,
        [ConnectionState::Connecting, ConnectionState::Stopped]
    );
    worker.join().unwrap();
}

#[test]
fn server_disconnect_retries_only_when_requested_and_retry_wait_is_cancellable() {
    for reconnect in [false, true] {
        let socket = peer();
        let engine = client(config(socket.local_addr().unwrap().port()));
        let server = thread::spawn(move || {
            let (address, id) = negotiate(&socket);
            socket.send_to(&closed_packet(&id), address).unwrap();
        });
        let cancel = CancellationToken::default();
        let mut states = Vec::new();
        let mut retries = 0;
        let start = Instant::now();
        let outcome = engine.run(
            &cancel,
            {
                let mut options = RunOptions::default();
                options.reconnect = reconnect;
                options
            },
            |event| {
                match event {
                    ClientEvent::Status(session_status) => states.push(session_status.state),
                    ClientEvent::Reconnecting { delay_seconds, .. } => {
                        retries += 1;
                        assert_eq!(delay_seconds, 30);
                        cancel.cancel();
                    }
                    _ => (),
                }
                Ok(())
            },
        );
        server.join().unwrap();
        assert_eq!(retries, usize::from(reconnect));
        assert_eq!(
            &states[..2],
            &[ConnectionState::Connecting, ConnectionState::Disconnected]
        );
        if reconnect {
            outcome.unwrap();
            assert_eq!(states.last(), Some(&ConnectionState::Stopped));
        } else {
            assert!(outcome.is_err());
        }
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}

#[test]
fn invalid_settings_fail_before_start_without_disclosing_credentials() {
    let mut settings = config(1);
    settings.character = "x".repeat(64);
    let result: Result<_> = Client::new(settings, ClientIdentity::new("test", "test"));
    let error = result.err().unwrap().to_string();
    assert!(error.contains("character name"));
    assert!(!error.contains("EXAMPLE_ACCOUNT") && !error.contains("EXAMPLE_PASSWORD"));
}

/// Construct encrypted replies from synthetic values, never from a packet capture.
fn login_reply() -> Vec<u8> {
    use eq_network_login::crypto::{des_encrypt, DesKeyIv};
    let mut clear = vec![0; 32];
    clear[0] = 1;
    clear[8..12].copy_from_slice(&eq_network_login::LOGIN_RESULT_FAILURE_STATUS.to_le_bytes());
    let mut body = vec![0; 10];
    body[0] = 3;
    body[5] = 2;
    body.extend(des_encrypt(&clear, DesKeyIv::default()));
    body
}

fn application_packet(sequence: u16, opcode: u16, body: &[u8]) -> Vec<u8> {
    let mut packet = vec![0, 9];
    packet.extend(sequence.to_be_bytes());
    packet.extend(opcode.to_le_bytes());
    packet.extend(body);
    let mut crc = crc32fast::Hasher::new();
    crc.update(&1234u32.to_le_bytes());
    crc.update(&packet);
    packet.extend(&crc.finalize().to_be_bytes()[2..]);
    packet
}

#[test]
fn rejected_credentials_close_login_and_never_enter_the_retry_loop() {
    for reconnect in [true, false] {
        let socket = peer();
        let engine = client(config(socket.local_addr().unwrap().port()));
        let cancel = CancellationToken::default();
        let worker_cancel = cancel.clone();
        let (done, result) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut retries = 0;
            let mut stages = Vec::new();
            let outcome = engine.run(
                &worker_cancel,
                {
                    let mut options = RunOptions::default();
                    options.reconnect = reconnect;
                    options
                },
                |event| {
                    if let ClientEvent::Progress(stage) = &event {
                        stages.push(*stage);
                    }
                    if matches!(event, ClientEvent::Reconnecting { .. }) {
                        retries += 1;
                    }
                    Ok(())
                },
            );
            done.send((outcome, retries, stages)).unwrap();
        });
        let (address, id) = negotiate(&socket);
        socket
            .send_to(&application_packet(0, 0x16, &[]), address)
            .unwrap();
        loop {
            let (packet, sender) = receive(&socket);
            assert_eq!(sender, address);
            if packet.starts_with(&[0, 9, 0, 1, 2, 0]) {
                break;
            }
        }
        let mut response = login_reply();
        // The proven SSO detector also accepts the optional one-byte trailer.
        if reconnect {
            response.push(0);
        }
        socket
            .send_to(&application_packet(1, 0x17, &response), address)
            .unwrap();
        let outcome = result.recv_timeout(Duration::from_secs(3));
        cancel.cancel();
        worker.join().unwrap();
        let (outcome, retries, stages) =
            outcome.expect("credential rejection must end immediately");
        let error = outcome.unwrap_err();
        assert_eq!(
            error.downcast_ref::<LoginError>(),
            Some(&LoginError::InvalidCredentials)
        );
        assert_eq!(retries, 0);
        assert_eq!(
            stages,
            [
                ConnectionStage::ConnectingLogin,
                ConnectionStage::Authenticating
            ]
        );
        assert!(
            !error.to_string().contains("EXAMPLE_ACCOUNT")
                && !error.to_string().contains("EXAMPLE_PASSWORD")
        );
        loop {
            let (packet, _) = receive(&socket);
            if packet.starts_with(&[0, 5]) {
                assert_eq!(packet, closed_packet(&id));
                break;
            }
        }
        // No server-list or world request follows the rejected login.
        socket
            .set_read_timeout(Some(Duration::from_millis(150)))
            .unwrap();
        assert!(socket.recv_from(&mut [0; 2048]).is_err());
    }
}

/// The next packet from the client that starts with `prefix`, skipping
/// acknowledgements, statistics and retransmissions.
fn packet_starting(socket: &UdpSocket, address: SocketAddr, prefix: &[u8]) -> Vec<u8> {
    loop {
        let (packet, sender) = receive(socket);
        assert_eq!(sender, address);
        if packet.starts_with(prefix) {
            return packet;
        }
    }
}

/// A login reply that accepts a synthetic account with a synthetic key.
fn accepted_reply() -> Vec<u8> {
    use eq_network_login::crypto::{des_encrypt, DesKeyIv};
    let mut clear = vec![0; 32];
    clear[0] = 1;
    clear[8..12].copy_from_slice(&12345u32.to_le_bytes());
    clear[12..22].copy_from_slice(b"EXAMPLEKEY");
    let mut body = vec![0; 10];
    body[0] = 3;
    body[5] = 2;
    body.extend(des_encrypt(&clear, DesKeyIv::default()));
    body
}

/// A Titanium server list as `EQEmu`'s login server writes it: the address,
/// type flags, runtime id, name, two codes, status and player count.
fn server_list(worlds: &[(&str, u32, u32, u32, u32)]) -> Vec<u8> {
    let mut body = vec![0; 16];
    body.extend(u32::try_from(worlds.len()).unwrap().to_le_bytes());
    for (name, flags, runtime_id, status, players) in worlds {
        body.extend(b"127.0.0.1\0");
        body.extend(flags.to_le_bytes());
        body.extend(runtime_id.to_le_bytes());
        body.extend(name.as_bytes());
        body.extend(b"\0us\0en\0");
        body.extend(status.to_le_bytes());
        body.extend(players.to_le_bytes());
    }
    body
}

/// A Titanium play response: whether the world takes the player, and the
/// login string id that says why not.
fn play_response(accepted: bool, message: u32) -> Vec<u8> {
    let mut body = vec![0; 20];
    body[10] = u8::from(accepted);
    body[11..15].copy_from_slice(&message.to_le_bytes());
    body
}

/// A login server that lists three worlds, refuses the first one the
/// player asks for as unavailable, and lets them play on the next.
fn serve_a_refusal_then_a_world(socket: &UdpSocket) {
    let (address, id) = negotiate(socket);
    socket
        .send_to(&application_packet(0, 0x16, &[]), address)
        .unwrap();
    packet_starting(socket, address, &[0, 9, 0, 1, 2, 0]);
    socket
        .send_to(&application_packet(1, 0x17, &accepted_reply()), address)
        .unwrap();
    packet_starting(socket, address, &[0, 9, 0, 2, 4, 0]);
    let worlds = server_list(&[
        ("Example Down", 1, 11, 1, 0),
        ("Example Busy", 8, 22, 0, 40),
        ("Example Up", 1, 33, 0, 7),
    ]);
    socket
        .send_to(&application_packet(2, 0x18, &worlds), address)
        .unwrap();
    // The player's choice, by the world's runtime id; the login server
    // refuses it as unavailable, and the list stays up.
    let play = packet_starting(socket, address, &[0, 9, 0, 3, 0x0d, 0]);
    assert_eq!(play[16..20], 22u32.to_le_bytes());
    socket
        .send_to(
            &application_packet(3, 0x21, &play_response(false, 326)),
            address,
        )
        .unwrap();
    let play = packet_starting(socket, address, &[0, 9, 0, 4, 0x0d, 0]);
    assert_eq!(play[16..20], 33u32.to_le_bytes());
    socket
        .send_to(
            &application_packet(4, 0x21, &play_response(true, 101)),
            address,
        )
        .unwrap();
    assert_eq!(
        packet_starting(socket, address, &[0, 5]),
        closed_packet(&id)
    );
}

#[test]
fn a_player_chooses_the_world_from_the_login_servers_list() {
    use eq_network::world::WorldEvent;
    use eq_network_game::servers::{ServerChoice, ServerRefusal, ServerStatus};
    let socket = peer();
    let mut settings = config(socket.local_addr().unwrap().port());
    settings.server.clear();
    let engine = client(settings);
    let cancel = CancellationToken::default();
    let worker_cancel = cancel.clone();
    let (done, result) = mpsc::channel();
    let worker = thread::spawn(move || {
        let (choose, commands) = mpsc::channel();
        let mut lists = Vec::new();
        let mut refusals = Vec::new();
        let outcome =
            engine.run_with_commands(&worker_cancel, RunOptions::default(), &commands, |event| {
                match event {
                    ClientEvent::World(WorldEvent::ServerSelection {
                        selection_id,
                        servers,
                    }) => {
                        lists.push(servers);
                        // The world that is down can't be chosen, so only the
                        // second choice reaches the login server.
                        for index in [0, 1] {
                            choose.send(ClientCommand::SelectServer {
                                selection_id,
                                index,
                            })?;
                        }
                    }
                    ClientEvent::World(WorldEvent::ServerRefused {
                        selection_id,
                        refusal,
                    }) => {
                        refusals.push(refusal);
                        choose.send(ClientCommand::SelectServer {
                            selection_id,
                            index: 2,
                        })?;
                    }
                    ClientEvent::Progress(ConnectionStage::ConnectingWorld) => {
                        worker_cancel.cancel();
                    }
                    _ => (),
                }
                Ok(())
            });
        done.send((outcome, lists, refusals)).unwrap();
    });
    serve_a_refusal_then_a_world(&socket);
    let (outcome, lists, refusals) = result.recv_timeout(Duration::from_secs(3)).unwrap();
    cancel.cancel();
    worker.join().unwrap();
    outcome.unwrap();
    let world = |name: &str, status, players, preferred| ServerChoice {
        name: name.into(),
        status,
        players: Some(players),
        preferred,
    };
    assert_eq!(
        lists,
        [vec![
            world("Example Down", ServerStatus::Down, 0, false),
            world("Example Busy", ServerStatus::Up, 40, true),
            world("Example Up", ServerStatus::Up, 7, false),
        ]]
    );
    assert_eq!(refusals, [ServerRefusal::Message(326)]);
}

#[test]
fn a_front_end_can_tell_what_the_zone_session_lets_the_player_do() {
    use eq_network::world::{titanium_player, Capability, WorldEvent};
    let mut spawn = vec![0; 385];
    spawn[340..344].copy_from_slice(&7_u32.to_le_bytes());
    let player = titanium_player(&vec![0; 19592], &spawn, 256.0).unwrap();
    let entered = WorldEvent::Entered {
        capabilities: vec![Capability::Moving, Capability::Talking],
        choices: vec![Capability::Map],
        session_id: 1,
        zone: "qeynos".into(),
        player: Box::new(player),
        far_clip: None,
    };
    // The front end matches the event exhaustively and keeps the report.
    let (offered, choices) = match entered {
        WorldEvent::Entered {
            capabilities,
            choices,
            ..
        } => (capabilities, choices),
        _ => (Vec::new(), Vec::new()),
    };
    // What the session leaves to the player is not offered until the front
    // end's player turns it on.
    assert_eq!(choices, [Capability::Map]);
    assert!(!offered.contains(&Capability::Map));
    let allowed = |command: &ClientCommand| {
        command
            .capability()
            .is_none_or(|needed| offered.contains(&needed))
    };
    let say = ClientCommand::SendChat(OutboundChat::Say("Hail".into()));
    let cast = ClientCommand::CastSpell {
        session_id: 1,
        gem: 0,
        spell_id: 202,
        target_id: 7,
        created: Instant::now(),
    };
    let choose = ClientCommand::SelectCharacter {
        selection_id: 1,
        slot: 0,
    };
    assert!(allowed(&say));
    assert!(!allowed(&cast), "casting is greyed out");
    assert!(allowed(&choose), "the world server's commands need no zone");
    assert!(offered
        .iter()
        .all(|capability| Capability::ALL.contains(capability)));
}
