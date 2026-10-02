//! Tradeskill combines in the player's own containers: a sewing kit, a
//! mixing bowl or any other tradeskill container carried in a pack slot
//! combines what it holds when the player asks.
//!
//! The Titanium client sends `OP_TradeSkillCombine` with `NewCombine_Struct`:
//! the container's slot and a guild tribute slot, 16 bits each. `EQEmu`
//! answers with the same opcode and no body once it has judged the combine
//! (`Object::HandleCombine`, `zone/tradeskills.cpp`), whatever the verdict:
//! after a success the components leave the container and what was made
//! arrives on the cursor only after that answer. The verdict itself arrives
//! as an ordinary message.
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

/// The official client's string (`eqstr_us.txt`) refusing a combine while
/// the cursor holds an item or coins: a combine's product lands there.
pub const HANDS_FULL: u32 = 12024;

/// The guild tribute slot a combine names when there is none. `EQEmu` only
/// logs it; what the official client sends here is unrecorded.
const NO_TRIBUTE_SLOT: i16 = -1;

/// Where a combine stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum CombineUpdate {
    /// The session asked the server to combine what the container in this
    /// pack slot holds.
    Started(InventorySlot),
    /// The server judged the combine. What it made and used up arrives as
    /// inventory news, and its verdict as a message.
    Answered,
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
/// Rejects a slot that does not fit the packet's 16 bits.
pub fn titanium_combine(container: InventorySlot) -> Result<EncodedCommand> {
    let slot = i16::try_from(container.0)?;
    let mut body = Vec::with_capacity(4);
    body.extend(slot.to_le_bytes());
    body.extend(NO_TRIBUTE_SLOT.to_le_bytes());
    Ok(EncodedCommand {
        opcode: COMBINE_OPCODE,
        body,
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
    fn a_combine_names_its_container_and_no_tribute_slot() {
        let combine = titanium_combine(InventorySlot(23)).unwrap();
        assert_eq!(combine.opcode, 0x0b40);
        assert_eq!(combine.body, [23, 0, 0xff, 0xff]);
        assert!(titanium_combine(InventorySlot(40_000)).is_err());
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
