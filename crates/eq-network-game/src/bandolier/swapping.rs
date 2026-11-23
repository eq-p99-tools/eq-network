//! What stock `EQEmu` does to the inventory when the player swaps to a
//! bandolier set (6.566): `Client::SetBandolier` and
//! `Client::MoveItemToInventory` (`zone/inventory.cpp`), which make up to
//! eight moves and tell the client none of them. [`swap`] works out every
//! move on a copy of the session's inventory, or refuses when any would
//! need what the session does not know; it is the session's whole
//! projection, never the server's word.
//!
//! `EQEmu`'s comment says it copies the program's own order; which order
//! the program uses is read R49-45 d (inferred until then).
use super::{BandolierSet, SET_ITEMS, WEAPON_SLOTS};
use crate::inventory::{Inventory, InventoryItem, InventorySlot};
use std::collections::BTreeMap;

/// Titanium's first and last carried pack slots, then the cursor, in the
/// order `MoveItemToInventory` looks through them (`GENERAL_BEGIN` to
/// `slotCursor`).
const RETURN_SLOTS: std::ops::RangeInclusive<i32> = 22..=30;
/// The worn slots, in the order `HasItem` looks through them.
const WORN: std::ops::RangeInclusive<i32> = 0..=21;
/// The carried pack slots, in the order `HasItem` looks through them, each
/// one's bag contents straight after it.
const PACKS: std::ops::RangeInclusive<i32> = 22..=29;
/// The set's range and ammo places, whose stack gives up one unit.
const SPLIT_PLACES: [usize; 2] = [2, 3];
/// `ItemClassBag`.
const BAG_CLASS: u8 = 1;
/// `BagTypeQuiver`, which takes only arrows.
const QUIVER: u8 = 2;
/// `BagTypeBandolier`, which takes only small throwing items.
const THROWING_BAG: u8 = 8;
/// `ItemTypeArrow`.
const ARROW: u8 = 27;
/// `ItemTypeSmallThrowing`.
const SMALL_THROWING: u8 = 19;

/// The whole inventory after the swap to `set`, as stock `EQEmu` leaves
/// it, for an [`crate::inventory::InventoryUpdate::Prediction`].
///
/// `HasItem` also matches an augment socketed in an item, and then the
/// weapon slot stays as it is. A set naming an id socketed in any worn or
/// carried item is refused (6.570 G3), and so is a pack item that is not a
/// bag but has room or contents, whose record is not what `EQEmu` holds.
///
/// # Errors
/// Refuses, in words for the player, an inventory that is not known
/// exactly, anything on the cursor, a set with nothing to do, any item the
/// rules would read whose record lacks what they read, and a swap that
/// would put anything on the cursor (ours: `EQEmu` would).
pub fn swap(set: &BandolierSet, inventory: &Inventory) -> Result<Vec<InventoryItem>, &'static str> {
    if !inventory.received()
        || inventory.stale()
        || inventory.predicted()
        || inventory.unresolved_predictions() > 0
        || inventory.queued().next().is_some()
        || !(0..=30)
            .chain(251..=340)
            .all(|slot| inventory.custody_known(InventorySlot(slot)))
    {
        return Err("Wait until your inventory is known to swap sets.");
    }
    let mut items: BTreeMap<i32, InventoryItem> = inventory
        .items()
        .iter()
        .filter(|(slot, _)| slot.is_equipment() || slot.is_carried() || slot.0 == 30)
        .map(|(slot, item)| (slot.0, item.clone()))
        .collect();
    if items.contains_key(&InventorySlot::CURSOR.0) {
        return Err("Put down what you hold before swapping sets.");
    }
    if items.values().any(|item| {
        item.augments.iter().flatten().any(|augment| {
            set.items
                .iter()
                .flatten()
                .any(|named| named.item_id == *augment)
        })
    }) {
        return Err("That set names an augment in one of your items.");
    }
    for slot in PACKS {
        let Some(item) = items.get(&slot) else {
            continue;
        };
        let first = InventorySlot(slot).child(0).ok_or(UNKNOWN)?.0;
        if !is_bag(item)? && (item.bag_slots > 0 || items.range(first..first + 10).next().is_some())
        {
            return Err(UNKNOWN);
        }
    }
    if set.items.iter().all(Option::is_none)
        && WEAPON_SLOTS.iter().all(|slot| !items.contains_key(&slot.0))
    {
        return Err("That set has nothing to swap.");
    }
    let mut found: [Option<InventoryItem>; SET_ITEMS] = Default::default();
    // Pass 1: take each named item out, so the worn ones have room to go back.
    for (place, named) in set.items.iter().enumerate() {
        let Some(named) = named else { continue };
        if let Some(slot) = find(&items, named.item_id)? {
            let mut item = items.remove(&slot).ok_or(UNKNOWN)?;
            if SPLIT_PLACES.contains(&place) {
                if let Some(count) = item.stack_count {
                    if count > 1 {
                        let mut rest = item.clone();
                        rest.stack_count = Some(count - 1);
                        items.insert(slot, rest);
                        item.stack_count = Some(1);
                    }
                }
            }
            found[place] = Some(item);
        } else if let Some(worn) = items.remove(&WEAPON_SLOTS[place].0) {
            give_back(&mut items, worn)?;
        }
    }
    // Pass 2: put each found item in place, and give back what it replaces
    // or what a set's empty place leaves.
    for (place, named) in set.items.iter().enumerate() {
        let weapon = WEAPON_SLOTS[place];
        if named.is_some() {
            if let Some(mut item) = found[place].take() {
                let replaced = items.remove(&weapon.0);
                item.slot = weapon;
                items.insert(weapon.0, item);
                if let Some(replaced) = replaced {
                    give_back(&mut items, replaced)?;
                }
            }
        } else if let Some(worn) = items.remove(&weapon.0) {
            give_back(&mut items, worn)?;
        }
    }
    // The bank and anything else the session holds stay as they were.
    let mut whole: Vec<_> = inventory
        .items()
        .iter()
        .filter(|(slot, _)| !(slot.is_equipment() || slot.is_carried() || slot.0 == 30))
        .map(|(_, item)| item.clone())
        .collect();
    whole.extend(items.into_values());
    whole.sort_by_key(|item| item.slot);
    Ok(whole)
}

/// Why an item's record cannot be followed.
const UNKNOWN: &str = "An item the swap would move is not known well enough.";

fn is_bag(item: &InventoryItem) -> Result<bool, &'static str> {
    item.rules
        .titanium_item_class
        .map(|class| class == BAG_CLASS)
        .ok_or(UNKNOWN)
}

/// Where `HasItem(id, 1, worn | personal)` finds the item: the worn slots
/// in order, then each pack slot and its bag's contents in order.
fn find(items: &BTreeMap<i32, InventoryItem>, id: u32) -> Result<Option<i32>, &'static str> {
    for slot in WORN {
        if items.get(&slot).is_some_and(|item| item.details.id == id) {
            return Ok(Some(slot));
        }
    }
    for slot in PACKS {
        let Some(item) = items.get(&slot) else {
            continue;
        };
        if item.details.id == id {
            return Ok(Some(slot));
        }
        if is_bag(item)? {
            // The bag's contents in its own order, whatever its size.
            let first = InventorySlot(slot).child(0).ok_or(UNKNOWN)?.0;
            if let Some((child, _)) = items
                .range(first..first + 10)
                .find(|(_, inside)| inside.details.id == id)
            {
                return Ok(Some(*child));
            }
        }
    }
    Ok(None)
}

/// `MoveItemToInventory`: top up stacks of the same item first, then the
/// first empty pack slot or room in a bag that takes it, in slot order.
/// The cursor would come last; the session refuses that instead.
fn give_back(
    items: &mut BTreeMap<i32, InventoryItem>,
    mut item: InventoryItem,
) -> Result<(), &'static str> {
    if item.bag_slots > 0 || is_bag(&item)? {
        return Err(UNKNOWN);
    }
    if let Some(mut left) = item.stack_count {
        for slot in RETURN_SLOTS {
            let Some(held) = items.get(&slot) else {
                continue;
            };
            let bag = is_bag(held)?;
            let size = held.bag_slots;
            if top_up(items, slot, &item, &mut left)? {
                return Ok(());
            }
            if bag {
                for index in 0..size {
                    let child = InventorySlot(slot).child(index).ok_or(UNKNOWN)?.0;
                    if top_up(items, child, &item, &mut left)? {
                        return Ok(());
                    }
                }
            }
        }
        item.stack_count = Some(left);
    }
    for slot in RETURN_SLOTS {
        let Some(held) = items.get(&slot) else {
            if slot == InventorySlot::CURSOR.0 {
                break;
            }
            item.slot = InventorySlot(slot);
            items.insert(slot, item);
            return Ok(());
        };
        if is_bag(held)? && fits(&item, held) {
            for index in 0..held.bag_slots {
                let child = InventorySlot(slot).child(index).ok_or(UNKNOWN)?;
                if let std::collections::btree_map::Entry::Vacant(place) = items.entry(child.0) {
                    item.slot = child;
                    place.insert(item);
                    return Ok(());
                }
            }
        }
    }
    Err("There is no room to put back what the swap would return.")
}

/// Adds what fits of the returned stack to the stack in `slot`, if it is
/// the same item below its stack size; true once nothing is left.
fn top_up(
    items: &mut BTreeMap<i32, InventoryItem>,
    slot: i32,
    item: &InventoryItem,
    left: &mut u32,
) -> Result<bool, &'static str> {
    let Some(held) = items.get_mut(&slot) else {
        return Ok(false);
    };
    if held.details.id != item.details.id {
        return Ok(false);
    }
    let count = held.stack_count.ok_or(UNKNOWN)?;
    let most = held.rules.stack_size;
    if most == 0 {
        return Err(UNKNOWN);
    }
    if count < most {
        let moved = (*left).min(most - count);
        held.stack_count = Some(count + moved);
        *left -= moved;
    }
    Ok(*left == 0)
}

/// `CanItemFitInContainer`: the item's size within the bag's, a quiver
/// for arrows only and a bandolier bag for small throwing items only.
fn fits(item: &InventoryItem, bag: &InventoryItem) -> bool {
    item.rules.size <= bag.rules.bag_size
        && (bag.rules.bag_type != QUIVER || item.rules.item_type == ARROW)
        && (bag.rules.bag_type != THROWING_BAG || item.rules.item_type == SMALL_THROWING)
}

#[cfg(test)]
mod tests;
