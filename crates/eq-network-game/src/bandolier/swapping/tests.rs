use super::*;
use crate::{
    bandolier::BeltItem,
    inventory::{InventoryUpdate, ItemPlacement},
    items::ItemDetails,
};

/// A plain item `id` in `slot`: not a bag, size 1.
fn item(slot: i32, id: u32) -> InventoryItem {
    InventoryItem {
        activation: crate::inventory::ItemActivation::default(),
        scroll_spell: None,
        book: None,
        slot: InventorySlot(slot),
        stack_count: None,
        charges: 0,
        bag_slots: 0,
        augments: Default::default(),
        rules: ItemPlacement {
            titanium_item_class: Some(0),
            stack_size: 1,
            size: 1,
            item_type: 0,
            ..Default::default()
        },
        details: ItemDetails {
            base_held_model: None,
            equipment: None,
            bonuses: None,
            id,
            name: format!("Item {id}"),
            lore: String::new(),
            weight_tenths: 1,
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

/// A stack of `count` arrows `id`, at most 100 to a stack.
fn arrows(slot: i32, id: u32, count: u32) -> InventoryItem {
    let mut arrows = item(slot, id);
    arrows.stack_count = Some(count);
    arrows.rules.stack_size = 100;
    arrows.rules.item_type = ARROW;
    arrows
}

/// A bag of `slots` places in `slot`, taking items up to size 2.
fn bag(slot: i32, id: u32, slots: u8, kind: u8) -> InventoryItem {
    let mut bag = item(slot, id);
    bag.rules.titanium_item_class = Some(BAG_CLASS);
    bag.rules.bag_size = 2;
    bag.rules.bag_type = kind;
    bag.bag_slots = slots;
    bag
}

fn inventory(items: Vec<InventoryItem>) -> Inventory {
    let mut inventory = Inventory::default();
    inventory.apply(InventoryUpdate::Snapshot(items));
    inventory
}

fn set(ids: [Option<u32>; SET_ITEMS]) -> BandolierSet {
    BandolierSet {
        name: "Set".into(),
        items: ids.map(|id| {
            id.map(|item_id| BeltItem {
                item_id,
                icon: 0,
                name: String::new(),
            })
        }),
    }
}

/// Where each item ends, as (slot, id, stack count).
fn placed(items: &[InventoryItem]) -> Vec<(i32, u32, Option<u32>)> {
    items
        .iter()
        .map(|item| (item.slot.0, item.details.id, item.stack_count))
        .collect()
}

/// Packs 22 to 29 full of plain items 900 and up, but for the slots given.
fn full_packs(except: &[i32]) -> Vec<InventoryItem> {
    (22..=29)
        .filter(|slot| !except.contains(slot))
        .map(|slot| item(slot, 900 + u32::try_from(slot).unwrap()))
        .collect()
}

#[test]
fn a_swap_puts_the_set_in_place_and_returns_the_worn_to_the_first_empty_packs() {
    // A sword and a shield worn; the set's mace and buckler in 24 and 26.
    let before = inventory(vec![
        item(13, 1),
        item(14, 2),
        item(22, 50),
        item(24, 3),
        item(26, 4),
    ]);
    let after = swap(&set([Some(3), Some(4), None, None]), &before).unwrap();
    // Pass 1 empties 24 and 26; pass 2 returns the sword to 23, the first
    // empty pack, and the shield to 24.
    assert_eq!(
        placed(&after),
        [
            (13, 3, None),
            (14, 4, None),
            (22, 50, None),
            (23, 1, None),
            (24, 2, None)
        ]
    );
}

#[test]
fn the_range_slot_takes_one_arrow_and_the_rest_stays() {
    let before = inventory(vec![arrows(22, 7, 20)]);
    let after = swap(&set([None, None, Some(7), None]), &before).unwrap();
    assert_eq!(placed(&after), [(11, 7, Some(1)), (22, 7, Some(19))]);
}

#[test]
fn returned_arrows_top_up_stacks_in_slot_order_then_take_an_empty_place() {
    // 30 arrows worn in the ammo slot, an empty set: they go back.
    // 22 has 90 of the same (10 to fill), the bag in 23 holds 95 (5 to
    // fill), and the last 15 take the bag's next empty place (262), which
    // comes before the empty pack 24.
    let before = inventory(vec![
        arrows(21, 7, 30),
        arrows(22, 7, 90),
        bag(23, 60, 4, 0),
        arrows(261, 7, 95),
    ]);
    let after = swap(&set([None, None, None, None]), &before).unwrap();
    assert_eq!(
        placed(&after),
        [
            (22, 7, Some(100)),
            (23, 60, None),
            (261, 7, Some(100)),
            (262, 7, Some(15)),
        ]
    );
}

#[test]
fn a_bag_earlier_in_order_takes_the_return_before_a_later_empty_pack() {
    let mut packs = full_packs(&[22, 27]);
    packs.extend([bag(22, 60, 4, 0), item(251, 61), item(13, 1)]);
    let after = swap(&set([None, None, None, None]), &inventory(packs)).unwrap();
    // The bag in 22 has room at its second place (252), before pack 27.
    assert!(placed(&after).contains(&(252, 1, None)));
    assert!(!after.iter().any(|item| item.slot.0 == 27));
}

#[test]
fn a_quiver_takes_only_arrows_and_a_too_small_bag_nothing_larger() {
    let mut packs = full_packs(&[22, 23, 28]);
    packs.extend([bag(22, 60, 4, QUIVER), bag(23, 61, 4, 0), item(13, 1)]);
    // The sword is too large for the bag in 23 as well.
    packs.last_mut().unwrap().rules.size = 3;
    let after = swap(&set([None, None, None, None]), &inventory(packs)).unwrap();
    assert!(placed(&after).contains(&(28, 1, None)));
    // A plain size-1 item skips the quiver and goes in the bag in 23.
    let mut packs = full_packs(&[22, 23]);
    packs.extend([bag(22, 60, 4, QUIVER), bag(23, 61, 4, 0), item(13, 1)]);
    let after = swap(&set([None, None, None, None]), &inventory(packs)).unwrap();
    assert!(placed(&after).contains(&(261, 1, None)));
}

#[test]
fn a_set_item_not_carried_leaves_its_weapon_slot_empty() {
    // The set names a mace the player lost; the worn sword goes back.
    let before = inventory(vec![item(13, 1)]);
    let after = swap(&set([Some(3), None, None, None]), &before).unwrap();
    assert_eq!(placed(&after), [(22, 1, None)]);
}

#[test]
fn a_set_with_an_empty_place_returns_what_that_slot_holds() {
    let before = inventory(vec![item(13, 1), item(14, 2), item(22, 3)]);
    let after = swap(&set([Some(1), None, None, None]), &before).unwrap();
    // The sword is found where it is and put back; the shield returns.
    assert_eq!(
        placed(&after),
        [(13, 1, None), (22, 3, None), (23, 2, None)]
    );
}

#[test]
fn the_same_sword_in_both_hands_is_found_in_worn_order() {
    // Both hands hold sword 1; the set wants sword 1 in each.
    let before = inventory(vec![item(13, 1), item(14, 1)]);
    let after = swap(&set([Some(1), Some(1), None, None]), &before).unwrap();
    assert_eq!(placed(&after), [(13, 1, None), (14, 1, None)]);
    // The set wants it in the off hand only: the one in 13 moves there, and
    // the one in 14 goes back to the packs.
    let after = swap(&set([None, Some(1), None, None]), &before).unwrap();
    assert_eq!(placed(&after), [(14, 1, None), (22, 1, None)]);
}

#[test]
fn the_bank_and_other_slots_stay_as_they_were() {
    let before = inventory(vec![item(13, 1), item(2000, 5), item(1, 6)]);
    let after = swap(&set([None, None, None, None]), &before).unwrap();
    assert_eq!(
        placed(&after),
        [(1, 6, None), (22, 1, None), (2000, 5, None)]
    );
}

#[test]
fn what_cannot_be_followed_exactly_is_refused() {
    let sword = || item(13, 1);
    let empty = set([None, None, None, None]);
    // No inventory yet.
    assert!(swap(&empty, &Inventory::default()).is_err());
    // Something on the cursor.
    assert_eq!(
        swap(&empty, &inventory(vec![sword(), item(30, 2)])),
        Err("Put down what you hold before swapping sets.")
    );
    // A move in flight.
    let mut moving = inventory(vec![sword()]);
    moving.apply(InventoryUpdate::Prediction(vec![item(14, 1)]));
    assert!(swap(&empty, &moving).is_err());
    // A stale inventory, as an unreadable (augmented) record leaves it.
    let mut stale = inventory(vec![sword()]);
    stale.apply(InventoryUpdate::Invalidated);
    assert!(swap(&empty, &stale).is_err());
    // Nothing to do.
    assert_eq!(
        swap(&empty, &inventory(vec![item(22, 2)])),
        Err("That set has nothing to swap.")
    );
    // No room but the cursor (ours: EQEmu would push it there).
    let mut packs = full_packs(&[]);
    packs.push(sword());
    assert_eq!(
        swap(&empty, &inventory(packs)),
        Err("There is no room to put back what the swap would return.")
    );
    // A pack item whose class is unknown, where a return looks at it.
    let mut unknown = item(22, 2);
    unknown.rules.titanium_item_class = None;
    assert!(swap(&empty, &inventory(vec![unknown, sword()])).is_err());
}

#[test]
fn a_pack_item_that_is_not_a_bag_but_holds_room_or_contents_is_refused() {
    let sword = || item(13, 1);
    let empty = set([None, None, None, None]);
    // A common item with capacity, as the reader keeps it.
    let mut roomy = item(22, 2);
    roomy.bag_slots = 1;
    assert_eq!(swap(&empty, &inventory(vec![roomy, sword()])), Err(UNKNOWN));
    // A common item with a child, which may be an augment.
    assert_eq!(
        swap(&empty, &inventory(vec![item(22, 2), item(251, 3), sword()])),
        Err(UNKNOWN)
    );
    // A real bag with contents goes.
    assert!(swap(
        &empty,
        &inventory(vec![bag(22, 2, 4, 0), item(251, 3), sword()])
    )
    .is_ok());
}

#[test]
fn a_swap_still_waits_after_its_moves_settle() {
    // EQEmu confirms no move, so settled slots keep projected custody
    // until the server sends them again.
    let before = inventory(vec![item(13, 1)]);
    let after = swap(&set([None, None, None, None]), &before).unwrap();
    let mut settled = before;
    settled.apply(InventoryUpdate::Prediction(after));
    settled.apply(InventoryUpdate::Settled);
    assert!(!settled.predicted());
    assert_eq!(
        swap(&set([Some(1), None, None, None]), &settled),
        Err("Wait until your inventory is known to swap sets.")
    );
}

#[test]
fn a_set_naming_an_augment_socketed_in_any_held_item_is_refused() {
    // `HasItem` matches an augment too (6.570 G3), worn, packed or bagged.
    for host in [13, 22, 252] {
        let mut socketed = item(host, 50);
        socketed.augments[1] = Some(77);
        let mut held = vec![socketed, item(23, 77)];
        if host == 252 {
            held.push(bag(22, 90, 4, 0));
        }
        let before = inventory(held);
        assert_eq!(
            swap(&set([Some(77), None, None, None]), &before),
            Err("That set names an augment in one of your items."),
            "{host}"
        );
        assert!(
            swap(&set([Some(50), None, None, None]), &before).is_ok(),
            "{host}"
        );
    }
}
