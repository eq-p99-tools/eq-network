//! Synthetic fixtures encoded with the server's documented zlib/word cipher.
//! No live profiles, credentials, character names, or packet captures.
use super::*;

#[test]
fn profile_attributes_preserve_signed_width_and_field_order() {
    let mut data = vec![0; PROFILE_SIZE];
    data[6..13].copy_from_slice(b"Example");
    data[144..146].copy_from_slice(&14u16.to_le_bytes());
    for (index, value) in [71i16, 82, 93, 104, 115, 126, -7].into_iter().enumerate() {
        let offset = 164 + index * 2;
        data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    let player = decoded_profile(&data, "Example").unwrap();
    assert_eq!(player.class, Some(14));
    assert_eq!(
        player.base_attributes,
        Some(BaseAttributes {
            strength: 71,
            stamina: 82,
            charisma: 93,
            dexterity: 104,
            intelligence: 115,
            agility: 126,
            wisdom: -7,
        })
    );
    assert!(decoded_profile(&data[..177], "Example").is_err());
}
const PROFILE: &str = "ae05f5a6083907c71a8fa0b9df18601d2fadb4d6b80a3099d83162b5c95015d3bd8bd662ad969f16538260e8249c5a9d1c78803e51a3f8d3d48f5ca3fb28b2bff7a48403366d5c880b3422059ae86574971744def74907496b270882";
const SPAWNS: &str = "28e67b5d096069468c09009a236b6b8bace06fcd5ad543cb0f2173178769ee91ddec0b81713d9ef952c724020b109b20e8bbf4f4b5b2e79e";

#[test]
fn synthetic_profile_decrypts_and_projects_only_known_fields() {
    let wire = hex::decode(PROFILE).unwrap();
    let player = profile(&wire, "Example").unwrap();
    assert_eq!(
        (player.race, player.gender, player.level, player.mana),
        (4, 1, 12, 123)
    );
    assert_eq!(
        player.position,
        Position {
            x: -11.5,
            y: 22.5,
            z: 3.25,
            heading: 384.0
        }
    );
    assert_eq!(player.memorized_spells[..2], [Some(42), None]);
    assert!(profile(&wire, "Other").is_err());
    assert!(spawns(&wire).is_err());
    let mut broken = wire.clone();
    broken[3] ^= 128;
    assert!(profile(&broken, "Example").is_err());
    for len in 0..8 {
        assert!(profile(&wire[..len], "Example").is_err());
    }
}
#[test]
fn synthetic_spawn_batch_preserves_units_identity_and_visibility() {
    let spawns = spawns(&hex::decode(SPAWNS).unwrap()).unwrap();
    assert_eq!(spawns.len(), 2);
    assert_eq!(spawns[0].spawn_id, 7);
    assert_eq!(spawns[0].kind, SpawnKind::Player);
    assert_eq!(spawns[1].kind, SpawnKind::Npc);
    assert!(!spawns[0].invisible && spawns[1].invisible);
    assert_eq!(
        spawns[0].position,
        Position {
            x: 123.0,
            y: -210.0,
            z: -32.5,
            heading: 384.0
        }
    );
}
#[test]
fn updates_reject_partial_batches_and_preserve_negative_positions() {
    let mut update = [0; 15];
    update[..2].copy_from_slice(&7u16.to_le_bytes());
    update[3] = 255;
    update[5..7].copy_from_slice(&(-20i16).to_le_bytes());
    update[7..9].copy_from_slice(&42i16.to_le_bytes());
    update[9..11].copy_from_slice(&(-123i16).to_le_bytes());
    let expected = WorldEvent::Position {
        spawn_id: 7,
        position: Position {
            x: 42.0,
            y: -20.0,
            z: -12.3,
            heading: 510.0,
        },
        velocity: [0.0; 3],
    };
    assert_eq!(updates(0xf340, &update).unwrap(), vec![expected.clone()]);
    let mut batch = 2u32.to_le_bytes().to_vec();
    batch.extend(update);
    batch.extend(update);
    assert_eq!(
        updates(0x9f40, &batch).unwrap(),
        vec![expected.clone(), expected]
    );
    batch.pop();
    assert!(updates(0x9f40, &batch).is_err());
    assert!(updates(0x9f40, &u32::MAX.to_le_bytes()).is_err());
    assert!(updates(0xf340, &[0; 15]).is_err());
    assert!(
        updates(0x1234, &[]).unwrap().is_empty(),
        "an unknown opcode decodes to nothing"
    );
    assert_eq!(
        updates(0x2940, &7u16.to_le_bytes()).unwrap(),
        vec![WorldEvent::Despawn(7)]
    );
}
#[test]
fn own_entry_uses_corrected_position_and_id_comes_only_from_announcement() {
    let mut entry = [0; 356];
    entry[5..12].copy_from_slice(b"Example");
    entry[80..84].copy_from_slice(&42.0f32.to_le_bytes());
    entry[88..92].copy_from_slice(&128.0f32.to_le_bytes());
    assert_eq!(
        own_spawn(&entry, "Example").unwrap().position,
        Position {
            x: 42.0,
            y: 0.0,
            z: 0.0,
            heading: 256.0
        }
    );
    assert!(own_spawn(&entry, "Other").is_err());
    entry[244..248].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(own_spawn(&entry, "Example").is_err());
    assert_eq!(assigned_id(&[0, 0, 16, 0, 7, 0, 0, 0]).unwrap(), Some(7));
    assert_eq!(assigned_id(&[0, 0, 0, 1, 7, 0, 4, 0]).unwrap(), None);
    assert!(assigned_id(&[0, 0, 16, 0, 0, 0, 0, 0]).is_err());
}
#[test]
fn health_updates_provide_absolute_and_percentage_values() {
    let mut body = 7u32.to_le_bytes().to_vec();
    body.extend(75i32.to_le_bytes());
    body.extend(150i32.to_le_bytes());
    assert_eq!(
        updates(0xb240, &body).unwrap(),
        vec![
            WorldEvent::HitPoints {
                spawn_id: 7,
                current: 75,
                maximum: 150,
                items: ItemHitPoints::LeftOutOfCurrent,
            },
            WorldEvent::HealthPercent {
                spawn_id: 7,
                percent: 50
            }
        ]
    );
    body[8..12].fill(0);
    assert!(updates(0xb240, &body).is_err());
    for (op, len) in [(0xf340, 15), (0xb240, 12), (0x9941, 4), (0x2940, 2)] {
        for n in 0..len {
            assert!(updates(op, &vec![0; n]).is_err());
        }
    }
}

#[test]
fn visibility_and_mana_use_eqmac_fields_without_inventing_endurance() {
    assert_eq!(
        updates(0xf540, &[7, 0, 3, 0, 1, 0, 0, 0]).unwrap(),
        vec![WorldEvent::Visibility {
            spawn_id: 7,
            invisible: true
        }]
    );
    assert_eq!(
        updates(0xf540, &[7, 0, 3, 0, 0, 0, 0, 0]).unwrap(),
        vec![WorldEvent::Visibility {
            spawn_id: 7,
            invisible: false
        }]
    );
    assert_eq!(
        updates(0x7f41, &[123, 0, 255, 255]).unwrap(),
        vec![WorldEvent::Mana(123)]
    );
    assert!(profile(&hex::decode(PROFILE).unwrap(), "Example")
        .unwrap()
        .endurance
        .is_none());
}

#[test]
fn dll_version_ignores_other_features_responses_and_malformed_messages() {
    let request = [0, 0, 0, 1, 0, 0, 4, 0];
    for length in 0..8 {
        assert!(dll_version_reply(&request[..length]).is_none());
    }
    let mut oversized = request.to_vec();
    oversized.push(0);
    assert!(dll_version_reply(&oversized).is_none());
    for (offset, value) in [
        (0, 1),
        (2, 1),
        (3, 0),
        (6, 2),
        (6, 3),
        (6, 5),
        (6, 7),
        (6, 255),
        (7, 128),
    ] {
        let mut other = request;
        other[offset] = value;
        assert!(dll_version_reply(&other).is_none());
    }
    let mut arbitrary_value = request;
    arbitrary_value[4..6].fill(255);
    assert_eq!(
        dll_version_reply(&arbitrary_value),
        Some([0, 0, 0, 1, 7, 0, 4, 128])
    );
}

#[test]
fn camping_logging_out_and_a_stance_are_eqmacs_own_packets() {
    use crate::command::Posture;
    assert_eq!(camp().opcode, 0x0742);
    assert_eq!(camp().body.len(), 0);
    assert_eq!((logout().opcode, logout().body.len()), (ZONE_LOGOUT, 0));
    let sitting = posture(7, Posture::Sitting).unwrap();
    assert_eq!(sitting.opcode, ZONE_SPAWN_APPEARANCE);
    assert_eq!(sitting.body, [7, 0, 14, 0, 110, 0, 0, 0]);
    assert!(posture(0, Posture::Standing).is_err());
}

#[test]
fn a_position_update_lays_out_what_takp_reads() {
    use crate::movement::PositionPacket;
    let sample = PositionPacket {
        spawn_id: 7,
        sequence: 3,
        position: Position {
            x: -12.6,
            y: 456.9,
            z: 3.75,
            heading: 300.0,
        },
        delta: [-1.5, 2.25, -0.5],
        animation: 24,
        delta_heading: -2,
    };
    let packet = client_update(&sample).unwrap();
    assert_eq!(packet.opcode, ZONE_CLIENT_UPDATE);
    let body = packet.body;
    assert_eq!(body.len(), 15);
    assert_eq!(&body[..5], &[7, 0, 24, 150, 0xfe]);
    // y, x and z in tenths, each cut toward zero.
    assert_eq!(i16::from_le_bytes([body[5], body[6]]), 456);
    assert_eq!(i16::from_le_bytes([body[7], body[8]]), -12);
    assert_eq!(i16::from_le_bytes([body[9], body[10]]), 37);
    // TAKP unpacks the velocity: x from bits 22 to 31, z from 11 to 21 and
    // y from 0 to 10, each a signed count of sixteenths.
    let value = u32::from_le_bytes(body[11..15].try_into().unwrap());
    let signed = |bits: u32, width: u32| {
        let value = i32::try_from(bits).unwrap();
        let value = if bits & (1 << (width - 1)) == 0 {
            value
        } else {
            value - (1 << width)
        };
        f32::from(i16::try_from(value).unwrap()) / 16.0
    };
    assert!((signed(value >> 22, 10) + 1.5).abs() < f32::EPSILON);
    assert!((signed((value >> 11) & 0x7ff, 11) + 0.5).abs() < f32::EPSILON);
    assert!((signed(value & 0x7ff, 11) - 2.25).abs() < f32::EPSILON);
    // A turn and speed beyond a byte are held at its ends.
    let fast = PositionPacket {
        animation: 300,
        delta_heading: -300,
        ..sample
    };
    assert_eq!(&client_update(&fast).unwrap().body[2..5], &[127, 150, 0x80]);
    assert!(client_update(&PositionPacket {
        spawn_id: 0,
        ..sample
    })
    .is_err());
}

#[test]
fn the_client_answers_only_version_checks_by_itself() {
    let request = [0, 0, 0, 1, 0, 0, 4, 0];
    let reply = answer(ZONE_SPAWN_APPEARANCE, &request).unwrap();
    assert_eq!(reply.opcode, ZONE_SPAWN_APPEARANCE);
    assert_eq!(reply.body, dll_version_message(true));
    // Its own reply, another appearance or another opcode needs no answer.
    assert!(answer(ZONE_SPAWN_APPEARANCE, &dll_version_message(true)).is_none());
    assert!(answer(ZONE_SPAWN_APPEARANCE, &[7, 0, 16, 0, 7, 0, 0, 0]).is_none());
    assert!(answer(ZONE_WEATHER, &request).is_none());
}

#[test]
fn a_zone_request_names_the_zone_place_and_reason() {
    let mut body = [0; 24];
    body[..4].copy_from_slice(&2u32.to_le_bytes());
    body[4..8].copy_from_slice(&(-162.0f32).to_le_bytes());
    body[8..12].copy_from_slice(&(-259.0f32).to_le_bytes());
    body[12..16].copy_from_slice(&3.75f32.to_le_bytes());
    body[16..20].copy_from_slice(&64.0f32.to_le_bytes());
    body[20..].copy_from_slice(&11u32.to_le_bytes());
    let offer = zone_request(&body).unwrap();
    assert_eq!((offer.zone_id, offer.instance_id, offer.reason), (2, 0, 11));
    assert_eq!(
        offer.position,
        Position {
            x: -259.0,
            y: -162.0,
            z: 3.75,
            heading: 64.0
        }
    );
    assert!(offer.solicited && !offer.to_bind);
    assert!(zone_request(&body[..23]).is_err());
    body[..4].copy_from_slice(&70_000u32.to_le_bytes());
    assert!(zone_request(&body).is_err());
}

#[test]
fn mac_filters_enable_every_chat_and_combat_category() {
    let filters = server_filters();
    for index in 0..17 {
        let value = u32::from_le_bytes(filters[index * 4..index * 4 + 4].try_into().unwrap());
        assert_eq!(value, u32::from((5..=14).contains(&index)));
    }
}

#[test]
fn zoning_asks_with_the_76_byte_request_and_reads_the_answer() {
    let request = zone_change("Example", 2, 0).unwrap();
    assert_eq!((request.opcode, request.body.len()), (ZONE_CHANGE, 76));
    assert_eq!(&request.body[..8], b"Example\0");
    assert_eq!(request.body[64..68], 2u32.to_le_bytes());
    assert_eq!(request.body[68..76], [0; 8]);
    assert!(zone_change("", 2, 0).is_err());
    let mut answer = request.body;
    answer[72..76].copy_from_slice(&1i32.to_le_bytes());
    let read = zone_answer(&answer).unwrap();
    assert_eq!(
        (read.character.as_str(), read.zone_id, read.instance_id),
        ("Example", 2, 0)
    );
    assert_eq!((read.position, read.success), (None, 1));
    assert!(zone_answer(&answer[..75]).is_err());
    answer[64..68].copy_from_slice(&70_000u32.to_le_bytes());
    assert!(zone_answer(&answer).is_err());
}

#[test]
fn departing_saves_in_192_bytes_and_names_the_spawn_in_16_bits() {
    assert_eq!(
        (save_on_zone().opcode, save_on_zone().body),
        (ZONE_SAVE_ON_ZONE, vec![0; 192])
    );
    assert_eq!(
        (depart(0x0107).opcode, depart(0x0107).body),
        (ZONE_DEPART, vec![7, 1])
    );
}

#[test]
fn the_zone_points_and_answer_are_read_as_messages() {
    use crate::message::{eqmac, Message};
    // One destination, then TAKP's unused final record; the unused field
    // is no instance.
    let mut points = vec![0; 4 + 2 * 24];
    points[..4].copy_from_slice(&1u32.to_le_bytes());
    points[4..8].copy_from_slice(&3u32.to_le_bytes());
    points[24..26].copy_from_slice(&4u16.to_le_bytes());
    points[26..28].copy_from_slice(&9u16.to_le_bytes());
    let read = eqmac(ZONE_POINTS, &points);
    let [Message::ZonePoints(table)] = read.as_slice() else {
        panic!("{read:?}");
    };
    let point = table.get(3).unwrap();
    assert_eq!((point.zone_id, point.instance_id), (4, 0));
    let mut answer = zone_change("Example", 4, 0).unwrap().body;
    answer[72..76].copy_from_slice(&(-1i32).to_le_bytes());
    assert!(matches!(
        eqmac(ZONE_CHANGE, &answer).as_slice(),
        [Message::ZoneAnswer(read)] if read.success == -1
    ));
    assert!(matches!(
        eqmac(ZONE_CHANGE, &answer[..10]).as_slice(),
        [Message::Unreadable { .. }]
    ));
}

#[test]
fn a_death_names_who_died_their_killer_and_corpse() {
    let mut body = vec![0; 20];
    body[..2].copy_from_slice(&7u16.to_le_bytes());
    body[2..4].copy_from_slice(&9u16.to_le_bytes());
    body[4..6].copy_from_slice(&7u16.to_le_bytes());
    let read = death(&body).unwrap();
    assert_eq!((read.spawn_id, read.killer_id, read.corpse_id), (7, 9, 7));
    assert_eq!(read.bind_zone_id, 0);
    assert!(matches!(
        updates(ZONE_DEATH, &body).unwrap().as_slice(),
        [WorldEvent::Death(_)]
    ));
    assert!(death(&body[..19]).is_err());
    body[..2].fill(0);
    assert!(death(&body).is_err());
}

#[test]
fn the_profile_names_the_first_bind_point() {
    let mut data = vec![0; PROFILE_SIZE];
    data[3784..3788].copy_from_slice(&2u32.to_le_bytes());
    // A second bind point, which is not the one the player goes home to.
    data[3788..3792].copy_from_slice(&4u32.to_le_bytes());
    for (offset, value) in [(3804, 428.0f32), (3824, -74.0), (3844, 3.75), (3864, 128.0)] {
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let bind = decoded_bind(&data).unwrap();
    assert_eq!(bind.zone_id, 2);
    assert_eq!(
        (
            bind.position.x,
            bind.position.y,
            bind.position.z,
            bind.position.heading
        ),
        (-74.0, 428.0, 3.75, 128.0)
    );
    let home = bind.offer();
    assert_eq!((home.zone_id, home.reason, home.to_bind), (2, 10, true));
    data[3784..3788].copy_from_slice(&70_000u32.to_le_bytes());
    assert!(decoded_bind(&data).is_err());
    assert!(decoded_bind(&data[1..]).is_err());
}
