//! Bodies written by our own writers, from the server's structs.
use super::*;
use crate::message::{rof2, Message, Part};

fn pool_body(current: u32, maximum: u32, spawn: u16) -> Vec<u8> {
    [
        &current.to_le_bytes()[..],
        &maximum.to_le_bytes(),
        &spawn.to_le_bytes(),
    ]
    .concat()
}

fn percent_body(spawn: u16, percent: u8) -> Vec<u8> {
    [&spawn.to_le_bytes()[..], &[percent]].concat()
}

#[test]
fn a_pool_reads_each_field() {
    for (opcode, kind) in [
        (MANA_OPCODE, Pool::Mana),
        (ENDURANCE_OPCODE, Pool::Endurance),
    ] {
        assert_eq!(
            read(opcode, &pool_body(300, 1200, 4321)).unwrap(),
            Some(WorldEvent::ResourcePool {
                spawn_id: 4321,
                pool: kind,
                current: 300,
                maximum: 1200,
            })
        );
    }
}

#[test]
fn a_pools_values_are_kept_as_sent() {
    for (current, maximum) in [(0, 1200), (1200, 1200), (1300, 1200), (u32::MAX, u32::MAX)] {
        assert!(matches!(
            pool(Pool::Mana, &pool_body(current, maximum, 77)).unwrap(),
            WorldEvent::ResourcePool { current: read_current, maximum: read_maximum, .. }
                if read_current == current && read_maximum == maximum
        ));
    }
    assert!(pool(Pool::Mana, &pool_body(300, 1200, 0)).is_err());
    assert!(pool(Pool::Endurance, &pool_body(300, 1200, 0)).is_err());
}

#[test]
fn a_percent_reads_each_field_and_refuses_past_100() {
    for (opcode, kind) in [
        (MANA_PERCENT_OPCODE, Pool::Mana),
        (ENDURANCE_PERCENT_OPCODE, Pool::Endurance),
    ] {
        for value in [0, 57, 100] {
            assert_eq!(
                read(opcode, &percent_body(77, value)).unwrap(),
                Some(WorldEvent::ResourcePercent {
                    spawn_id: 77,
                    pool: kind,
                    percent: value,
                })
            );
        }
        for value in [101, 255] {
            assert!(read(opcode, &percent_body(77, value)).is_err());
        }
        assert!(read(opcode, &percent_body(0, 50)).is_err());
    }
}

#[test]
fn every_message_refuses_a_cut_and_a_trailing_byte() {
    for (opcode, body) in [
        (MANA_OPCODE, pool_body(300, 1200, 4321)),
        (ENDURANCE_OPCODE, pool_body(300, 1200, 4321)),
        (MANA_PERCENT_OPCODE, percent_body(77, 50)),
        (ENDURANCE_PERCENT_OPCODE, percent_body(77, 50)),
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
fn no_other_generation_reads_these_numbers() {
    for opcode in [
        MANA_OPCODE,
        ENDURANCE_OPCODE,
        MANA_PERCENT_OPCODE,
        ENDURANCE_PERCENT_OPCODE,
    ] {
        for body in [pool_body(300, 1200, 4321), percent_body(77, 50)] {
            assert!(
                crate::message::titanium(opcode, &body).is_empty(),
                "{opcode:#06x}"
            );
            assert!(
                crate::message::eqmac(opcode, &body).is_empty(),
                "{opcode:#06x}"
            );
        }
    }
    assert_eq!(read(0x5467, &pool_body(300, 1200, 4321)).unwrap(), None);
}

#[test]
fn the_zone_reads_each_message_as_the_worlds() {
    for (opcode, body) in [
        (MANA_OPCODE, pool_body(300, 1200, 4321)),
        (ENDURANCE_OPCODE, pool_body(300, 1200, 4321)),
        (MANA_PERCENT_OPCODE, percent_body(77, 50)),
        (ENDURANCE_PERCENT_OPCODE, percent_body(77, 50)),
    ] {
        assert!(
            matches!(rof2(opcode, &body)[..], [Message::Event(_)]),
            "{opcode:#06x}"
        );
        assert!(
            matches!(
                rof2(opcode, &[1])[..],
                [Message::Unreadable {
                    part: Part::World,
                    ..
                }]
            ),
            "{opcode:#06x}"
        );
    }
}
