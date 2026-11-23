//! Bodies written by our own writers, from the layouts the module doc
//! cites; where the encoder writes a constant or a zero, the writers put
//! filler, so a reader that strays shows.
use super::*;
use crate::buffs::Action;
use crate::message::{rof2, Message, Part};
use crate::spells::SpellUpdate;

/// What `ENCODE(OP_Action)` makes of the server's integer modifier, in the
/// encoder's own unsigned, single-precision arithmetic.
#[allow(clippy::cast_precision_loss)]
fn encoded(modifier: u32) -> f32 {
    1.0 + (modifier.wrapping_sub(10) as f32) / 10.0
}

/// A begin-cast record: the spell in 32 bits, the caster, the cast time.
fn begin_cast_body(spell_id: u32, caster_id: u16, duration_ms: u32) -> Vec<u8> {
    let mut body = spell_id.to_le_bytes().to_vec();
    body.extend_from_slice(&caster_id.to_le_bytes());
    body.extend_from_slice(&duration_ms.to_le_bytes());
    body
}

/// One action's values, in the server's own terms.
#[derive(Clone, Copy)]
struct Values {
    target: u16,
    source: u16,
    level: u16,
    modifier: u32,
    push: [f32; 3],
    kind: u8,
    spell_id: u16,
    spell_level: u8,
    effect_flag: u8,
}

const SPELL: Values = Values {
    target: 73,
    source: 40,
    level: 51,
    modifier: 18,
    push: [2.5, 128.0, 0.5],
    kind: 231,
    spell_id: 1234,
    spell_level: 49,
    effect_flag: 4,
};

/// `ActionAlt_Struct` as the encoder fills it, with `filler` in its
/// constant and zero fields and the float `modifier` as given.
fn action_body_with(values: Values, modifier: f32, spell_id: u32, filler: u8) -> Vec<u8> {
    let mut body = Vec::new();
    for short in [values.target, values.source, values.level] {
        body.extend_from_slice(&short.to_le_bytes());
    }
    body.extend_from_slice(&[filler; 4]);
    body.extend_from_slice(&modifier.to_bits().to_le_bytes());
    for value in values.push {
        body.extend_from_slice(&value.to_bits().to_le_bytes());
    }
    body.push(values.kind);
    body.extend_from_slice(&[filler; 6]);
    body.extend_from_slice(&spell_id.to_le_bytes());
    body.push(values.spell_level);
    body.push(values.effect_flag);
    body.extend_from_slice(&[filler; 17]);
    assert_eq!(body.len(), 56);
    body
}

fn action_body(values: Values, filler: u8) -> Vec<u8> {
    action_body_with(
        values,
        encoded(values.modifier),
        values.spell_id.into(),
        filler,
    )
}

/// The same action in Titanium's 31-byte record.
fn titanium_action_body(values: Values) -> Vec<u8> {
    let mut body = Vec::new();
    for short in [values.target, values.source, values.level] {
        body.extend_from_slice(&short.to_le_bytes());
    }
    body.extend_from_slice(&values.modifier.to_le_bytes());
    for value in values.push {
        body.extend_from_slice(&value.to_bits().to_le_bytes());
    }
    body.push(values.kind);
    body.extend_from_slice(&[0; 4]);
    body.extend_from_slice(&values.spell_id.to_le_bytes());
    body.push(values.spell_level);
    body.push(values.effect_flag);
    body
}

/// One illusion's values.
#[derive(Clone, Copy)]
struct Look {
    spawn: u32,
    race: u16,
    gender: u8,
    texture: u8,
    helm: u8,
    face: u32,
    hair: [u8; 4],
    size: f32,
}

const LOOK: Look = Look {
    spawn: 312,
    race: 75,
    gender: 1,
    texture: 3,
    helm: 2,
    face: 4,
    hair: [5, 6, 7, 8],
    size: 6.5,
};

/// `RoF2`'s 336-byte illusion with `filler` where the encoder copies or
/// writes what is not read.
fn illusion_body(look: Look, filler: u8) -> Vec<u8> {
    let mut body = vec![filler; 336];
    body[..4].copy_from_slice(&look.spawn.to_le_bytes());
    body[4..13].copy_from_slice(b"Testname\0");
    body[68..70].copy_from_slice(&look.race.to_le_bytes());
    body[72] = look.gender;
    body[73] = look.texture;
    body[76] = look.helm;
    body[80..84].copy_from_slice(&look.face.to_le_bytes());
    body[84..88].copy_from_slice(&look.hair);
    body[88..92].copy_from_slice(&look.size.to_bits().to_le_bytes());
    body
}

/// The same illusion in Titanium's 168 bytes.
fn titanium_illusion_body(look: Look, race: i32) -> Vec<u8> {
    let mut body = vec![0; 168];
    body[..4].copy_from_slice(&look.spawn.to_le_bytes());
    body[4..13].copy_from_slice(b"Testname\0");
    body[68..72].copy_from_slice(&race.to_le_bytes());
    body[72] = look.gender;
    body[73] = look.texture;
    body[74] = look.helm;
    body[76..80].copy_from_slice(&look.face.to_le_bytes());
    body[80..84].copy_from_slice(&look.hair);
    body[84..88].copy_from_slice(&look.size.to_bits().to_le_bytes());
    body
}

/// A face change: hair color, beard color, two eye colors, hair style,
/// beard, face, padding, the Drakkin words as `filler`, then the spawn.
fn face_body(spawn: u32, filler: u8) -> Vec<u8> {
    let mut body = vec![10, 11, 12, 13, 14, 15, 16, filler];
    body.extend_from_slice(&[filler; 12]);
    body.extend_from_slice(&spawn.to_le_bytes());
    body
}

fn stun_body(duration_ms: u32, filler: u8) -> Vec<u8> {
    [duration_ms.to_le_bytes(), [filler; 4]].concat()
}

/// Every fixed-length message, with its opcode and a body that reads.
fn fixed() -> [(u16, Vec<u8>); 5] {
    [
        (BEGIN_CAST_OPCODE, begin_cast_body(1234, 40, 3000)),
        (ACTION_OPCODE, action_body(SPELL, 0x5a)),
        (STUN_OPCODE, stun_body(2000, 0x5a)),
        (ILLUSION_OPCODE, illusion_body(LOOK, 0x5a)),
        (FACE_OPCODE, face_body(312, 0x5a)),
    ]
}

#[test]
fn a_begin_cast_reads_each_field_as_titaniums() {
    let read = begin_cast(&begin_cast_body(1234, 40, 3000)).unwrap();
    assert_eq!(
        read,
        SpellUpdate::Began {
            caster_id: 40,
            spell_id: 1234,
            duration_ms: 3000,
        }
    );
    let mut titanium = 40u16.to_le_bytes().to_vec();
    titanium.extend_from_slice(&1234u16.to_le_bytes());
    titanium.extend_from_slice(&3000u32.to_le_bytes());
    assert_eq!(
        crate::spells::decode(0x3990, &titanium).unwrap(),
        Some(read)
    );
    let spell = begin_cast(&begin_cast_body(u32::from(u16::MAX), 1, 0)).unwrap();
    assert!(matches!(
        spell,
        SpellUpdate::Began {
            spell_id: u16::MAX,
            caster_id: 1,
            duration_ms: 0
        }
    ));
    assert!(begin_cast(&begin_cast_body(0x1_0000, 40, 3000)).is_err());
}

#[test]
fn a_spells_action_reads_each_field_as_titaniums() {
    let read = action(&action_body(SPELL, 0x5a)).unwrap();
    assert_eq!(
        read,
        crate::buffs::titanium_action(&titanium_action_body(SPELL)).unwrap()
    );
    let Action::Spell(effect) = read.clone() else {
        panic!("not a spell's action");
    };
    assert_eq!(
        (
            effect.target_id,
            effect.caster_id,
            effect.caster_level,
            effect.instrument_modifier,
            effect.spell_id,
            effect.spell_level,
            effect.effect_flag,
        ),
        (73, 40, 51, 18, 1234, 49, 4)
    );
    let push = effect.push.unwrap();
    assert_eq!((push.force, push.heading, push.pitch), (2.5, 128.0, 0.5));
    // The encoder's constants and zeros change nothing.
    assert_eq!(action(&action_body(SPELL, 0)).unwrap(), read);
    assert_eq!(action(&action_body(SPELL, 0xff)).unwrap(), read);
    // No push, as Titanium's.
    let still = Values {
        push: [0.0, 64.0, 0.0],
        ..SPELL
    };
    assert_eq!(
        action(&action_body(still, 0x5a)).unwrap(),
        crate::buffs::titanium_action(&titanium_action_body(still)).unwrap()
    );
}

#[test]
fn other_actions_read_as_titaniums_by_their_kind() {
    let melee = Values {
        kind: 1,
        modifier: 3,
        ..SPELL
    };
    let spin = Values {
        target: 0x0a0b,
        source: 0x0a0b,
        level: 0x00e7,
        modifier: 1101,
        push: [0.0, 0.0, 425.0],
        kind: 0,
        spell_id: 0,
        spell_level: 0,
        effect_flag: 0,
    };
    let near_spin = Values { source: 41, ..spin };
    for (values, expected) in [
        (melee, Action::Other(1)),
        (spin, Action::Spin),
        (near_spin, Action::Other(0)),
    ] {
        let read = action(&action_body(values, 0x5a)).unwrap();
        assert_eq!(read, expected);
        assert_eq!(
            read,
            crate::buffs::titanium_action(&titanium_action_body(values)).unwrap()
        );
    }
    // Another kind reads no modifier, so none refuses it.
    assert_eq!(
        action(&action_body_with(melee, f32::NAN, 0x1_0000, 0x5a)).unwrap(),
        Action::Other(1)
    );
}

fn modifier_of(modifier: f32) -> Result<u32> {
    action(&action_body_with(SPELL, modifier, 1234, 0x5a)).map(|read| match read {
        Action::Spell(effect) => effect.instrument_modifier,
        other => panic!("{other:?}"),
    })
}

#[test]
fn the_instrument_modifier_reads_back_the_servers_integer() {
    for modifier in [10, 11, 15, 18, 25, 100, 1_000, 99_999, 100_000] {
        assert_eq!(modifier_of(encoded(modifier)).unwrap(), modifier);
    }
    // Every integer up to the limit survives the encoder's rounding.
    for modifier in 10..=100_000 {
        assert_eq!(
            modifier_of(encoded(modifier)).unwrap(),
            modifier,
            "{modifier}"
        );
    }
}

#[test]
fn a_modifier_below_ten_wraps_and_reads_zero() {
    // 0 is what Client::SendSpellAnim leaves; 1 to 9 a weak focus; the last
    // a focus result below zero, which the server's uint32 wraps.
    for modifier in [
        0,
        1,
        5,
        9,
        10u32.wrapping_sub(20),
        10u32.wrapping_sub(50_000),
    ] {
        assert_eq!(modifier_of(encoded(modifier)).unwrap(), 0, "{modifier}");
    }
    // Titanium gets the 0 as it is.
    let unset = Values {
        modifier: 0,
        ..SPELL
    };
    assert_eq!(
        action(&action_body(unset, 0x5a)).unwrap(),
        crate::buffs::titanium_action(&titanium_action_body(unset)).unwrap()
    );
}

#[test]
fn a_modifier_no_server_integer_gives_refuses() {
    for modifier in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        -1.0,
        0.0,
        0.5,
        0.95,
        1.05,
        1.55,
        encoded(100_001),
        encoded(5_000_000),
        1.0e9,
    ] {
        assert!(modifier_of(modifier).is_err(), "{modifier}");
    }
}

#[test]
fn a_spell_past_sixteen_bits_refuses_its_action() {
    let body = action_body_with(SPELL, encoded(10), 0x1_0000, 0x5a);
    assert!(action(&body).is_err());
    let body = action_body_with(SPELL, encoded(10), u32::from(u16::MAX), 0x5a);
    assert!(action(&body).is_ok());
}

#[test]
fn an_interruption_reads_as_titaniums() {
    let mut named = 40u32.to_le_bytes().to_vec();
    named.extend_from_slice(&439u32.to_le_bytes());
    named.extend_from_slice(b"Testname\0");
    for body in [&named[..8], &named[..]] {
        let read = read(INTERRUPT_OPCODE, body).unwrap().unwrap();
        assert_eq!(
            read,
            Casting::Event(WorldEvent::Spell(
                crate::spells::decode(0x0b97, body).unwrap().unwrap()
            ))
        );
    }
    assert!(read(INTERRUPT_OPCODE, &named[..7]).is_err());
    assert!(read(INTERRUPT_OPCODE, &named[..12]).is_err());
}

#[test]
fn a_stun_reads_its_duration_and_only_its_length_holds_the_rest() {
    for filler in [0, 0x5a, 0xff] {
        assert_eq!(
            stun(&stun_body(2000, filler)).unwrap(),
            Some(std::time::Duration::from_millis(2000))
        );
        assert_eq!(stun(&stun_body(0, filler)).unwrap(), None);
    }
    // As Titanium's four bytes.
    assert_eq!(
        stun(&[0, 163, 67, 0, 0, 163, 67, 0]).unwrap(),
        crate::control::stun(&[0, 163, 67, 0]).unwrap()
    );
}

#[test]
fn an_illusion_reads_each_field_as_titaniums() {
    let read = illusion(&illusion_body(LOOK, 0x5a)).unwrap();
    assert_eq!(
        read,
        crate::appearance::titanium_illusion(&titanium_illusion_body(LOOK, 75)).unwrap()
    );
    assert_eq!(
        read,
        WorldEvent::Appearance {
            spawn_id: 312,
            change: AppearanceChange::Illusion(Illusion {
                race: 75,
                gender: 1,
                texture: Some(3),
                helm_texture: Some(2),
                size: 6.5,
                features: Features {
                    face: 4,
                    hair_style: 5,
                    hair_color: 6,
                    beard: 7,
                    beard_color: 8,
                },
            }),
        }
    );
    // The name, bytes 70 and 71, the zero word, the Drakkin words and the
    // constant change nothing.
    assert_eq!(illusion(&illusion_body(LOOK, 0)).unwrap(), read);
    assert_eq!(illusion(&illusion_body(LOOK, 0xff)).unwrap(), read);
    // 0xff textures are the race's own gear, as Titanium's.
    let bare = Look {
        texture: 0xff,
        helm: 0xff,
        race: u16::MAX,
        ..LOOK
    };
    let read = illusion(&illusion_body(bare, 0x5a)).unwrap();
    assert_eq!(
        read,
        crate::appearance::titanium_illusion(&titanium_illusion_body(bare, 0xffff)).unwrap()
    );
    let WorldEvent::Appearance {
        change: AppearanceChange::Illusion(look),
        ..
    } = read
    else {
        panic!("not an illusion");
    };
    assert_eq!(
        (look.texture, look.helm_texture, look.race),
        (None, None, 0xffff)
    );
}

#[test]
fn an_illusion_keeps_titaniums_refusals() {
    for look in [
        Look { spawn: 0, ..LOOK },
        Look {
            spawn: 0x1_0000,
            ..LOOK
        },
        Look { face: 256, ..LOOK },
        Look { size: -0.5, ..LOOK },
        Look {
            size: f32::NAN,
            ..LOOK
        },
        Look {
            size: f32::INFINITY,
            ..LOOK
        },
    ] {
        assert!(illusion(&illusion_body(look, 0x5a)).is_err());
        assert!(
            crate::appearance::titanium_illusion(&titanium_illusion_body(look, look.race.into()))
                .is_err()
        );
    }
    let zero = Look { size: 0.0, ..LOOK };
    assert!(illusion(&illusion_body(zero, 0x5a)).is_ok());
}

#[test]
fn a_face_change_reads_its_spawn_and_both_eye_colors() {
    let read = face_change(&face_body(312, 0x5a)).unwrap();
    let features = Features {
        face: 16,
        hair_style: 14,
        hair_color: 10,
        beard: 15,
        beard_color: 11,
    };
    assert_eq!(
        read,
        WorldEvent::FaceChanged {
            spawn_id: Some(312),
            features,
            eye_colors: Some([12, 13]),
        }
    );
    // The padding byte and the Drakkin words change nothing.
    assert_eq!(face_change(&face_body(312, 0)).unwrap(), read);
    assert_eq!(face_change(&face_body(312, 0xff)).unwrap(), read);
    // Titanium's encoder sends the same change as an illusion of race -1,
    // which carries no eye colors.
    let mut titanium = vec![0; 168];
    titanium[..4].copy_from_slice(&312u32.to_le_bytes());
    titanium[68..72].copy_from_slice(&(-1i32).to_le_bytes());
    titanium[76..80].copy_from_slice(&16u32.to_le_bytes());
    titanium[80..84].copy_from_slice(&[14, 10, 15, 11]);
    assert_eq!(
        crate::appearance::titanium_illusion(&titanium).unwrap(),
        WorldEvent::FaceChanged {
            spawn_id: Some(312),
            features,
            eye_colors: None,
        }
    );
    assert!(face_change(&face_body(0, 0x5a)).is_err());
    assert!(face_change(&face_body(0x1_0000, 0x5a)).is_err());
}

#[test]
fn every_fixed_message_refuses_a_cut_and_a_trailing_byte() {
    for (opcode, body) in fixed() {
        assert!(read(opcode, &body).unwrap().is_some(), "{opcode:#06x}");
        for length in 0..body.len() {
            assert!(
                read(opcode, &body[..length]).is_err(),
                "{opcode:#06x} {length}"
            );
        }
        let mut long = body.clone();
        long.push(0);
        assert!(read(opcode, &long).is_err(), "{opcode:#06x}");
    }
}

#[test]
fn another_opcode_is_not_this_modules() {
    for opcode in [0x3990, 0x497c, 0x0b97, 0x1e51, 0x448d, 0x5089] {
        assert_eq!(read(opcode, &[0; 336]).unwrap(), None, "{opcode:#06x}");
    }
}

#[test]
fn the_zone_reads_each_message_with_titaniums_parts() {
    for (opcode, body) in fixed() {
        assert!(
            matches!(rof2(opcode, &body)[..], [Message::Event(_)]),
            "{opcode:#06x}"
        );
    }
    let melee = Values { kind: 8, ..SPELL };
    assert!(matches!(
        rof2(ACTION_OPCODE, &action_body(melee, 0x5a))[..],
        [Message::UnknownKind(8)]
    ));
    for (opcode, part) in [
        (BEGIN_CAST_OPCODE, Part::Spells),
        (INTERRUPT_OPCODE, Part::Spells),
        (ACTION_OPCODE, Part::World),
        (STUN_OPCODE, Part::World),
        (ILLUSION_OPCODE, Part::World),
        (FACE_OPCODE, Part::World),
    ] {
        assert!(
            matches!(&rof2(opcode, &[1, 2, 3])[..], [Message::Unreadable { part: read, .. }] if *read == part),
            "{opcode:#06x}"
        );
    }
}
