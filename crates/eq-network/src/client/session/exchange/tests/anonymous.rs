//! The captured unnamed NPC cancellation is terminal, not an item-ID receipt.
use super::*;
use eq_network_game::inventory::{Inventory, InventorySlot, InventoryUpdate};

fn cancellation() -> Message {
    let update = exchange::decode(exchange::CANCEL_OPCODE, &[0, 0, 0, 0, 1, 0, 0, 0])
        .unwrap()
        .unwrap();
    news(update)
}

#[test]
fn p99_submitted_npc_cancellation_retires_only_escrow_and_never_guesses_returns() {
    for returned_before in [0, 1, 2] {
        let mut feature = Exchanges::project1999();
        let mut world = giving_to_npc();
        world.exchange.0.as_mut().unwrap().stage = Stage::Accepted;
        let mut offered = testing::item(3000);
        offered.rules.ground_drop_serial = std::num::NonZeroU32::new(1);
        let mut second = offered.clone();
        second.slot = InventorySlot(3001);
        second.rules.ground_drop_serial = std::num::NonZeroU32::new(2);
        let mut third = offered.clone();
        third.slot = InventorySlot(3002);
        third.rules.ground_drop_serial = std::num::NonZeroU32::new(3);
        let mut separate = offered.clone();
        separate.slot = InventorySlot(27);
        separate.rules.ground_drop_serial = std::num::NonZeroU32::new(99);
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![
            offered.clone(),
            second,
            third,
            separate.clone(),
        ]));
        let mut returned = offered.clone();
        returned.slot = InventorySlot(23);
        let mut distinct = offered;
        distinct.slot = InventorySlot(24);
        distinct.rules.ground_drop_serial = std::num::NonZeroU32::new(77);
        if returned_before >= 1 {
            inventory.apply(InventoryUpdate::Set(vec![returned.clone()]));
        }
        if returned_before == 2 {
            inventory.apply(InventoryUpdate::Set(vec![distinct.clone()]));
        }
        world.inventory = inventory.into();
        let mut message = cancellation();
        feature.explain(&mut message, &world);
        assert!(matches!(
            message,
            Message::Event(WorldEvent::Exchange(ExchangeUpdate::Cancelled { by: 0 }))
        ));
        let outcome = testing::run(|out| feature.observe(&message, &mut world, out));
        outcome.result.unwrap();
        assert!(world.exchange.is_none());
        assert_eq!(
            outcome.sent,
            [],
            "the NPC cancellation sends no second cancel"
        );
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Inventory(InventoryUpdate::TradeEmptied))
        )));
        assert!(!world
            .inventory
            .items()
            .keys()
            .any(|slot| slot.is_in_trade()));
        assert_eq!(world.inventory.items()[&separate.slot], separate);
        assert_eq!(
            world.inventory.items().contains_key(&returned.slot),
            returned_before >= 1
        );
        if returned_before >= 1 {
            assert_eq!(world.inventory.items()[&returned.slot], returned);
        }
        if returned_before == 2 {
            assert_eq!(world.inventory.items()[&distinct.slot], distinct);
        }
        // A return after cancellation remains an independent received record.
        if returned_before == 0 {
            let mut replica = (*world.inventory).clone();
            replica.apply(InventoryUpdate::Set(vec![returned.clone()]));
            assert_eq!(replica.items()[&returned.slot], returned);
            assert_eq!(replica.items()[&separate.slot], separate);
            assert!(!replica.items().keys().any(|slot| slot.is_in_trade()));
        }
    }
}

#[test]
fn anonymous_cancel_stays_unattributed_without_a_submitted_p99_npc_give() {
    for partner in [Partner::Npc, Partner::Player] {
        for stage in [Stage::Asked(Instant::now()), Stage::Open, Stage::Accepted] {
            for p99 in [false, true] {
                let mut feature = if p99 {
                    Exchanges::project1999()
                } else {
                    Exchanges::default()
                };
                let mut world = beside_npc();
                world.exchange.0 = Some(Exchange {
                    with: NPC,
                    partner,
                    stage,
                });
                let mut message = cancellation();
                feature.explain(&mut message, &world);
                assert_eq!(
                    matches!(message, Message::Event(_)),
                    p99 && partner == Partner::Npc && stage == Stage::Accepted
                );
            }
        }
    }
    let mut world = beside_npc();
    let mut feature = Exchanges::project1999();
    let mut message = cancellation();
    feature.explain(&mut message, &world);
    assert!(matches!(message, Message::Withheld));
    let outcome = testing::run(|out| feature.observe(&message, &mut world, out));
    outcome.result.unwrap();
    assert_eq!(outcome.events.len(), 0);
    assert_eq!(outcome.sent, []);
}

#[test]
fn local_cancel_then_new_give_can_receive_its_later_legitimate_cancellation() {
    let mut feature = Exchanges::project1999();
    let mut world = giving_to_npc();
    world.exchange.0.as_mut().unwrap().stage = Stage::Accepted;
    testing::run(|out| {
        feature.handle(
            &ClientCommand::CancelTrade { session_id: 5 },
            &mut world,
            out,
        )
    })
    .result
    .unwrap();
    assert!(world.exchange.is_none());
    // A late terminal when no exchange is active is ignored, not remembered
    // as an admission-wide lockout. Reliable delivery keeps record order.
    let mut late = cancellation();
    feature.explain(&mut late, &world);
    assert!(matches!(late, Message::Withheld));
    let outcome = testing::run(|out| feature.handle(&offer(NPC), &mut world, out));
    outcome.result.unwrap();
    let mut opened = news(ExchangeUpdate::Opened {
        with: u32::from(NPC),
    });
    feature.explain(&mut opened, &world);
    testing::run(|out| feature.observe(&opened, &mut world, out))
        .result
        .unwrap();
    assert_eq!(world.exchange.unwrap().stage, Stage::Open);
    testing::run(|out| {
        feature.handle(
            &ClientCommand::AcceptTrade {
                session_id: 5,
                created: Instant::now(),
            },
            &mut world,
            out,
        )
    })
    .result
    .unwrap();
    let mut terminal = cancellation();
    feature.explain(&mut terminal, &world);
    assert!(matches!(terminal, Message::Event(_)));
    testing::run(|out| feature.observe(&terminal, &mut world, out))
        .result
        .unwrap();
    assert!(world.exchange.is_none());
}
