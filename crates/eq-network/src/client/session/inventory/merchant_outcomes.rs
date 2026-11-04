//! Production-path sequences for unanswered merchant trades and late results.
use super::{
    tests::{admitted, inventory_events, shopping_at},
    *,
};
use crate::client::session::feature::testing::{self, item, Outcome};

fn sell(slot: i32) -> ClientCommand {
    ClientCommand::Sell {
        session_id: 5,
        merchant_id: 9,
        own_id: 7,
        slot,
        quantity: 1,
        created: Instant::now(),
    }
}

fn buy(slot: u32) -> ClientCommand {
    ClientCommand::Buy {
        session_id: 5,
        merchant_id: 9,
        own_id: 7,
        slot,
        quantity: 1,
        created: Instant::now(),
    }
}

fn hear(
    belongings: &mut Belongings,
    world: &mut World,
    update: MerchantUpdate,
) -> Outcome<Result<()>> {
    let mut news = Message::Event(WorldEvent::Merchant(update));
    belongings.explain(&mut news, world);
    testing::run(|out| belongings.observe(&news, world, out))
}

fn expire(belongings: &mut Belongings, world: &mut World) -> Outcome<Result<()>> {
    testing::run(|out| belongings.tick(Instant::now() + Duration::from_secs(3), world, out))
}

fn sent(belongings: &mut Belongings, world: &mut World, command: &ClientCommand) {
    let output = testing::run(|out| belongings.handle(command, world, out));
    output.result.unwrap();
    assert_eq!(output.sent.len(), 1, "events: {:?}", output.events);
}

fn sold(slot: i32, quantity: u32, price: u32) -> MerchantUpdate {
    MerchantUpdate::Sold {
        slot,
        quantity,
        price,
    }
}

#[test]
fn timeout_drops_purse_knowledge_without_changing_items_or_resending() {
    for command in [buy(3), sell(22)] {
        let (mut belongings, mut world) = admitted();
        shopping_at(&mut belongings, &mut world);
        let items = world.inventory.items().clone();
        assert_eq!(world.coins.purse, Some(Coins::from_copper(1000)));
        sent(&mut belongings, &mut world, &command);
        let output = expire(&mut belongings, &mut world);
        output.result.unwrap();
        assert!(matches!(
            &output.events[..],
            [ClientEvent::World(WorldEvent::PurseUncertain { session_id: 5 }), ClientEvent::World(WorldEvent::MerchantUnanswered { session_id: 5, reason }), ClientEvent::Diagnostic(_)]
                if reason.contains("unknown")
        ));
        assert!(
            output.sent.is_empty(),
            "unexpected packets: {:?}",
            output.sent
        );
        assert!(
            belongings.holds(&world, Instant::now()).is_empty(),
            "unreleased holds: {:?}",
            belongings.holds(&world, Instant::now())
        );
        assert_eq!(world.inventory.items(), &items);
        assert_eq!(world.coins.purse, None);
        assert!(!world.inventory.stale());

        let repeated = expire(&mut belongings, &mut world);
        repeated.result.unwrap();
        assert!(
            repeated.events.is_empty(),
            "unexpected events: {:?}",
            repeated.events
        );
        assert!(
            repeated.sent.is_empty(),
            "unexpected packets: {:?}",
            repeated.sent
        );
        let retry = testing::run(|out| belongings.handle(&command, &mut world, out));
        retry.result.unwrap();
        assert!(
            retry.sent.is_empty(),
            "an ambiguous retry reached transport"
        );
        assert!(matches!(
            &retry.events[..],
            [ClientEvent::World(WorldEvent::MerchantRefused { reason, .. }), ClientEvent::Diagnostic(_)]
                if reason.contains("unanswered")
        ));
    }
}

#[test]
fn purchase_and_sale_results_use_only_the_original_known_coin_basis() {
    for (purchase, unanswered) in [(false, false), (true, false), (false, true), (true, true)] {
        let (mut belongings, mut world) = admitted();
        shopping_at(&mut belongings, &mut world);
        let items = world.inventory.items().clone();
        let command = if purchase { buy(3) } else { sell(22) };
        sent(&mut belongings, &mut world, &command);
        if unanswered {
            expire(&mut belongings, &mut world).result.unwrap();
        }
        let result = if purchase {
            MerchantUpdate::Bought {
                slot: 3,
                quantity: 1,
                price: 40,
            }
        } else {
            sold(22, 1, 40)
        };
        let output = hear(&mut belongings, &mut world, result.clone());
        output.result.unwrap();
        assert!(
            output.sent.is_empty(),
            "unexpected packets: {:?}",
            output.sent
        );
        assert_eq!(
            world.coins.purse,
            (!unanswered).then(|| Coins::from_copper(if purchase { 960 } else { 1040 }))
        );
        if purchase {
            assert_eq!(
                world.inventory.items(),
                &items,
                "an echo is not purchased item delivery"
            );
        } else {
            assert!(!world.inventory.items().contains_key(&InventorySlot(22)));
        }
        let after = world.inventory.items().clone();
        let duplicate = hear(&mut belongings, &mut world, result);
        duplicate.result.unwrap();
        assert!(
            duplicate.sent.is_empty(),
            "unexpected packets: {:?}",
            duplicate.sent
        );
        assert_eq!(
            world.coins.purse, None,
            "an unmatched duplicate cannot establish a current purse"
        );
        assert_eq!(world.inventory.items(), &after);
        if !purchase {
            assert_eq!(
                inventory_events(&duplicate.events),
                [&InventoryUpdate::Invalidated]
            );
        }
        let snapshot = Message::Event(WorldEvent::Coins(Coins::from_copper(987)));
        testing::run(|out| belongings.observe(&snapshot, &mut world, out))
            .result
            .unwrap();
        assert_eq!(world.coins.purse, Some(Coins::from_copper(987)));
    }
}

#[test]
fn distinct_unanswered_sales_survive_out_of_order_results_and_an_uncorrelated_refusal() {
    let (mut belongings, mut world) = admitted();
    shopping_at(&mut belongings, &mut world);
    let items = Message::Event(WorldEvent::Inventory(InventoryUpdate::Set(vec![
        item(23),
        item(24),
    ])));
    testing::run(|out| belongings.observe(&items, &mut world, out))
        .result
        .unwrap();
    for slot in [22, 23] {
        sent(&mut belongings, &mut world, &sell(slot));
        expire(&mut belongings, &mut world).result.unwrap();
    }
    sent(&mut belongings, &mut world, &sell(24));
    let refused = hear(&mut belongings, &mut world, sold(30, 0, 999));
    refused.result.unwrap();
    assert!(!refused.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::MerchantRefused { .. })
    )));
    assert_eq!(world.coins.purse, None);
    assert!(
        belongings.trades.active(),
        "the refusal identifies no unique trade"
    );
    expire(&mut belongings, &mut world).result.unwrap();
    for (slot, price) in [(23, 60), (22, 40)] {
        let output = hear(&mut belongings, &mut world, sold(slot, 1, price));
        output.result.unwrap();
        assert!(
            output.sent.is_empty(),
            "unexpected packets: {:?}",
            output.sent
        );
        assert!(!world.inventory.items().contains_key(&InventorySlot(slot)));
        assert!(!world.inventory.stale());
    }
    assert!(world.inventory.items().contains_key(&InventorySlot(24)));
    assert_eq!(world.coins.purse, None);
    // Only sale 24 remains, so a later refusal now has a unique owner.
    let last = hear(&mut belongings, &mut world, sold(30, 0, 999));
    last.result.unwrap();
    assert!(last.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::MerchantRefused { .. })
    )));
    assert_eq!(world.coins.purse, None);
    assert!(belongings.trades.check(&sell(24)).is_ok());
}

#[test]
fn an_uncorrelated_refusal_cannot_reject_a_newer_same_kind_trade() {
    for refusal in [sold(30, 0, 999), sold(-1, 1, 999)] {
        let (mut belongings, mut world) = admitted();
        shopping_at(&mut belongings, &mut world);
        let second = Message::Event(WorldEvent::Inventory(InventoryUpdate::Set(vec![item(23)])));
        testing::run(|out| belongings.observe(&second, &mut world, out))
            .result
            .unwrap();
        sent(&mut belongings, &mut world, &sell(22));
        expire(&mut belongings, &mut world).result.unwrap();
        sent(&mut belongings, &mut world, &sell(23));
        let ambiguous = hear(&mut belongings, &mut world, refusal.clone());
        ambiguous.result.unwrap();
        assert!(
            ambiguous.events.is_empty(),
            "unexpected events: {:?}",
            ambiguous.events
        );
        assert!(belongings.trades.active());
        hear(&mut belongings, &mut world, sold(22, 1, 40))
            .result
            .unwrap();
        assert!(belongings.trades.active());
        assert!(!world.inventory.items().contains_key(&InventorySlot(22)));
        assert!(world.inventory.items().contains_key(&InventorySlot(23)));
        let refusal = hear(&mut belongings, &mut world, refusal);
        refusal.result.unwrap();
        assert!(refusal.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::MerchantRefused { .. })
        )));
        assert!(!belongings.trades.active());
        assert_eq!(world.coins.purse, None);
        assert!(world.inventory.items().contains_key(&InventorySlot(23)));
    }
}

#[test]
fn a_replacement_cannot_be_sold_again_while_the_old_slot_result_is_unanswered() {
    let (mut belongings, mut world) = admitted();
    shopping_at(&mut belongings, &mut world);
    sent(&mut belongings, &mut world, &sell(22));
    expire(&mut belongings, &mut world).result.unwrap();
    let mut replacement = item(22);
    replacement.details.id += 1;
    let arrived = Message::Event(WorldEvent::Inventory(InventoryUpdate::Set(vec![
        replacement.clone(),
    ])));
    testing::run(|out| belongings.observe(&arrived, &mut world, out))
        .result
        .unwrap();
    let retry = testing::run(|out| belongings.handle(&sell(22), &mut world, out));
    retry.result.unwrap();
    assert!(
        retry.sent.is_empty(),
        "unexpected packets: {:?}",
        retry.sent
    );
    let late = hear(&mut belongings, &mut world, sold(22, 1, 40));
    late.result.unwrap();
    assert_eq!(
        inventory_events(&late.events),
        [&InventoryUpdate::Invalidated]
    );
    assert_eq!(world.inventory.items()[&InventorySlot(22)], replacement);
    assert_eq!(world.coins.purse, None);
    assert!(late.sent.is_empty(), "unexpected packets: {:?}", late.sent);
}

#[test]
fn explicit_refusals_leave_no_item_or_coin_authority_for_a_later_positive_echo() {
    for refusal in [sold(30, 0, 999), sold(-1, 1, 999)] {
        let (mut belongings, mut world) = admitted();
        shopping_at(&mut belongings, &mut world);
        let before = world.inventory.items().clone();
        sent(&mut belongings, &mut world, &sell(22));
        let output = hear(&mut belongings, &mut world, refusal);
        output.result.unwrap();
        assert!(output.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::MerchantRefused { .. })
        )));
        assert_eq!(world.coins.purse, Some(Coins::from_copper(1000)));
        assert_eq!(world.inventory.items(), &before);
        assert!(!world.inventory.stale());
        assert!(
            belongings.holds(&world, Instant::now()).is_empty(),
            "unreleased holds: {:?}",
            belongings.holds(&world, Instant::now())
        );
        assert!(belongings.trades.check(&sell(22)).is_ok());
        let contradictory = hear(&mut belongings, &mut world, sold(22, 1, 40));
        contradictory.result.unwrap();
        assert_eq!(world.coins.purse, None);
        assert_eq!(world.inventory.items(), &before);
        assert!(world.inventory.stale());
    }
}

#[test]
fn an_old_late_sale_leaves_a_new_purchase_held_and_preserves_unknown_coins() {
    let (mut belongings, mut world) = admitted();
    shopping_at(&mut belongings, &mut world);
    sent(&mut belongings, &mut world, &sell(22));
    expire(&mut belongings, &mut world).result.unwrap();
    let fresh = Message::Event(WorldEvent::Coins(Coins::from_copper(1000)));
    testing::run(|out| belongings.observe(&fresh, &mut world, out))
        .result
        .unwrap();
    sent(&mut belongings, &mut world, &buy(3));
    hear(&mut belongings, &mut world, sold(22, 1, 40))
        .result
        .unwrap();
    assert!(
        belongings.trades.active(),
        "the newer purchase lost its hold"
    );
    assert_eq!(world.coins.purse, None);
    let before = world.inventory.items().clone();
    let bought = MerchantUpdate::Bought {
        slot: 3,
        quantity: 1,
        price: 25,
    };
    hear(&mut belongings, &mut world, bought.clone())
        .result
        .unwrap();
    assert!(!belongings.trades.active());
    assert_eq!(world.coins.purse, None);
    assert_eq!(world.inventory.items(), &before);
    let fresh = Message::Event(WorldEvent::Coins(Coins::from_copper(1015)));
    testing::run(|out| belongings.observe(&fresh, &mut world, out))
        .result
        .unwrap();
    sent(&mut belongings, &mut world, &buy(3));
    let unmatched = hear(
        &mut belongings,
        &mut world,
        MerchantUpdate::Bought {
            slot: 4,
            quantity: 1,
            price: 500,
        },
    );
    unmatched.result.unwrap();
    assert!(belongings.trades.active());
    assert_eq!(world.coins.purse, None);
    let wrong_kind = hear(&mut belongings, &mut world, sold(30, 0, 500));
    wrong_kind.result.unwrap();
    assert!(belongings.trades.active());
    assert_eq!(world.coins.purse, None);
}

#[test]
fn a_new_admission_discards_old_waits_and_late_trade_authority() {
    let (mut belongings, mut world) = admitted();
    shopping_at(&mut belongings, &mut world);
    sent(&mut belongings, &mut world, &sell(22));
    expire(&mut belongings, &mut world).result.unwrap();
    let fresh = Message::Event(WorldEvent::Coins(Coins::from_copper(1000)));
    testing::run(|out| belongings.observe(&fresh, &mut world, out))
        .result
        .unwrap();
    sent(&mut belongings, &mut world, &buy(3));
    world.session_id = 6;
    belongings
        .admit(
            &Message::Event(WorldEvent::Coins(Coins::from_copper(900))),
            &mut world,
        )
        .unwrap();
    testing::run(|out| belongings.admitted(&mut world, out))
        .result
        .unwrap();
    let output = expire(&mut belongings, &mut world);
    output.result.unwrap();
    assert!(
        output.events.is_empty(),
        "unexpected events: {:?}",
        output.events
    );
    assert!(
        output.sent.is_empty(),
        "unexpected packets: {:?}",
        output.sent
    );
    assert!(
        belongings.holds(&world, Instant::now()).is_empty(),
        "unreleased holds: {:?}",
        belongings.holds(&world, Instant::now())
    );
    assert!(belongings.trades.check(&sell(22)).is_ok());
    assert_eq!(world.coins.purse, Some(Coins::from_copper(900)));
    let before = world.inventory.items().clone();
    let late = hear(&mut belongings, &mut world, sold(22, 1, 40));
    late.result.unwrap();
    assert_eq!(world.inventory.items(), &before);
    assert!(world.inventory.stale());
    assert_eq!(world.coins.purse, None);
}

#[test]
fn closing_and_reopening_cannot_clear_an_unanswered_trade_or_restore_the_purse() {
    let (mut belongings, mut world) = admitted();
    world.spawns.insert(testing::spawn(9, SpawnKind::Npc));
    shopping_at(&mut belongings, &mut world);
    sent(&mut belongings, &mut world, &buy(3));
    let close = ClientCommand::Shop {
        session_id: 5,
        merchant_id: 9,
        own_id: 7,
        open: false,
        created: Instant::now(),
    };
    sent(&mut belongings, &mut world, &close);
    assert!(
        belongings.trades.active(),
        "sending close established no outcome"
    );
    hear(&mut belongings, &mut world, MerchantUpdate::Closed)
        .result
        .unwrap();
    assert!(!belongings.trades.active());
    assert_eq!(world.coins.purse, None);
    hear(
        &mut belongings,
        &mut world,
        MerchantUpdate::Opened {
            merchant_id: 9,
            accepted: true,
            rate: 1.0,
        },
    )
    .result
    .unwrap();
    assert_eq!(world.coins.purse, None);
    let retry = testing::run(|out| belongings.handle(&buy(3), &mut world, out));
    retry.result.unwrap();
    assert!(
        retry.sent.is_empty(),
        "unexpected packets: {:?}",
        retry.sent
    );
    let fresh = Message::Event(WorldEvent::Coins(Coins::from_copper(975)));
    testing::run(|out| belongings.observe(&fresh, &mut world, out))
        .result
        .unwrap();
    assert!(
        belongings.trades.check(&buy(3)).is_err(),
        "a purse snapshot is not the trade's outcome"
    );
    hear(
        &mut belongings,
        &mut world,
        MerchantUpdate::Bought {
            slot: 3,
            quantity: 1,
            price: 25,
        },
    )
    .result
    .unwrap();
    assert!(belongings.trades.check(&buy(3)).is_ok());
    assert_eq!(
        world.coins.purse, None,
        "the snapshot may already include the late effect"
    );
    testing::run(|out| belongings.observe(&fresh, &mut world, out))
        .result
        .unwrap();
    assert_eq!(world.coins.purse, Some(Coins::from_copper(975)));
}

#[test]
fn an_absolute_snapshot_before_a_result_cannot_be_charged_again() {
    for purchase in [false, true] {
        let (mut belongings, mut world) = admitted();
        shopping_at(&mut belongings, &mut world);
        sent(
            &mut belongings,
            &mut world,
            &if purchase { buy(3) } else { sell(22) },
        );
        let coins = Coins::from_copper(if purchase { 960 } else { 1040 });
        let fresh = Message::Event(WorldEvent::Coins(coins));
        testing::run(|out| belongings.observe(&fresh, &mut world, out))
            .result
            .unwrap();
        assert_eq!(world.coins.purse, Some(coins));
        let result = if purchase {
            MerchantUpdate::Bought {
                slot: 3,
                quantity: 1,
                price: 40,
            }
        } else {
            sold(22, 1, 40)
        };
        let output = hear(&mut belongings, &mut world, result);
        output.result.unwrap();
        assert_eq!(
            world.coins.purse, None,
            "the result cannot attribute an effect already in the snapshot"
        );
        assert!(output.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::PurseUncertain { session_id: 5 })
        )));
        assert!(!belongings.trades.active());
        testing::run(|out| belongings.observe(&fresh, &mut world, out))
            .result
            .unwrap();
        assert_eq!(world.coins.purse, Some(coins));
    }
}
