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
    buy_at(9, slot)
}

fn buy_at(merchant_id: u16, slot: u32) -> ClientCommand {
    ClientCommand::Buy {
        session_id: 5,
        merchant_id,
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
    hear_message(
        belongings,
        world,
        Message::Event(WorldEvent::Merchant(update)),
    )
}

fn hear_message(
    belongings: &mut Belongings,
    world: &mut World,
    mut news: Message,
) -> Outcome<Result<()>> {
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
    assert!(matches!(&last.events[..], [ClientEvent::Diagnostic(_)]));
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

#[test]
fn titanium_results_and_removals_keep_their_wire_merchant_identity() {
    use eq_network_game::{merchant, message};
    let (mut belongings, mut world) = admitted();
    shopping_at(&mut belongings, &mut world);

    let mut removal = [0; 12];
    removal[..4].copy_from_slice(&8_u32.to_le_bytes());
    removal[8..].copy_from_slice(&3_u32.to_le_bytes());
    let mut news = message::titanium(merchant::DELETE_OPCODE, &removal).remove(0);
    belongings.explain(&mut news, &world);
    assert!(matches!(news, Message::Withheld));

    sent(&mut belongings, &mut world, &buy(3));
    let accepted = |merchant_id| {
        let mut body = merchant::buy(merchant_id, 7, 3, 1).unwrap();
        body[20..].copy_from_slice(&25_u32.to_le_bytes());
        message::titanium(merchant::BUY_OPCODE, &body).remove(0)
    };
    let mut stale = accepted(8);
    belongings.explain(&mut stale, &world);
    assert!(matches!(stale, Message::Withheld));
    assert!(belongings.trades.active());
    assert_eq!(world.coins.purse, Some(Coins::from_copper(1000)));

    let mut matching = accepted(9);
    belongings.explain(&mut matching, &world);
    assert!(matches!(
        matching,
        Message::Event(WorldEvent::Merchant(MerchantUpdate::Bought { .. }))
    ));
    testing::run(|out| belongings.observe(&matching, &mut world, out))
        .result
        .unwrap();
    assert!(!belongings.trades.active());
    assert_eq!(
        world.coins.purse.map(|coins| coins.total_copper()),
        Some(975)
    );
}

#[test]
fn merchant_commands_require_the_current_shop_and_available_quantity() {
    let (mut belongings, mut world) = admitted();
    shopping_at(&mut belongings, &mut world);
    let mut wrong_shop = buy(3);
    if let ClientCommand::Buy { merchant_id, .. } = &mut wrong_shop {
        *merchant_id = 8;
    }
    let mut overbuy = buy(3);
    if let ClientCommand::Buy { quantity, .. } = &mut overbuy {
        *quantity = 2;
    }
    let mut oversell = sell(22);
    if let ClientCommand::Sell { quantity, .. } = &mut oversell {
        *quantity = 2;
    }
    for command in [wrong_shop, overbuy, oversell, sell(24)] {
        let result = testing::run(|out| belongings.handle(&command, &mut world, out));
        result.result.unwrap();
        assert_eq!(
            result.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );
        assert!(matches!(
            result.events.first(),
            Some(ClientEvent::World(WorldEvent::MerchantRefused { .. }))
        ));
        assert!(!belongings.trades.active());
    }
    sent(&mut belongings, &mut world, &sell(22));
}

#[test]
fn closing_a_shop_reports_unknown_and_preserves_the_late_sale() {
    let (mut belongings, mut world) = admitted();
    shopping_at(&mut belongings, &mut world);
    sent(&mut belongings, &mut world, &sell(22));
    let close = hear(&mut belongings, &mut world, MerchantUpdate::Closed);
    close.result.unwrap();
    assert_eq!(
        close.sent,
        [] as [eq_network_game::command::EncodedCommand; 0]
    );
    assert!(close.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::MerchantUnanswered { .. })
    )));
    assert!(world.inventory.items().contains_key(&InventorySlot(22)));
    assert_eq!(world.coins.purse, None);
    assert!(!belongings.trades.active());

    let mut body = eq_network_game::merchant::sell(9, 22, 1).unwrap();
    body[12..].copy_from_slice(&40_u32.to_le_bytes());
    let mut reply =
        eq_network_game::message::titanium(eq_network_game::merchant::SELL_OPCODE, &body).remove(0);
    belongings.explain(&mut reply, &world);
    testing::run(|out| belongings.observe(&reply, &mut world, out))
        .result
        .unwrap();
    assert!(!world.inventory.items().contains_key(&InventorySlot(22)));
    assert_eq!(world.coins.purse, None);
}

#[test]
fn a_bounded_stack_purchase_and_partial_sale_settle_independently() {
    let (mut belongings, mut world) = admitted();
    shopping_at(&mut belongings, &mut world);
    let mut stack = item(3);
    stack.stack_count = Some(4);
    stack.rules.stack_size = 20;
    let mut catalog_row = Message::Event(WorldEvent::Merchant(MerchantUpdate::Item(Box::new(
        eq_network_game::merchant::MerchantItem {
            slot: 3,
            price: 25,
            quantity: 4,
            item: stack.clone(),
        },
    ))));
    belongings.explain(&mut catalog_row, &world);
    let mut purchase = buy(3);
    if let ClientCommand::Buy { quantity, .. } = &mut purchase {
        *quantity = 2;
    }
    sent(&mut belongings, &mut world, &purchase);
    hear(
        &mut belongings,
        &mut world,
        MerchantUpdate::Bought {
            slot: 3,
            quantity: 2,
            price: 50,
        },
    )
    .result
    .unwrap();
    assert!(!world.inventory.items().contains_key(&InventorySlot(3)));
    assert_eq!(
        world.coins.purse.map(|coins| coins.total_copper()),
        Some(950)
    );

    stack.slot = InventorySlot(22);
    let arrival = Message::Event(WorldEvent::Inventory(InventoryUpdate::Set(vec![stack])));
    testing::run(|out| belongings.observe(&arrival, &mut world, out))
        .result
        .unwrap();
    sent(&mut belongings, &mut world, &sell(22));
    hear(&mut belongings, &mut world, sold(22, 1, 18))
        .result
        .unwrap();
    assert_eq!(
        world.inventory.items()[&InventorySlot(22)].stack_count,
        Some(3)
    );
    assert_eq!(
        world.coins.purse.map(|coins| coins.total_copper()),
        Some(968)
    );
}

fn opened_at(merchant_id: u16) -> MerchantUpdate {
    MerchantUpdate::Opened {
        merchant_id,
        accepted: true,
        rate: 1.0,
    }
}

fn stock(slot: u32) -> MerchantUpdate {
    MerchantUpdate::Item(Box::new(eq_network_game::merchant::MerchantItem {
        slot,
        price: 25,
        quantity: 0,
        item: item(22),
    }))
}

fn wire_refusal(merchant_id: u16) -> Message {
    let mut packet = [0; 24];
    packet[..4].copy_from_slice(&u32::from(merchant_id).to_le_bytes());
    eq_network_game::message::titanium(eq_network_game::merchant::BUY_OPCODE, &packet).remove(0)
}

fn refreshed_purse(belongings: &mut Belongings, world: &mut World) {
    hear_message(
        belongings,
        world,
        Message::Event(WorldEvent::Coins(Coins::from_copper(1000))),
    )
    .result
    .unwrap();
}

#[test]
fn a_titanium_refusal_uses_the_wire_merchant_to_select_one_purchase() {
    for (current, refused) in [(8, 8), (8, 9), (9, 9)] {
        let (mut belongings, mut world) = admitted();
        shopping_at(&mut belongings, &mut world);
        sent(&mut belongings, &mut world, &buy(3));
        hear(&mut belongings, &mut world, MerchantUpdate::Closed)
            .result
            .unwrap();
        refreshed_purse(&mut belongings, &mut world);
        world.spawns.insert(testing::spawn(current, SpawnKind::Npc));
        hear(&mut belongings, &mut world, opened_at(current))
            .result
            .unwrap();
        hear(&mut belongings, &mut world, stock(4)).result.unwrap();
        sent(&mut belongings, &mut world, &buy_at(current, 4));

        let items = world.inventory.items().clone();
        let ambiguous = hear_message(&mut belongings, &mut world, wire_refusal(0));
        ambiguous.result.unwrap();
        assert!(ambiguous.events.is_empty());
        assert!(belongings.trades.active());
        assert!(belongings.trades.check(&buy(3)).is_err());

        let reply = hear_message(&mut belongings, &mut world, wire_refusal(refused));
        reply.result.unwrap();
        let current_refused = current == 8 && refused == 8;
        assert_eq!(belongings.trades.active(), !current_refused);
        assert_eq!(
            belongings.trades.check(&buy(3)).is_ok(),
            current == 8 && refused == 9,
            "only a uniquely identified refusal may retire the old purchase"
        );
        assert_eq!(
            reply.events.iter().any(|event| matches!(
                event,
                ClientEvent::World(WorldEvent::MerchantRefused { .. })
            )),
            current_refused,
            "the host should only close the current action for its own refusal"
        );
        assert_eq!(
            reply.events.iter().any(|event| matches!(
                event,
                ClientEvent::World(WorldEvent::Merchant(MerchantUpdate::Bought {
                    quantity: 0,
                    ..
                }))
            )),
            current_refused
        );
        assert_eq!(
            reply.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );
        assert_eq!(world.inventory.items(), &items);
        assert_eq!(world.coins.purse, Some(Coins::from_copper(1000)));
    }
}

#[test]
fn a_merchant_spawn_replacement_requires_a_fresh_window_and_stock() {
    use crate::client::session::entities::Entities;
    use eq_network_game::GameDialect;

    for despawn_first in [true, false] {
        let (mut belongings, mut world) = admitted();
        shopping_at(&mut belongings, &mut world);
        hear(&mut belongings, &mut world, stock(4)).result.unwrap();
        let items = world.inventory.items().clone();
        sent(&mut belongings, &mut world, &buy(3));

        let mut entities = Entities::new(GameDialect::Titanium);
        let mut replacement = testing::spawn(9, SpawnKind::Npc);
        replacement.name = "Replacement merchant".into();
        replacement.class = Some(41);
        let spawns = WorldEvent::Spawns(vec![replacement]);
        let sequence = despawn_first
            .then_some(WorldEvent::Despawn(9))
            .into_iter()
            .chain([spawns]);
        let retired = testing::run(|out| -> Result<()> {
            for event in sequence {
                let mut news = Message::Event(event);
                belongings.explain(&mut news, &world);
                belongings.observe(&news, &mut world, out)?;
                entities.observe(&news, &mut world, out)?;
            }
            Ok(())
        });
        retired.result.unwrap();
        assert_eq!(
            retired.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );
        assert!(matches!(
            &retired.events[..],
            [
                ClientEvent::World(WorldEvent::PurseUncertain { .. }),
                ClientEvent::World(WorldEvent::MerchantUnanswered { .. }),
                ClientEvent::Diagnostic(_),
                ClientEvent::World(WorldEvent::Merchant(MerchantUpdate::Closed))
            ]
        ));
        assert!(!belongings.trades.active());
        assert!(belongings.trades.check(&buy(3)).is_err());
        assert_eq!(world.inventory.items(), &items);
        assert_eq!(world.coins.purse, None);
        assert!(world.spawns.visible(9).is_some());

        refreshed_purse(&mut belongings, &mut world);
        // A late stock row has no open catalog to populate.
        hear(&mut belongings, &mut world, stock(4)).result.unwrap();
        for command in [buy(4), sell(22)] {
            let stale = testing::run(|out| belongings.handle(&command, &mut world, out));
            stale.result.unwrap();
            assert_eq!(
                stale.sent,
                [] as [eq_network_game::command::EncodedCommand; 0]
            );
            assert!(matches!(
                stale.events.first(),
                Some(ClientEvent::World(WorldEvent::MerchantRefused { reason, .. }))
                    if reason.contains("window")
            ));
        }

        let open = ClientCommand::Shop {
            session_id: 5,
            own_id: 7,
            merchant_id: 9,
            open: true,
            created: Instant::now(),
        };
        sent(&mut belongings, &mut world, &open);
        hear(&mut belongings, &mut world, opened_at(9))
            .result
            .unwrap();
        let unlisted = testing::run(|out| belongings.handle(&buy(4), &mut world, out));
        unlisted.result.unwrap();
        assert_eq!(
            unlisted.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );
        hear(&mut belongings, &mut world, stock(4)).result.unwrap();
        sent(&mut belongings, &mut world, &buy(4));

        // The old result remains separate from the new window and its hold.
        let mut late = eq_network_game::merchant::buy(9, 7, 3, 1).unwrap();
        late[20..].copy_from_slice(&25_u32.to_le_bytes());
        let late = eq_network_game::message::titanium(eq_network_game::merchant::BUY_OPCODE, &late)
            .remove(0);
        let result = hear_message(&mut belongings, &mut world, late);
        result.result.unwrap();
        assert!(belongings.trades.check(&buy(3)).is_ok());
        assert!(belongings.trades.active());
        assert_eq!(world.coins.purse, None);
        assert_eq!(world.inventory.items(), &items);
    }
}

#[test]
fn a_late_buy_refusal_does_not_resolve_an_unanswered_sale_at_the_reopened_shop() {
    for close in [
        WorldEvent::Merchant(MerchantUpdate::Closed),
        WorldEvent::Despawn(9),
    ] {
        let (mut belongings, mut world) = admitted();
        shopping_at(&mut belongings, &mut world);
        sent(&mut belongings, &mut world, &buy(3));
        hear_message(&mut belongings, &mut world, Message::Event(close))
            .result
            .unwrap();
        shopping_at(&mut belongings, &mut world);
        sent(&mut belongings, &mut world, &sell(22));
        let timeout = expire(&mut belongings, &mut world);
        timeout.result.unwrap();
        assert!(timeout.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::MerchantUnanswered { .. })
        )));
        assert!(!belongings.trades.active());
        assert!(belongings.trades.check(&buy(3)).is_err());
        assert!(belongings.trades.check(&sell(22)).is_err());

        let items = world.inventory.items().clone();
        let late = hear_message(&mut belongings, &mut world, wire_refusal(9));
        late.result.unwrap();
        assert_eq!(
            late.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );
        assert!(
            matches!(&late.events[..], [ClientEvent::Diagnostic(_)]),
            "a generic refusal or terminal merchant event can dismiss the unknown sale"
        );
        assert!(belongings.trades.check(&buy(3)).is_ok());
        assert!(belongings.trades.check(&sell(22)).is_err());
        assert_eq!(world.inventory.items(), &items);
        assert_eq!(world.coins.purse, None);
    }
}
