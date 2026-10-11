//! Independent synthetic records; no game captures or account data.
use eq_network_game::{
    appearance::{AppearanceChange, FlyMode, IllusionSize},
    clock::{Precipitation, Weather},
    confirmation::{Confirmation, TranslocateState},
    control::{CharmCommand, ControlUpdate},
    message::{self, Message},
    reuse::ReuseUpdate,
    world::WorldEvent,
    GameDialect::{self, EqMac, Titanium},
};

fn messages(dialect: GameDialect, opcode: u16, body: &[u8]) -> Vec<Message> {
    match dialect {
        Titanium => message::titanium(opcode, body),
        EqMac => message::eqmac(opcode, body),
        _ => unreachable!(),
    }
}

fn event(dialect: GameDialect, opcode: u16, body: &[u8]) -> WorldEvent {
    let mut events = messages(dialect, opcode, body);
    assert_eq!(events.len(), 1);
    match events.pop().unwrap() {
        Message::Event(event) => event,
        other => panic!("expected event, got {other:?}"),
    }
}

fn put(body: &mut [u8], offset: usize, value: u32) {
    body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn translocate() -> Vec<u8> {
    let mut body = vec![0; 92];
    put(&mut body, 0, 9);
    put(&mut body, 4, 1422);
    put(&mut body, 8, 0x8765_4321);
    body[12..22].copy_from_slice(b"TestCaster");
    put(&mut body, 76, 25.5f32.to_bits());
    put(&mut body, 80, (-13.25f32).to_bits());
    put(&mut body, 84, 4.5f32.to_bits());
    body
}

fn illusion(dialect: GameDialect) -> Vec<u8> {
    if dialect == Titanium {
        let mut body = vec![0; 168];
        put(&mut body, 0, 321);
        put(&mut body, 68, 128);
        body[72..75].copy_from_slice(&[2, 255, 254]);
        put(&mut body, 76, 700);
        body[80..84].copy_from_slice(&[1, 2, 3, 4]);
        put(&mut body, 84, 6.25f32.to_bits());
        body
    } else {
        vec![
            65, 1, 128, 0, 2, 255, 254, 255, 188, 2, 1, 2, 3, 4, 0, 0, 255, 255, 255, 255,
        ]
    }
}

#[test]
fn stun_duration_and_release_are_not_seconds_or_boolean_flags() {
    for (dialect, opcode) in [(Titanium, 0x1e51), (EqMac, 0x5b41)] {
        for duration_ms in [0u32, 1, 2500, u32::MAX] {
            assert_eq!(
                event(dialect, opcode, &duration_ms.to_le_bytes()),
                WorldEvent::Control(ControlUpdate::Stun { duration_ms })
            );
        }
    }
}

#[test]
fn charm_widths_differ_but_unknown_commands_survive() {
    for command in [0u16, 1, 789] {
        let expected = WorldEvent::Control(ControlUpdate::Charm {
            owner_id: 300,
            pet_id: 400,
            command: CharmCommand::from(u32::from(command)),
        });
        let mut titanium = vec![44, 1, 0, 0, 144, 1, 0, 0];
        titanium.extend(u32::from(command).to_le_bytes());
        let mut eqmac = vec![44, 1, 144, 1];
        eqmac.extend(command.to_le_bytes());
        assert_eq!(event(Titanium, 0x12e5, &titanium), expected);
        assert_eq!(event(EqMac, 0x4442, &eqmac), expected);
    }
}

#[test]
fn forced_target_is_distinct_from_submission_and_can_clear() {
    for id in [0u16, 513] {
        let expected = WorldEvent::Control(ControlUpdate::Target {
            spawn_id: (id != 0).then_some(id),
        });
        assert_eq!(
            event(Titanium, 0x1477, &u32::from(id).to_le_bytes()),
            expected
        );
        assert_eq!(event(EqMac, 0xfe41, &id.to_le_bytes()), expected);
    }
    assert!(eq_network_game::control::decode(Titanium, 0x1477, &65536u32.to_le_bytes()).is_err());
    assert!(eq_network_game::control::decode(Titanium, 0x12e5, &[0; 12]).is_err());
}

#[test]
fn reuse_preserves_timer_groups_units_and_wrapping_server_timestamps() {
    assert_eq!(
        event(
            Titanium,
            0x53df,
            &[9, 0, 0, 0, 120, 0, 0, 0, 99, 88, 77, 66]
        ),
        WorldEvent::Reuse(ReuseUpdate::DisciplineTimer {
            timer_id: 9,
            remaining_seconds: 120
        })
    );
    assert_eq!(
        event(Titanium, 0x53df, &[0; 12]),
        WorldEvent::Reuse(ReuseUpdate::DisciplineTimer {
            timer_id: 0,
            remaining_seconds: 0
        })
    );
    assert_eq!(
        event(
            Titanium,
            0x6a00,
            &[6, 0, 0, 0, 3, 0, 0, 0, 254, 255, 255, 255]
        ),
        WorldEvent::Reuse(ReuseUpdate::LinkedSpell {
            timer_id: 6,
            end_seconds: 3,
            start_seconds: u32::MAX - 1
        })
    );
    // The last three EQMac bytes are padding, not a 32-bit discipline ID.
    assert_eq!(
        event(EqMac, 0xf241, &[17, 255, 255, 255]),
        WorldEvent::Reuse(ReuseUpdate::DisciplineActivated(17))
    );
}

#[test]
fn discipline_list_keeps_empty_slots_and_full_width_spell_ids() {
    let mut body = vec![0; 400];
    put(&mut body, 4, 1234);
    put(&mut body, 396, u32::MAX);
    let WorldEvent::Reuse(ReuseUpdate::Disciplines(list)) = event(Titanium, 0x7180, &body) else {
        panic!()
    };
    assert_eq!(list.len(), 100);
    assert_eq!((list[0], list[1], list[99]), (0, 1234, u32::MAX));
}

#[test]
fn translocate_prompt_is_not_a_zone_order_and_axes_are_not_swapped() {
    for (dialect, opcode) in [(Titanium, 0x8258), (EqMac, 0x0642)] {
        let mut body = translocate();
        for (state, expected) in [
            (0, TranslocateState::Offered),
            (1, TranslocateState::Accepted),
            (77, TranslocateState::Unknown(77)),
        ] {
            put(&mut body, 88, state);
            let WorldEvent::Confirmation(Confirmation::Translocate(offer)) =
                event(dialect, opcode, &body)
            else {
                panic!()
            };
            assert_eq!((offer.x, offer.y, offer.z), (-13.25, 25.5, 4.5));
            assert_eq!((offer.zone_id, offer.spell_id), (9, 1422));
            assert_eq!(offer.caster, "TestCaster");
            assert_eq!(offer.response_word, 0x8765_4321);
            assert_eq!(offer.state, expected);
        }
        put(&mut body, 88, u32::MAX);
        let WorldEvent::Confirmation(Confirmation::Translocate(offer)) =
            event(dialect, opcode, &body)
        else {
            panic!()
        };
        assert_eq!(
            offer.state,
            if dialect == EqMac {
                TranslocateState::Declined
            } else {
                TranslocateState::Unknown(u32::MAX)
            }
        );
        put(&mut body, 76, f32::NAN.to_bits());
        assert!(eq_network_game::confirmation::decode(dialect, opcode, &body).is_err());
    }
}

#[test]
fn sacrifice_preserves_confirmation_without_automatically_accepting() {
    for (dialect, opcode) in [(Titanium, 0x727a), (EqMac, 0xea41)] {
        assert_eq!(
            event(dialect, opcode, &[1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]),
            WorldEvent::Confirmation(Confirmation::Sacrifice {
                caster_id: 1,
                target_id: 2,
                confirm: 0
            })
        );
    }
}

#[test]
fn weather_keeps_stop_intensity_unknown_types_and_admission_layout_separate() {
    for (dialect, opcode) in [(Titanium, 0x254d), (EqMac, 0x3641)] {
        assert_eq!(
            event(dialect, opcode, &[1, 0, 0, 0, 0, 0, 0, 0]),
            WorldEvent::Weather(Weather::Precipitation {
                kind: Precipitation::Snow,
                intensity: 0
            })
        );
        assert_eq!(
            event(dialect, opcode, &[250, 0, 0, 0, 255, 255, 255, 255]),
            WorldEvent::Weather(Weather::Precipitation {
                kind: Precipitation::Unknown(250),
                intensity: u32::MAX
            })
        );
    }
    assert_eq!(
        event(Titanium, 0x254d, &[255, 0, 0, 0, 49, 0, 0, 0, 7, 0, 0, 0]),
        WorldEvent::Weather(Weather::Admission {
            marker: 255,
            kind: Some(Precipitation::Rain),
            mode: 7
        })
    );
    assert!(eq_network_game::clock::weather(EqMac, 0x3641, &[0; 12]).is_err());
}

#[test]
fn illusions_keep_integer_default_and_float_sizes_distinct() {
    for (dialect, opcode) in [(Titanium, 0x448d), (EqMac, 0x9140)] {
        let WorldEvent::Illusion(value) = event(dialect, opcode, &illusion(dialect)) else {
            panic!()
        };
        assert_eq!((value.spawn_id, value.race, value.gender), (321, 128, 2));
        assert_eq!(
            (value.texture, value.helm_texture, value.face),
            (255, 254, 700)
        );
        assert_eq!(
            (
                value.hair_style,
                value.hair_color,
                value.beard,
                value.beard_color
            ),
            (1, 2, 3, 4)
        );
        assert_eq!(
            value.size,
            if dialect == Titanium {
                IllusionSize::Titanium(6.25)
            } else {
                IllusionSize::EqMac(-1)
            }
        );
    }
    let mut body = illusion(Titanium);
    put(&mut body, 84, f32::INFINITY.to_bits());
    assert!(eq_network_game::appearance::illusion(Titanium, 0x448d, &body).is_err());
}

#[test]
fn appearance_retains_unknown_subtypes_including_spawn_zero_extensions() {
    for (dialect, opcode) in [(Titanium, 0x7c32), (EqMac, 0xf540)] {
        for (kind, value, change) in [
            (5u16, 7u32, AppearanceChange::Light(7)),
            (15, 1, AppearanceChange::Sneaking(true)),
            (19, 2, AppearanceChange::FlyMode(FlyMode::Levitating)),
            (19, 999, AppearanceChange::FlyMode(FlyMode::Unknown(999))),
            (29, 8, AppearanceChange::Size(8)),
            (
                99,
                123,
                AppearanceChange::Unknown {
                    kind: 99,
                    value: 123,
                },
            ),
            (15, 2, AppearanceChange::Unknown { kind: 15, value: 2 }),
        ] {
            let mut body = vec![73, 0];
            body.extend(kind.to_le_bytes());
            body.extend(value.to_le_bytes());
            assert_eq!(
                event(dialect, opcode, &body),
                WorldEvent::Appearance {
                    spawn_id: 73,
                    change
                }
            );
        }
        assert_eq!(
            event(dialect, opcode, &[0, 0, 99, 0, 1, 0, 0, 0]),
            WorldEvent::Appearance {
                spawn_id: 0,
                change: AppearanceChange::Unknown { kind: 99, value: 1 }
            }
        );
    }
}

#[test]
fn all_added_opcodes_are_routed_and_reject_every_truncation_and_trailing_byte() {
    let entries = [
        (Titanium, 0x1e51, vec![0; 4]),
        (EqMac, 0x5b41, vec![0; 4]),
        (Titanium, 0x12e5, vec![1, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0]),
        (EqMac, 0x4442, vec![1, 0, 2, 0, 1, 0]),
        (Titanium, 0x1477, vec![0; 4]),
        (EqMac, 0xfe41, vec![0; 2]),
        (Titanium, 0x53df, vec![0; 12]),
        (Titanium, 0x7180, vec![0; 400]),
        (Titanium, 0x6a00, vec![0; 12]),
        (EqMac, 0xf241, vec![0; 4]),
        (Titanium, 0x8258, translocate()),
        (EqMac, 0x0642, translocate()),
        (Titanium, 0x727a, vec![1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]),
        (EqMac, 0xea41, vec![1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]),
        (Titanium, 0x254d, vec![0; 8]),
        (EqMac, 0x3641, vec![0; 8]),
        (Titanium, 0x448d, illusion(Titanium)),
        (EqMac, 0x9140, illusion(EqMac)),
    ];
    for (dialect, opcode, mut body) in entries {
        event(dialect, opcode, &body);
        for length in 0..body.len() {
            assert!(
                matches!(
                    messages(dialect, opcode, &body[..length]).as_slice(),
                    [Message::Unreadable { .. }]
                ),
                "{dialect:?} {opcode:#06x} length {length}"
            );
        }
        body.push(0);
        assert!(matches!(
            messages(dialect, opcode, &body).as_slice(),
            [Message::Unreadable { .. }]
        ));
    }
}

#[test]
fn unknown_opcodes_stay_unclaimed() {
    for dialect in [Titanium, EqMac] {
        assert!(messages(dialect, 0xffff, &[255; 17]).is_empty());
    }
}
