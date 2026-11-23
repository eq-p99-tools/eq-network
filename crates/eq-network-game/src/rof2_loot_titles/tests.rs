//! Bodies written by our own writers, from the server's structs.
use super::*;
use crate::{
    loot::{LootResponse, LootUpdate},
    message::{rof2, Message, Part},
    names::TitleChange,
    world::titanium_update,
};

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn money_body(response: u8) -> Vec<u8> {
    let mut body = vec![response, 0x42, 0xef, 0];
    body.extend(words(&[1, 2, 3, 4]));
    body
}

fn surname_body(name: &str, answer: u32, last_name: &str) -> Vec<u8> {
    let mut body = vec![0; 100];
    body[..name.len()].copy_from_slice(name.as_bytes());
    body[64..68].copy_from_slice(&answer.to_le_bytes());
    body[68..68 + last_name.len()].copy_from_slice(last_name.as_bytes());
    body
}

fn title_list_body(titles: &[(u32, &str, &str)]) -> Vec<u8> {
    let count = u32::try_from(titles.len()).unwrap();
    let mut body = count.to_le_bytes().to_vec();
    for (id, prefix, suffix) in titles {
        body.extend(id.to_le_bytes());
        body.extend(prefix.as_bytes());
        body.push(0);
        body.extend(suffix.as_bytes());
        body.push(0);
    }
    body
}

fn title_body(kind: u32, text: &str, spawn: u32) -> Vec<u8> {
    let mut body = kind.to_le_bytes().to_vec();
    let mut field = vec![0; 32];
    field[..text.len()].copy_from_slice(text.as_bytes());
    body.extend(field);
    body.extend(spawn.to_le_bytes());
    body
}

const COINS: crate::world::Coins = crate::world::Coins {
    platinum: 1,
    gold: 2,
    silver: 3,
    copper: 4,
};

/// Every message under its `RoF2` number, with Titanium's number for it.
fn messages() -> [(u16, u16, Vec<u8>); 5] {
    [
        (MONEY_OPCODE, crate::loot::MONEY_OPCODE, money_body(1)),
        (
            LOOT_COMPLETE_OPCODE,
            crate::loot::COMPLETE_OPCODE,
            Vec::new(),
        ),
        (
            SURNAME_OPCODE,
            crate::names::SURNAME_OPCODE,
            surname_body("Testplayer", 1, "Testsurname"),
        ),
        (
            TITLE_LIST_OPCODE,
            crate::names::TITLE_LIST_OPCODE,
            title_list_body(&[(3, "Testprefix", ""), (9, "", "Testsuffix")]),
        ),
        (
            TITLE_OPCODE,
            crate::names::TITLE_OPCODE,
            title_body(1, "Testsuffix", 77),
        ),
    ]
}

#[test]
fn each_message_reads_as_titaniums() {
    for (opcode, titanium_opcode, body) in messages() {
        let read = read(opcode, &body).unwrap();
        assert!(read.is_some(), "{opcode:#06x}");
        assert_eq!(
            read,
            titanium_update(titanium_opcode, &body).unwrap(),
            "{opcode:#06x}"
        );
    }
    // Every response Titanium's reader reads, but the Loot All marker.
    for response in (0..=u8::MAX).filter(|response| *response != LOOT_ALL) {
        let body = money_body(response);
        assert_eq!(
            read(MONEY_OPCODE, &body).unwrap(),
            titanium_update(crate::loot::MONEY_OPCODE, &body).unwrap(),
            "{response}"
        );
    }
    // The loot close reads any body as Titanium's does.
    assert_eq!(
        read(LOOT_COMPLETE_OPCODE, &[1, 2]).unwrap(),
        titanium_update(crate::loot::COMPLETE_OPCODE, &[1, 2]).unwrap()
    );
}

#[test]
fn each_message_reads_each_field() {
    for (response, read_as) in [(1, LootResponse::Normal), (5, LootResponse::TooFar)] {
        assert_eq!(
            read(MONEY_OPCODE, &money_body(response)).unwrap(),
            Some(WorldEvent::Loot(LootUpdate::Opened {
                response: read_as,
                coins: COINS,
            }))
        );
    }
    assert_eq!(
        read(LOOT_COMPLETE_OPCODE, &[]).unwrap(),
        Some(WorldEvent::Loot(LootUpdate::Closed))
    );
    for (answer, accepted) in [(0, false), (1, true)] {
        assert_eq!(
            read(
                SURNAME_OPCODE,
                &surname_body("Testplayer", answer, "Testsurname")
            )
            .unwrap(),
            Some(WorldEvent::SurnameAnswer {
                accepted,
                last_name: "Testsurname".into(),
            })
        );
    }
    let Some(WorldEvent::TitleList(titles)) = read(
        TITLE_LIST_OPCODE,
        &title_list_body(&[(3, "Testprefix", ""), (9, "", "Testsuffix")]),
    )
    .unwrap() else {
        panic!("title list")
    };
    let titles: Vec<_> = titles
        .iter()
        .map(|title| (title.id, title.prefix.as_str(), title.suffix.as_str()))
        .collect();
    assert_eq!(titles, [(3, "Testprefix", ""), (9, "", "Testsuffix")]);
    assert_eq!(
        read(TITLE_OPCODE, &title_body(0, "Testprefix", 77)).unwrap(),
        Some(WorldEvent::TitleChanged {
            spawn_id: 77,
            change: TitleChange::Title("Testprefix".into()),
        })
    );
    assert_eq!(
        read(TITLE_OPCODE, &title_body(1, "Testsuffix", 77)).unwrap(),
        Some(WorldEvent::TitleChanged {
            spawn_id: 77,
            change: TitleChange::Suffix("Testsuffix".into()),
        })
    );
}

#[test]
fn the_loot_all_marker_gives_no_event() {
    // Stock EQEmu's last record of a SoD and later listing: a part the
    // coverage row names as unread, whose event the loot listing batch
    // decides. Lifting validate waits on it.
    assert_eq!(read(MONEY_OPCODE, &money_body(LOOT_ALL)).unwrap(), None);
    assert!(rof2(MONEY_OPCODE, &money_body(LOOT_ALL)).is_empty());
    // Any other length is refused as Titanium's reader refuses it.
    assert!(read(MONEY_OPCODE, &money_body(LOOT_ALL)[..19]).is_err());
    // Titanium's reader is untouched: 6 reads there as it always has.
    assert!(matches!(
        titanium_update(crate::loot::MONEY_OPCODE, &money_body(LOOT_ALL)).unwrap(),
        Some(WorldEvent::Loot(LootUpdate::Opened {
            response: LootResponse::Normal,
            ..
        }))
    ));
}

#[test]
fn each_message_refuses_what_titaniums_refuses() {
    let fixed = [
        (MONEY_OPCODE, money_body(1)),
        (SURNAME_OPCODE, surname_body("Testplayer", 1, "Testsurname")),
        (TITLE_OPCODE, title_body(1, "Testsuffix", 77)),
    ];
    for (opcode, body) in fixed {
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
    // Any word but 0 is taken, as Titanium's program reads it (R49-10).
    assert!(read(
        SURNAME_OPCODE,
        &surname_body("Testplayer", 2, "Testsurname")
    )
    .is_ok());
    assert!(read(
        SURNAME_OPCODE,
        &surname_body("Testplayer", 1, &"a".repeat(32))
    )
    .is_err());
    assert!(read(TITLE_OPCODE, &title_body(2, "Testsuffix", 77)).is_err());
    assert!(read(TITLE_OPCODE, &title_body(1, &"a".repeat(32), 77)).is_err());
    for spawn in [0, 0x1_0000] {
        assert!(read(TITLE_OPCODE, &title_body(1, "Testsuffix", spawn)).is_err());
    }
    let list = title_list_body(&[(3, "Testprefix", "")]);
    for length in 0..list.len() {
        assert!(
            read(TITLE_LIST_OPCODE, &list[..length]).is_err(),
            "{length}"
        );
    }
    assert!(read(TITLE_LIST_OPCODE, &[list, vec![0]].concat()).is_err());
    assert!(read(
        TITLE_LIST_OPCODE,
        &title_list_body(&[(0, "Testprefix", "")])
    )
    .is_err());
}

#[test]
fn a_refused_answer_names_nothing() {
    let refused = [
        (
            SURNAME_OPCODE,
            surname_body("Testplayer", 1, "Testsurname")[..99].to_vec(),
        ),
        (
            TITLE_LIST_OPCODE,
            title_list_body(&[(3, "Testprefix", "Testsuffix")])[..20].to_vec(),
        ),
        (TITLE_OPCODE, title_body(1, "Testsuffix", 0)),
    ];
    for (opcode, body) in refused {
        let error = format!("{:#}", read(opcode, &body).unwrap_err());
        let messages = rof2(opcode, &body);
        assert!(
            matches!(messages[..], [Message::Unreadable { .. }]),
            "{messages:?}"
        );
        for text in ["Testplayer", "Testsurname", "Testprefix", "Testsuffix"] {
            assert!(!error.contains(text), "{error}");
            assert!(!format!("{messages:?}").contains(text));
        }
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
        // The loot close, like Titanium's, reads any body.
        if opcode == LOOT_COMPLETE_OPCODE {
            continue;
        }
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
