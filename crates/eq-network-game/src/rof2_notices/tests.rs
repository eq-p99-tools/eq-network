//! Bodies written by our own writers, from the server's structs.
use super::*;
use crate::{
    message::{rof2, Message, Part},
    world::titanium_update,
};

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn reward_body() -> Vec<u8> {
    let mut values = vec![77, 4321, 1500, 0, 0, 4, 3, 2, 1, 1001];
    values.extend([u32::MAX; 7]);
    words(&values)
}

fn rename_body(old: &str, new: &str) -> Vec<u8> {
    let mut body = vec![0; 200];
    body[..old.len()].copy_from_slice(old.as_bytes());
    body[64..64 + old.len()].copy_from_slice(old.as_bytes());
    body[128..128 + new.len()].copy_from_slice(new.as_bytes());
    body[196..200].copy_from_slice(&1u32.to_le_bytes());
    body
}

fn banker_body() -> Vec<u8> {
    words(&[1, 2, 3, 4, 10, 20, 30, 40])
}

/// Every message under its `RoF2` number, with Titanium's number for it.
fn messages() -> [(u16, u16, Vec<u8>); 5] {
    [
        (SOUND_OPCODE, crate::rewards::SOUND_OPCODE, reward_body()),
        (
            SOUND_OPCODE,
            crate::rewards::SOUND_OPCODE,
            crate::rewards::DING.to_vec(),
        ),
        (
            RENAME_OPCODE,
            crate::names::RENAME_OPCODE,
            rename_body("Testmob", "Testmob_corpse"),
        ),
        (
            NEW_TITLES_OPCODE,
            crate::names::NEW_TITLES_OPCODE,
            Vec::new(),
        ),
        (
            BANKER_CHANGE_OPCODE,
            crate::banker::CHANGE_OPCODE,
            banker_body(),
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
}

#[test]
fn each_message_reads_each_field() {
    let Some(WorldEvent::Reward(reward)) = read(SOUND_OPCODE, &reward_body()).unwrap() else {
        panic!("reward")
    };
    assert_eq!(
        (
            reward.coins.copper,
            reward.coins.silver,
            reward.coins.gold,
            reward.coins.platinum
        ),
        (4, 3, 2, 1)
    );
    assert_eq!(
        read(SOUND_OPCODE, &crate::rewards::DING).unwrap(),
        Some(WorldEvent::Ding)
    );
    assert_eq!(
        read(RENAME_OPCODE, &rename_body("Testmob", "Testmob_corpse")).unwrap(),
        Some(WorldEvent::Renamed {
            spawn_id: None,
            old_name: "Testmob".into(),
            new_name: "Testmob_corpse".into(),
        })
    );
    assert_eq!(
        read(NEW_TITLES_OPCODE, &[]).unwrap(),
        Some(WorldEvent::NewTitles)
    );
    let Some(WorldEvent::CoinsChanged(changed)) =
        read(BANKER_CHANGE_OPCODE, &banker_body()).unwrap()
    else {
        panic!("banker")
    };
    let coins =
        |coins: crate::world::Coins| (coins.platinum, coins.gold, coins.silver, coins.copper);
    assert_eq!(coins(changed.purse), (1, 2, 3, 4));
    assert_eq!(coins(changed.bank), (10, 20, 30, 40));
}

#[test]
fn a_refused_rename_names_no_one() {
    let body = rename_body("Testmob", "Testmob_corpse");
    for refused in [
        body[..199].to_vec(),
        [body.clone(), vec![0]].concat(),
        rename_body("Testmob", ""),
    ] {
        let error = read(RENAME_OPCODE, &refused).unwrap_err();
        assert!(!format!("{error:#}").contains("Testmob"), "{error:#}");
        let messages = rof2(RENAME_OPCODE, &refused);
        assert!(
            matches!(messages[..], [Message::Unreadable { .. }]),
            "{messages:?}"
        );
        assert!(!format!("{messages:?}").contains("Testmob"));
    }
}

#[test]
fn every_message_refuses_a_cut_and_a_trailing_byte() {
    for (opcode, _, body) in messages() {
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
