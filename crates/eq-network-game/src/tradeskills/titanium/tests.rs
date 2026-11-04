use super::*;
use crate::{
    inventory::{InventoryItem, InventoryUpdate},
    items::ItemDetails,
};

fn item(slot: InventorySlot) -> InventoryItem {
    InventoryItem {
        activation: crate::inventory::ItemActivation::default(),
        scroll_spell: None,
        book: None,
        rules: crate::inventory::ItemPlacement::default(),
        slot,
        stack_count: None,
        charges: 0,
        bag_slots: 0,
        details: ItemDetails {
            equipment: None,
            bonuses: None,
            id: 42,
            name: "Synthetic component".into(),
            lore: String::new(),
            weight_tenths: 0,
            slots: 0,
            classes: 0,
            races: 0,
            flags: Vec::new(),
            stats: Vec::new(),
            price: None,
            icon: None,
        },
    }
}

fn contents(kind: u8) -> Vec<InventoryItem> {
    let mut kit = item(InventorySlot(23));
    kit.rules.bag_type = kind;
    kit.bag_slots = 10;
    vec![kit, item(InventorySlot(23).child(0).unwrap())]
}

fn inventory(items: Vec<InventoryItem>) -> Inventory {
    let mut inventory = Inventory::default();
    inventory.apply(InventoryUpdate::Snapshot(items));
    inventory
}

#[test]
fn an_ordinary_carried_combine_neither_consumes_nor_predicts_a_product() {
    let inventory = inventory(contents(16));
    let before = inventory.items().clone();
    assert_eq!(
        check_carried(&inventory, InventorySlot(23), 1, Some(1), false),
        Ok(())
    );
    assert_eq!(inventory.items(), &before);
    assert!(!inventory.predicted());
}

#[test]
fn hands_and_stack_quantities_are_distinct_from_tool_charges() {
    let mut items = contents(16);
    items[1].charges = -1;
    assert!(check_carried(&inventory(items.clone()), InventorySlot(23), 1, None, false).is_ok());
    items[1].charges = 10;
    assert!(check_carried(&inventory(items.clone()), InventorySlot(23), 1, None, false).is_ok());
    items[1].stack_count = Some(2);
    assert_eq!(
        check_carried(&inventory(items.clone()), InventorySlot(23), 1, None, false)
            .unwrap_err()
            .string_id,
        Some(8504)
    );
    items[1].stack_count = Some(1);
    assert!(check_carried(&inventory(items.clone()), InventorySlot(23), 1, None, false).is_ok());
    assert_eq!(
        check_carried(&inventory(items.clone()), InventorySlot(23), 1, None, true)
            .unwrap_err()
            .string_id,
        Some(12024)
    );
    items.push(item(InventorySlot::CURSOR));
    assert_eq!(
        check_carried(&inventory(items), InventorySlot(23), 1, None, false)
            .unwrap_err()
            .string_id,
        Some(12024)
    );
}

#[test]
fn research_and_cultural_containers_use_known_native_identity() {
    let research = inventory(contents(24));
    assert!(check_carried(&research, InventorySlot(23), 1, Some(12), false).is_ok());
    for class in [None, Some(1), Some(13)] {
        assert_eq!(
            check_carried(&research, InventorySlot(23), 1, class, false)
                .unwrap_err()
                .string_id,
            Some(12010)
        );
    }
    let cultural = inventory(contents(38));
    assert!(check_carried(&cultural, InventorySlot(23), 128, None, false).is_ok());
    assert_eq!(
        check_carried(&cultural, InventorySlot(23), 1, None, false)
            .unwrap_err()
            .string_id,
        Some(12017)
    );
    let human = inventory(contents(40));
    assert!(check_carried(&human, InventorySlot(23), 1, None, false).is_ok());
    assert!(check_carried(&human, InventorySlot(23), 128, None, false).is_err());
}

#[test]
fn unknown_or_stale_inventory_and_noncarried_or_empty_containers_refuse() {
    let mut stale = inventory(contents(16));
    stale.apply(InventoryUpdate::Invalidated);
    for unavailable in [Inventory::default(), stale] {
        assert!(check_carried(&unavailable, InventorySlot(23), 1, None, false).is_err());
    }
    for kind in [0, 5, 8, 51, 58, 59, 255] {
        assert!(check_carried(
            &inventory(contents(kind)),
            InventorySlot(23),
            1,
            None,
            false
        )
        .is_err());
    }
    let mut empty = contents(16);
    empty.truncate(1);
    assert_eq!(
        check_carried(&inventory(empty), InventorySlot(23), 1, None, false)
            .unwrap_err()
            .string_id,
        Some(12025)
    );
    for slot in [
        InventorySlot::CURSOR,
        InventorySlot(2000),
        super::super::WORLD_CONTAINER,
    ] {
        let mut items = contents(16);
        items[0].slot = slot;
        assert!(check_carried(&inventory(items), slot, 1, None, false).is_err());
    }
}
