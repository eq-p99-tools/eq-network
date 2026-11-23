//! Bodies written by our own writers, from the server's structs.
use super::*;
use crate::message::{rof2, Message, Part};
use crate::spells::decode;

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// A deletion as the server copies it back: the slot, the client's two
/// padding bytes, success, the client's three.
fn deletion(slot: u16, success: u8, filler: u8) -> Vec<u8> {
    let mut body = slot.to_le_bytes().to_vec();
    body.extend_from_slice(&[filler; 2]);
    body.push(success);
    body.extend_from_slice(&[filler; 3]);
    body
}

#[test]
fn each_message_reads_as_titaniums_on_the_same_body() {
    for (rof2_opcode, titanium_opcode, body) in [
        (MEMORIZE_OPCODE, 0x308e, words(&[2, 1234, 0, 0])),
        (MEMORIZE_OPCODE, 0x308e, words(&[2, 1234, 1, 0])),
        (MEMORIZE_OPCODE, 0x308e, words(&[2, 1234, 2, 0])),
        (MEMORIZE_OPCODE, 0x308e, words(&[7, 1234, 3, 1500])),
        (SWAP_OPCODE, 0x2126, words(&[12, 300])),
        (DELETE_OPCODE, 0x4f37, deletion(12, 1, 0)),
        (DELETE_OPCODE, 0x4f37, deletion(12, 0, 0)),
    ] {
        let read = read(rof2_opcode, &body).unwrap().unwrap();
        assert_eq!(Some(read), decode(titanium_opcode, &body).unwrap());
    }
}

#[test]
fn each_field_reads_from_its_place() {
    assert_eq!(
        read(MEMORIZE_OPCODE, &words(&[5, 1234, 1, 99])).unwrap(),
        Some(SpellUpdate::Slot {
            slot: 5,
            spell_id: 1234,
            mode: 1,
        })
    );
    assert_eq!(
        read(SWAP_OPCODE, &words(&[12, 300])).unwrap(),
        Some(SpellUpdate::BookSwap { from: 12, to: 300 })
    );
    assert_eq!(
        read(DELETE_OPCODE, &deletion(12, 1, 0x5a)).unwrap(),
        Some(SpellUpdate::BookDeletion {
            slot: 12,
            success: true,
        })
    );
    // The client's echoed padding is not read.
    assert_eq!(
        read(DELETE_OPCODE, &deletion(12, 1, 0x5a)).unwrap(),
        read(DELETE_OPCODE, &deletion(12, 1, 0)).unwrap()
    );
    assert!(read(DELETE_OPCODE, &deletion(12, 2, 0)).is_err());
}

#[test]
fn the_book_is_720_slots() {
    for (from, to) in [(0, 719), (719, 0), (400, 719)] {
        assert!(read(SWAP_OPCODE, &words(&[from, to])).unwrap().is_some());
    }
    for (from, to) in [(720, 0), (0, 720), (u32::MAX, 0)] {
        assert!(read(SWAP_OPCODE, &words(&[from, to])).is_err());
    }
    assert!(read(DELETE_OPCODE, &deletion(719, 1, 0)).unwrap().is_some());
    assert!(read(DELETE_OPCODE, &deletion(720, 1, 0)).is_err());
    assert!(read(DELETE_OPCODE, &deletion(u16::MAX, 1, 0)).is_err());
    // Between Titanium's book and RoF2's, only RoF2 reads.
    assert!(read(SWAP_OPCODE, &words(&[399, 400])).is_ok());
    assert!(decode(0x2126, &words(&[399, 400])).is_err());
    assert!(read(DELETE_OPCODE, &deletion(400, 1, 0)).is_ok());
    assert!(decode(0x4f37, &deletion(400, 1, 0)).is_err());
    assert!(decode(0x2126, &words(&[399, 0])).is_ok());
}

#[test]
fn a_bar_refresh_keeps_the_servers_casting_slot_and_reduction() {
    assert_eq!(
        read(MEMORIZE_OPCODE, &words(&[11, 1234, 3, 1500])).unwrap(),
        Some(SpellUpdate::BarRefresh {
            slot: 11,
            spell_id: 1234,
            reduction_ms: 1500,
        })
    );
    // The bar's unlock: slot 0, SPELLBAR_UNLOCK (zone/client.h), as
    // Titanium's.
    let unlock = words(&[0, 0x2bc, 3, 0]);
    assert_eq!(
        read(MEMORIZE_OPCODE, &unlock).unwrap(),
        decode(0x308e, &unlock).unwrap()
    );
}

#[test]
fn every_message_refuses_a_cut_and_a_trailing_byte() {
    for (opcode, body) in [
        (MEMORIZE_OPCODE, words(&[2, 1234, 1, 0])),
        (SWAP_OPCODE, words(&[12, 300])),
        (DELETE_OPCODE, deletion(12, 1, 0)),
    ] {
        for length in 0..body.len() {
            assert!(
                read(opcode, &body[..length]).is_err(),
                "{opcode:#06x} {length}"
            );
        }
        assert!(
            read(opcode, &[body, vec![0]].concat()).is_err(),
            "{opcode:#06x}"
        );
    }
}

#[test]
fn no_generations_number_reads_under_the_other() {
    for opcode in [0x308e, 0x2126, 0x4f37] {
        assert_eq!(read(opcode, &[0; 16]).unwrap(), None, "{opcode:#06x}");
        assert!(rof2(opcode, &[0; 16]).is_empty(), "{opcode:#06x}");
    }
    for opcode in [MEMORIZE_OPCODE, SWAP_OPCODE, DELETE_OPCODE] {
        assert_eq!(decode(opcode, &[0; 16]).unwrap(), None, "{opcode:#06x}");
    }
}

#[test]
fn the_zone_reads_each_message_as_the_spells() {
    for (opcode, body) in [
        (MEMORIZE_OPCODE, words(&[2, 1234, 1, 0])),
        (SWAP_OPCODE, words(&[12, 300])),
        (DELETE_OPCODE, deletion(12, 1, 0)),
    ] {
        assert!(
            matches!(
                rof2(opcode, &body)[..],
                [Message::Event(crate::world::WorldEvent::Spell(_))]
            ),
            "{opcode:#06x}"
        );
        assert!(
            matches!(
                rof2(opcode, &[1, 2, 3])[..],
                [Message::Unreadable {
                    part: Part::Spells,
                    ..
                }]
            ),
            "{opcode:#06x}"
        );
    }
}

#[test]
fn a_negative_deletion_slot_and_a_later_mode_read_as_titaniums() {
    for slot in [-1i16, -720, i16::MIN] {
        let body = deletion(slot.cast_unsigned(), 1, 0);
        assert!(read(DELETE_OPCODE, &body).is_err(), "{slot}");
        assert!(decode(0x4f37, &body).is_err(), "{slot}");
    }
    for mode in [4, 5, u32::MAX] {
        let body = words(&[2, 1234, mode, 0]);
        let read = read(MEMORIZE_OPCODE, &body).unwrap();
        assert_eq!(read, decode(0x308e, &body).unwrap());
        assert_eq!(
            read,
            Some(SpellUpdate::Slot {
                slot: 2,
                spell_id: 1234,
                mode,
            })
        );
    }
}

#[test]
fn each_generations_book_has_its_size() {
    use crate::spells::book_size;
    use crate::GameDialect;
    assert_eq!(book_size(GameDialect::Titanium), 400);
    assert_eq!(book_size(GameDialect::Rof2), 720);
    assert_eq!(
        usize::try_from(book_size(GameDialect::EqMac)).unwrap(),
        crate::spells::EQMAC_BOOK_SLOTS
    );
}
