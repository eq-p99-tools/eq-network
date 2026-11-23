//! Bodies written by our own writers, from the layouts the module doc
//! cites; where the encoder writes a constant or a zero, the writers put
//! filler, so a reader that strays shows.
use super::*;
use crate::message::{rof2, Message, Part};

/// What `ENCODE(OP_Buff)` makes of the server's byte modifier.
fn encoded(modifier: u8) -> f32 {
    if modifier == 10 {
        1.0
    } else {
        f32::from(modifier) / 10.0
    }
}

/// One buff update's values, in the server's own terms.
#[derive(Clone, Copy)]
struct Values {
    entity: u32,
    effect_type: u8,
    level: u8,
    modifier: u8,
    spell_id: u32,
    duration: i32,
    caster: u32,
    slot: u32,
    fade: u32,
}

const DURATION: Values = Values {
    entity: 4321,
    effect_type: 2,
    level: 51,
    modifier: 18,
    spell_id: 1234,
    duration: 120,
    caster: 77,
    slot: 3,
    fade: 0,
};

/// The 100-byte `RoF2` update for the server's values, with `filler` in
/// the fields not read, and the float modifier as given.
fn update_body_with(values: Values, modifier: f32, filler: u8) -> Vec<u8> {
    let mut body = vec![filler; 100];
    body[..4].copy_from_slice(&values.entity.to_le_bytes());
    body[4] = values.effect_type;
    body[5] = values.level;
    body[8..12].copy_from_slice(&modifier.to_bits().to_le_bytes());
    body[12..16].copy_from_slice(&values.spell_id.to_le_bytes());
    body[16..20].copy_from_slice(&values.duration.to_le_bytes());
    body[20..24].copy_from_slice(&values.caster.to_le_bytes());
    body[92..96].copy_from_slice(&values.slot.to_le_bytes());
    let fade: u32 = if values.fade == 1 { 1 } else { 2 };
    body[96..].copy_from_slice(&fade.to_le_bytes());
    body
}

fn update_body(values: Values, filler: u8) -> Vec<u8> {
    update_body_with(values, encoded(values.modifier), filler)
}

/// The same update in Titanium's 32 bytes, counters 0.
fn titanium_body(values: Values) -> Vec<u8> {
    let mut body = values.entity.to_le_bytes().to_vec();
    body.extend_from_slice(&[values.effect_type, values.level, values.modifier, 0]);
    body.extend_from_slice(&values.spell_id.to_le_bytes());
    body.extend_from_slice(&values.duration.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&values.caster.to_le_bytes());
    body.extend_from_slice(&values.slot.to_le_bytes());
    body.extend_from_slice(&values.fade.to_le_bytes());
    body
}

/// One icon entry: slot, spell, ticks, hits, name.
fn entry(slot: u32, spell_id: u32, ticks: i32, hits: u32, name: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    for value in [slot, spell_id, ticks.cast_unsigned(), hits] {
        body.extend_from_slice(&value.to_le_bytes());
    }
    body.extend_from_slice(name);
    body.push(0);
    body
}

/// An icon message: spawn, timer, all-buffs byte, count, entries, type.
fn icons_body(spawn: u32, all: u8, count: u16, entries: &[Vec<u8>], window: u8) -> Vec<u8> {
    let mut body = spawn.to_le_bytes().to_vec();
    body.extend_from_slice(&5400i32.to_le_bytes());
    body.push(all);
    body.extend_from_slice(&count.to_le_bytes());
    for entry in entries {
        body.extend_from_slice(entry);
    }
    body.push(window);
    body
}

fn count_of(entries: &[Vec<u8>]) -> u16 {
    u16::try_from(entries.len()).unwrap()
}

/// A pet window as the encoder writes it, with `filler` in its zero word.
fn pet_body(pet: u32, spells: &[(u32, u32, i32)], filler: u8) -> Vec<u8> {
    let entries: Vec<Vec<u8>> = spells
        .iter()
        .map(|(slot, spell, ticks)| entry(*slot, *spell, *ticks, 0, b""))
        .collect();
    let mut body = icons_body(pet, 1, count_of(&entries), &entries, 0);
    body[4..8].fill(filler);
    body
}

/// The same pet window in Titanium's 248 bytes.
fn titanium_pet_body(pet: u32, spells: &[(u32, u32, i32)]) -> Vec<u8> {
    let mut body = vec![0; 248];
    body[..4].copy_from_slice(&pet.to_le_bytes());
    for (slot, spell, ticks) in spells {
        let slot = usize::try_from(*slot).unwrap();
        body[4 + slot * 4..8 + slot * 4].copy_from_slice(&spell.to_le_bytes());
        body[124 + slot * 4..128 + slot * 4].copy_from_slice(&ticks.to_le_bytes());
    }
    body[244..].copy_from_slice(&u32::try_from(spells.len()).unwrap().to_le_bytes());
    body
}

#[test]
fn a_duration_reads_each_field_as_titaniums() {
    let read = update(&update_body(DURATION, 0x5a)).unwrap();
    assert_eq!(
        read,
        crate::buffs::titanium_update(&titanium_body(DURATION)).unwrap()
    );
    assert_eq!(
        read,
        BuffUpdate {
            spell_id: 1234,
            entity_id: 4321,
            slot: 3,
            applies: Applies::BySpell,
            record_type: 2,
            buff: Some(Buff {
                spell_id: 1234,
                caster_level: 51,
                effect_type: 2,
                bard_modifier: 18,
                duration_ticks: 120,
                counters: 0,
                caster_id: 77,
            }),
        }
    );
    // The hit count, the location, the unread word and the slot data
    // change nothing.
    assert_eq!(update(&update_body(DURATION, 0)).unwrap(), read);
    assert_eq!(update(&update_body(DURATION, 0xff)).unwrap(), read);
}

#[test]
fn a_permanent_duration_reads_negative_as_titaniums() {
    let permanent = Values {
        duration: -1,
        ..DURATION
    };
    let read = update(&update_body(permanent, 0x5a)).unwrap();
    assert_eq!(
        read,
        crate::buffs::titanium_update(&titanium_body(permanent)).unwrap()
    );
    assert_eq!(read.buff.unwrap().duration_ticks, -1);
}

#[test]
fn a_fade_reads_as_titaniums() {
    // Client::MakeBuffFadePacket: effect type 2, the spell, the slot, 1.
    let fade = Values {
        level: 0,
        modifier: 0,
        duration: 0,
        caster: 0,
        fade: 1,
        ..DURATION
    };
    let read = update(&update_body(fade, 0x5a)).unwrap();
    assert_eq!(
        read,
        crate::buffs::titanium_update(&titanium_body(fade)).unwrap()
    );
    assert_eq!(
        (read.applies, read.buff, read.slot),
        (Applies::FadeBySpell, None, 3)
    );
}

#[test]
fn an_empty_record_reads_none_as_titaniums() {
    for values in [
        Values {
            effect_type: 0,
            ..DURATION
        },
        Values {
            spell_id: 0,
            ..DURATION
        },
        Values {
            spell_id: 0xffff,
            ..DURATION
        },
        Values {
            spell_id: u32::MAX,
            ..DURATION
        },
    ] {
        let read = update(&update_body(values, 0x5a)).unwrap();
        assert_eq!(read.buff, None);
        assert_eq!(
            read,
            crate::buffs::titanium_update(&titanium_body(values)).unwrap()
        );
    }
}

#[test]
fn only_the_two_fade_words_read() {
    for fade in [0u32, 3, 4, u32::MAX] {
        let mut body = update_body(DURATION, 0x5a);
        body[96..].copy_from_slice(&fade.to_le_bytes());
        assert!(update(&body).is_err(), "{fade}");
    }
}

#[test]
fn the_bard_modifier_reads_back_the_servers_byte() {
    for modifier in 0..=u8::MAX {
        let values = Values {
            modifier,
            ..DURATION
        };
        let read = update(&update_body(values, 0x5a)).unwrap();
        assert_eq!(read.buff.unwrap().bard_modifier, modifier, "{modifier}");
    }
    // The encoder's 1.0 for 10, and its neighbours by division.
    for (value, modifier) in [(1.0, 10), (0.9, 9), (1.1, 11)] {
        let read = update(&update_body_with(DURATION, value, 0x5a)).unwrap();
        assert_eq!(read.buff.unwrap().bard_modifier, modifier);
    }
    for value in [f32::NAN, f32::INFINITY, -0.1, -1.0, 1.05, 25.6, 1000.0] {
        assert!(
            update(&update_body_with(DURATION, value, 0x5a)).is_err(),
            "{value}"
        );
    }
}

#[test]
fn icons_read_each_field() {
    let entries = [
        entry(0, 1234, 120, 0, b"Testname"),
        entry(41, 2345, -1, 3, b""),
        entry(5, u32::MAX, 0, 0, b""),
    ];
    let read = icons(&icons_body(4321, 1, 3, &entries, 0)).unwrap();
    assert_eq!(
        read,
        BuffIcons {
            spawn_id: 4321,
            window: BuffWindow::Own,
            all: true,
            tick_timer_ms: 5400,
            icons: vec![
                BuffIcon {
                    slot: 0,
                    spell_id: Some(1234),
                    ticks: 120,
                    hits: 0,
                    caster_name: Some("Testname".into()),
                },
                BuffIcon {
                    slot: 41,
                    spell_id: Some(2345),
                    ticks: -1,
                    hits: 3,
                    caster_name: None,
                },
                BuffIcon {
                    slot: 5,
                    spell_id: None,
                    ticks: 0,
                    hits: 0,
                    caster_name: None,
                },
            ],
        }
    );
    for (window, read) in [
        (5, BuffWindow::TargetPlayer),
        (7, BuffWindow::TargetNpc),
        (1, BuffWindow::Other(1)),
    ] {
        let body = icons_body(77, 1, 3, &entries, window);
        assert_eq!(icons(&body).unwrap().window, read);
        assert_eq!(
            super::read(TARGET_ICONS_OPCODE, &body).unwrap(),
            super::read(ICONS_OPCODE, &body).unwrap()
        );
    }
    let empty = icons(&icons_body(4321, 1, 0, &[], 0)).unwrap();
    assert!(empty.all && empty.icons.is_empty());
}

#[test]
fn icons_take_every_slot_and_the_longest_name() {
    let name = [b'x'; 63];
    let entries: Vec<Vec<u8>> = (0..97)
        .map(|slot| entry(slot, 1000 + slot, 6, 0, &name))
        .collect();
    let read = icons(&icons_body(77, 1, 97, &entries, 7)).unwrap();
    assert_eq!(read.icons.len(), 97);
    assert_eq!(read.icons[96].slot, 96);
    assert_eq!(
        read.icons[96].caster_name.as_deref().map(str::len),
        Some(63)
    );
}

#[test]
fn the_fades_own_icon_message_empties_its_slot() {
    // What ENCODE(OP_Buff) sends after a fade: 29 bytes.
    let body = icons_body(4321, 0, 1, &[entry(3, u32::MAX, 0, 0, b"")], 0);
    let body = [&body[..4], &[0; 4], &body[8..]].concat();
    assert_eq!(body.len(), 29);
    let read = icons(&body).unwrap();
    assert!(!read.all);
    assert_eq!(read.tick_timer_ms, 0);
    assert_eq!(
        read.icons,
        [BuffIcon {
            slot: 3,
            spell_id: None,
            ticks: 0,
            hits: 0,
            caster_name: None,
        }]
    );
}

#[test]
fn icons_refuse_what_no_encoder_writes() {
    let entries = [
        entry(0, 1234, 120, 0, b"Testname"),
        entry(1, 2345, 6, 0, b""),
    ];
    let good = icons_body(4321, 1, 2, &entries, 0);
    assert!(icons(&good).is_ok());
    // A count off by one either way.
    assert!(icons(&icons_body(4321, 1, 1, &entries, 0)).is_err());
    assert!(icons(&icons_body(4321, 1, 3, &entries, 0)).is_err());
    // No type byte, and one too many.
    assert!(icons(&good[..good.len() - 1]).is_err());
    assert!(icons(&[good.clone(), vec![0]].concat()).is_err());
    // A spawn of 0 or past 16 bits.
    assert!(icons(&icons_body(0, 1, 2, &entries, 0)).is_err());
    assert!(icons(&icons_body(0x1_0000, 1, 2, &entries, 0)).is_err());
    // A slot named twice, and a slot past 96.
    let twice = [entry(4, 1234, 1, 0, b""), entry(4, 2345, 1, 0, b"")];
    assert!(icons(&icons_body(4321, 1, 2, &twice, 0)).is_err());
    assert!(icons(&icons_body(4321, 1, 1, &[entry(97, 1, 1, 0, b"")], 0)).is_err());
    // A count past 97.
    let many: Vec<Vec<u8>> = (0..98).map(|slot| entry(slot, 1, 1, 0, b"")).collect();
    assert!(icons(&icons_body(4321, 1, 98, &many, 0)).is_err());
    // A name of 64 bytes, a name not ASCII, and one without its end.
    assert!(icons(&icons_body(
        4321,
        1,
        1,
        &[entry(0, 1, 1, 0, &[b'x'; 64])],
        0
    ))
    .is_err());
    assert!(icons(&icons_body(4321, 1, 1, &[entry(0, 1, 1, 0, &[0xe9])], 0)).is_err());
    let open = [
        &4321u32.to_le_bytes()[..],
        &[0; 4],
        &[1],
        &1u16.to_le_bytes(),
        &[0; 16],
        b"Test",
    ]
    .concat();
    assert!(icons(&open).is_err());
    // A cut anywhere.
    for length in 0..good.len() {
        assert!(icons(&good[..length]).is_err(), "{length}");
    }
}

#[test]
fn icon_errors_say_nothing_of_the_body() {
    let bodies = [
        icons_body(4321, 1, 1, &[entry(0, 1, 1, 0, &[b'Q'; 64])], 0),
        icons_body(4321, 1, 1, &[entry(0, 1, 1, 0, b"Testcaster\xe9")], 0),
        icons_body(
            4321,
            1,
            2,
            &[
                entry(4, 1, 1, 0, b"Testcaster"),
                entry(4, 1, 1, 0, b"Testcaster"),
            ],
            0,
        ),
    ];
    for body in bodies {
        let error = format!("{:#}", icons(&body).unwrap_err());
        assert!(
            !error.contains("Testcaster") && !error.contains("QQ"),
            "{error}"
        );
    }
}

#[test]
fn a_pet_window_reads_as_titaniums() {
    let spells = [(0, 1234, 120), (7, 2345, -1), (29, 3456, 6)];
    let read = pet_buffs(&pet_body(312, &spells, 0x5a)).unwrap();
    assert_eq!(
        read,
        crate::pets::decode_buffs(&titanium_pet_body(312, &spells)).unwrap()
    );
    assert_eq!(read.pet, 312);
    assert_eq!(read.slots.len(), 30);
    assert_eq!(
        read.slots[7],
        Some(PetBuff {
            spell_id: 2345,
            ticks: -1
        })
    );
    assert_eq!(pet_buffs(&pet_body(312, &spells, 0)).unwrap(), read);
    // An empty window is 30 empty slots.
    let empty = pet_buffs(&pet_body(312, &[], 0x5a)).unwrap();
    assert_eq!(
        empty,
        crate::pets::decode_buffs(&titanium_pet_body(312, &[])).unwrap()
    );
    assert!(empty.slots.iter().all(Option::is_none));
    // A spell of -1 leaves its slot empty, as Titanium's.
    let unset = [(4, u32::MAX, 6)];
    assert_eq!(
        pet_buffs(&pet_body(312, &unset, 0x5a)).unwrap(),
        crate::pets::decode_buffs(&titanium_pet_body(312, &unset)).unwrap()
    );
}

#[test]
fn a_pet_window_whose_count_and_entries_differ_refuses() {
    let spells = [(0, 1234, 120), (7, 2345, -1)];
    let entries: Vec<Vec<u8>> = spells
        .iter()
        .map(|(slot, spell, ticks)| entry(*slot, *spell, *ticks, 0, b""))
        .collect();
    // The server counted three, the encoder wrote two: the body the
    // encoder sizes for three ends in zero padding.
    let mut padded = icons_body(312, 1, 3, &entries, 0);
    padded.resize(12 + 17 * 3, 0);
    assert!(pet_buffs(&padded).is_err());
    // The server counted one, the encoder wrote two: the count reads one
    // entry and the body runs on.
    assert!(pet_buffs(&icons_body(312, 1, 1, &entries, 0)).is_err());
}

#[test]
fn a_pet_window_refuses_what_its_encoder_never_writes() {
    let good = pet_body(312, &[(0, 1234, 120)], 0x5a);
    assert!(pet_buffs(&good).is_ok());
    for body in [
        pet_body(312, &[(30, 1234, 120)], 0x5a),
        pet_body(312, &[(3, 1234, 120), (3, 2345, 6)], 0x5a),
        pet_body(312, &[(3, 0, 120)], 0x5a),
        pet_body(0x1_0000, &[(0, 1234, 120)], 0x5a),
        icons_body(312, 1, 1, &[entry(0, 1234, 120, 2, b"")], 0),
        icons_body(312, 1, 1, &[entry(0, 1234, 120, 0, b"Testname")], 0),
        icons_body(312, 0, 1, &[entry(0, 1234, 120, 0, b"")], 0),
    ] {
        assert!(pet_buffs(&body).is_err());
    }
    let many: Vec<(u32, u32, i32)> = (0..31).map(|slot| (slot, 1, 1)).collect();
    assert!(pet_buffs(&pet_body(312, &many, 0x5a)).is_err());
    for length in 0..good.len() {
        assert!(pet_buffs(&good[..length]).is_err(), "{length}");
    }
    assert!(pet_buffs(&[good, vec![0]].concat()).is_err());
}

#[test]
fn an_update_refuses_a_cut_and_a_trailing_byte() {
    let body = update_body(DURATION, 0x5a);
    for length in 0..body.len() {
        assert!(update(&body[..length]).is_err(), "{length}");
    }
    assert!(update(&[body, vec![0]].concat()).is_err());
}

#[test]
fn another_opcode_is_not_this_modules() {
    for opcode in [0x6a53, 0x4e31, 0x744c, 0x5089] {
        assert_eq!(read(opcode, &[0; 100]).unwrap(), None, "{opcode:#06x}");
    }
}

#[test]
fn the_zone_reads_each_message_as_the_worlds() {
    let icons = icons_body(4321, 1, 1, &[entry(0, 1234, 120, 0, b"")], 0);
    for (opcode, body) in [
        (UPDATE_OPCODE, update_body(DURATION, 0x5a)),
        (ICONS_OPCODE, icons.clone()),
        (TARGET_ICONS_OPCODE, icons),
        (PET_OPCODE, pet_body(312, &[(0, 1234, 120)], 0x5a)),
    ] {
        assert!(
            matches!(rof2(opcode, &body)[..], [Message::Event(_)]),
            "{opcode:#06x}"
        );
        assert!(
            matches!(
                rof2(opcode, &[1, 2, 3])[..],
                [Message::Unreadable {
                    part: Part::World,
                    ..
                }]
            ),
            "{opcode:#06x}"
        );
    }
}
