//! Bodies written by our own writers, from the server's structs.
use super::*;
use crate::{
    abilities::{AskAnswer, AskOutcome},
    message::{rof2, Message, Part},
    money::Coin,
    world::titanium_update,
};

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn begging_body(result: u32, amount: u32) -> Vec<u8> {
    words(&[0, 0, 0, result, amount])
}

fn pick_body(kind: u32, coin: u32, item: &str) -> Vec<u8> {
    let mut body = words(&[77, 4321, 150, kind, coin]);
    let mut name = vec![0; 64];
    name[..item.len()].copy_from_slice(item.as_bytes());
    body.extend(name);
    body
}

/// Every message under its `RoF2` number, with Titanium's number for it.
fn messages() -> [(u16, u16, Vec<u8>); 8] {
    [
        (
            TARGET_OPCODE,
            crate::targets::TARGET_COMMAND_OPCODE,
            words(&[77]),
        ),
        (
            TARGET_OF_TARGET_OPCODE,
            crate::targets::TARGET_OF_TARGET_OPCODE,
            words(&[553]),
        ),
        (
            CHARM_OPCODE,
            crate::control::CHARM_OPCODE,
            words(&[4321, 77, 1]),
        ),
        (
            CASH_REWARD_OPCODE,
            crate::rewards::CASH_REWARD_OPCODE,
            words(&[4, 3, 2, 1]),
        ),
        (
            LINKED_REUSE_OPCODE,
            crate::disciplines::LINKED_REUSE_OPCODE,
            words(&[7, 5030, 5000]),
        ),
        (
            BEGGING_OPCODE,
            crate::abilities::BEGGING_OPCODE,
            begging_body(3, 7),
        ),
        (
            PICK_POCKET_OPCODE,
            crate::abilities::PICK_POCKET_OPCODE,
            pick_body(5, 0, "Test Dagger"),
        ),
        (
            BIND_WOUND_OPCODE,
            crate::bind_wound::OPCODE,
            words(&[77, 1]),
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
    // Each begging and pick pocket result and bandage type, as Titanium's.
    for result in [0, 1, 2, 3, 4, 5, 6, 0xffff_ffff] {
        let body = begging_body(result, 9);
        assert_eq!(
            read(BEGGING_OPCODE, &body).unwrap(),
            titanium_update(crate::abilities::BEGGING_OPCODE, &body).unwrap()
        );
        let body = pick_body(result, 9, "Test Dagger");
        assert_eq!(
            read(PICK_POCKET_OPCODE, &body).unwrap(),
            titanium_update(crate::abilities::PICK_POCKET_OPCODE, &body).unwrap()
        );
    }
    for kind in 0..10 {
        let body = words(&[77, kind]);
        let ours = read(BIND_WOUND_OPCODE, &body);
        let titaniums = titanium_update(crate::bind_wound::OPCODE, &body);
        assert_eq!(ours.is_ok(), titaniums.is_ok(), "{kind}");
        if let (Ok(ours), Ok(titaniums)) = (ours, titaniums) {
            assert_eq!(ours, titaniums, "{kind}");
        }
    }
}

#[test]
fn each_message_reads_each_field() {
    assert_eq!(
        read(TARGET_OPCODE, &words(&[77])).unwrap(),
        Some(WorldEvent::Target(Some(77)))
    );
    assert_eq!(
        read(TARGET_OPCODE, &words(&[0])).unwrap(),
        Some(WorldEvent::Target(None))
    );
    assert_eq!(
        read(TARGET_OF_TARGET_OPCODE, &words(&[553])).unwrap(),
        Some(WorldEvent::TargetOfTarget(Some(553)))
    );
    let Some(WorldEvent::Charm(charm)) = read(CHARM_OPCODE, &words(&[4321, 77, 1])).unwrap() else {
        panic!("charm")
    };
    assert_eq!((charm.owner_id, charm.pet_id), (Some(4321), 77));
    let Some(WorldEvent::Reward(reward)) = read(CASH_REWARD_OPCODE, &words(&[4, 3, 2, 1])).unwrap()
    else {
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
    let Some(WorldEvent::LinkedReuse(reuse)) =
        read(LINKED_REUSE_OPCODE, &words(&[7, 5030, 5000])).unwrap()
    else {
        panic!("reuse")
    };
    assert_eq!(
        (reuse.timer, reuse.remaining),
        (7, std::time::Duration::from_secs(30))
    );
    assert_eq!(
        read(BEGGING_OPCODE, &begging_body(3, 7)).unwrap(),
        Some(WorldEvent::AskAnswered(AskAnswer {
            ability: Ability::Begging,
            outcome: AskOutcome::Coins {
                coin: Coin::Silver,
                amount: 7,
            },
        }))
    );
    assert_eq!(
        read(PICK_POCKET_OPCODE, &pick_body(5, 0, "Test Dagger")).unwrap(),
        Some(WorldEvent::AskAnswered(AskAnswer {
            ability: Ability::PickPockets,
            outcome: AskOutcome::Item("Test Dagger".into()),
        }))
    );
    assert_eq!(
        read(BIND_WOUND_OPCODE, &words(&[77, 1])).unwrap(),
        Some(WorldEvent::BindWound(
            crate::bind_wound::BindWoundUpdate::Ended(crate::bind_wound::BindWoundEnd::Complete)
        ))
    );
}

#[test]
fn each_message_refuses_what_titaniums_refuses() {
    for opcode in [TARGET_OPCODE, TARGET_OF_TARGET_OPCODE] {
        assert!(read(opcode, &words(&[0x1_0000])).is_err());
    }
    assert!(read(CHARM_OPCODE, &words(&[0x1_0000, 77, 1])).is_err());
    assert!(read(CHARM_OPCODE, &words(&[4321, 0x1_0000, 1])).is_err());
    assert!(read(BIND_WOUND_OPCODE, &words(&[77, 2])).is_err());
    // Begging and pick pocket keep Titanium's floors: a cut refuses and
    // a longer answer reads.
    for (opcode, body) in [
        (BEGGING_OPCODE, begging_body(3, 7)),
        (PICK_POCKET_OPCODE, pick_body(5, 0, "Test Dagger")),
    ] {
        for length in 0..body.len() {
            assert!(read(opcode, &body[..length]).is_err(), "{length}");
        }
        assert!(read(opcode, &[body, vec![0]].concat()).is_ok());
    }
}

#[test]
fn a_refused_pick_pocket_answer_names_nothing() {
    let body = pick_body(5, 0, "Test Dagger");
    let cut = &body[..83];
    let error = read(PICK_POCKET_OPCODE, cut).unwrap_err();
    assert!(!format!("{error:#}").contains("Test Dagger"), "{error:#}");
    let messages = rof2(PICK_POCKET_OPCODE, cut);
    assert!(
        matches!(messages[..], [Message::Unreadable { .. }]),
        "{messages:?}"
    );
    assert!(!format!("{messages:?}").contains("Test Dagger"));
}

#[test]
fn every_exact_length_refuses_a_cut_and_a_trailing_byte() {
    for (opcode, _, body) in messages() {
        if [BEGGING_OPCODE, PICK_POCKET_OPCODE].contains(&opcode) {
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
