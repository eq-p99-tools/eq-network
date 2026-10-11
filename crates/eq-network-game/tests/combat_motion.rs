//! Synthetic action and damage layouts, including nonzero push parameters.
use eq_network_game::{
    combat::{self, ActionDetails, HitMotion},
    message::{self, Message},
    world::WorldEvent,
    GameDialect::{self, EqMac, Titanium},
};

fn put(body: &mut [u8], offset: usize, value: u32) {
    body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn motion(body: &mut [u8], at: usize) -> HitMotion {
    for (offset, value) in [(at, 0.125f32), (at + 4, -32.5), (at + 8, 0.75)] {
        put(body, offset, value.to_bits());
    }
    HitMotion {
        force: 0.125,
        heading: -32.5,
        pitch: 0.75,
    }
}

fn action(dialect: GameDialect) -> Vec<u8> {
    let mut body = vec![0; if dialect == Titanium { 31 } else { 36 }];
    body[..6].copy_from_slice(&[7, 0, 8, 0, 60, 0]);
    if dialect == Titanium {
        put(&mut body, 6, 10);
        motion(&mut body, 10);
        body[22..].copy_from_slice(&[1, 123, 0, 188, 2, 42, 0, 55, 4]);
    } else {
        body[6..8].copy_from_slice(&[50, 0]);
        put(&mut body, 8, 10);
        motion(&mut body, 12);
        body[24..].copy_from_slice(&[1, 255, 77, 0, 254, 255, 42, 0, 0, 4, 255, 255]);
    }
    body
}

#[test]
fn actions_retain_generation_specific_fields_and_negative_taps() {
    for dialect in [Titanium, EqMac] {
        let value = combat::action(dialect, &action(dialect)).unwrap();
        assert_eq!(
            (
                value.target_id,
                value.source_id,
                value.level,
                value.modifier_word
            ),
            (7, 8, 60, 10)
        );
        assert_eq!((value.kind, value.spell_id, value.effect_flag), (1, 42, 4));
        assert_eq!(
            value.motion,
            HitMotion {
                force: 0.125,
                heading: -32.5,
                pitch: 0.75
            }
        );
        assert_eq!(
            value.details,
            if dialect == Titanium {
                ActionDetails::Titanium {
                    unknown23: 123,
                    unknown25: 700,
                    spell_level: 55,
                }
            } else {
                ActionDetails::EqMac {
                    target_level: 50,
                    unused_spell_id: 77,
                    tap_amount: -2,
                }
            }
        );
    }
}

#[test]
fn one_action_packet_emits_either_a_spell_effect_or_a_non_spell_action() {
    for (dialect, opcode, kind_offset) in [(Titanium, 0x497c, 22), (EqMac, 0x4640, 24)] {
        let mut body = action(dialect);
        for kind in [1, 231, 255] {
            body[kind_offset] = kind;
            let messages = if dialect == Titanium {
                message::titanium(opcode, &body)
            } else {
                message::eqmac(opcode, &body)
            };
            assert_eq!(messages.len(), 1);
            if kind == 231 {
                let Message::Event(WorldEvent::SpellEffect(spell)) = &messages[0] else {
                    panic!()
                };
                assert_eq!((spell.spell_id, spell.effect_flag), (42, 4));
                assert_eq!(spell.motion.force, 0.125);
                if dialect == EqMac {
                    assert!(matches!(
                        spell.details,
                        ActionDetails::EqMac { tap_amount: -2, .. }
                    ));
                }
                assert_eq!(spell.spell_level, if dialect == Titanium { 55 } else { 0 });
            } else {
                assert!(
                    matches!(&messages[0], Message::Event(WorldEvent::Action(value)) if value.kind == kind)
                );
            }
        }
    }
}

#[test]
fn damage_push_uses_different_offsets_in_the_two_generations() {
    let mut titanium = [0; 23];
    let expected = motion(&mut titanium, 11);
    let mut eqmac = [0; 24];
    motion(&mut eqmac, 12);
    assert_eq!(combat::damage(&titanium).unwrap().motion, expected);
    assert_eq!(combat::eqmac_damage(&eqmac).unwrap().motion, expected);
}

#[test]
fn actions_reject_every_truncation_and_nonfinite_push_component() {
    for dialect in [Titanium, EqMac] {
        let mut body = action(dialect);
        for len in 0..body.len() {
            assert!(combat::action(dialect, &body[..len]).is_err());
        }
        body.push(0);
        assert!(combat::action(dialect, &body).is_err());
        let push = if dialect == Titanium { 10 } else { 12 };
        for offset in [push, push + 4, push + 8] {
            let mut body = action(dialect);
            put(&mut body, offset, f32::NAN.to_bits());
            assert!(combat::action(dialect, &body).is_err());
        }
    }
}

#[test]
fn damage_rejects_nonfinite_push_instead_of_emitting_unserializable_values() {
    for offset in [11, 15, 19] {
        let mut body = [0; 23];
        put(&mut body, offset, f32::INFINITY.to_bits());
        assert!(combat::damage(&body).is_err());
    }
    for offset in [12, 16, 20] {
        let mut body = [0; 24];
        put(&mut body, offset, f32::NEG_INFINITY.to_bits());
        assert!(combat::eqmac_damage(&body).is_err());
    }
}
