use super::*;

#[test]
fn zero_quantity_removal_clears_cursor_but_does_not_confirm_other_predictions() {
    let mut state = Inventory::default();
    state.apply(
        decode(0x5394, wire(22, 42, 0, false, 0, &[]).as_bytes())
            .unwrap()
            .unwrap(),
    );
    let mut scroll = state.items[&InventorySlot(22)].clone();
    scroll.slot = InventorySlot(30);
    state.apply(InventoryUpdate::Prediction(vec![scroll]));
    let mutation = |opcode, count: u32| {
        let body = [30u32, u32::MAX, count]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        decode(opcode, &body).unwrap().unwrap()
    };
    state.apply(mutation(0x420f, 0));
    assert!(!state.items.contains_key(&InventorySlot(30)));
    // Removal alone does not prove whether this was consumption or correction.
    // Keep the existing correction guard until the other end is authoritative.
    assert!(state.awaiting_correction());
    // The source move is still unconfirmed; cursor removal confirms only the cursor.
    assert_eq!(
        state
            .prediction_origins()
            .map(|(slot, _)| slot)
            .collect::<Vec<_>>(),
        vec![InventorySlot(22)]
    );
    state.apply(InventoryUpdate::Remove(InventorySlot(22)));
    assert!(!state.stale());
    for (opcode, count) in [(0x420f, 1), (0x4d81, 0), (0x1c4a, 0)] {
        assert_eq!(mutation(opcode, count), InventoryUpdate::Invalidated);
    }
}

#[test]
fn prediction_origins_survive_round_trips_until_server_confirmation() {
    let mut state = Inventory::default();
    let item = match decode(0x5394, wire(22, 42, 0, false, 0, &[]).as_bytes())
        .unwrap()
        .unwrap()
    {
        InventoryUpdate::Snapshot(items) => items[0].clone(),
        other => panic!("expected snapshot, got {other:?}"),
    };
    state.apply(InventoryUpdate::Snapshot(vec![item.clone()]));
    let mut cursor = item.clone();
    cursor.slot = InventorySlot(30);
    state.apply(InventoryUpdate::Prediction(vec![cursor]));
    state.apply(InventoryUpdate::Prediction(vec![item.clone()]));
    let origins: Vec<_> = state.prediction_origins().collect();
    assert_eq!(
        origins,
        vec![(InventorySlot(22), Some(&item)), (InventorySlot(30), None)]
    );
    state.apply(InventoryUpdate::Set(vec![item]));
    assert_eq!(
        state.prediction_origins().collect::<Vec<_>>(),
        vec![(InventorySlot(30), None)]
    );
    state.apply(InventoryUpdate::Remove(InventorySlot(30)));
    assert_eq!(state.prediction_origins().count(), 0);
}

// Synthetic fixtures only: no captured packets, names, accounts or credentials.
#[test]
fn partial_correction_blocks_moves_until_both_ends_are_authoritative() {
    let mut state = Inventory::default();
    state.apply(
        decode(0x5394, wire(22, 42, 0, false, 0, &[]).as_bytes())
            .unwrap()
            .unwrap(),
    );
    let original = state.items[&InventorySlot(22)].clone();
    let mut cursor = original.clone();
    cursor.slot = InventorySlot(30);
    state.apply(InventoryUpdate::Prediction(vec![cursor]));
    assert!(state.predicted());
    assert!(!state.stale());
    // The server restores the source before clearing the predicted cursor.
    state.apply(InventoryUpdate::Set(vec![original]));
    assert!(state.stale());
    assert!(state.predicted());
    let actor = InventoryActor {
        bank_access: false,
        class: Some(1),
        deity: None,
        dual_wield: None,
        race: 1,
        level: 1,
    };
    assert!(state.auto_store_destination(actor).is_err());
    state.apply(InventoryUpdate::Remove(InventorySlot(30)));
    assert!(!state.stale());
    assert!(!state.predicted());
    assert_eq!(state.items.len(), 1);
    assert!(state.items.contains_key(&InventorySlot(22)));
    // A resolved prediction never clears a separate malformed-packet invalidation.
    let original = state.items[&InventorySlot(22)].clone();
    let mut cursor = original;
    cursor.slot = InventorySlot(30);
    state.apply(InventoryUpdate::Prediction(vec![cursor.clone()]));
    state.apply(InventoryUpdate::Invalidated);
    state.apply(InventoryUpdate::Set(vec![cursor]));
    state.apply(InventoryUpdate::Remove(InventorySlot(22)));
    assert!(state.stale());
    assert!(!state.predicted());
}

#[test]
fn matching_slot_updates_confirm_prediction_without_disabling_inventory() {
    let mut state = Inventory::default();
    state.apply(
        decode(0x5394, wire(22, 42, 0, false, 0, &[]).as_bytes())
            .unwrap()
            .unwrap(),
    );
    let mut cursor = state.items[&InventorySlot(22)].clone();
    cursor.slot = InventorySlot(30);
    state.apply(InventoryUpdate::Prediction(vec![cursor.clone()]));
    state.apply(InventoryUpdate::Set(vec![cursor]));
    assert!(state.predicted());
    assert!(!state.stale());
    state.apply(InventoryUpdate::Remove(InventorySlot(22)));
    assert!(!state.predicted());
    assert!(!state.stale());
}

#[test]
fn container_correction_resolves_children_and_snapshot_resets_all_uncertainty() {
    let mut state = Inventory::default();
    let child = wire(251, 43, 0, false, 1, &[]);
    state.apply(
        decode(0x5394, wire(22, 42, 2, false, 0, &[(0, child)]).as_bytes())
            .unwrap()
            .unwrap(),
    );
    let original: Vec<_> = state.items.values().cloned().collect();
    let predicted: Vec<_> = original
        .iter()
        .cloned()
        .map(|mut item| {
            item.slot = if item.slot == InventorySlot(22) {
                InventorySlot(30)
            } else {
                InventorySlot(331)
            };
            item
        })
        .collect();
    state.apply(InventoryUpdate::Prediction(predicted.clone()));
    state.apply(InventoryUpdate::Set(original.clone()));
    assert!(state.awaiting_correction());
    state.apply(InventoryUpdate::Remove(InventorySlot(30)));
    assert!(!state.stale());
    assert!(!state.predicted());
    assert!(!state.items.contains_key(&InventorySlot(331)));
    assert!(state.items.contains_key(&InventorySlot(251)));
    state.apply(InventoryUpdate::Prediction(predicted));
    state.apply(InventoryUpdate::Set(original.clone()));
    state.apply(InventoryUpdate::Invalidated);
    state.apply(InventoryUpdate::Snapshot(original));
    assert!(!state.stale());
    assert!(!state.predicted());
}

fn wire(
    slot: i32,
    id: u32,
    bag: u8,
    stack: bool,
    depth: usize,
    children: &[(usize, String)],
) -> String {
    let mut fields = vec!["0".to_owned(); 159];
    fields[0] = if bag > 0 { "1" } else { "0" }.into();
    fields[1] = format!("Test item {id}");
    fields[4] = id.to_string();
    fields[5] = "25".into();
    fields[11] = "500".into();
    fields[21] = "-3".into();
    fields[97] = bag.to_string();
    fields[131] = if stack { "100" } else { "0" }.into();
    fields[133] = u8::from(stack).to_string();
    fields[154] = if id == 301 { "73" } else { "-1" }.into();
    if id == 401 {
        for (index, value) in [
            (60, "2500"),
            (119, "15"),
            (120, "7"),
            (134, "73"),
            (135, "4"),
            (136, "20"),
            (137, "5"),
        ] {
            fields[index] = value.into();
        }
    }
    let wrapper = if depth == 0 {
        String::new()
    } else {
        format!("{}\"", "\\".repeat(depth - 1))
    };
    let quote = format!("{}\"", "\\".repeat(depth));
    let mut text = format!(
        "{wrapper}7|0|{slot}|0|1|0|123|0|-1|0|0|{quote}{}{quote}",
        fields.join("|")
    );
    for i in 0..10 {
        text.push('|');
        if let Some((_, child)) = children.iter().find(|(index, _)| *index == i) {
            text.push_str(child);
        }
    }
    text.push_str(&wrapper);
    if depth == 0 {
        text.push('\0');
    }
    text
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the ordered integration scenario and its assertions together"
)]
fn item_cast_uses_current_effect_and_titanium_item_slot_without_consuming_inventory() {
    let mut inventory = Inventory::default();
    inventory.apply(
        decode(0x5394, wire(13, 401, 0, false, 0, &[]).as_bytes())
            .unwrap()
            .unwrap(),
    );
    let before = inventory.clone();
    let packet = inventory
        .item_cast_packet(inventory.revision(), InventorySlot(13), 20, 7)
        .unwrap();
    let expected: Vec<u8> = [10u32, 73, 13, 7, 0]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    assert_eq!(packet.as_slice(), expected);
    assert_eq!(inventory, before);
    let now = std::time::Instant::now();
    let mut request = ItemUse {
        request_id: 1,
        session_id: 12,
        revision: inventory.revision(),
        slot: InventorySlot(13),
        target_id: 7,
        created: now,
    };
    assert_eq!(
        inventory
            .prepare_item_cast(&request, 12, 20, true, now)
            .unwrap(),
        (73, packet)
    );
    assert!(inventory
        .prepare_item_cast(&request, 13, 20, true, now)
        .is_err());
    assert!(inventory
        .prepare_item_cast(&request, 12, 20, false, now)
        .is_err());
    assert!(inventory
        .prepare_item_cast(
            &request,
            12,
            20,
            true,
            now + std::time::Duration::from_secs(1)
        )
        .is_err());
    request.created = now + std::time::Duration::from_millis(1);
    assert!(inventory
        .prepare_item_cast(&request, 12, 20, true, now)
        .is_err());
    assert!(inventory
        .item_cast_packet(inventory.revision(), InventorySlot(13), 19, 7)
        .is_err());
    assert!(inventory
        .item_cast_packet(inventory.revision(), InventorySlot(13), 20, 0)
        .is_err());
    assert!(inventory
        .item_cast_packet(inventory.revision(), InventorySlot(14), 20, 7)
        .is_err());
    assert!(inventory
        .item_cast_packet(inventory.revision() + 1, InventorySlot(13), 20, 7)
        .is_err());
    inventory.apply(InventoryUpdate::Invalidated);
    assert!(inventory
        .item_cast_packet(inventory.revision(), InventorySlot(13), 20, 7)
        .is_err());

    let mut item = before.items()[&InventorySlot(13)].clone();
    let accepts = |item: &InventoryItem| {
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item.clone()]));
        inventory
            .item_cast_packet(inventory.revision(), item.slot, 20, 7)
            .is_ok()
    };
    item.slot = InventorySlot(22);
    assert!(!accepts(&item)); // Equipped-only effect is in a pack slot.
    item.activation.effect.as_mut().unwrap().kind = ClickKind::Click;
    assert!(accepts(&item));
    for slot in [30, 251, 2000, 2531, -1] {
        item.slot = InventorySlot(slot);
        assert!(!accepts(&item));
    }
    item.slot = InventorySlot(22);
    item.activation.maximum_charges = 5;
    item.charges = 0;
    assert!(!accepts(&item));
    item.charges = 1;
    assert!(accepts(&item));
    item.charges = -2;
    assert!(!accepts(&item));
    item.charges = -1;
    assert!(accepts(&item));
    item.activation.maximum_charges = 0;
    item.activation.effect.as_mut().unwrap().kind = ClickKind::Expendable;
    item.charges = 0;
    assert!(!accepts(&item));
    item.stack_count = Some(2);
    assert!(accepts(&item));
    item.stack_count = Some(0);
    assert!(!accepts(&item));
    item.stack_count = None;
    item.charges = 1;
    item.activation.effect.as_mut().unwrap().kind = ClickKind::Unknown(42);
    assert!(!accepts(&item));
    item.activation.effect = None;
    assert!(!accepts(&item));
}

#[test]
fn click_effect_metadata_survives_inventory_updates_and_nested_slot_resolution() {
    let mut inventory = Inventory::default();
    let child = wire(0, 401, 0, false, 1, &[]);
    let packet = wire(22, 500, 2, false, 0, &[(0, child)]);
    inventory.apply(decode(0x5394, packet.as_bytes()).unwrap().unwrap());
    let item = &inventory.items()[&InventorySlot(251)];
    let effect = item.activation.effect.as_ref().unwrap();
    assert_eq!(effect.spell_id, 73);
    assert_eq!(effect.kind, ClickKind::Equipped);
    assert_eq!(effect.required_level, 20);
    assert_eq!(effect.effect_level, 5);
    assert_eq!(effect.cast_time_ms, 2500);
    assert_eq!(effect.recast_delay_seconds, 15);
    assert_eq!(effect.recast_type, 7);
    assert_eq!(item.charges, -1);
    assert!(item.scroll_spell.is_none());
    assert!(inventory.items()[&InventorySlot(22)]
        .activation
        .effect
        .is_none());
    inventory.apply(InventoryUpdate::Set(vec![{
        let mut replacement = item.clone();
        replacement.activation = ItemActivation::default();
        replacement
    }]));
    assert!(inventory.items()[&InventorySlot(251)]
        .activation
        .effect
        .is_none());
}

#[test]
fn scribing_uses_current_cursor_definition_without_consuming_it_optimistically() {
    let mut inventory = Inventory::default();
    inventory.apply(
        decode(0x5394, wire(30, 301, 0, false, 0, &[]).as_bytes())
            .unwrap()
            .unwrap(),
    );
    assert_eq!(inventory.items()[&InventorySlot(30)].scroll_spell, Some(73));
    let profile = vec![0; 19592];
    let mut book = crate::spells::SpellBook::titanium_profile(&profile).unwrap();
    let revision = inventory.revision();
    let before = inventory.clone();
    let packet = book.scribe_packet(&inventory, revision, 399, 73).unwrap();
    assert_eq!(&packet[..4], &399u32.to_le_bytes());
    assert_eq!(&packet[4..8], &73u32.to_le_bytes());
    assert_eq!(&packet[8..], &[0; 8]);
    assert_eq!(inventory, before);
    assert!(book.scribe_packet(&inventory, revision, 400, 73).is_err());
    assert!(book.scribe_packet(&inventory, revision, 0, 74).is_err());
    assert!(book
        .scribe_packet(&inventory, revision.wrapping_add(1), 0, 73)
        .is_err());
    book.apply(&crate::spells::SpellUpdate::Slot {
        slot: 399,
        spell_id: 73,
        mode: 0,
    });
    assert!(book.scribe_packet(&inventory, revision, 399, 73).is_err());
    assert!(book.scribe_packet(&inventory, revision, 0, 73).is_err());
    inventory.apply(InventoryUpdate::Invalidated);
    assert!(book
        .scribe_packet(&inventory, inventory.revision(), 0, 73)
        .is_err());
}

#[test]
fn snapshot_decodes_equipment_bags_stack_quantities_and_unlimited_charges() {
    let child = wire(0, 42, 0, true, 1, &[]);
    let bag = wire(22, 100, 8, false, 0, &[(2, child)]);
    let equipment = wire(13, 200, 0, false, 0, &[]);
    let update = decode(0x5394, (equipment + &bag).as_bytes())
        .unwrap()
        .unwrap();
    let mut inventory = Inventory::default();
    assert!(!inventory.received());
    inventory.apply(update);
    assert!(inventory.received() && !inventory.stale());
    assert_eq!(inventory.items.len(), 3);
    assert_eq!(inventory.items[&InventorySlot(253)].stack_count, Some(7));
    assert_eq!(inventory.items[&InventorySlot(253)].rules.stack_size, 100);
    assert_eq!(inventory.items[&InventorySlot(13)].charges, -1);
    assert_eq!(inventory.items[&InventorySlot(13)].scroll_spell, None);
    assert_eq!(inventory.items[&InventorySlot(22)].bag_slots, 8);
    assert_eq!(
        inventory.items[&InventorySlot(253)].details.name,
        "Test item 42"
    );
    assert!(inventory.items[&InventorySlot(13)]
        .details
        .stats
        .iter()
        .any(|s| s.value == -3));
}

#[test]
fn link_merchant_and_loot_views_never_become_inventory() {
    for kind in [0u32, 0x64, 0x65, 0x66, 0x6b, 0xdead_beef] {
        let mut packet = kind.to_le_bytes().to_vec();
        packet.extend(wire(22, 42, 0, false, 0, &[]).bytes());
        assert!(decode(0x3397, &packet).unwrap().is_none());
    }
}

#[test]
fn replacement_and_removal_clear_old_bag_contents_without_changing_other_slots() {
    let mut state = Inventory::default();
    let bag = wire(22, 100, 8, false, 0, &[(0, wire(0, 42, 0, true, 1, &[]))]);
    state.apply(
        decode(0x5394, (bag + &wire(13, 200, 0, false, 0, &[])).as_bytes())
            .unwrap()
            .unwrap(),
    );
    let mut packet = 0x69u32.to_le_bytes().to_vec();
    packet.extend(wire(22, 101, 4, false, 0, &[]).bytes());
    state.apply(decode(0x3397, &packet).unwrap().unwrap());
    assert!(!state.items.contains_key(&InventorySlot(251)));
    assert!(state.items.contains_key(&InventorySlot(13)));
    let mut deletion = 22u32.to_le_bytes().to_vec();
    deletion.extend(u32::MAX.to_le_bytes());
    deletion.extend(u32::MAX.to_le_bytes());
    state.apply(decode(0x420f, &deletion).unwrap().unwrap());
    assert!(!state.items.contains_key(&InventorySlot(22)));
    state.apply(decode(0x4d81, &deletion).unwrap().unwrap());
    assert!(!state.stale());
    state.apply(decode(0x5394, &[]).unwrap().unwrap());
    assert!(state.items.is_empty() && state.received() && !state.stale());
}

#[test]
fn malformed_frames_duplicates_and_bad_container_addresses_are_rejected_atomically() {
    let bag = wire(22, 100, 4, false, 0, &[(0, wire(0, 42, 0, true, 1, &[]))]);
    for end in 1..bag.len() {
        assert!(decode(0x5394, &bag.as_bytes()[..end]).is_err(), "{end}");
    }
    assert!(decode(0x5394, (bag.clone() + &bag).as_bytes()).is_err());
    assert!(decode(
        0x5394,
        wire(13, 100, 4, false, 0, &[(0, wire(0, 42, 0, true, 1, &[]))]).as_bytes()
    )
    .is_err());
    assert!(decode(
        0x5394,
        wire(22, 100, 1, false, 0, &[(2, wire(0, 42, 0, true, 1, &[]))]).as_bytes()
    )
    .is_err());
    assert!(decode(0x5394, &vec![0; 1024 * 1024 + 1]).is_err());
    for n in 0..12 {
        assert!(decode(0x4d81, &vec![0; n]).is_err());
    }
}

#[test]
fn slots_use_titanium_offsets_and_preserve_unknown_addresses() {
    for (root, first) in [
        (22, 251),
        (29, 321),
        (30, 331),
        (2000, 2031),
        (2015, 2181),
        (2501, 2541),
    ] {
        for index in 0..10 {
            let parent = InventorySlot(root);
            let slot = parent.child(index).unwrap();
            assert_eq!(slot.0, first + i32::from(index));
            assert_eq!(slot.parent(), Some((parent, index)));
        }
    }
    assert!(InventorySlot(13).child(0).is_none());
    assert_eq!(InventorySlot(9999).label(), "Slot 9999");
    assert_eq!(InventorySlot(251).label(), "Pack 1 / 1");
}

#[test]
fn admission_replay_preserves_partial_complete_and_stale_states() {
    let mut before = Inventory::default();
    let mut packet = 0x69u32.to_le_bytes().to_vec();
    packet.extend(wire(22, 100, 4, false, 0, &[(0, wire(0, 42, 0, true, 1, &[]))]).bytes());
    before.apply(decode(0x3397, &packet).unwrap().unwrap());
    before.apply(InventoryUpdate::Invalidated);
    let mut after = Inventory::default();
    for update in before.admission_updates() {
        after.apply(update);
    }
    assert_eq!(after.items(), before.items());
    assert_eq!(after.received(), before.received());
    assert_eq!(after.stale(), before.stale());
    assert!(!after.received() && after.stale());
    before.apply(decode(0x5394, &packet[4..]).unwrap().unwrap());
    let mut after = Inventory::default();
    for update in before.admission_updates() {
        after.apply(update);
    }
    assert_eq!(after.items(), before.items());
    assert_eq!(after.received(), before.received());
    assert_eq!(after.stale(), before.stale());
    assert!(after.received() && !after.stale());
}

#[test]
fn server_resync_and_consumption_override_predicted_contents() {
    let mut state = Inventory::default();
    state.apply(
        decode(0x5394, wire(22, 42, 0, true, 0, &[]).as_bytes())
            .unwrap()
            .unwrap(),
    );
    let items = state.items.values().cloned().collect();
    state.apply(InventoryUpdate::Prediction(items));
    let mut deletion = 22u32.to_le_bytes().to_vec();
    deletion.extend(u32::MAX.to_le_bytes());
    deletion.extend(u32::MAX.to_le_bytes());
    state.apply(decode(0x4d81, &deletion).unwrap().unwrap());
    assert_eq!(state.items[&InventorySlot(22)].stack_count, Some(6));
    // Resync uses Trade packet kind even for our own inventory.
    let mut resync = 0x67u32.to_le_bytes().to_vec();
    resync.extend(wire(22, 22292, 0, false, 0, &[]).bytes());
    state.apply(decode(0x3397, &resync).unwrap().unwrap());
    state.apply(decode(0x4d81, &deletion).unwrap().unwrap());
    assert!(!state.items.contains_key(&InventorySlot(22)));
    resync = 0x67u32.to_le_bytes().to_vec();
    resync.extend(wire(22, 42, 0, true, 0, &[]).bytes());
    state.apply(decode(0x3397, &resync).unwrap().unwrap());
    assert_eq!(state.items[&InventorySlot(22)].stack_count, Some(7));
    resync = 0x67u32.to_le_bytes().to_vec();
    resync.extend(wire(3000, 42, 0, true, 0, &[]).bytes());
    assert!(decode(0x3397, &resync).unwrap().is_none());
    // Consuming a finite charge retains the item when it reaches zero.
    let mut charged = state.items[&InventorySlot(22)].clone();
    charged.stack_count = None;
    charged.charges = 1;
    state.apply(InventoryUpdate::Set(vec![charged]));
    state.apply(decode(0x1c4a, &deletion).unwrap().unwrap());
    assert_eq!(state.items[&InventorySlot(22)].charges, 0);
}

#[test]
fn deductions_shrink_stacks_and_remove_whole_items() {
    let mut state = Inventory::default();
    state.apply(
        decode(0x5394, wire(22, 42, 0, true, 0, &[]).as_bytes())
            .unwrap()
            .unwrap(),
    );
    let mut charged = state.items[&InventorySlot(22)].clone();
    charged.slot = InventorySlot(23);
    charged.stack_count = None;
    charged.charges = 3;
    state.apply(InventoryUpdate::Set(vec![charged]));
    state.apply(InventoryUpdate::Deduct {
        slot: InventorySlot(22),
        quantity: 5,
    });
    assert_eq!(state.items[&InventorySlot(22)].stack_count, Some(2));
    state.apply(InventoryUpdate::Deduct {
        slot: InventorySlot(22),
        quantity: 2,
    });
    assert!(!state.items.contains_key(&InventorySlot(22)));
    // An unstacked item leaves whole, whatever quantity the server reports.
    state.apply(InventoryUpdate::Deduct {
        slot: InventorySlot(23),
        quantity: 1,
    });
    assert!(!state.items.contains_key(&InventorySlot(23)));
    assert!(!state.stale());
    state.apply(InventoryUpdate::Deduct {
        slot: InventorySlot(24),
        quantity: 1,
    });
    assert!(state.stale());
}
