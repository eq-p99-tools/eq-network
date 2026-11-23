//! A Rogue's poison put on the weapon (`OP_ApplyPoison`): the request
//! names the inventory slot the poison sits in, and the server answers with
//! the same opcode, the slot and whether the poison took.
//!
//! What the servers do, from their source (the program's own side is
//! pending read R49-1, recording checklist 6.493):
//! - `EQEmu`'s `Client::Handle_OP_ApplyPoison` (`zone/client_packet.cpp`)
//!   takes the item in the slot sent, wants item type 42 and a Rogue, says
//!   its own line when the poison is above the player's level, and deletes
//!   one of the item whether it took or not, with its own inventory record.
//!   It answers every request, a refused one with success 0 and nothing
//!   deleted.
//! - TAKP's (`EQMacEmu`) also wants a charge left and a one-handed piercer in
//!   the primary hand; without the piercer it answers success 0 and deletes
//!   nothing.
//!
//! Layout reference: `ApplyPoison_Struct` (`common/eq_packet_structs.h`,
//! `common/patches/titanium_structs.h`), two 32-bit words, the slot and the
//! success. Titanium's translator maps the slot to and from the server's
//! numbering (`common/patches/titanium.cpp`); TAKP's carries its own.
use crate::{command::EncodedCommand, inventory::InventorySlot};
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// Titanium's `OP_ApplyPoison`.
pub const TITANIUM_OPCODE: u16 = 0x0c2c;
/// `EQMac`'s `OP_ApplyPoison`.
pub const EQMAC_OPCODE: u16 = 0xba41;
/// The item type both servers take as a poison (`EQEmu`'s
/// `ItemTypePoison`). Which type the program itself tests, if any, is
/// pending read R49-1 (recording checklist 6.493).
pub const POISON_ITEM_TYPE: u8 = 42;
/// The body's length, both ways.
const LENGTH: usize = 8;

/// The server's answer to a poison put on.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct PoisonAnswer {
    /// Where the poison sat, as the request named it.
    pub slot: InventorySlot,
    /// Whether it took. The servers delete one of the item either way, when
    /// they took the request at all, with their own inventory record.
    pub applied: bool,
}

fn body(slot: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(LENGTH);
    body.extend_from_slice(&slot.to_le_bytes());
    body.extend_from_slice(&0_u32.to_le_bytes());
    body
}

/// The Titanium request for the poison in a slot: the slot and a success
/// word of 0.
///
/// # Errors
/// Refuses a slot that is not one of the player's own.
pub fn titanium_request(slot: InventorySlot) -> Result<EncodedCommand> {
    ensure!(
        slot.is_held(),
        "slot {} holds none of the player's items",
        slot.0
    );
    let slot = u32::try_from(slot.0).context("negative inventory slot")?;
    Ok(EncodedCommand {
        opcode: TITANIUM_OPCODE,
        body: body(slot),
    })
}

/// The `EQMac` request for the poison in a slot, in that generation's slot
/// numbering ([`InventorySlot::to_eqmac`]).
///
/// # Errors
/// Refuses a slot `EQMac` has no number for or that is not the player's own.
pub fn eqmac_request(slot: InventorySlot) -> Result<EncodedCommand> {
    ensure!(
        slot.is_held(),
        "slot {} holds none of the player's items",
        slot.0
    );
    let number = slot
        .to_eqmac()
        .with_context(|| format!("EQMac has no slot {}", slot.0))?;
    let number = u32::try_from(number).context("negative inventory slot")?;
    Ok(EncodedCommand {
        opcode: EQMAC_OPCODE,
        body: body(number),
    })
}

fn words(body: &[u8]) -> Result<(i32, u32)> {
    ensure!(
        body.len() == LENGTH,
        "an answer to a poison is {LENGTH} bytes, not {}",
        body.len()
    );
    let slot = i32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    let success = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
    Ok((slot, success))
}

/// Reads Titanium's answer: the slot and a success word, any value but 0
/// taking (`EQEmu` sends 0 or 1).
///
/// # Errors
/// Rejects any length but 8 bytes.
pub fn titanium_answer(body: &[u8]) -> Result<PoisonAnswer> {
    let (slot, success) = words(body)?;
    Ok(PoisonAnswer {
        slot: InventorySlot(slot),
        applied: success != 0,
    })
}

/// Reads `EQMac`'s answer, its slot taken back to this library's numbering
/// ([`InventorySlot::from_eqmac`]).
///
/// # Errors
/// Rejects any length but 8 bytes, and a slot `EQMac` does not use.
pub fn eqmac_answer(body: &[u8]) -> Result<PoisonAnswer> {
    let (slot, success) = words(body)?;
    let slot =
        InventorySlot::from_eqmac(slot).with_context(|| format!("EQMac has no slot {slot}"))?;
    Ok(PoisonAnswer {
        slot,
        applied: success != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titanium_names_the_slot_the_poison_sits_in() {
        let sent = titanium_request(InventorySlot(251)).unwrap();
        assert_eq!(sent.opcode, 0x0c2c);
        assert_eq!(sent.body, [251, 0, 0, 0, 0, 0, 0, 0]);
        let cursor = titanium_request(InventorySlot::CURSOR).unwrap();
        assert_eq!(cursor.body, [30, 0, 0, 0, 0, 0, 0, 0]);
        assert!(titanium_request(InventorySlot(3000)).is_err());
        assert!(titanium_request(InventorySlot(-1)).is_err());
    }

    #[test]
    fn eqmac_numbers_the_slot_its_own_way() {
        let sent = eqmac_request(InventorySlot(251)).unwrap();
        assert_eq!(sent.opcode, 0xba41);
        assert_eq!(sent.body, [250, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(eqmac_request(InventorySlot::CURSOR).unwrap().body, [0; 8]);
        // The charm has no EQMac number.
        assert!(eqmac_request(InventorySlot(0)).is_err());
    }

    #[test]
    fn answers_name_the_slot_and_whether_it_took() {
        assert_eq!(
            titanium_answer(&[23, 0, 0, 0, 1, 0, 0, 0]).unwrap(),
            PoisonAnswer {
                slot: InventorySlot(23),
                applied: true
            }
        );
        assert!(!titanium_answer(&[23, 0, 0, 0, 0, 0, 0, 0]).unwrap().applied);
        assert!(titanium_answer(&[23, 0, 0, 0, 1, 0, 0]).is_err());
        assert!(titanium_answer(&[0; 9]).is_err());
        assert_eq!(
            eqmac_answer(&[250, 0, 0, 0, 1, 0, 0, 0]).unwrap(),
            PoisonAnswer {
                slot: InventorySlot(251),
                applied: true
            }
        );
        assert_eq!(eqmac_answer(&[0; 8]).unwrap().slot, InventorySlot::CURSOR);
        assert!(eqmac_answer(&[0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn each_generation_hears_the_answer_as_one_event() {
        use crate::world::WorldEvent;
        let answer = PoisonAnswer {
            slot: InventorySlot(251),
            applied: true,
        };
        assert_eq!(
            crate::world::titanium_update(TITANIUM_OPCODE, &[251, 0, 0, 0, 1, 0, 0, 0]).unwrap(),
            Some(WorldEvent::PoisonAnswered(answer))
        );
        assert_eq!(
            crate::quarm::updates(EQMAC_OPCODE, &[250, 0, 0, 0, 1, 0, 0, 0]).unwrap(),
            [WorldEvent::PoisonAnswered(answer)]
        );
        assert!(crate::world::titanium_update(TITANIUM_OPCODE, &[0; 4]).is_err());
    }
}
