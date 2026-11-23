//! Bodies written by our own writers, from the server's structs.
use super::*;
use crate::{
    clock::Weather,
    doors::Door,
    message::{rof2, Message, Part},
};

/// One door's server values.
#[derive(Clone, Copy)]
struct DoorValues {
    id: u8,
    name: &'static str,
    /// y, x, z and heading, in the record's order.
    place: [f32; 4],
    incline: u32,
    size: u32,
    open_type: u8,
    state_at_spawn: u8,
    invert_state: u8,
    parameter: u32,
}

const DOOR: DoorValues = DoorValues {
    id: 7,
    name: "TESTDOOR01",
    place: [-12.5, 27.25, 9.0, 128.0],
    incline: 3,
    size: 75,
    open_type: 58,
    state_at_spawn: 1,
    invert_state: 1,
    parameter: 0xffff_ffff,
};

/// `ENCODE(OP_SpawnDoor)`'s record, with `filler` where it writes nothing
/// from the server: the four bytes at 56, the words at 68 to 80 and the
/// bytes from 87 it leaves unset.
fn door_record(door: DoorValues, filler: u8) -> Vec<u8> {
    let mut record = vec![filler; DOOR_RECORD];
    record[..32].fill(0);
    record[..door.name.len()].copy_from_slice(door.name.as_bytes());
    for (index, value) in door.place.iter().enumerate() {
        record[32 + 4 * index..36 + 4 * index].copy_from_slice(&value.to_le_bytes());
    }
    record[48..52].copy_from_slice(&door.incline.to_le_bytes());
    record[52..56].copy_from_slice(&door.size.to_le_bytes());
    record[60..64].copy_from_slice(&[
        door.id,
        door.open_type,
        door.state_at_spawn,
        door.invert_state,
    ]);
    record[64..68].copy_from_slice(&door.parameter.to_le_bytes());
    record[80..87].copy_from_slice(&[0, 1, 0, 1, 0, 0, 0]);
    record
}

/// Titanium's 80-byte record for the same server values.
fn titanium_door_record(door: DoorValues) -> Vec<u8> {
    let mut record = vec![0; 80];
    record[..door.name.len()].copy_from_slice(door.name.as_bytes());
    for (index, value) in door.place.iter().enumerate() {
        record[32 + 4 * index..36 + 4 * index].copy_from_slice(&value.to_le_bytes());
    }
    record[48..52].copy_from_slice(&door.incline.to_le_bytes());
    record[52..54].copy_from_slice(&u16::try_from(door.size).unwrap().to_le_bytes());
    record[60..64].copy_from_slice(&[
        door.id,
        door.open_type,
        door.state_at_spawn,
        door.invert_state,
    ]);
    record[64..68].copy_from_slice(&door.parameter.to_le_bytes());
    record
}

fn door_table(doors: &[DoorValues], filler: u8) -> Vec<u8> {
    doors
        .iter()
        .flat_map(|door| door_record(*door, filler))
        .collect()
}

/// One ground object's server values.
#[derive(Clone, Copy)]
struct ObjectValues {
    drop_id: u32,
    name: &'static str,
    /// heading, y, x and z.
    place: [f32; 4],
    object_type: u32,
}

const OBJECT: ObjectValues = ObjectValues {
    drop_id: 4321,
    name: "IT63_ACTORDEF",
    place: [256.0, -40.5, 120.25, 3.75],
    object_type: 1,
};

/// `ENCODE(OP_GroundSpawn)`'s body, with `second` as the second drop id
/// and `filler` in the 52 bytes it leaves unset.
fn ground_body(object: ObjectValues, second: u32, filler: u8) -> Vec<u8> {
    let [heading, y, x, z] = object.place;
    let floats = [heading, 0.5, -0.25, 0.75, y, x, z];
    [
        &object.drop_id.to_le_bytes()[..],
        object.name.as_bytes(),
        &[0],
        &54u16.to_le_bytes(),
        &3u16.to_le_bytes(),
        &second.to_le_bytes(),
        &0u32.to_le_bytes(),
        &floats
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>(),
        &object.object_type.to_le_bytes(),
        &[filler; 52],
    ]
    .concat()
}

/// Titanium's 92-byte `Object_Struct` for the same server values.
fn titanium_ground_body(object: ObjectValues) -> Vec<u8> {
    let [heading, y, x, z] = object.place;
    let mut body = vec![0; 92];
    body[12..16].copy_from_slice(&object.drop_id.to_le_bytes());
    for (offset, value) in [(28, heading), (32, z), (36, x), (40, y)] {
        body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    body[44..44 + object.name.len()].copy_from_slice(object.name.as_bytes());
    body[80..84].copy_from_slice(&object.object_type.to_le_bytes());
    body
}

fn click_body(drop_id: u32, player_id: u32) -> Vec<u8> {
    [drop_id.to_le_bytes(), player_id.to_le_bytes()].concat()
}

fn weather_body(kind: u32, intensity: u32) -> Vec<u8> {
    [kind.to_le_bytes(), intensity.to_le_bytes()].concat()
}

fn spawned(body: &[u8]) -> GroundObject {
    let ObjectUpdate::Spawn(object) = ground_object(body).unwrap() else {
        panic!("spawn")
    };
    object
}

#[test]
fn a_door_table_reads_each_field() {
    assert_eq!(
        doors(&door_record(DOOR, 0)).unwrap(),
        DoorUpdate::Spawn(vec![Door {
            id: 7,
            model: "TESTDOOR01".into(),
            position: Position {
                x: 27.25,
                y: -12.5,
                z: 9.0,
                heading: 128.0,
            },
            incline: 3,
            size: 75,
            open_type: 58,
            state_at_spawn: 1,
            invert_state: 1,
            parameter: 0xffff_ffff,
            action: None,
        }])
    );
    // A name filling its 32 bytes reads whole, as Titanium's does.
    let long = DoorValues {
        name: "ABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
        ..DOOR
    };
    let DoorUpdate::Spawn(read) = doors(&door_record(long, 0)).unwrap() else {
        panic!("spawn")
    };
    assert_eq!(read[0].model, long.name);
}

#[test]
fn a_door_table_reads_as_titaniums() {
    let second = DoorValues {
        id: 8,
        name: "TESTDOOR02",
        size: 100,
        parameter: 0,
        ..DOOR
    };
    let titanium: Vec<u8> = [DOOR, second]
        .iter()
        .flat_map(|door| titanium_door_record(*door))
        .collect();
    assert_eq!(
        doors(&door_table(&[DOOR, second], 0)).unwrap(),
        crate::doors::decode(0x4c24, &titanium).unwrap().unwrap()
    );
    assert_eq!(
        read(MOVE_DOOR_OPCODE, &[7, 2]).unwrap(),
        Some(WorldEvent::Doors(
            crate::doors::decode(0x700d, &[7, 2]).unwrap().unwrap()
        ))
    );
}

#[test]
fn a_door_tables_length_is_titaniums() {
    assert_eq!(doors(&[]).unwrap(), DoorUpdate::Spawn(Vec::new()));
    let most: Vec<DoorValues> = (0u32..500)
        .map(|index| DoorValues {
            id: u8::try_from(index).unwrap_or(255),
            ..DOOR
        })
        .collect();
    let DoorUpdate::Spawn(read) = doors(&door_table(&most, 0)).unwrap() else {
        panic!("spawn")
    };
    assert_eq!(read.len(), 500);
    let too_many = [most, vec![DoorValues { id: 255, ..DOOR }]].concat();
    assert!(doors(&door_table(&too_many, 0)).is_err());
    // Titanium's 80-byte record is a partial RoF2 record.
    assert!(doors(&titanium_door_record(DOOR)).is_err());
}

#[test]
fn a_door_table_refuses_a_repeated_id_but_the_static_objects() {
    assert!(doors(&door_table(&[DOOR, DOOR], 0)).is_err());
    let fixture = DoorValues { id: 255, ..DOOR };
    let DoorUpdate::Spawn(read) = doors(&door_table(&[fixture, fixture], 0)).unwrap() else {
        panic!("spawn")
    };
    assert_eq!(read.len(), 2);
}

#[test]
fn a_door_table_refuses_a_placement_not_finite_and_a_size_past_16_bits() {
    for index in 0..4 {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut door = DOOR;
            door.place[index] = value;
            assert!(doors(&door_record(door, 0)).is_err(), "{index}");
        }
    }
    let widest = DoorValues {
        size: u32::from(u16::MAX),
        ..DOOR
    };
    let DoorUpdate::Spawn(read) = doors(&door_record(widest, 0)).unwrap() else {
        panic!("spawn")
    };
    assert_eq!(read[0].size, u16::MAX);
    for size in [0x1_0000, u32::MAX] {
        assert!(doors(&door_record(DoorValues { size, ..DOOR }, 0)).is_err());
    }
}

#[test]
fn a_door_records_constants_and_unset_bytes_change_nothing() {
    let plain = doors(&door_record(DOOR, 0)).unwrap();
    for filler in [0x5a, 0xff] {
        let mut record = door_record(DOOR, filler);
        assert_eq!(doors(&record).unwrap(), plain);
        record[80..87].copy_from_slice(&[filler; 7]);
        assert_eq!(doors(&record).unwrap(), plain);
    }
}

#[test]
fn a_ground_object_reads_each_field() {
    let read = spawned(&ground_body(OBJECT, OBJECT.drop_id, 0));
    assert_eq!(
        read,
        GroundObject {
            drop_id: 4321,
            model: "IT63_ACTORDEF".into(),
            position: Position {
                x: 120.25,
                y: -40.5,
                z: 3.75,
                heading: 256.0,
            },
            object_type: 1,
        }
    );
    for name in ["A", "ABCDEFGHIJKLMNOPQRSTUVWXYZ01234"] {
        let object = ObjectValues { name, ..OBJECT };
        assert_eq!(spawned(&ground_body(object, object.drop_id, 0)).model, name);
    }
    // Titanium's text rule: a name with a space reads, a blank one does not.
    let spaced = ObjectValues {
        name: "TEST FORGE",
        ..OBJECT
    };
    assert_eq!(
        spawned(&ground_body(spaced, spaced.drop_id, 0)).model,
        "TEST FORGE"
    );
    for name in ["", " ", "IT\u{7f}"] {
        let object = ObjectValues { name, ..OBJECT };
        assert!(ground_object(&ground_body(object, object.drop_id, 0)).is_err());
    }
}

#[test]
fn a_ground_object_reads_as_titaniums() {
    for object in [
        OBJECT,
        ObjectValues {
            drop_id: u32::MAX,
            name: "FORGE",
            place: [0.0, 1.0, -1.0, 0.0],
            object_type: 15,
        },
    ] {
        assert_eq!(
            read(GROUND_SPAWN_OPCODE, &ground_body(object, object.drop_id, 0)).unwrap(),
            Some(WorldEvent::Objects(
                crate::objects::decode(0x0f47, &titanium_ground_body(object))
                    .unwrap()
                    .unwrap()
            ))
        );
    }
}

#[test]
fn a_ground_object_refuses_a_name_past_31_characters_or_unterminated() {
    let object = ObjectValues {
        name: "ABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
        ..OBJECT
    };
    assert!(ground_object(&ground_body(object, object.drop_id, 0)).is_err());
    let mut unterminated = OBJECT.drop_id.to_le_bytes().to_vec();
    unterminated.extend(std::iter::repeat_n(b'A', 40));
    assert!(ground_object(&unterminated).is_err());
}

#[test]
fn a_ground_objects_length_is_its_names_plus_101() {
    let body = ground_body(OBJECT, OBJECT.drop_id, 0);
    assert_eq!(body.len(), OBJECT.name.len() + 101);
    for length in 0..body.len() {
        assert!(ground_object(&body[..length]).is_err(), "{length}");
    }
    assert!(ground_object(&[body, vec![0]].concat()).is_err());
}

#[test]
fn a_ground_object_refuses_drop_ids_that_disagree_or_are_0() {
    for second in [0, 4320, 4322, u32::MAX] {
        assert!(ground_object(&ground_body(OBJECT, second, 0)).is_err());
    }
    let unnamed = ObjectValues {
        drop_id: 0,
        ..OBJECT
    };
    assert!(ground_object(&ground_body(unnamed, 0, 0)).is_err());
}

#[test]
fn a_ground_object_refuses_a_placement_not_finite() {
    for index in 0..4 {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut object = OBJECT;
            object.place[index] = value;
            assert!(
                ground_object(&ground_body(object, object.drop_id, 0)).is_err(),
                "{index}"
            );
        }
    }
}

#[test]
fn a_ground_objects_unset_tail_and_unread_fields_change_nothing() {
    let plain = ground_object(&ground_body(OBJECT, OBJECT.drop_id, 0)).unwrap();
    for filler in [0x5a, 0xff] {
        let mut body = ground_body(OBJECT, OBJECT.drop_id, filler);
        assert_eq!(ground_object(&body).unwrap(), plain);
        // The zone, the instance, the zero word, the tilts and the size.
        let after_name = 4 + OBJECT.name.len() + 1;
        for at in (0..4)
            .chain(8..12)
            .chain(16..28)
            .map(|offset| after_name + offset)
        {
            body[at] = filler;
        }
        assert_eq!(ground_object(&body).unwrap(), plain);
    }
}

#[test]
fn a_click_reads_as_titaniums() {
    for (drop_id, player_id) in [(4321, 77), (4321, 0), (0, 77)] {
        let body = click_body(drop_id, player_id);
        assert_eq!(
            read(CLICK_OBJECT_OPCODE, &body).unwrap(),
            Some(WorldEvent::Objects(
                crate::objects::decode(0x3bc2, &body).unwrap().unwrap()
            ))
        );
    }
    assert_eq!(
        read(CLICK_OBJECT_OPCODE, &click_body(4321, 77)).unwrap(),
        Some(WorldEvent::Objects(ObjectUpdate::Remove {
            drop_id: 4321,
            taken_by: Some(77),
        }))
    );
}

#[test]
fn the_weather_reads_as_titaniums() {
    for (kind, intensity, weather) in [
        (0, 5, Weather::Rain { intensity: 5 }),
        (1, 3, Weather::Snow { intensity: 3 }),
        (0, 0, Weather::Clear),
        (
            9,
            2,
            Weather::Unknown {
                kind: 9,
                intensity: 2,
            },
        ),
    ] {
        let body = weather_body(kind, intensity);
        assert_eq!(
            read(WEATHER_OPCODE, &body).unwrap(),
            Some(WorldEvent::Weather(weather))
        );
        assert_eq!(
            crate::clock::titanium_weather(crate::clock::TITANIUM_WEATHER_OPCODE, &body).unwrap(),
            Some(weather)
        );
    }
}

#[test]
fn the_zone_in_weather_gives_nothing_and_is_not_unreadable() {
    let zone_in = [weather_body(0, 5), vec![0; 4]].concat();
    assert_eq!(read(WEATHER_OPCODE, &zone_in).unwrap(), None);
    assert!(rof2(WEATHER_OPCODE, &zone_in).is_empty());
}

#[test]
fn every_message_refuses_a_cut_and_a_trailing_byte() {
    for (opcode, body) in [
        (SPAWN_DOOR_OPCODE, door_record(DOOR, 0)),
        (MOVE_DOOR_OPCODE, vec![7, 2]),
        (GROUND_SPAWN_OPCODE, ground_body(OBJECT, OBJECT.drop_id, 0)),
        (CLICK_OBJECT_OPCODE, click_body(4321, 77)),
        (WEATHER_OPCODE, weather_body(1, 3)),
    ] {
        for length in 1..body.len() {
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
    // An empty door table is a table of no doors; the others refuse it.
    for opcode in [
        MOVE_DOOR_OPCODE,
        GROUND_SPAWN_OPCODE,
        CLICK_OBJECT_OPCODE,
        WEATHER_OPCODE,
    ] {
        assert!(read(opcode, &[]).is_err(), "{opcode:#06x}");
    }
}

#[test]
fn no_other_generation_reads_these_numbers() {
    for opcode in [
        SPAWN_DOOR_OPCODE,
        MOVE_DOOR_OPCODE,
        GROUND_SPAWN_OPCODE,
        CLICK_OBJECT_OPCODE,
        WEATHER_OPCODE,
    ] {
        for body in [
            door_record(DOOR, 0),
            vec![7, 2],
            ground_body(OBJECT, OBJECT.drop_id, 0),
            click_body(4321, 77),
            weather_body(1, 3),
        ] {
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
    // Titanium's numbers say nothing here.
    for opcode in [0x4c24, 0x700d, 0x0f47, 0x3bc2, 0x254d] {
        assert_eq!(read(opcode, &weather_body(1, 3)).unwrap(), None);
    }
}

#[test]
fn the_zone_reads_each_message_as_the_worlds() {
    for (opcode, body) in [
        (SPAWN_DOOR_OPCODE, door_record(DOOR, 0)),
        (MOVE_DOOR_OPCODE, vec![7, 2]),
        (GROUND_SPAWN_OPCODE, ground_body(OBJECT, OBJECT.drop_id, 0)),
        (CLICK_OBJECT_OPCODE, click_body(4321, 77)),
        (WEATHER_OPCODE, weather_body(1, 3)),
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
