//! Bodies written by our own writers, from the layouts the module doc
//! cites; skipped fields hold filler so a reader that strays shows.
use super::*;
use crate::appearance::SpawnStates;

/// An appearance record: spawn, kind, value.
fn appearance_body(spawn_id: u16, kind: u16, value: u32) -> Vec<u8> {
    let mut body = spawn_id.to_le_bytes().to_vec();
    body.extend_from_slice(&kind.to_le_bytes());
    body.extend_from_slice(&value.to_le_bytes());
    body
}

/// A size change: entity, size, then the encoder's two constants.
fn size_body(entity: u32, size: f32) -> Vec<u8> {
    let mut body = entity.to_le_bytes().to_vec();
    body.extend_from_slice(&size.to_bits().to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&1.0f32.to_bits().to_le_bytes());
    body
}

/// The player's hit points: spawn, current, maximum.
fn hit_points_body(spawn_id: u16, current: i32, maximum: i32) -> Vec<u8> {
    let mut body = spawn_id.to_le_bytes().to_vec();
    body.extend_from_slice(&current.to_le_bytes());
    body.extend_from_slice(&maximum.to_le_bytes());
    body
}

/// A wear change: spawn, material, four unread words of `filler`, the
/// tint (blue, green, red, use) and the slot.
fn wear_body(spawn_id: u16, material: u32, filler: u32, tint: [u8; 4], slot: u8) -> Vec<u8> {
    let mut body = spawn_id.to_le_bytes().to_vec();
    body.extend_from_slice(&material.to_le_bytes());
    for _ in 0..4 {
        body.extend_from_slice(&filler.to_le_bytes());
    }
    body.extend_from_slice(&tint);
    body.push(slot);
    body
}

/// Every cut of `body` and `body` with a trailing byte refuse.
fn refuses_cuts_and_trailing(opcode: u16, body: &[u8]) {
    for end in 0..body.len() {
        assert!(
            read(opcode, &body[..end]).is_err(),
            "{opcode:#06x} cut at {end}"
        );
    }
    let mut long = body.to_vec();
    long.push(0);
    assert!(read(opcode, &long).is_err(), "{opcode:#06x} trailing byte");
}

#[test]
fn every_fixed_message_refuses_a_cut_and_a_trailing_byte() {
    refuses_cuts_and_trailing(APPEARANCE_OPCODE, &appearance_body(73, 14, 110));
    refuses_cuts_and_trailing(SIZE_OPCODE, &size_body(73, 6.0));
    refuses_cuts_and_trailing(HIT_POINTS_OPCODE, &hit_points_body(73, 10, 20));
    refuses_cuts_and_trailing(HEALTH_OPCODE, &[73, 0, 50]);
    refuses_cuts_and_trailing(WEAR_CHANGE_OPCODE, &wear_body(73, 2, 0, [0; 4], 1));
    refuses_cuts_and_trailing(ANIMATION_OPCODE, &[73, 0, 5, 10]);
    for opcode in [STATES_ADDED_OPCODE, STATES_REMOVED_OPCODE] {
        refuses_cuts_and_trailing(opcode, &appearance_body(73, 0, 32)[..]);
    }
}

#[test]
fn other_opcodes_are_not_this_modules() {
    for opcode in [0x7c32, 0x3bcf, 0x0695, 0x7441, 0x5089] {
        assert_eq!(read(opcode, &[0; 8]).unwrap(), None);
    }
}

#[test]
fn appearances_read_as_titaniums_reader_reads_the_same_record() {
    let pet = crate::pets::PET_APPEARANCE;
    for (kind, value) in [
        // A look: light, sneaking, linkdead, the damage state and an
        // unknown kind.
        (5, 3),
        (15, 1),
        (18, 0),
        (44, 2),
        (99, 7),
        // The /who listing: level, anonymity, guild, rank and away.
        (1, 30),
        (21, 1),
        (22, 9),
        (23, 2),
        (24, 1),
        // Invisibility, posture and the pet owner.
        (3, 0),
        (3, 1),
        (3, 2),
        (14, 110),
        (pet, 40),
        (pet, 0),
    ] {
        let body = appearance_body(73, kind, value);
        let titanium = crate::appearance::titanium(crate::listing::APPEARANCE_OPCODE, &body)
            .unwrap()
            .unwrap();
        assert_eq!(read(APPEARANCE_OPCODE, &body).unwrap(), Some(titanium));
    }
}

#[test]
fn invisibility_reads_as_titaniums_values() {
    // Inferred (6.518): the server sends both generations the same values.
    assert_eq!(
        appearance(&appearance_body(73, 3, 2)).unwrap(),
        Some(WorldEvent::Visibility {
            spawn_id: 73,
            state: crate::vision::Invisibility::Titanium(2),
        })
    );
}

#[test]
fn kind_20_comes_out_through_the_listing() {
    // The GM-flag arm is Titanium's alone, and the listing claims kind 20
    // first on every generation, so RoF2's false changes nothing.
    for value in [0, 1] {
        assert_eq!(
            appearance(&appearance_body(73, 20, value)).unwrap(),
            Some(WorldEvent::Listing {
                spawn_id: 73,
                change: crate::listing::ListingChange::GameMaster(value != 0),
            })
        );
    }
}

#[test]
fn an_appearance_naming_no_spawn_keeps_the_shared_rule() {
    // A look naming no spawn says nothing; a listing, visibility, posture
    // or pet update naming none refuses, as Titanium's.
    assert_eq!(appearance(&appearance_body(0, 5, 3)).unwrap(), None);
    for kind in [1, 3, 14, crate::pets::PET_APPEARANCE] {
        assert!(appearance(&appearance_body(0, kind, 1)).is_err(), "{kind}");
    }
}

#[test]
fn sizes_keep_their_fraction_and_titaniums_range() {
    for size in [0.0, 6.0, 2.5, 65535.0] {
        assert_eq!(
            read(SIZE_OPCODE, &size_body(73, size)).unwrap(),
            Some(WorldEvent::Appearance {
                spawn_id: 73,
                change: AppearanceChange::Size(size),
            })
        );
    }
    for size in [-1.0, 65536.0, f32::NAN, f32::INFINITY] {
        assert!(super::size(&size_body(73, size)).is_err(), "{size}");
    }
    // An entity of zero or past 16 bits refuses: the server sends no size
    // for a mob without an id.
    for entity in [0, 70_000] {
        assert!(super::size(&size_body(entity, 6.0)).is_err());
    }
}

#[test]
fn the_size_changes_constants_change_nothing() {
    let mut body = size_body(73, 6.0);
    body[8..16].fill(0xa5);
    assert_eq!(
        super::size(&body).unwrap(),
        super::size(&size_body(73, 6.0)).unwrap()
    );
}

#[test]
fn hit_points_put_the_spawn_first_and_stay_signed() {
    assert_eq!(
        read(HIT_POINTS_OPCODE, &hit_points_body(73, -5, 120)).unwrap(),
        Some(WorldEvent::HitPoints {
            spawn_id: 73,
            current: -5,
            maximum: 120,
            items: ItemHitPoints::LeftOut,
        })
    );
    // Each field alone.
    assert!(matches!(
        hit_points(&hit_points_body(9, 0, 0)).unwrap(),
        WorldEvent::HitPoints {
            spawn_id: 9,
            current: 0,
            maximum: 0,
            ..
        }
    ));
    assert!(matches!(
        hit_points(&hit_points_body(0, 70_000, 0)).unwrap(),
        WorldEvent::HitPoints {
            spawn_id: 0,
            current: 70_000,
            maximum: 0,
            ..
        }
    ));
    assert!(hit_points(&hit_points_body(73, 10, -1)).is_err());
}

#[test]
fn health_takes_titaniums_percent_range() {
    for percent in [0, 37, 100] {
        assert_eq!(
            health(&[73, 1, percent]).unwrap(),
            WorldEvent::HealthPercent {
                spawn_id: 0x0149,
                percent,
            }
        );
    }
    assert!(health(&[73, 0, 101]).is_err());
}

#[test]
fn wear_changes_keep_all_32_material_bits() {
    let change = wear_change(&wear_body(73, 0x0012_3456, 0, [0x10, 0x20, 0xc0, 0xff], 1)).unwrap();
    assert_eq!(
        change,
        WearChange {
            spawn_id: 73,
            slot: TextureSlot::Chest,
            material: 0x0012_3456,
            tint: Some([0xc0, 0x20, 0x10]),
        }
    );
    // An unused tint is none, and the unread words change nothing.
    let plain = wear_change(&wear_body(73, 2, 0, [9, 9, 9, 0], 8)).unwrap();
    assert_eq!((plain.slot, plain.tint), (TextureSlot::Secondary, None));
    assert_eq!(
        wear_change(&wear_body(73, 2, 0xa5a5_a5a5, [9, 9, 9, 0], 8)).unwrap(),
        plain
    );
    assert!(wear_change(&wear_body(0, 2, 0, [0; 4], 1)).is_err());
    assert!(wear_change(&wear_body(73, 2, 0, [0; 4], 9)).is_err());
}

#[test]
fn animations_put_the_action_before_the_speed() {
    let animation = super::animation(&[73, 0, 5, 20]).unwrap();
    assert_eq!(
        (animation.spawn_id, animation.action, animation.speed),
        (73, 5, 2.0)
    );
    // Zero speed gives what Titanium's zero gives.
    let titanium = crate::combat::animation(&[73, 0, 0, 5]).unwrap();
    assert_eq!(super::animation(&[73, 0, 5, 0]).unwrap(), titanium);
}

#[test]
fn player_states_come_and_go_by_their_own_opcodes() {
    let body = appearance_body(73, 0, 32);
    for (opcode, change) in [
        (
            STATES_ADDED_OPCODE,
            AppearanceChange::StatesAdded(SpawnStates::STUNNED),
        ),
        (
            STATES_REMOVED_OPCODE,
            AppearanceChange::StatesRemoved(SpawnStates::STUNNED),
        ),
    ] {
        assert_eq!(
            read(opcode, &body).unwrap(),
            Some(WorldEvent::Appearance {
                spawn_id: 73,
                change
            })
        );
    }
    // The shared reader refuses an opcode outside the pair it is given.
    assert!(crate::appearance::states(
        crate::appearance::STATES_ADDED_OPCODE,
        [STATES_ADDED_OPCODE, STATES_REMOVED_OPCODE],
        &body
    )
    .is_err());
    for spawn_id in [0u32, 70_000] {
        let mut body = spawn_id.to_le_bytes().to_vec();
        body.extend_from_slice(&32u32.to_le_bytes());
        assert!(read(STATES_ADDED_OPCODE, &body).is_err());
    }
}

#[test]
fn rof2s_zone_decoder_reads_exactly_its_listed_numbers() {
    use crate::message::{rof2, Message, Part, ROF2_READS};
    // A one-byte body is too short for every RoF2 record, so a read number
    // says it is unreadable and any other says nothing.
    for opcode in 0..=u16::MAX {
        let messages = rof2(opcode, &[0xa5]);
        if ROF2_READS.contains(&opcode) {
            assert!(
                matches!(
                    messages.as_slice(),
                    [Message::Unreadable {
                        part: Part::World,
                        ..
                    }]
                ),
                "{opcode:#06x}"
            );
        } else {
            assert!(messages.is_empty(), "{opcode:#06x}");
        }
    }
}

#[test]
fn rof2s_zone_decoder_gives_one_event_for_each_look_and_health_message() {
    use crate::message::{rof2, Message};
    for (opcode, body) in [
        (APPEARANCE_OPCODE, appearance_body(73, 14, 110)),
        (SIZE_OPCODE, size_body(73, 6.0)),
        (HIT_POINTS_OPCODE, hit_points_body(73, 10, 20)),
        (HEALTH_OPCODE, vec![73, 0, 50]),
        (WEAR_CHANGE_OPCODE, wear_body(73, 2, 0, [0; 4], 1)),
        (ANIMATION_OPCODE, vec![73, 0, 5, 10]),
        (STATES_ADDED_OPCODE, appearance_body(73, 0, 32)),
        (STATES_REMOVED_OPCODE, appearance_body(73, 0, 32)),
    ] {
        assert!(
            matches!(rof2(opcode, &body).as_slice(), [Message::Event(_)]),
            "{opcode:#06x}"
        );
    }
    // An appearance naming no spawn says nothing.
    assert!(rof2(APPEARANCE_OPCODE, &appearance_body(0, 5, 3)).is_empty());
}
