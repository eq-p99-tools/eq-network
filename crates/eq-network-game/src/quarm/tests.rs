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
                without_items: false,
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
