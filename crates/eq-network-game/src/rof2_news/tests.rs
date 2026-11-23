//! Bodies written by our own writers, from the layouts the module doc
//! cites; skipped fields hold filler so a reader that strays shows.
use super::*;
use crate::combat::ConColor;
use crate::message::{rof2, Message, Part};

/// Little-endian words, joined.
fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// The six records stock `EQEmu` sends both generations unencoded, with
/// `RoF2`'s number and Titanium's for each.
fn same_bytes() -> [(u16, u16, Vec<u8>); 6] {
    [
        (
            DEATH_OPCODE,
            0x6160,
            words(&[73, 40, 74, 9, u32::MAX, 3, 120, 0]),
        ),
        (LEVEL_OPCODE, 0x6d44, words(&[12, 11, 200])),
        (EXPERIENCE_OPCODE, 0x5ecd, words(&[150, 0])),
        (
            STAMINA_OPCODE,
            crate::food::STAMINA_OPCODE,
            words(&[90, 80]),
        ),
        (MONEY_OPCODE, 0x267c, words(&[1, 2, 3, 4])),
        (
            TIME_OPCODE,
            crate::clock::TIME_OPCODE,
            [vec![7, 30, 2, 5], 3100u32.to_le_bytes().to_vec()].concat(),
        ),
    ]
}

/// A mana change: mana, endurance, spell, keep casting, padding, slot.
fn mana_body(keep_casting: u8, padding: u8, slot: i32) -> Vec<u8> {
    let mut body = words(&[300, 200, 1234]);
    body.push(keep_casting);
    body.extend_from_slice(&[padding; 3]);
    body.extend_from_slice(&slot.to_le_bytes());
    body
}

/// A damage record: target, source, kind, spell, damage, push, byte 25 and
/// the special word.
fn damage_body(kind: u8, spell_id: u32, damage: i32, push: [f32; 3], filler: u8) -> Vec<u8> {
    let mut body = 73u16.to_le_bytes().to_vec();
    body.extend_from_slice(&40u16.to_le_bytes());
    body.push(kind);
    body.extend_from_slice(&spell_id.to_le_bytes());
    body.extend_from_slice(&damage.to_le_bytes());
    for value in push {
        body.extend_from_slice(&value.to_bits().to_le_bytes());
    }
    body.push(filler);
    body.extend_from_slice(&u32::from(filler).to_le_bytes());
    body
}

/// The same damage as Titanium's 23-byte record.
fn titanium_damage_body(kind: u8, spell_id: u16, damage: i32, push: [f32; 3]) -> Vec<u8> {
    let mut body = 73u16.to_le_bytes().to_vec();
    body.extend_from_slice(&40u16.to_le_bytes());
    body.push(kind);
    body.extend_from_slice(&spell_id.to_le_bytes());
    body.extend_from_slice(&damage.to_le_bytes());
    for value in push {
        body.extend_from_slice(&value.to_bits().to_le_bytes());
    }
    body
}

/// A consider answer: player, target, faction, color, `PvP` byte and three
/// bytes of `filler`.
fn consider_body(target: u32, filler: u8) -> Vec<u8> {
    let mut body = words(&[40, target, 5, 4]);
    body.push(filler);
    body.extend_from_slice(&[filler; 3]);
    body
}

#[test]
fn the_same_bytes_read_as_titaniums_reader_reads_them() {
    for (opcode, titanium, body) in same_bytes() {
        let expected = crate::world::titanium_update(titanium, &body)
            .unwrap()
            .unwrap();
        assert_eq!(
            read(opcode, &body).unwrap(),
            Some(vec![expected]),
            "{opcode:#06x}"
        );
        // And refuse what it refuses.
        for end in 0..body.len() {
            assert!(
                read(opcode, &body[..end]).is_err(),
                "{opcode:#06x} cut at {end}"
            );
        }
        let mut long = body.clone();
        long.push(0);
        assert!(read(opcode, &long).is_err(), "{opcode:#06x} trailing byte");
    }
    // The level's own refusals, as Titanium's.
    assert!(read(LEVEL_OPCODE, &words(&[0, 11, 200])).is_err());
    assert!(read(LEVEL_OPCODE, &words(&[256, 11, 200])).is_err());
}

#[test]
fn a_mana_change_gives_titaniums_two_events_and_leaves_the_slot() {
    let events = mana_change(&mana_body(1, 0, -1)).unwrap();
    let titanium = crate::message::titanium(0x4839, &mana_body(1, 0, -1)[..16]);
    let titanium: Vec<WorldEvent> = titanium
        .into_iter()
        .map(|message| match message {
            Message::Event(event) => event,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(events, titanium);
    assert_eq!(
        events,
        [
            WorldEvent::Spell(crate::spells::SpellUpdate::Mana {
                spell_id: 1234,
                keep_casting: true,
            }),
            WorldEvent::Resources {
                mana: 300,
                endurance: 200,
            },
        ]
    );
    // The padding and the slot change nothing.
    assert_eq!(mana_change(&mana_body(1, 0xa5, 7)).unwrap(), events);
    assert!(mana_change(&mana_body(2, 0, -1)).is_err());
    // Titanium's 16 bytes are not RoF2's.
    assert!(mana_change(&mana_body(1, 0, -1)[..16]).is_err());
    let body = mana_body(0, 0, -1);
    for end in 0..body.len() {
        assert!(read(MANA_OPCODE, &body[..end]).is_err(), "cut at {end}");
    }
    assert!(read(MANA_OPCODE, &[body, vec![0]].concat()).is_err());
}

#[test]
fn damage_reads_as_titaniums_with_the_spell_widened_and_the_push_at_13() {
    let push = [0.5, 128.0, 0.25];
    for (kind, spell_id, damage) in [
        (crate::combat::SPELL_DAMAGE_KIND, 1234, 55),
        (crate::combat::SPELL_DAMAGE_KIND, 0xffff, 55),
        (0, 0, -1),
        (30, 0, 12),
    ] {
        assert_eq!(
            super::damage(&damage_body(kind, u32::from(spell_id), damage, push, 0)).unwrap(),
            crate::combat::damage(&titanium_damage_body(kind, spell_id, damage, push)).unwrap(),
            "{kind} {spell_id} {damage}"
        );
    }
    let read = super::damage(&damage_body(231, 1234, 55, push, 0)).unwrap();
    assert_eq!((read.target_id, read.source_id, read.kind), (73, 40, 231));
    assert_eq!(read.spell_id, Some(1234));
    let read_push = read.push.unwrap();
    assert_eq!(
        (read_push.force, read_push.heading, read_push.pitch),
        (0.5, 128.0, 0.25)
    );
    // Byte 25 and the special word change nothing.
    assert_eq!(
        super::damage(&damage_body(231, 1234, 55, push, 2)).unwrap(),
        read
    );
    // A spell past 16 bits cannot come from the server.
    assert!(super::damage(&damage_body(231, 0x1_0000, 55, push, 0)).is_err());
    let body = damage_body(0, 0, 3, push, 0);
    for end in 0..body.len() {
        assert!(super::damage(&body[..end]).is_err(), "cut at {end}");
    }
    assert!(super::damage(&[body, vec![0]].concat()).is_err());
}

#[test]
fn a_consider_answer_carries_no_hit_points() {
    assert_eq!(
        consideration(&consider_body(73, 0)).unwrap(),
        Consideration {
            target_id: 73,
            faction: 5,
            color: ConColor::from(4),
            hit_points: None,
        }
    );
    // The PvP byte and the rest change nothing.
    assert_eq!(
        consideration(&consider_body(73, 1)).unwrap(),
        consideration(&consider_body(73, 0)).unwrap()
    );
    for target in [0, 70_000] {
        assert!(consideration(&consider_body(target, 0)).is_err());
    }
    let body = consider_body(73, 0);
    for end in 0..body.len() {
        assert!(consideration(&body[..end]).is_err(), "cut at {end}");
    }
    assert!(consideration(&[body, vec![0]].concat()).is_err());
}

#[test]
fn a_skill_update_leaves_the_encoders_constants() {
    let body = [words(&[22, 150]), vec![1, 80, 136, 54]].concat();
    assert_eq!(
        skill(&body).unwrap(),
        WorldEvent::Skill {
            skill_id: 22,
            value: 150,
        }
    );
    let other = [words(&[22, 150]), vec![0xa5; 4]].concat();
    assert_eq!(skill(&other).unwrap(), skill(&body).unwrap());
    for end in 0..body.len() {
        assert!(skill(&body[..end]).is_err(), "cut at {end}");
    }
    assert!(skill(&[body, vec![0]].concat()).is_err());
}

#[test]
fn rof2s_zone_decoder_gives_each_news_message_its_events() {
    for (opcode, _, body) in same_bytes() {
        assert!(
            matches!(rof2(opcode, &body).as_slice(), [Message::Event(_)]),
            "{opcode:#06x}"
        );
    }
    assert!(matches!(
        rof2(MANA_OPCODE, &mana_body(1, 0, -1)).as_slice(),
        [
            Message::Event(WorldEvent::Spell(_)),
            Message::Event(WorldEvent::Resources { .. })
        ]
    ));
    for (opcode, body) in [
        (DAMAGE_OPCODE, damage_body(0, 0, 3, [0.0; 3], 0)),
        (CONSIDER_OPCODE, consider_body(73, 0)),
        (
            SKILL_OPCODE,
            [words(&[22, 150]), vec![1, 80, 136, 54]].concat(),
        ),
    ] {
        assert!(
            matches!(rof2(opcode, &body).as_slice(), [Message::Event(_)]),
            "{opcode:#06x}"
        );
    }
    assert!(matches!(
        rof2(DAMAGE_OPCODE, &[0; 23]).as_slice(),
        [Message::Unreadable {
            part: Part::World,
            ..
        }]
    ));
    assert_eq!(read(0x7c32, &[0; 8]).unwrap(), None);
}
