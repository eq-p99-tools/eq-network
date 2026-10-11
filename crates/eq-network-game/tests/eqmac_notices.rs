//! Synthetic `EQMac` replies, based on the reference servers' writers.
use eq_network_game::{
    appearance::TextureSlot,
    message::{eqmac, Message},
    world::WorldEvent,
};

fn event(opcode: u16, body: &[u8]) -> WorldEvent {
    let mut messages = eqmac(opcode, body);
    assert_eq!(messages.len(), 1);
    match messages.pop().unwrap() {
        Message::Event(event) => event,
        other => panic!("expected event, got {other:?}"),
    }
}

#[test]
fn wear_change_ignores_padding_and_reads_tint_in_bgr_order() {
    let body = [42, 0, 5, 255, 188, 2, 255, 255, 48, 32, 16, 255];
    let WorldEvent::WearChange(wear) = event(0x9240, &body) else {
        panic!()
    };
    assert_eq!(
        (wear.spawn_id, wear.slot, wear.material),
        (42, TextureSlot::Legs, 700)
    );
    assert_eq!(wear.tint, Some([16, 32, 48]));
    let mut invalid = body;
    invalid[2] = 9;
    assert!(matches!(
        eqmac(0x9240, &invalid).as_slice(),
        [Message::Unreadable { .. }]
    ));
}

#[test]
fn level_and_skill_updates_reach_existing_typed_events() {
    assert_eq!(
        event(0x9841, &[9, 0, 0, 0, 10, 0, 0, 0, 165, 0, 0, 0]),
        WorldEvent::Level {
            current: 9,
            previous: 10,
            experience: 165
        }
    );
    assert_eq!(
        event(0x8941, &[75, 0, 0, 0, 230, 0, 0, 0]),
        WorldEvent::Skill {
            skill_id: 75,
            value: 230
        }
    );
    assert!(matches!(
        eqmac(0x9841, &[0; 12]).as_slice(),
        [Message::Unreadable { .. }]
    ));
}

#[test]
fn book_kind_is_first_byte_and_missing_terminator_is_rejected() {
    let WorldEvent::BookText(book) = event(0xce40, b"\x01A synthetic story.\0") else {
        panic!()
    };
    assert_eq!((book.kind, book.text.as_str()), (1, "A synthetic story."));
    let WorldEvent::BookText(book) = event(0xce40, b"\x09\0") else {
        panic!()
    };
    assert_eq!((book.kind, book.text.as_str()), (9, ""));
    assert!(matches!(
        eqmac(0xce40, b"\x01unterminated").as_slice(),
        [Message::Unreadable { .. }]
    ));
}

#[test]
fn assist_no_target_sentinel_is_not_entity_65535() {
    for (wire, expected) in [(0u16, None), (u16::MAX, None), (300, Some(300))] {
        let WorldEvent::Assisted(answer) = event(0x0042, &wire.to_le_bytes()) else {
            panic!()
        };
        assert_eq!(answer.target, expected);
    }
}

#[test]
fn rolls_and_consent_use_the_shared_body_not_the_titanium_opcode() {
    let mut body = vec![0; 76];
    body[0] = 1;
    body[4] = 100;
    body[8] = 42;
    body[12..18].copy_from_slice(b"Tester");
    let WorldEvent::Roll(roll) = event(0x1640, &body) else {
        panic!()
    };
    assert_eq!(
        (roll.low, roll.high, roll.result, roll.name.as_str()),
        (1, 100, 42, "Tester")
    );
    let mut body = vec![0; 161];
    body[..6].copy_from_slice(b"Helper");
    body[64..69].copy_from_slice(b"Owner");
    body[128] = 1;
    body[129..138].copy_from_slice(b"Test Zone");
    let WorldEvent::Consent(consent) = event(0xd540, &body) else {
        panic!()
    };
    assert_eq!(
        (
            consent.granted.as_str(),
            consent.owner.as_str(),
            consent.zone.as_str()
        ),
        ("Helper", "Owner", "Test Zone")
    );
    assert!(consent.given);
}

#[test]
fn fixed_eqmac_notice_lengths_are_exact() {
    for (opcode, length) in [
        (0x9841, 12),
        (0x8941, 8),
        (0x9240, 12),
        (0xd540, 161),
        (0x1640, 76),
        (0x0042, 2),
    ] {
        for len in (0..length).chain([length + 1]) {
            assert!(
                matches!(
                    eqmac(opcode, &vec![0; len]).as_slice(),
                    [Message::Unreadable { .. }]
                ),
                "{opcode:#06x} length {len}"
            );
        }
    }
}
