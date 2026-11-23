//! Bodies written by our own writers, from the server's structs.
use super::*;
use crate::{
    effects::{Cue, Effect, Particle},
    message::{rof2, Message, Part},
    socials::{Assisted, Roll},
};
use std::time::Duration;

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn roll_body(low: u32, high: u32, result: u32, name: &str) -> Vec<u8> {
    let mut body = words(&[low, high, result]);
    let mut field = vec![0; 64];
    field[..name.len()].copy_from_slice(name.as_bytes());
    body.extend(field);
    body
}

fn particles_body(spawn: u32) -> Vec<u8> {
    let mut values = vec![spawn];
    for (effect, slot, ground) in [(77, 1, 0), (0, 0, 0), (79, 2, 1), (0, 0, 0), (81, 9, 1)] {
        values.extend([effect, slot, ground]);
    }
    words(&values)
}

fn camera_body(duration: u32, intensity: f32) -> Vec<u8> {
    words(&[duration, intensity.to_bits()])
}

fn nimbus_body(caster: u32, target: u32) -> Vec<u8> {
    let mut body = words(&[1234, caster, target, 6000, 250, 3000]);
    body.extend([1, 1, 0, 0]);
    body
}

/// Every message under its `RoF2` number, with Titanium's number for it.
fn messages() -> [(u16, u16, Vec<u8>); 7] {
    [
        (
            ROLL_OPCODE,
            crate::socials::ROLL_OPCODE,
            roll_body(1, 100, 42, "Testroller"),
        ),
        (ASSIST_OPCODE, crate::socials::ASSIST_OPCODE, words(&[77])),
        (YELL_OPCODE, crate::socials::YELL_OPCODE, words(&[4321])),
        (
            LEVEL_APPEARANCE_OPCODE,
            crate::effects::LEVEL_APPEARANCE_OPCODE,
            particles_body(77),
        ),
        (
            CAMERA_EFFECT_OPCODE,
            crate::effects::CAMERA_EFFECT_OPCODE,
            camera_body(1500, 0.25),
        ),
        (
            PLAY_MP3_OPCODE,
            crate::effects::PLAY_MP3_OPCODE,
            b"testsound.mp3\0".to_vec(),
        ),
        (
            SPELL_EFFECT_OPCODE,
            crate::effects::SPELL_EFFECT_OPCODE,
            nimbus_body(4321, 77),
        ),
    ]
}

/// What Titanium's own reader says of a body under Titanium's number.
fn titanium(opcode: u16, body: &[u8]) -> WorldEvent {
    if let Some(roll) = crate::socials::decode_roll(opcode, body).unwrap() {
        WorldEvent::Roll(roll)
    } else if let Some(answer) = crate::socials::decode_assist(opcode, body).unwrap() {
        WorldEvent::Assisted(answer)
    } else if let Some(event) = crate::socials::titanium(opcode, body).unwrap() {
        event
    } else {
        WorldEvent::Effect(crate::effects::titanium(opcode, body).unwrap().unwrap())
    }
}

#[test]
fn each_message_reads_as_titaniums() {
    for (opcode, titanium_opcode, body) in messages() {
        assert_eq!(
            read(opcode, &body).unwrap(),
            Some(titanium(titanium_opcode, &body)),
            "{opcode:#06x}"
        );
    }
}

#[test]
fn each_message_reads_each_field() {
    assert_eq!(
        read(ROLL_OPCODE, &roll_body(5, 250, 117, "Testroller")).unwrap(),
        Some(WorldEvent::Roll(Roll {
            name: "Testroller".into(),
            low: 5,
            high: 250,
            result: 117,
        }))
    );
    for (target, read_target) in [(77, Some(77)), (0, None)] {
        assert_eq!(
            read(ASSIST_OPCODE, &words(&[target])).unwrap(),
            Some(WorldEvent::Assisted(Assisted {
                target: read_target
            }))
        );
    }
    assert_eq!(
        read(YELL_OPCODE, &words(&[4321])).unwrap(),
        Some(WorldEvent::YellForHelp { spawn_id: 4321 })
    );
    assert_eq!(
        read(LEVEL_APPEARANCE_OPCODE, &particles_body(77)).unwrap(),
        Some(WorldEvent::Effect(Effect::Particles {
            spawn_id: 77,
            effects: vec![
                Particle {
                    effect: 77,
                    slot: 1,
                    ground: 0,
                },
                Particle {
                    effect: 79,
                    slot: 2,
                    ground: 1,
                },
                Particle {
                    effect: 81,
                    slot: 9,
                    ground: 1,
                },
            ],
        }))
    );
    assert_eq!(
        read(CAMERA_EFFECT_OPCODE, &camera_body(1500, 0.25)).unwrap(),
        Some(WorldEvent::Effect(Effect::CameraShake {
            duration: Duration::from_millis(1500),
            intensity: 0.25,
        }))
    );
    assert_eq!(
        read(PLAY_MP3_OPCODE, b"testsound.mp3\0").unwrap(),
        Some(WorldEvent::Effect(Effect::Sound(Cue::File(
            "testsound.mp3".into()
        ))))
    );
    assert_eq!(
        read(SPELL_EFFECT_OPCODE, &nimbus_body(4321, 77)).unwrap(),
        Some(WorldEvent::Effect(Effect::Nimbus {
            effect: 1234,
            caster_id: 4321,
            target_id: 77,
            duration: Duration::from_millis(6000),
            finish_delay: Duration::from_millis(250),
        }))
    );
}

#[test]
fn each_message_refuses_what_titaniums_refuses() {
    for spawn in [0, 0x1_0000] {
        assert!(read(YELL_OPCODE, &words(&[spawn])).is_err());
        assert!(read(LEVEL_APPEARANCE_OPCODE, &particles_body(spawn)).is_err());
        assert!(read(SPELL_EFFECT_OPCODE, &nimbus_body(spawn, 77)).is_err());
        assert!(read(SPELL_EFFECT_OPCODE, &nimbus_body(4321, spawn)).is_err());
    }
    assert!(read(ASSIST_OPCODE, &words(&[0x1_0000])).is_err());
    for intensity in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(read(CAMERA_EFFECT_OPCODE, &camera_body(1500, intensity)).is_err());
    }
    for body in [&b""[..], b"testsound.mp3"] {
        assert!(read(PLAY_MP3_OPCODE, body).is_err());
    }
    // The nimbus's field after the delay and its last bytes change nothing,
    // as on Titanium.
    let plain = read(SPELL_EFFECT_OPCODE, &nimbus_body(4321, 77)).unwrap();
    let mut body = nimbus_body(4321, 77);
    body[20..28].fill(0x5a);
    assert_eq!(read(SPELL_EFFECT_OPCODE, &body).unwrap(), plain);
}

#[test]
fn a_refused_roll_names_no_one() {
    let body = roll_body(1, 100, 42, "Testroller");
    for cut in [&body[..75], &[body.clone(), vec![0]].concat()[..]] {
        let error = read(ROLL_OPCODE, cut).unwrap_err();
        assert!(!format!("{error:#}").contains("Testroller"), "{error:#}");
        let messages = rof2(ROLL_OPCODE, cut);
        let [Message::Unreadable { .. }] = &messages[..] else {
            panic!("unreadable")
        };
        assert!(!format!("{messages:?}").contains("Testroller"));
    }
}

#[test]
fn every_fixed_length_refuses_a_cut_and_a_trailing_byte() {
    for (opcode, _, body) in messages() {
        if opcode == PLAY_MP3_OPCODE {
            continue;
        }
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
    for (opcode, titanium_opcode, body) in messages() {
        assert!(
            crate::message::titanium(opcode, &body).is_empty(),
            "{opcode:#06x}"
        );
        assert!(
            crate::message::eqmac(opcode, &body).is_empty(),
            "{opcode:#06x}"
        );
        assert_eq!(read(titanium_opcode, &body).unwrap(), None);
    }
}

#[test]
fn the_zone_reads_each_message_as_the_worlds() {
    for (opcode, _, body) in messages() {
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
