//! `RoF2`'s world stage against a synthetic stock `EQEmu` world on loopback,
//! with a synthetic account and key.
use super::*;
use crate::client::{
    session::login::{tests::peer, Credentials},
    CancellationToken, ClientConfig, ClientIdentity, ServerProtocol,
};
use std::{net::UdpSocket, sync::mpsc, thread};

/// What a world stage ended with, and the diagnostics it gave.
type Outcome = (Result<Option<(String, String, u16)>, String>, Vec<String>);

/// Runs the world stage against `socket` for `character`, in a thread.
fn run(socket: &UdpSocket, character: &str, flags: (bool, bool)) -> mpsc::Receiver<Outcome> {
    let address = socket.local_addr().unwrap();
    let character = character.to_owned();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let config = ClientConfig::for_protocol(
            ServerProtocol::EqEmuRof2,
            "ACCOUNT",
            "PASSWORD",
            "Example Up",
            &character,
        );
        let identity = ClientIdentity::new("example-device", "example-user");
        let credentials = Credentials {
            account: "12345".into(),
            key: *b"EXAMPLEKEY",
        };
        let stop = CancellationToken::default();
        let context = CharacterSession {
            config: config.clone(),
            identity: &identity,
            credentials: &credentials,
            stop: &stop,
            duration: None,
            commands: None,
            sysinfo_reported_at: std::cell::Cell::new(0),
            answer_list_polls: std::cell::Cell::new(false),
            server_filters: std::cell::RefCell::default(),
            riding: std::sync::Arc::default(),
            chat: std::sync::Arc::default(),
            chat_pace: std::sync::Arc::default(),
        };
        let mut diagnostics = Vec::new();
        let mut handler = |event| {
            if let ClientEvent::Diagnostic(message) = event {
                diagnostics.push(message);
            }
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        let outcome = world_at(&context, address, flags, &mut log).map(|destination| {
            destination
                .map(|destination| (destination.character, destination.host, destination.port))
        });
        drop(log);
        done.send((outcome.map_err(|error| error.to_string()), diagnostics))
            .unwrap();
    });
    result
}

fn world_socket() -> UdpSocket {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    socket
}

/// A one-character `RoF2` list, every fixed byte zero.
fn one_character(name: &str) -> Vec<u8> {
    let mut body = 1u32.to_le_bytes().to_vec();
    body.extend(name.as_bytes());
    body.push(0);
    body.extend([0; 274]);
    body
}

fn handoff(host: &str, port: u16) -> Vec<u8> {
    let mut body = vec![0; ZONE_SERVER_LENGTH];
    body[..host.len()].copy_from_slice(host.as_bytes());
    body[128..130].copy_from_slice(&port.to_le_bytes());
    body
}

/// The client's login info, checked, once the session is up.
fn logged_in(socket: &UdpSocket, zoning: bool) -> std::net::SocketAddr {
    let (address, _) = peer::negotiate(socket);
    let info = peer::next_starting(socket, &[0, 9, 0, 0, 0x09, 0x7a]);
    // Six header bytes, the body, two CRC bytes.
    assert_eq!(info.len(), 6 + 488 + 2);
    assert_eq!(&info[6..23], b"12345\0EXAMPLEKEY\0");
    assert_eq!(info[6 + 188], u8::from(zoning));
    address
}

#[test]
fn rof2_enters_the_listed_character_and_follows_its_handoff() {
    let socket = world_socket();
    let result = run(&socket, "Example", (false, false));
    let address = logged_in(&socket, false);
    let send = |sequence, opcode, body: &[u8]| {
        socket
            .send_to(&peer::application(sequence, opcode, body), address)
            .unwrap();
    };
    send(0, POST_ENTER_WORLD, &[]);
    // Titanium's list is not RoF2's: it is left unread.
    send(1, 0x4513, &[0; 8]);
    send(2, characters::ROF2_OPCODE, &one_character("Example"));
    let enter = peer::next_starting(&socket, &[0, 9, 0, 1, 0x8f, 0x57]);
    assert_eq!(enter.len(), 6 + 72 + 2);
    assert_eq!(&enter[6..14], b"Example\0");
    assert!(enter[14..78].iter().all(|byte| *byte == 0));
    send(3, ZONE_SERVER, &handoff("127.0.0.1", 7000));
    let (outcome, diagnostics) = result.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        outcome.unwrap(),
        Some(("Example".to_owned(), "127.0.0.1".to_owned(), 7000))
    );
    assert!(
        diagnostics
            .iter()
            .any(|line| line == "RoF2 world sent 0x4513 (8 bytes), left unread"),
        "{diagnostics:?}"
    );
    // The key never reaches a diagnostic.
    assert!(!diagnostics.iter().any(|line| line.contains("EXAMPLEKEY")));
}

#[test]
fn rof2_ends_a_world_only_session_at_the_list() {
    let socket = world_socket();
    let result = run(&socket, "Example", (true, false));
    let address = logged_in(&socket, false);
    socket
        .send_to(
            &peer::application(0, characters::ROF2_OPCODE, &one_character("Example")),
            address,
        )
        .unwrap();
    let (outcome, _) = result.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(outcome.unwrap(), None);
}

#[test]
fn rof2_between_zones_enters_its_character_after_the_worlds_greeting() {
    let socket = world_socket();
    let result = run(&socket, "Example", (false, true));
    let address = logged_in(&socket, true);
    let send = |sequence, opcode, body: &[u8]| {
        socket
            .send_to(&peer::application(sequence, opcode, body), address)
            .unwrap();
    };
    // The world names the character in play.
    send(0, ENTER_WORLD, b"Example\0");
    send(1, POST_ENTER_WORLD, &[]);
    let enter = peer::next_starting(&socket, &[0, 9, 0, 1, 0x8f, 0x57]);
    assert_eq!(&enter[6..14], b"Example\0");
    send(2, ZONE_SERVER, &handoff("127.0.0.1", 7001));
    let (outcome, _) = result.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        outcome.unwrap(),
        Some(("Example".to_owned(), "127.0.0.1".to_owned(), 7001))
    );
}

#[test]
fn rof2_ends_the_attempt_when_the_zone_is_unavailable() {
    let socket = world_socket();
    let result = run(&socket, "Example", (false, false));
    let address = logged_in(&socket, false);
    socket
        .send_to(
            &peer::application(0, characters::ROF2_OPCODE, &one_character("Example")),
            address,
        )
        .unwrap();
    peer::next_starting(&socket, &[0, 9, 0, 1, 0x8f, 0x57]);
    let mut down = vec![0; ZONE_UNAVAILABLE_LENGTH];
    down[..6].copy_from_slice(b"qeynos");
    socket
        .send_to(&peer::application(1, ZONE_UNAVAILABLE, &down), address)
        .unwrap();
    let (outcome, _) = result.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(outcome.unwrap_err(), "zone qeynos is unavailable");
}

#[test]
fn rof2s_handoff_refuses_every_length_and_address_it_cannot_trust() {
    assert_eq!(
        zone_server(&handoff("127.0.0.1", 7000)).unwrap(),
        ("127.0.0.1".to_owned(), 7000)
    );
    let whole = handoff("127.0.0.1", 7000);
    let mut long = whole.clone();
    long.push(0);
    for body in [&whole[..129], &long[..], &[][..]] {
        assert!(zone_server(body).is_err(), "{}", body.len());
    }
    let mut unterminated = whole.clone();
    unterminated[..128].fill(b'a');
    assert!(zone_server(&unterminated).is_err());
    assert!(zone_server(&handoff("", 7000)).is_err());
    assert!(zone_server(&handoff("bad host", 7000)).is_err());
    assert!(zone_server(&handoff("127.0.0.1", 0)).is_err());
}

#[test]
fn rof2s_entry_refuses_a_name_it_would_have_to_cut() {
    let body = enter_body(&"A".repeat(63)).unwrap();
    assert_eq!(body[62], b'A');
    assert_eq!(body[63..], [0; 9]);
    for name in ["", "Exa\0mple", &"A".repeat(64)] {
        assert!(enter_body(name).is_err(), "{name:?}");
    }
}
