//! `EQMac` corpse access, source-derived from Quarm c2cd265:
//! `common/eq_packet_structs.h`, `common/patches/mac.cpp`,
//! `utils/patches/patch_Mac.conf` and `zone/corpse.cpp`.
//!
//! The open echo ends the list; the item echo precedes the server's checks.
//! Coins in a successful open are already transferred. Item delivery comes
//! separately through the inventory decoder, including a bag's contents.
use super::{LootResponse, LootUpdate};
use crate::{inventory, world::Coins};
use anyhow::{ensure, Result};

/// Open request and the echo ending a corpse's listing.
pub const REQUEST_OPCODE: u16 = 0x4e40;
/// Coins transferred, or the reason opening failed.
pub const MONEY_OPCODE: u16 = 0x5040;
/// Item request and its unconditional receipt echo.
pub const ITEM_OPCODE: u16 = 0xa040;
/// Request to close the corpse.
pub const END_OPCODE: u16 = 0x4f40;
/// The server closed the loot window, including on item failure.
pub const COMPLETE_OPCODE: u16 = 0x4541;
/// An item displayed on the corpse; never carried inventory.
pub const VIEW_OPCODE: u16 = 0x5240;

/// Encodes an open or close request's 16-bit corpse ID.
///
/// # Errors
/// Rejects the reserved zero ID.
pub fn request(corpse_id: u16) -> Result<[u8; 2]> {
    ensure!(corpse_id != 0, "loot requires a corpse");
    Ok(corpse_id.to_le_bytes())
}

/// Encodes an item request with the listed place unchanged. The server
/// chooses the destination when `auto` is true; otherwise it uses the cursor.
///
/// # Errors
/// Rejects zero entity IDs and the server's reserved unlisted-place sentinel.
pub fn item_request(corpse_id: u16, looter_id: u16, place: u16, auto: bool) -> Result<[u8; 12]> {
    ensure!(
        corpse_id != 0 && looter_id != 0,
        "loot requires two entities"
    );
    ensure!(place != u16::MAX, "unlisted corpse place");
    let mut body = [0; 12];
    body[..2].copy_from_slice(&corpse_id.to_le_bytes());
    body[2..4].copy_from_slice(&looter_id.to_le_bytes());
    body[4..6].copy_from_slice(&place.to_le_bytes());
    body[8..].copy_from_slice(&u32::from(auto).to_le_bytes());
    Ok(body)
}

/// Decodes `EQMac` corpse messages; unrelated opcodes return None.
///
/// # Errors
/// Rejects malformed recognized packets and negative item-view places.
pub fn decode(opcode: u16, body: &[u8], rules: inventory::ItemRules) -> Result<Option<LootUpdate>> {
    Ok(Some(match opcode {
        MONEY_OPCODE => {
            ensure!(body.len() == 20, "invalid EQMac corpse money length");
            let word = |offset| {
                u32::from_le_bytes([
                    body[offset],
                    body[offset + 1],
                    body[offset + 2],
                    body[offset + 3],
                ])
            };
            // Quarm's corpse source uses only 0, 1 and 2. Do not apply
            // Titanium's other success codes to an unread EQMac response.
            let response = match body[0] {
                0 => LootResponse::SomeoneElse,
                1 => LootResponse::Normal,
                2 => LootResponse::NotAtThisTime,
                other => LootResponse::Other(other),
            };
            LootUpdate::Opened {
                response,
                coins: Coins {
                    platinum: word(4),
                    gold: word(8),
                    silver: word(12),
                    copper: word(16),
                },
            }
        }
        REQUEST_OPCODE => {
            ensure!(body.len() == 2, "invalid EQMac loot request echo length");
            let corpse_id = u16::from_le_bytes([body[0], body[1]]);
            ensure!(corpse_id != 0, "empty EQMac corpse ID");
            LootUpdate::Listed { corpse_id }
        }
        ITEM_OPCODE => {
            ensure!(body.len() == 12, "invalid EQMac loot item echo length");
            let place = u16::from_le_bytes([body[4], body[5]]);
            ensure!(place != u16::MAX, "unlisted corpse place");
            LootUpdate::Acknowledged { place }
        }
        COMPLETE_OPCODE => {
            ensure!(body.is_empty(), "invalid EQMac loot completion length");
            LootUpdate::Closed
        }
        VIEW_OPCODE => {
            let (place, item) = loot_view(body, rules)?;
            LootUpdate::Item {
                place,
                item: Box::new(item),
            }
        }
        _ => return Ok(None),
    }))
}

/// The raw place is not an inventory slot. Read it without translating it
/// through the carried/cursor slot vocabulary, then reuse the item reader.
fn loot_view(body: &[u8], rules: inventory::ItemRules) -> Result<(u16, inventory::InventoryItem)> {
    ensure!(body.len() == 360, "invalid EQMac loot item length");
    let place = u16::try_from(i16::from_le_bytes([body[184], body[185]]))?;
    let item = inventory::eqmac_record_at(body, inventory::InventorySlot(i32::from(place)), rules)?;
    Ok((place, item))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(opcode: u16, body: &[u8]) -> Result<Option<LootUpdate>> {
        super::decode(opcode, body, inventory::ItemRules::Quarm)
    }

    #[test]
    fn requests_and_unconditional_echo_use_eqmac_layout() {
        assert_eq!(request(513).unwrap(), [1, 2]);
        assert!(request(0).is_err());
        let packet = item_request(513, 7, 0, true).unwrap();
        assert_eq!(packet, [1, 2, 7, 0, 0, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(
            decode(ITEM_OPCODE, &packet).unwrap(),
            Some(LootUpdate::Acknowledged { place: 0 })
        );
        assert!(item_request(513, 0, 0, false).is_err());
        assert!(item_request(513, 7, u16::MAX, false).is_err());
        assert_eq!(
            decode(REQUEST_OPCODE, &[1, 2]).unwrap(),
            Some(LootUpdate::Listed { corpse_id: 513 })
        );
        assert_eq!(
            decode(COMPLETE_OPCODE, &[]).unwrap(),
            Some(LootUpdate::Closed)
        );
        for (opcode, body) in [
            (REQUEST_OPCODE, vec![0; 4]),
            (ITEM_OPCODE, vec![0; 16]),
            (MONEY_OPCODE, vec![0; 19]),
            (COMPLETE_OPCODE, vec![0]),
        ] {
            assert!(decode(opcode, &body).is_err());
        }
    }

    #[test]
    fn money_response_does_not_assume_titanium_success_codes() {
        let mut body = [0; 20];
        body[4..8].copy_from_slice(&2u32.to_le_bytes());
        for (code, response) in [
            (0, LootResponse::SomeoneElse),
            (1, LootResponse::Normal),
            (2, LootResponse::NotAtThisTime),
            (3, LootResponse::Other(3)),
        ] {
            body[0] = code;
            assert_eq!(
                decode(MONEY_OPCODE, &body).unwrap(),
                Some(LootUpdate::Opened {
                    response,
                    coins: Coins {
                        platinum: 2,
                        ..Coins::default()
                    }
                })
            );
        }
    }

    #[test]
    fn views_keep_raw_places_and_server_item_rules_without_entering_inventory() {
        use crate::{
            inventory::ItemRules,
            message::{self, Message},
            world::WorldEvent,
        };
        let mut record = [0; 360];
        record[..7].copy_from_slice(b"Fixture");
        record[180..182].copy_from_slice(&101u16.to_le_bytes());
        record[246] = 20;
        record[253] = 14;
        record[278] = 5;
        for (rules, count) in [(ItemRules::Quarm, None), (ItemRules::Takp, Some(5))] {
            assert!(
                matches!(&message::eqmac_with(VIEW_OPCODE, &record, rules)[..], [Message::Event(WorldEvent::Loot(LootUpdate::Item { place: 0, item }))] if item.slot == inventory::InventorySlot(0) && item.stack_count == count)
            );
            assert!(inventory::decode_eqmac_with(VIEW_OPCODE, &record, rules)
                .unwrap()
                .is_none());
        }
        record[276] = 1;
        assert!(
            matches!(decode(VIEW_OPCODE, &record).unwrap(), Some(LootUpdate::Item { place: 0, item }) if item.stack_count == Some(5))
        );
        record[184..186].copy_from_slice(&(-1i16).to_le_bytes());
        assert!(decode(VIEW_OPCODE, &record).is_err());
        assert!(decode(VIEW_OPCODE, &record[..359]).is_err());
        assert!(matches!(
            &message::eqmac_with(VIEW_OPCODE, &record, ItemRules::Quarm)[..],
            [Message::Unreadable {
                part: message::Part::World,
                ..
            }]
        ));
    }
}
