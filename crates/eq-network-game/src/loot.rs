//! Titanium corpse looting: request, listing, item transfer and completion.
//!
//! The server answers a request with `OP_MoneyOnCorpse`, one loot-view item packet per
//! item, and an echo of the request. Each item request is echoed back as its
//! acknowledgement; the item itself then arrives as an ordinary inventory update.
use crate::inventory::InventoryItem;
use crate::world::Coins;
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_LootRequest`, a four-byte corpse ID in both directions.
pub const REQUEST_OPCODE: u16 = 0x6f90;
/// `OP_MoneyOnCorpse`: the response code and coins given to the looter.
pub const MONEY_OPCODE: u16 = 0x7fe4;
/// `OP_LootItem`: a 16-byte request, echoed as its acknowledgement.
pub const ITEM_OPCODE: u16 = 0x7081;
/// `OP_EndLootRequest`: closes the loot session.
pub const END_OPCODE: u16 = 0x2316;
/// `OP_LootComplete`: the server closed the loot window.
pub const COMPLETE_OPCODE: u16 = 0x0a94;
/// `ItemPacketLoot` inside `OP_ItemPacket`.
pub const ITEM_PACKET_KIND: u32 = 0x66;
/// The server's slot for a corpse's first place on the Titanium wire:
/// `EQEmu`'s `CORPSE_BEGIN`. That the official client's loot window shows it
/// first is inferred from that numbering and every live loot so far.
const FIRST_CORPSE_SLOT: u16 = 22;

/// Why a corpse can or cannot be looted.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum LootResponse {
    /// Someone else is looting.
    SomeoneElse,
    /// Looting may proceed.
    Normal,
    /// The corpse cannot be looted yet.
    NotAtThisTime,
    /// Nearby hostile creatures prevent looting.
    Hostiles,
    /// The corpse is too far away.
    TooFar,
    /// Unrecognized code, kept for diagnostics.
    Other(u8),
}

impl From<u8> for LootResponse {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::SomeoneElse,
            1 | 3 | 6 => Self::Normal,
            2 => Self::NotAtThisTime,
            4 => Self::Hostiles,
            5 => Self::TooFar,
            other => Self::Other(other),
        }
    }
}

/// Server-driven loot session changes.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum LootUpdate {
    /// Response to a loot request, with the coins handed over.
    Opened {
        /// Whether looting may proceed.
        response: LootResponse,
        /// Coins moved from the corpse to the looter.
        coins: Coins,
    },
    /// One item on the corpse, at its place there, from 0.
    Item {
        /// The item's place on the corpse, from 0.
        place: u16,
        /// The item, with the server's slot for it.
        item: Box<InventoryItem>,
    },
    /// Every item has been listed for this corpse.
    Listed {
        /// Corpse being looted.
        corpse_id: u16,
    },
    /// Acknowledgement of one item request.
    Taken {
        /// The corpse's place that was requested, from 0.
        place: u16,
        /// False when the server refused the item.
        accepted: bool,
    },
    /// The loot window closed.
    Closed,
}

/// Encodes a request to open a corpse.
///
/// # Errors
/// Rejects the reserved zero ID.
pub fn request(corpse_id: u16) -> Result<[u8; 4]> {
    ensure!(corpse_id != 0, "loot requires a corpse");
    Ok(u32::from(corpse_id).to_le_bytes())
}

/// Encodes a request for the item at one place on the corpse, from 0;
/// `auto` places it in the inventory directly.
///
/// # Errors
/// Rejects reserved zero IDs and places past the wire's slots.
pub fn item_request(corpse_id: u16, looter_id: u16, place: u16, auto: bool) -> Result<[u8; 16]> {
    ensure!(
        corpse_id != 0 && looter_id != 0,
        "loot requires two entities"
    );
    let slot = FIRST_CORPSE_SLOT
        .checked_add(place)
        .ok_or_else(|| anyhow::anyhow!("no such place on a corpse"))?;
    let mut body = [0; 16];
    body[..4].copy_from_slice(&u32::from(corpse_id).to_le_bytes());
    body[4..8].copy_from_slice(&u32::from(looter_id).to_le_bytes());
    body[8..10].copy_from_slice(&slot.to_le_bytes());
    body[12..].copy_from_slice(&i32::from(auto).to_le_bytes());
    Ok(body)
}

/// Encodes the end of a loot session.
///
/// # Errors
/// Rejects the reserved zero ID.
pub fn end(corpse_id: u16) -> Result<[u8; 4]> {
    request(corpse_id)
}

/// Decodes loot packets; other opcodes and item packet kinds return None.
///
/// # Errors
/// Rejects malformed recognized packets.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<LootUpdate>> {
    Ok(Some(match opcode {
        MONEY_OPCODE => {
            ensure!(body.len() == 20, "invalid corpse money length");
            let word = |offset: usize| {
                u32::from_le_bytes([
                    body[offset],
                    body[offset + 1],
                    body[offset + 2],
                    body[offset + 3],
                ])
            };
            LootUpdate::Opened {
                response: body[0].into(),
                coins: Coins {
                    platinum: word(4),
                    gold: word(8),
                    silver: word(12),
                    copper: word(16),
                },
            }
        }
        REQUEST_OPCODE => {
            ensure!(body.len() == 4, "invalid loot request echo length");
            let corpse_id =
                u16::try_from(u32::from_le_bytes([body[0], body[1], body[2], body[3]]))?;
            LootUpdate::Listed { corpse_id }
        }
        ITEM_OPCODE => {
            ensure!(body.len() == 16, "invalid loot item acknowledgement length");
            LootUpdate::Taken {
                place: place(u16::from_le_bytes([body[8], body[9]]))?,
                accepted: i32::from_le_bytes([body[12], body[13], body[14], body[15]]) >= 0,
            }
        }
        COMPLETE_OPCODE => LootUpdate::Closed,
        0x3397 if body.get(..4) == Some(&ITEM_PACKET_KIND.to_le_bytes()) => {
            let mut items = crate::inventory::parse_items(&body[4..])?;
            ensure!(items.len() == 1, "loot view must hold one item");
            let item = items.remove(0);
            LootUpdate::Item {
                place: place(u16::try_from(item.slot.0)?)?,
                item: Box::new(item),
            }
        }
        _ => return Ok(None),
    }))
}

/// A corpse's place, from 0, for the server's slot for it.
fn place(slot: u16) -> Result<u16> {
    slot.checked_sub(FIRST_CORPSE_SLOT)
        .ok_or_else(|| anyhow::anyhow!("corpse slot {slot} lies before the first place"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loot_requests_use_titanium_layouts() {
        assert_eq!(request(513).unwrap(), [1, 2, 0, 0]);
        assert!(request(0).is_err());
        // The second place on the corpse is the server's slot 23.
        let item = item_request(513, 7, 1, true).unwrap();
        assert_eq!(&item[..10], &[1, 2, 0, 0, 7, 0, 0, 0, 23, 0]);
        assert_eq!(&item[12..], &[1, 0, 0, 0]);
        assert_eq!(&item_request(513, 7, 1, false).unwrap()[12..], &[0; 4]);
        assert!(item_request(513, 0, 1, true).is_err());
        assert!(item_request(513, 7, u16::MAX, true).is_err());
    }

    #[test]
    fn money_listing_acknowledgement_and_completion_decode() {
        let mut money = [0u8; 20];
        money[0] = 1;
        money[1] = 0x5a;
        money[4..8].copy_from_slice(&2u32.to_le_bytes());
        money[16..20].copy_from_slice(&9u32.to_le_bytes());
        assert_eq!(
            decode(MONEY_OPCODE, &money).unwrap(),
            Some(LootUpdate::Opened {
                response: LootResponse::Normal,
                coins: Coins {
                    platinum: 2,
                    gold: 0,
                    silver: 0,
                    copper: 9,
                },
            })
        );
        money[0] = 5;
        assert!(matches!(
            decode(MONEY_OPCODE, &money).unwrap(),
            Some(LootUpdate::Opened {
                response: LootResponse::TooFar,
                ..
            })
        ));
        assert!(decode(MONEY_OPCODE, &money[..19]).is_err());
        assert_eq!(
            decode(REQUEST_OPCODE, &[1, 2, 0, 0]).unwrap(),
            Some(LootUpdate::Listed { corpse_id: 513 })
        );
        let mut refused = item_request(513, 7, 1, true).unwrap();
        assert_eq!(
            decode(ITEM_OPCODE, &refused).unwrap(),
            Some(LootUpdate::Taken {
                place: 1,
                accepted: true
            })
        );
        refused[12..].copy_from_slice(&(-1i32).to_le_bytes());
        assert_eq!(
            decode(ITEM_OPCODE, &refused).unwrap(),
            Some(LootUpdate::Taken {
                place: 1,
                accepted: false
            })
        );
        // A slot before the corpse's first is no place on it.
        refused[8..10].copy_from_slice(&21u16.to_le_bytes());
        assert!(decode(ITEM_OPCODE, &refused).is_err());
        assert_eq!(
            decode(COMPLETE_OPCODE, &[]).unwrap(),
            Some(LootUpdate::Closed)
        );
        assert_eq!(decode(0x3397, &0x67u32.to_le_bytes()).unwrap(), None);
        assert_eq!(decode(0x1234, &[]).unwrap(), None);
    }
}
