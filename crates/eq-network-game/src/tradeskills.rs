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
    fn tradeskill_containers_combine_and_bags_do_not() {
        // A backpack, a bandolier, a medicine bag, a sewing kit, a trader's
        // satchel and a tradeskill storage bag.
        let verdicts = [5, 8, 9, 16, 51, 58].map(combines);
        assert_eq!(verdicts, [false, false, true, true, false, false]);
    }
}
