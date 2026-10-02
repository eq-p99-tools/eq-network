use super::*;
use crate::{inventory::ItemPlacement, items::ItemDetails};
use std::cell::Cell;

fn item(slot: i32, count: Option<u32>, bag: u8) -> InventoryItem {
    InventoryItem {
        activation: crate::inventory::ItemActivation::default(),
        scroll_spell: None,
        book: None,
        slot: InventorySlot(slot),
        icon: 0,
        stack_count: count,
        charges: 3,
        bag_slots: bag,
        rules: ItemPlacement {
            stack_size: 20,
            size: 1,
            bag_size: 4,
            item_type: 8,
            ..Default::default()
        },
        details: ItemDetails {
            equipment: None,
            bonuses: None,
            id: 42,
            name: "Synthetic item".into(),
            lore: String::new(),
            weight_tenths: 1,
            slots: (1 << 13) | (1 << 14),
            classes: 1,
            races: 1,
            flags: Vec::new(),
            stats: Vec::new(),
        },
    }
}
fn actor() -> InventoryActor {
    InventoryActor {
        bank_access: false,
        dual_wield: None,
        deity: None,
        class: Some(1),
        race: 1,
        level: 10,
        trade_slots: 0,
        world_container: false,
    }
}

#[test]
fn bank_moves_require_current_access_and_preserve_filled_bags() {
    let mut inventory = state(vec![item(2000, None, 2), item(2031, Some(7), 0)]);
    let action = request(&inventory, 2000, 30);
    let before = inventory.clone();
    let sent = Cell::new(false);
    assert!(inventory
        .submit_move(&action, actor(), |_| {
            sent.set(true);
            Ok(())
        })
        .is_err());
    assert!(!sent.get());
    assert_eq!(inventory, before);
    let banker = InventoryActor {
        bank_access: true,
        ..actor()
    };
    inventory
        .submit_move(&action, banker, |packet| {
            let body = &packet.body[..];
            assert_eq!(u32::from_le_bytes(body[..4].try_into().unwrap()), 2000);
            assert_eq!(u32::from_le_bytes(body[4..8].try_into().unwrap()), 30);
            Ok(())
        })
        .unwrap();
    assert_eq!(inventory.items[&InventorySlot(331)].stack_count, Some(7));
    assert!(!inventory.items.contains_key(&InventorySlot(2031)));
    // A queued deposit is denied if proximity has been lost before submission.
    let deposit = request(&inventory, 30, 2007);
    assert!(inventory.plan_move(&deposit, actor()).is_err());
    inventory.submit_move(&deposit, banker, |_| Ok(())).unwrap();
    assert_eq!(inventory.items[&InventorySlot(2101)].stack_count, Some(7));
    assert!(!inventory.items.contains_key(&InventorySlot(331)));
}

#[test]
fn bank_contents_obey_capacity_and_shared_bank_stays_unsupported() {
    let inventory = state(vec![
        item(30, Some(7), 0),
        item(2000, None, 2),
        item(2031, Some(18), 0),
    ]);
    let banker = InventoryActor {
        bank_access: true,
        ..actor()
    };
    let mut merge = request(&inventory, 30, 2031);
    merge.quantity = MoveQuantity::Count(NonZeroU32::new(2).unwrap());
    let update = inventory.plan_move(&merge, banker).unwrap();
    let mut merged = inventory.clone();
    merged.apply(update);
    assert_eq!(merged.items[&InventorySlot(2031)].stack_count, Some(20));
    assert_eq!(merged.items[&InventorySlot(30)].stack_count, Some(5));
    for to in [2033, 2111, 2008, 2500, 2531, -1] {
        assert!(inventory
            .plan_move(&request(&inventory, 30, to), banker)
            .is_err());
    }
    assert!(inventory
        .plan_move(&request(&inventory, 30, 2032), banker)
        .is_ok());
    assert!(inventory
        .plan_move(&request(&inventory, 30, 2031), actor())
        .is_err());
}

#[test]
fn a_world_container_takes_from_the_cursor_and_gives_back_whole_while_open() {
    let open = InventoryActor {
        world_container: true,
        ..actor()
    };
    // A stack on the cursor, a bag in a pack slot and a stack already in the
    // container.
    let inventory = state(vec![
        item(30, Some(7), 0),
        item(22, None, 2),
        item(4001, Some(2), 0),
    ]);
    // Nothing goes in while no container is open.
    assert!(inventory
        .plan_move(&request(&inventory, 30, 4000), actor())
        .is_err());
    // In from the cursor only.
    assert!(inventory
        .plan_move(&request(&inventory, 30, 4000), open)
        .is_ok());
    assert!(inventory
        .plan_move(&request(&inventory, 22, 4000), open)
        .is_err());
    // Out whole, onto an empty cursor.
    assert!(inventory
        .plan_move(&request(&inventory, 4001, 30), open)
        .is_err());
    let emptied = state(vec![item(4001, Some(2), 0)]);
    assert!(emptied
        .plan_move(&request(&emptied, 4001, 30), open)
        .is_ok());
    assert!(emptied
        .plan_move(&request(&emptied, 4001, 22), open)
        .is_err());
    // A bag never goes in.
    let bag = state(vec![item(30, None, 2)]);
    assert!(bag.plan_move(&request(&bag, 30, 4000), open).is_err());
}

#[test]
fn trade_slots_take_only_what_servers_accept_while_a_window_is_open() {
    let giving = InventoryActor {
        trade_slots: 4,
        ..actor()
    };
    // A stack on the cursor, a bag with something in it, and a stack already
    // handed over.
    let mut inventory = state(vec![
        item(30, Some(7), 0),
        item(22, None, 2),
        item(251, Some(3), 0),
        item(3001, Some(2), 0),
    ]);
    // Nothing goes into a trade slot without an open window, or past its
    // slots.
    assert!(inventory
        .plan_move(&request(&inventory, 30, 3000), actor())
        .is_err());
    assert!(inventory
        .plan_move(&request(&inventory, 30, 3004), giving)
        .is_err());
    // Only from the cursor: EQEmu disconnects anything else.
    assert!(inventory
        .plan_move(&request(&inventory, 251, 3000), giving)
        .is_err());
    // Whole into an empty slot, merged onto the same stack, and nothing else.
    assert!(inventory
        .plan_move(&request(&inventory, 30, 3001), giving)
        .is_err());
    let mut split = request(&inventory, 30, 3000);
    split.quantity = MoveQuantity::Count(NonZeroU32::new(2).unwrap());
    assert!(inventory.plan_move(&split, giving).is_err());
    let mut merge = request(&inventory, 30, 3001);
    merge.quantity = MoveQuantity::Count(NonZeroU32::new(7).unwrap());
    let mut merged = inventory.clone();
    merged.apply(merged.plan_move(&merge, giving).unwrap());
    assert_eq!(merged.items[&InventorySlot(3001)].stack_count, Some(9));
    // What is handed over stays there until the window closes.
    assert!(inventory
        .plan_move(&request(&inventory, 3001, 23), giving)
        .is_err());
    inventory
        .submit_move(&request(&inventory, 30, 3000), giving, |packet| {
            assert_eq!(packet.body[4..8], 3000u32.to_le_bytes());
            Ok(())
        })
        .unwrap();
    // A bag goes over with what it holds.
    inventory.apply(InventoryUpdate::Settled);
    inventory
        .submit_move(&request(&inventory, 22, 30), giving, |_| Ok(()))
        .unwrap();
    inventory
        .submit_move(&request(&inventory, 30, 3002), giving, |_| Ok(()))
        .unwrap();
    assert_eq!(
        inventory.items[&InventorySlot(3002).child(0).unwrap()].stack_count,
        Some(3)
    );
    assert_eq!(InventorySlot(3051).parent(), Some((InventorySlot(3002), 0)));
    // Closing the window empties every trade slot and resolves their
    // predictions; the rest of the inventory is untouched.
    inventory.apply(InventoryUpdate::TradeEmptied);
    assert!(!inventory.items.keys().any(|slot| slot.is_in_trade()));
    assert!(inventory
        .prediction_origins()
        .all(|(slot, _)| !slot.is_in_trade()));
}

#[test]
fn automatic_storage_fills_stacks_before_empty_slots_without_equipping() {
    let mut inventory = state(vec![item(30, Some(7), 0), item(22, Some(18), 0)]);
    assert_eq!(
        inventory.auto_store_destination(actor()).unwrap(),
        InventorySlot(22)
    );
    let mut action = request(&inventory, 30, 22);
    action.quantity = MoveQuantity::Count(NonZeroU32::new(2).unwrap());
    inventory.submit_move(&action, actor(), |_| Ok(())).unwrap();
    assert_eq!(
        inventory.auto_store_destination(actor()).unwrap(),
        InventorySlot(23)
    );
    inventory.apply(InventoryUpdate::Invalidated);
    assert!(inventory.auto_store_destination(actor()).is_err());
}

#[test]
fn automatic_storage_respects_bag_capacity_size_and_specialization() {
    let mut items: Vec<_> = (22..=29).map(|slot| item(slot, None, 1)).collect();
    for bag in &mut items {
        bag.rules.bag_type = 2;
    }
    items.push(item(30, None, 0));
    let mut inventory = state(items);
    assert!(inventory.auto_store_destination(actor()).is_err());
    let bag = inventory.items.get_mut(&InventorySlot(29)).unwrap();
    bag.rules.bag_type = 0;
    bag.rules.bag_size = 0;
    assert!(inventory.auto_store_destination(actor()).is_err());
    inventory
        .items
        .get_mut(&InventorySlot(29))
        .unwrap()
        .rules
        .bag_size = 4;
    assert_eq!(
        inventory.auto_store_destination(actor()).unwrap(),
        InventorySlot(321)
    );
    inventory
        .items
        .get_mut(&InventorySlot(30))
        .unwrap()
        .bag_slots = 1;
    assert!(inventory.auto_store_destination(actor()).is_err());
}

#[test]
fn merging_preserves_remainder_and_encodes_exact_quantity() {
    let mut inventory = state(vec![item(30, Some(7), 0), item(22, Some(18), 0)]);
    let mut action = request(&inventory, 30, 22);
    action.quantity = MoveQuantity::Count(NonZeroU32::new(2).unwrap());
    let before = inventory.clone();
    assert!(inventory
        .submit_move(&action, actor(), |_| anyhow::bail!(
            "synthetic send failure"
        ))
        .is_err());
    assert_eq!(inventory, before);
    inventory
        .submit_move(&action, actor(), |packet| {
            let body = &packet.body[..];
            assert_eq!(&body[8..], &2u32.to_le_bytes());
            Ok(())
        })
        .unwrap();
    assert_eq!(inventory.items[&InventorySlot(30)].stack_count, Some(5));
    assert_eq!(inventory.items[&InventorySlot(22)].stack_count, Some(20));
}

#[test]
fn merge_consumes_cursor_and_rejects_invalid_capacity_or_item() {
    let mut inventory = state(vec![item(30, Some(2), 0), item(22, Some(18), 0)]);
    let mut action = request(&inventory, 30, 22);
    action.quantity = MoveQuantity::Count(NonZeroU32::new(3).unwrap());
    assert!(inventory.plan_move(&action, actor()).is_err());
    action.quantity = MoveQuantity::Count(NonZeroU32::new(2).unwrap());
    for capacity in [0, 19] {
        inventory
            .items
            .get_mut(&InventorySlot(22))
            .unwrap()
            .rules
            .stack_size = capacity;
        assert!(inventory.plan_move(&action, actor()).is_err());
    }
    inventory
        .items
        .get_mut(&InventorySlot(22))
        .unwrap()
        .rules
        .stack_size = 20;
    inventory
        .items
        .get_mut(&InventorySlot(22))
        .unwrap()
        .details
        .id = 99;
    assert!(inventory.plan_move(&action, actor()).is_err());
    inventory
        .items
        .get_mut(&InventorySlot(22))
        .unwrap()
        .details
        .id = 42;
    inventory.submit_move(&action, actor(), |_| Ok(())).unwrap();
    assert!(!inventory.items.contains_key(&InventorySlot(30)));
    assert_eq!(inventory.items[&InventorySlot(22)].stack_count, Some(20));
}
fn state(items: Vec<InventoryItem>) -> Inventory {
    let mut state = Inventory::default();
    state.apply(InventoryUpdate::Snapshot(items));
    state
}
fn request(state: &Inventory, from: i32, to: i32) -> InventoryMove {
    InventoryMove {
        session_id: 7,
        revision: state.revision(),
        from: InventorySlot(from),
        to: InventorySlot(to),
        quantity: MoveQuantity::Whole,
        created: Instant::now(),
    }
}
#[test]
fn whole_bag_move_sends_zero_count_and_relocates_contents() {
    let mut state = state(vec![item(22, None, 8), item(251, Some(7), 0)]);
    let request = request(&state, 22, 23);
    let update = state
        .submit_move(&request, actor(), |packet| {
            let body = &packet.body[..];
            assert_eq!(*body, [22, 0, 0, 0, 23, 0, 0, 0, 0, 0, 0, 0]);
            Ok(())
        })
        .unwrap();
    assert!(matches!(update, InventoryUpdate::Prediction(_)));
    assert!(!state.items.contains_key(&InventorySlot(22)));
    assert!(!state.items.contains_key(&InventorySlot(251)));
    assert_eq!(state.items[&InventorySlot(261)].stack_count, Some(7));
    assert!(state.predicted());
}

#[test]
fn cursor_swap_preserves_both_items_and_bag_children() {
    let mut inventory = state(vec![
        item(30, None, 2),
        item(331, Some(3), 0),
        item(22, None, 2),
        item(251, Some(7), 0),
    ]);
    let action = request(&inventory, 30, 22);
    inventory
        .submit_move(&action, actor(), |packet| {
            let body = &packet.body[..];
            assert_eq!(&body[8..], &[0; 4]);
            Ok(())
        })
        .unwrap();
    assert_eq!(inventory.items[&InventorySlot(251)].stack_count, Some(3));
    assert_eq!(inventory.items[&InventorySlot(331)].stack_count, Some(7));
    assert_eq!(inventory.items.len(), 4);
}

#[test]
fn cursor_swap_checks_eligibility_and_keeps_state_on_send_failure() {
    let mut inventory = state(vec![item(30, None, 0), item(13, None, 0)]);
    let action = request(&inventory, 30, 13);
    let original = inventory.clone();
    assert!(inventory
        .submit_move(&action, actor(), |_| anyhow::bail!("send failed"))
        .is_err());
    assert_eq!(inventory, original);
    let stacks = state(vec![item(30, Some(3), 0), item(22, Some(7), 0)]);
    assert!(stacks
        .plan_move(&request(&stacks, 30, 22), actor())
        .is_err());
}
#[test]
fn split_preserves_remaining_stack_and_server_correction_overrides_prediction() {
    let mut state = state(vec![item(22, Some(7), 0)]);
    let mut request = request(&state, 22, 23);
    request.quantity = MoveQuantity::Count(NonZeroU32::new(1).unwrap());
    state
        .submit_move(&request, actor(), |packet| {
            let body = &packet.body[..];
            assert_eq!(*body, [22, 0, 0, 0, 23, 0, 0, 0, 1, 0, 0, 0]);
            Ok(())
        })
        .unwrap();
    assert_eq!(state.items[&InventorySlot(22)].stack_count, Some(6));
    assert_eq!(state.items[&InventorySlot(23)].stack_count, Some(1));
    state.apply(InventoryUpdate::Set(vec![item(22, Some(7), 0)]));
    state.apply(InventoryUpdate::Remove(InventorySlot(23)));
    assert_eq!(state.items[&InventorySlot(22)].stack_count, Some(7));
    assert!(state.plan_move(&request, actor()).is_err());
}
#[test]
fn old_revisions_and_failed_sends_never_mutate_or_retry() {
    let mut state = state(vec![item(22, None, 0)]);
    let original = state.clone();
    let request = request(&state, 22, 23);
    let sends = Cell::new(0);
    assert!(state
        .submit_move(&request, actor(), |_| {
            sends.set(sends.get() + 1);
            anyhow::bail!("synthetic send failure")
        })
        .is_err());
    assert_eq!(state, original);
    assert_eq!(sends.get(), 1);
    // A request for an older revision is never sent.
    let bad = InventoryMove {
        revision: request.revision + 1,
        ..request.clone()
    };
    assert!(state
        .submit_move(&bad, actor(), |_| panic!("invalid request sent"))
        .is_err());
    assert_eq!(state, original);
    state.apply(InventoryUpdate::Invalidated);
    assert!(state.plan_move(&request, actor()).is_err());
}
#[test]
fn placement_rejects_occupied_special_nested_oversized_and_out_of_capacity_slots() {
    let state = state(vec![
        item(22, None, 2),
        item(23, None, 0),
        item(24, None, 8),
    ]);
    for (from, to) in [
        (23, 22),
        (23, 2000),
        (23, 3000),
        (23, -1),
        (30, 25),
        (24, 251),
        (23, 253),
        (23, 330),
    ] {
        assert!(
            state
                .plan_move(&request(&state, from, to), actor())
                .is_err(),
            "{from} -> {to}"
        );
    }
    // The official client picks up an item onto empty cursor slot 30 first.
    assert!(state.plan_move(&request(&state, 23, 30), actor()).is_ok());
    let mut large = item(23, None, 0);
    large.rules.size = 5;
    let state = super::tests::state(vec![item(22, None, 8), large]);
    assert!(state.plan_move(&request(&state, 23, 251), actor()).is_err());
}
#[test]
fn equipment_checks_masks_handedness_and_allows_unequipping() {
    let mut sword = item(22, None, 0);
    sword.rules.item_type = 1;
    let mut state = state(vec![sword, item(14, None, 0)]);
    assert!(state.plan_move(&request(&state, 22, 13), actor()).is_err());
    state.apply(InventoryUpdate::Remove(InventorySlot(14)));
    assert!(state.plan_move(&request(&state, 22, 13), actor()).is_ok());
    assert!(state.plan_move(&request(&state, 22, 14), actor()).is_err());
    assert!(state.plan_move(&request(&state, 22, 2), actor()).is_err());
    for bad_actor in [
        InventoryActor {
            deity: None,
            class: None,
            ..actor()
        },
        InventoryActor {
            deity: None,
            class: Some(2),
            ..actor()
        },
        InventoryActor { race: 2, ..actor() },
    ] {
        assert!(state
            .plan_move(&request(&state, 22, 13), bad_actor)
            .is_err());
    }
    let mut restricted = item(13, None, 0);
    restricted.details.classes = 0;
    state.apply(InventoryUpdate::Set(vec![restricted]));
    assert!(state.plan_move(&request(&state, 13, 23), actor()).is_ok());
}
#[test]
fn deity_masks_admit_matching_followers_and_both_agnostic_identifiers() {
    for (mask, allowed) in [
        (1u32, vec![140, 396]),
        (2, vec![201]),
        (32768, vec![215]),
        (65536, vec![216]),
    ] {
        let mut restricted = item(30, None, 0);
        restricted.rules.deity_mask = mask;
        let inventory = state(vec![restricted]);
        for deity in [
            None,
            Some(0),
            Some(140),
            Some(201),
            Some(202),
            Some(215),
            Some(216),
            Some(396),
            Some(999),
        ] {
            let actor = InventoryActor { deity, ..actor() };
            assert_eq!(
                inventory
                    .plan_move(&request(&inventory, 30, 13), actor)
                    .is_ok(),
                deity.is_some_and(|id| allowed.contains(&id))
            );
            // Restrictions affect wearing an item, not storing it in an ordinary carried slot.
            assert!(inventory
                .plan_move(&request(&inventory, 30, 22), actor)
                .is_ok());
        }
    }
    let unrestricted = state(vec![item(30, None, 0)]);
    assert!(unrestricted
        .plan_move(&request(&unrestricted, 30, 13), actor())
        .is_ok());
}

#[test]
fn offhand_weapons_require_trained_skill_and_preserve_two_handed_exclusion() {
    for item_type in [0, 2, 3, 45] {
        let mut weapon = item(30, None, 0);
        weapon.rules.item_type = item_type;
        let mut inventory = state(vec![weapon]);
        let action = request(&inventory, 30, 14);
        for skill in [None, Some(0)] {
            assert!(inventory
                .plan_move(
                    &action,
                    InventoryActor {
                        dual_wield: skill,
                        ..actor()
                    }
                )
                .is_err());
        }
        let trained = InventoryActor {
            dual_wield: Some(1),
            ..actor()
        };
        assert!(inventory.plan_move(&action, trained).is_ok());
        let mut primary = item(13, None, 0);
        primary.rules.item_type = 1;
        inventory.apply(InventoryUpdate::Set(vec![primary]));
        assert!(inventory
            .plan_move(&request(&inventory, 30, 14), trained)
            .is_err());
    }
    let shield = state(vec![item(30, None, 0)]);
    assert!(shield.plan_move(&request(&shield, 30, 14), actor()).is_ok());
}

#[test]
fn invalid_stack_counts_are_rejected() {
    for count in [None, Some(2)] {
        let state = state(vec![item(22, count, 0)]);
        let mut request = request(&state, 22, 23);
        request.quantity = MoveQuantity::Count(NonZeroU32::new(3).unwrap());
        assert!(state.plan_move(&request, actor()).is_err());
    }
}
