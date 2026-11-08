//! Tradeskill combines in the player's own containers: a sewing kit, a
//! mixing bowl or any other tradeskill container carried in a pack slot
//! combines what it holds when the player asks.
//!
//! The Titanium client sends `OP_TradeSkillCombine` with a four-byte slot.
//! Its sender writes the whole word; the upper half is zero for ordinary
//! carried slots, rather than an invented no-tribute sentinel. `EQEmu`
//! answers with the same opcode and no body to reset the local transaction
//! gate (`Object::HandleCombine`, `zone/tradeskills.cpp`). The native receiver
//! only resets that gate: it establishes neither a verdict nor consumption.
//! Separate inventory updates supply components and products, and ordinary
//! server messages supply the verdict.
mod titanium;
pub use titanium::{check_carried, CarriedRefusal};

use crate::{
    command::EncodedCommand,
    inventory::{InventoryItem, InventorySlot},
};
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_TradeSkillCombine`, both ways.
pub const COMBINE_OPCODE: u16 = 0x0b40;

/// The slot a combine names for the world container open for the player,
/// such as a forge (`SLOT_TRADESKILL_EXPERIMENT_COMBINE`).
pub const WORLD_CONTAINER: InventorySlot = InventorySlot(1000);

/// The ordinary forge bag type, distinct from cultural and quest forges.
pub const FORGE: u8 = 17;

/// The official client's string (`eqstr_us.txt`) refusing a combine while
/// the cursor holds an item or coins: a combine's product lands there.
pub const HANDS_FULL: u32 = 12024;

/// Where a combine stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum CombineUpdate {
    /// The session asked the server to combine what the container in this
    /// pack slot holds.
    Started(InventorySlot),
    /// A received transaction reset. This does not establish success or
    /// consumption; items and the verdict arrive separately.
    Answered,
}

/// The offer needed for combining in this location. World stations are
/// separate from carried manual combines, even on the same wire generation.
#[must_use]
pub const fn combine_capability(container: InventorySlot) -> crate::world::Capability {
    if container.0 == WORLD_CONTAINER.0 {
        crate::world::Capability::WorldContainers
    } else {
        crate::world::Capability::Tradeskills
    }
}

/// Whether a container of this bag type combines what it holds: the
/// tradeskill and quest containers, from the medicine bag (9) on, as
/// `EQEmu`'s `BagType` numbers them. Plain bags, quivers, pouches, chests and
/// bandoliers (0 to 8) only hold things, as do the trader's satchel (51) and
/// the later tradeskill and collectible storage bags (58 and 59).
#[must_use]
pub const fn combines(bag_type: u8) -> bool {
    matches!(bag_type, 9..=57) && bag_type != 51
}

/// The installed client's string (`eqstr_us.txt`) naming a container
/// type, such as an oven's, as `EQEmu`'s `BagType` lists them; None for a
/// type it has no name for. A world container whose server sends no name
/// can show this one.
#[must_use]
pub const fn type_name(bag_type: u8) -> Option<u32> {
    let bag_type = bag_type as u32;
    Some(match bag_type {
        0..=7 => 3400 + bag_type,
        9..=27 => 3399 + bag_type,
        30..=36 => 3397 + bag_type,
        38..=40 => 3396 + bag_type,
        41 => 3439,
        42 => 3438,
        43 => 3440,
        44 => 3441,
        45 => 3437,
        46 => 3442,
        47 => 3443,
        48 => 3445,
        49 => 3444,
        50 => 3446,
        52 => 5785,
        53 => 3359,
        55 => 6325,
        56 => 6340,
        57 => 5400,
        58 => 7684,
        59 => 7692,
        _ => return None,
    })
}

/// Whether the player can combine in this item: a tradeskill container
/// carried in a pack slot. A container in the bank or in another's hands
/// does not combine.
#[must_use]
pub fn can_combine_in(item: &InventoryItem) -> bool {
    item.slot.is_pack() && item.bag_slots > 0 && combines(item.rules.bag_type)
}

/// Titanium's `OP_TradeSkillCombine` for the container in a pack slot.
///
/// # Errors
/// Rejects anything other than a carried pack or the open world container.
pub fn titanium_combine(container: InventorySlot) -> Result<EncodedCommand> {
    ensure!(
        container.is_pack() || container == WORLD_CONTAINER,
        "a combine needs a carried pack or an open world container"
    );
    Ok(EncodedCommand {
        opcode: COMBINE_OPCODE,
        body: container.0.to_le_bytes().to_vec(),
    })
}

/// The server's answer to a combine.
///
/// # Errors
/// Rejects an answer with a body.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<CombineUpdate>> {
    if opcode != COMBINE_OPCODE {
        return Ok(None);
    }
    ensure!(body.is_empty(), "invalid combine answer length");
    Ok(Some(CombineUpdate::Answered))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_combine_writes_the_entire_native_slot_word() {
        let combine = titanium_combine(InventorySlot(23)).unwrap();
        assert_eq!(combine.opcode, 0x0b40);
        assert_eq!(combine.body, [23, 0, 0, 0]);
        assert_eq!(
            titanium_combine(WORLD_CONTAINER).unwrap().body,
            [0xe8, 3, 0, 0]
        );
        for slot in [-1, 0, 21, 30, 251, 2000, 40_000] {
            assert!(titanium_combine(InventorySlot(slot)).is_err());
        }
    }

    #[test]
    fn the_answer_has_no_body() {
        assert_eq!(decode(0x0b40, &[]).unwrap(), Some(CombineUpdate::Answered));
        assert!(decode(0x0b40, &[1]).is_err());
        assert_eq!(decode(0x1496, &[]).unwrap(), None);
    }

    #[test]
    fn container_types_name_their_strings() {
        // A small bag, a medicine bag, an oven, a forge, the Always Works
        // container, Freeport's forge, Halfling tailoring, a tackle box, and
        // a bandolier, which has none.
        let names = [0, 9, 15, 17, 30, 39, 41, 46, 8].map(type_name);
        assert_eq!(
            names,
            [
                Some(3400),
                Some(3408),
                Some(3414),
                Some(3416),
                Some(3427),
                Some(3435),
                Some(3439),
                Some(3442),
                None
            ]
        );
    }

    #[test]
    fn tradeskill_containers_combine_and_bags_do_not() {
        // A backpack, a bandolier, a medicine bag, a sewing kit, a trader's
        // satchel and a tradeskill storage bag.
        let verdicts = [5, 8, 9, 16, 51, 58].map(combines);
        assert_eq!(verdicts, [false, false, true, true, false, false]);
    }
}
