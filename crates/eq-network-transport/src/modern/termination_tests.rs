use super::{
    tests::{packet, session},
    Application, Session, SessionTermination,
};
use std::{
    net::UdpSocket,
    sync::atomic::AtomicBool,
    thread,
    time::{Duration, Instant},
};

fn reason(error: &anyhow::Error) -> SessionTermination {
    *error.downcast_ref::<SessionTermination>().unwrap()
}

fn send(peer: &UdpSocket, session: &Session, bytes: &[u8]) {
    peer.send_to(bytes, session.socket.local_addr().unwrap())
        .unwrap();
}

#[test]
fn terminal_control_shapes_and_connection_codes_are_checked() {
    let plain = SessionTermination::PeerDisconnected { reason: None };
    assert_eq!(
        SessionTermination::disconnect(&7u32.to_be_bytes(), 7),
        Some(plain)
    );
    assert_eq!(
        SessionTermination::disconnect(&[0, 0, 0, 7, 0x12, 0x34], 7),
        Some(SessionTermination::PeerDisconnected {
            reason: Some(0x1234)
        })
    );
    for bytes in [
        &[][..],
        &[0, 0, 7],
        &[0, 0, 0, 7, 0],
        &[0; 7],
        &[0, 0, 0, 8],
    ] {
        assert_eq!(SessionTermination::disconnect(bytes, 7), None);
    }
    assert!(SessionTermination::out_of_session(&[0, 0x1d, 0, 0, 0, 0]));
    for bytes in [
        &[0, 0x1d][..],
        &[0, 0x1d, 0, 0, 0, 7],
        &[0, 0x1d, 0, 0, 0, 0, 0],
    ] {
        assert!(!SessionTermination::out_of_session(bytes));
    }
}

#[test]
fn stateless_out_of_session_is_read_before_negotiated_framing() {
    for crc_bytes in [0, 2, 4] {
        for compression in [false, true] {
            let (mut connection, peer) = session();
            connection.crc_bytes = crc_bytes;
            connection.compression = compression;
            send(&peer, &connection, &[0, 0x1d, 0, 0, 0, 0]);
            let error = connection.receive().unwrap_err();
            assert_eq!(reason(&error), SessionTermination::OutOfSession);
            assert_eq!(
                connection.termination,
                Some(SessionTermination::OutOfSession)
            );
            assert!(connection.closed);
            assert_eq!(connection.sent_count.get(), 0);
        }
    }
}

#[test]
fn matching_disconnect_retains_its_opaque_reason_and_closes_once() {
    let (mut connection, peer) = session();
    let control = connection.datagram(5, &[0, 0, 0, 7, 0x12, 0x34]);
    send(&peer, &connection, &control);
    let expected = SessionTermination::PeerDisconnected {
        reason: Some(0x1234),
    };
    assert_eq!(reason(&connection.receive().unwrap_err()), expected);
    // A different duplicate cannot replace the first terminal fact or trigger IO.
    assert!(!connection.process(&[0, 5, 0, 0, 0, 7], true, 0).unwrap());
    assert_eq!(reason(&connection.receive().unwrap_err()), expected);
    connection.close().unwrap();
    connection.close().unwrap();
    assert_eq!(connection.sent_count.get(), 0);
}

#[test]
fn malformed_and_wrong_session_disconnects_do_not_terminate_or_refresh_liveness() {
    let (mut connection, peer) = session();
    let received = connection.last_receive;
    for body in [
        &[][..],
        &[0, 0, 7],
        &[0, 0, 0, 7, 0],
        &[0; 7],
        &[0, 0, 0, 8],
    ] {
        let control = connection.datagram(5, body);
        send(&peer, &connection, &control);
        assert!(connection.receive().unwrap().is_none());
        assert_eq!(connection.termination, None);
        assert_eq!(connection.last_receive, received);
    }
    let application = packet(&connection, 9, 0, b"\x04\x10still here");
    send(&peer, &connection, &application);
    assert_eq!(connection.receive().unwrap().unwrap().body, b"still here");
}

#[test]
fn malformed_and_encoded_out_of_session_controls_are_not_authority() {
    let (mut connection, peer) = session();
    let received = connection.last_receive;
    let encoded = connection.datagram(0x1d, &[0; 4]);
    for bytes in [
        &[0, 0x1d][..],
        &[0, 0x1d, 0, 0, 0, 7],
        &[0, 0x1d, 0, 0, 0, 0, 0],
        &encoded,
    ] {
        send(&peer, &connection, bytes);
        assert!(connection.receive().unwrap().is_none());
        assert_eq!(connection.last_receive, received);
        assert_eq!(connection.termination, None);
    }
    assert!(!connection.process(&[0, 0x1d, 0, 0, 0, 0], true, 0).unwrap());
    assert_eq!(connection.termination, None);
}

#[test]
fn a_disconnect_with_damaged_crc_is_discarded_before_termination() {
    let (mut connection, peer) = session();
    let received = connection.last_receive;
    let mut control = connection.datagram(5, &7u32.to_be_bytes());
    *control.last_mut().unwrap() ^= 0x80;
    send(&peer, &connection, &control);
    assert!(connection.receive().unwrap().is_none());
    assert_eq!(connection.termination, None);
    assert_eq!(connection.last_receive, received);
}

#[test]
fn another_udp_endpoint_cannot_end_the_connected_session() {
    let (mut connection, _peer) = session();
    let stranger = UdpSocket::bind("127.0.0.1:0").unwrap();
    send(&stranger, &connection, &[0, 0x1d, 0, 0, 0, 0]);
    assert!(connection.receive().unwrap().is_none());
    assert_eq!(connection.termination, None);
}

#[test]
fn termination_keeps_completed_delivery_but_discards_unfinished_work_and_sends() {
    let (mut connection, _peer) = session();
    connection.send(0x1004, b"queued").unwrap();
    connection.reorder.insert(1, (9, vec![4, 16]));
    connection.fragment = Some((8, vec![4, 16]));
    connection.applications.push_back(Application {
        opcode: 0x1004,
        body: b"old".to_vec(),
    });
    connection.ack_pending = true;
    connection.stats_request = Some((1, Instant::now()));
    let sent = connection.sent_count.get();
    connection.terminate(SessionTermination::OutOfSession);
    connection.terminate(SessionTermination::PeerDisconnected { reason: None });
    connection.next_stats = Instant::now();
    connection.last_ack = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
    let received = connection.received_count;
    assert_eq!(connection.receive().unwrap().unwrap().body, b"old");
    for error in [
        connection.send(0x1004, b"late").unwrap_err(),
        connection.send_unreliable(0x14cb, &[0; 36]).unwrap_err(),
        connection.receive().unwrap_err(),
    ] {
        assert_eq!(reason(&error), SessionTermination::OutOfSession);
    }
    connection.close().unwrap();
    assert_eq!(connection.sent_count.get(), sent);
    assert_eq!(
        connection.received_count, received,
        "no post-terminal receive"
    );
    assert!(connection.pending.is_empty(), "no retransmission remains");
    assert!(connection.reorder.is_empty(), "no successor remains");
    assert!(
        connection.applications.is_empty(),
        "the completed application drained exactly once"
    );
    assert!(connection.fragment.is_none());
    assert!(!connection.ack_pending);
    assert!(connection.stats_request.is_none());
}

#[test]
fn a_terminal_control_in_a_combined_packet_preserves_only_its_complete_prefix() {
    let (mut connection, peer) = session();
    let parts: [&[u8]; 4] = [
        &[0, 9, 0, 0, 4, 16, b'a'],
        &[0, 9, 0, 1, 4, 16, b'b'],
        &[0, 5, 0, 0, 0, 7],
        // Structurally valid combined part; parsing it as an application would fail.
        &[0, 9, 0, 2, 4],
    ];
    let mut body = Vec::new();
    for part in parts {
        body.push(u8::try_from(part.len()).unwrap());
        body.extend(part);
    }
    let frame = connection.datagram(3, &body);
    send(&peer, &connection, &frame);
    assert_eq!(connection.receive().unwrap().unwrap().body, b"a");
    let expected = SessionTermination::PeerDisconnected { reason: None };
    assert_eq!(
        reason(&connection.send(0x1004, b"late").unwrap_err()),
        expected
    );
    // Even a different later terminal or application cannot replace the latch
    // or append to its completed prefix.
    assert!(!connection
        .process(&[0, 5, 0, 0, 0, 7, 0x12, 0x34], true, 0)
        .unwrap());
    let late = packet(&connection, 9, 2, b"\x04\x10late");
    send(&peer, &connection, &late);
    connection.close().unwrap(); // Peer termination owns the staged prefix.
    let received = connection.received_count;
    assert_eq!(connection.receive().unwrap().unwrap().body, b"b");
    assert_eq!(reason(&connection.receive().unwrap_err()), expected);
    assert_eq!(reason(&connection.receive().unwrap_err()), expected);
    assert_eq!(connection.next_receive, 2);
    assert!(connection.applications.is_empty());
    assert_eq!(connection.received_count, received);
    assert_eq!(connection.sent_count.get(), 0);
}

#[test]
fn termination_never_completes_a_fragment_or_a_gap() {
    let (mut connection, peer) = session();
    let mut first = 10u32.to_be_bytes().to_vec();
    first.extend_from_slice(b"\x04\x10pre");
    let mut reliable = vec![0, 13, 0, 0];
    reliable.extend(first);
    let mut body = Vec::new();
    for part in [&[0, 13, 0, 2, b'x'][..], &reliable, &[0, 5, 0, 0, 0, 7]] {
        body.push(u8::try_from(part.len()).unwrap());
        body.extend_from_slice(part);
    }
    send(&peer, &connection, &connection.datagram(3, &body));
    assert_eq!(
        reason(&connection.receive().unwrap_err()),
        SessionTermination::PeerDisconnected { reason: None }
    );
    assert!(connection.applications.is_empty());
    assert!(connection.reorder.is_empty());
    assert!(connection.fragment.is_none());
    assert_eq!(connection.next_receive, 1);
}

#[test]
fn a_complete_fragment_before_termination_is_delivered() {
    let (mut connection, peer) = session();
    let parts: [&[u8]; 3] = [
        &[0, 13, 0, 0, 0, 0, 0, 5, 4, 16, b'a'],
        &[0, 13, 0, 1, b'b', b'c'],
        &[0, 5, 0, 0, 0, 7],
    ];
    let mut body = Vec::new();
    for part in parts {
        body.push(u8::try_from(part.len()).unwrap());
        body.extend_from_slice(part);
    }
    send(&peer, &connection, &connection.datagram(3, &body));
    assert_eq!(connection.receive().unwrap().unwrap().body, b"abc");
    assert_eq!(
        reason(&connection.receive().unwrap_err()),
        SessionTermination::PeerDisconnected { reason: None }
    );
}

#[test]
fn local_close_discards_staged_data_and_ignores_later_peer_termination() {
    let (mut connection, peer) = session();
    connection.applications.push_back(Application {
        opcode: 0x1004,
        body: b"staged".to_vec(),
    });
    connection.close().unwrap();
    assert!(connection.applications.is_empty());
    let terminal = connection.datagram(5, &7u32.to_be_bytes());
    send(&peer, &connection, &terminal);
    let received = connection.received_count;
    let sent = connection.sent_count.get();
    assert_eq!(
        connection.receive().unwrap_err().to_string(),
        "session is closed"
    );
    assert!(connection.termination.is_none());
    assert_eq!(connection.received_count, received);
    connection.close().unwrap();
    assert_eq!(connection.sent_count.get(), sent);
}

#[test]
fn a_fresh_negotiation_ignores_unsolicited_controls_and_resets_terminal_state() {
    let (mut old, _peer) = session();
    old.terminate(SessionTermination::OutOfSession);
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let address = server.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let mut request = [0; 512];
        let (length, client) = server.recv_from(&mut request).unwrap();
        assert_eq!(length, 14);
        server.send_to(&[0, 0x1d, 0, 0, 0, 0], client).unwrap();
        let mut response = vec![0, 2];
        response.extend_from_slice(&request[6..10]);
        response.extend_from_slice(&1234u32.to_be_bytes());
        response.extend_from_slice(&[2, 0, 0]);
        response.extend_from_slice(&512u32.to_be_bytes());
        server.send_to(&response, client).unwrap();
        server
    });
    let mut fresh = Session::connect_cancellable(address, false, &AtomicBool::new(false)).unwrap();
    let server = worker.join().unwrap();
    assert_eq!(fresh.termination, None);
    assert!(!fresh.closed);
    fresh.send(0x1004, b"new attempt").unwrap();
    let mut bytes = [0; 512];
    assert!(server.recv(&mut bytes).unwrap() > 4);
    assert_eq!(
        reason(&old.receive().unwrap_err()),
        SessionTermination::OutOfSession
    );
}
