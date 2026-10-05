//! Merchants on the `EQMac` wire, as TAKP sends them: `common/patches/mac.cpp`
//! puts each in `common/patches/mac_structs.h`'s layout, with 16-bit IDs and
//! slots, and passes slots through unchanged (`MacToServerSlot`), so a sale
//! names the player's slot in `EQMac`'s numbers. The opcodes are
//! `utils/patches/patch_Mac.conf`'s, their bytes swapped.
use super::{MerchantItem, MerchantUpdate};
use crate::inventory::InventorySlot;
use anyhow::{ensure, Context, Result};

/// `OP_ShopRequest`: the client asking to open a merchant, and the answer.
pub const EQMAC_REQUEST_OPCODE: u16 = 0x0b40;
/// `OP_ShopInventoryPacket`: the merchant's whole list.
pub const EQMAC_STOCK_OPCODE: u16 = 0x0c40;
/// `OP_ShopDelItem`: an item gone from the list.
pub const EQMAC_DELETE_OPCODE: u16 = 0x3840;
/// `OP_ShopPlayerBuy`: a purchase, and its echo.
pub const EQMAC_BUY_OPCODE: u16 = 0x3540;
/// `OP_ShopPlayerSell`: a sale, and its echo.
pub const EQMAC_SELL_OPCODE: u16 = 0x2740;
/// `OP_ShopEnd`: the client closing the window.
pub const EQMAC_END_OPCODE: u16 = 0x3740;
/// `OP_ShopEndConfirm`: the server closing it.
pub const EQMAC_END_CONFIRM_OPCODE: u16 = 0x4641;

/// `EQMac`'s request opening a merchant (`Merchant_Click_Struct`, 12 bytes):
/// the merchant, the player, 1 to open, three unknown bytes and a rate.
/// TAKP reads only the merchant (`Client::Handle_OP_ShopRequest`); the
/// unknown bytes go as 0 and the rate as 1, as on Titanium (inferred: the
/// official client's are unrecorded).
///
/// # Errors
/// Rejects reserved zero IDs.
pub fn eqmac_request(merchant_id: u16, player_id: u16) -> Result<[u8; 12]> {
    ensure!(
        merchant_id != 0 && player_id != 0,
        "shopping requires two entities"
    );
    let mut body = [0; 12];
    body[..2].copy_from_slice(&merchant_id.to_le_bytes());
    body[2..4].copy_from_slice(&player_id.to_le_bytes());
    body[4] = 1;
    body[8..].copy_from_slice(&1.0f32.to_le_bytes());
    Ok(body)
}

/// `EQMac`'s request closing the merchant window: the merchant and the
/// player, 16 bits each. TAKP reads nothing in it
/// (`Client::Handle_OP_ShopEnd`; inferred: the official client's body is
/// unrecorded).
///
/// # Errors
/// Rejects reserved zero IDs.
pub fn eqmac_end(merchant_id: u16, player_id: u16) -> Result<[u8; 4]> {
    ensure!(
        merchant_id != 0 && player_id != 0,
        "shopping requires two entities"
    );
    let mut body = [0; 4];
    body[..2].copy_from_slice(&merchant_id.to_le_bytes());
    body[2..].copy_from_slice(&player_id.to_le_bytes());
    Ok(body)
}

/// `EQMac`'s purchase (`Merchant_Sell_Struct`, 16 bytes): the merchant, the
/// player, the item's place on the list, a sold flag and a pad byte, the
/// count, three pad bytes and a price. TAKP prices the purchase itself
/// (`Client::Handle_OP_ShopPlayerBuy`), so the price goes as 0 (inferred:
/// the official client's is unrecorded).
///
/// # Errors
/// Rejects zero IDs, a place past 16 bits and a count of 0 or past 255.
pub fn eqmac_buy(merchant_id: u16, player_id: u16, slot: u32, quantity: u32) -> Result<[u8; 16]> {
    ensure!(
        merchant_id != 0 && player_id != 0,
        "shopping requires two entities"
    );
    let place = u16::try_from(slot).context("merchant place past EQMac's numbers")?;
    let count = count(quantity, "buy")?;
    let mut body = [0; 16];
    body[..2].copy_from_slice(&merchant_id.to_le_bytes());
    body[2..4].copy_from_slice(&player_id.to_le_bytes());
    body[4..6].copy_from_slice(&place.to_le_bytes());
    body[8] = count;
    Ok(body)
}

/// `EQMac`'s sale (`Merchant_Purchase_Struct`, 16 bytes): the merchant, the
/// player, the item's slot in `EQMac`'s numbers, a price, the count and
/// seven pad bytes. TAKP reads neither the player nor the price (its
/// decoder leaves the player out, and `Client::Handle_OP_ShopPlayerSell`
/// prices the sale), so both go as 0 (inferred: the official client's are
/// unrecorded).
///
/// # Errors
/// Rejects a zero merchant ID, a slot `EQMac` has no number for, and a count
/// of 0 or past 255.
pub fn eqmac_sell(merchant_id: u16, slot: i32, quantity: u32) -> Result<[u8; 16]> {
    ensure!(merchant_id != 0, "shopping requires a merchant");
    let mac = InventorySlot(slot)
        .to_eqmac()
        .and_then(|slot| u16::try_from(slot).ok())
        .context("no EQMac inventory slot")?;
    let count = count(quantity, "sell")?;
    let mut body = [0; 16];
    body[..2].copy_from_slice(&merchant_id.to_le_bytes());
    body[4..6].copy_from_slice(&mac.to_le_bytes());
    body[8] = count;
    Ok(body)
}

/// A count of units, which `EQMac` carries in a byte.
fn count(quantity: u32, verb: &str) -> Result<u8> {
    u8::try_from(quantity)
        .ok()
        .filter(|count| *count != 0)
        .with_context(|| format!("{verb} between 1 and 255 units"))
}

/// Decodes `EQMac`'s merchant packets; other opcodes give none.
///
/// The open answer says whether the merchant trades, and the rate it
/// charges at (`CalcPriceMod`): a purchase costs a list price times it, and
/// a sale pays the item's price divided by it. The list comes whole, each
/// price before that rate (`Client::BulkSendMerchantInventory`, where
/// `EQEmu`'s list prices carry it), and with no counts, so each reads as 0,
/// as an unlimited count does on Titanium. A refused purchase is echoed
/// with nothing bought. A sale's echo names the player's slot in `EQMac`'s
/// numbers and prices nothing: `Client::Handle_OP_ShopPlayerSell` fills
/// the 16-bit `OldMerchant_Purchase_Struct`, which `ENCODE(OP_ShopPlayerSell)`
/// reads as the 32-bit `Merchant_Purchase_Struct`, so the price it sends
/// comes from the padding (inferred from the code). The field is read as
/// it comes.
///
/// # Errors
/// Rejects malformed lengths and lists, an open answer whose rate is not
/// finite and above 0 (a guessed rate could send a purchase TAKP refuses
/// for want of coins), and a sale from a slot `EQMac` does not number.
pub fn decode_eqmac(opcode: u16, body: &[u8]) -> Result<Option<Vec<MerchantUpdate>>> {
    let short = |at: usize| u16::from_le_bytes([body[at], body[at + 1]]);
    Ok(Some(match opcode {
        EQMAC_REQUEST_OPCODE => {
            ensure!(body.len() == 12, "invalid EQMac merchant answer length");
            let rate = f32::from_le_bytes([body[8], body[9], body[10], body[11]]);
            ensure!(
                rate.is_finite() && rate > 0.0,
                "invalid EQMac merchant rate"
            );
            vec![MerchantUpdate::Opened {
                merchant_id: short(0),
                accepted: body[4] == 1,
                rate,
            }]
        }
        EQMAC_STOCK_OPCODE => crate::inventory::eqmac_merchant_stock(body)?
            .into_iter()
            .map(|(item, slot, price)| {
                MerchantUpdate::Item(Box::new(MerchantItem {
                    slot,
                    price,
                    quantity: 0,
                    item,
                }))
            })
            .collect(),
        EQMAC_DELETE_OPCODE => {
            // The merchant, the player, the place, then a type of 0x40.
            ensure!(body.len() == 6, "invalid EQMac stock removal length");
            vec![MerchantUpdate::Removed {
                slot: u32::from(body[4]),
            }]
        }
        EQMAC_BUY_OPCODE => {
            ensure!(body.len() == 16, "invalid EQMac purchase echo length");
            vec![MerchantUpdate::Bought {
                slot: u32::from(short(4)),
                quantity: u32::from(body[8]),
                price: u32::from_le_bytes([body[12], body[13], body[14], body[15]]),
            }]
        }
        EQMAC_SELL_OPCODE => {
            ensure!(body.len() == 16, "invalid EQMac sale echo length");
            let slot = InventorySlot::from_eqmac(i32::from(i16::from_le_bytes([body[4], body[5]])))
                .context("unknown EQMac inventory slot")?;
            vec![MerchantUpdate::Sold {
                slot: slot.0,
                quantity: u32::from(body[8]),
                price: u32::from(short(6)),
            }]
        }
        EQMAC_END_CONFIRM_OPCODE => vec![MerchantUpdate::Closed],
        _ => return Ok(None),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A listed record of a made-up common item, `id`, at `place` on the
    /// list for `price` before the rate.
    fn listed(id: i16, place: i16, price: i32) -> Vec<u8> {
        let mut record = vec![0; 362];
        // Tagged with its class, common (0).
        let item = &mut record[2..];
        item[..14].copy_from_slice(b"Synthetic item");
        // Neither no rent nor no drop.
        item[175] = 1;
        item[176] = 1;
        item[180..182].copy_from_slice(&id.to_le_bytes());
        item[184..186].copy_from_slice(&place.to_le_bytes());
        item[192..196].copy_from_slice(&price.to_le_bytes());
        record
    }

    /// The list as TAKP sends it: a count, a byte, then the records
    /// compressed.
    fn stock(records: &[Vec<u8>]) -> Vec<u8> {
        let mut body = vec![u8::try_from(records.len()).unwrap(), 0];
        body.extend(miniz_oxide::deflate::compress_to_vec_zlib(
            &records.concat(),
            4,
        ));
        body
    }

    #[test]
    fn requests_are_takps_sixteen_bit_layouts() {
        let open = eqmac_request(900, 7).unwrap();
        assert_eq!(open[..8], [132, 3, 7, 0, 1, 0, 0, 0]);
        assert_eq!(open[8..], 1.0f32.to_le_bytes());
        assert_eq!(eqmac_end(900, 7).unwrap(), [132, 3, 7, 0]);
        assert_eq!(
            eqmac_buy(900, 7, 3, 2).unwrap(),
            [132, 3, 7, 0, 3, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]
        );
        // A sale names the slot in EQMac's numbers: the second bag's first
        // place, 262 here, is 261 there.
        assert_eq!(
            eqmac_sell(900, 262, 1).unwrap(),
            [132, 3, 0, 0, 5, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(eqmac_sell(900, 23, 20).unwrap()[4..6], [23, 0]);
        assert!(eqmac_request(0, 7).is_err());
        assert!(eqmac_end(900, 0).is_err());
        assert!(eqmac_buy(900, 7, 3, 0).is_err());
        assert!(eqmac_buy(900, 7, 3, 256).is_err());
        assert!(eqmac_buy(900, 7, 0x1_0000, 1).is_err());
        // The charm has no EQMac number.
        assert!(eqmac_sell(900, 0, 1).is_err());
        assert!(eqmac_sell(0, 23, 1).is_err());
    }

    #[test]
    fn the_answer_list_echoes_and_closing_decode() {
        let mut answer = eqmac_request(900, 7).unwrap();
        answer[8..].copy_from_slice(&1.25f32.to_le_bytes());
        assert_eq!(
            decode_eqmac(EQMAC_REQUEST_OPCODE, &answer).unwrap(),
            Some(vec![MerchantUpdate::Opened {
                merchant_id: 900,
                accepted: true,
                rate: 1.25
            }])
        );
        answer[4] = 0;
        assert!(matches!(
            decode_eqmac(EQMAC_REQUEST_OPCODE, &answer)
                .unwrap()
                .as_deref(),
            Some([MerchantUpdate::Opened {
                accepted: false,
                ..
            }])
        ));
        // A rate that is not finite and above 0 is not guessed at.
        for rate in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
            answer[8..].copy_from_slice(&rate.to_le_bytes());
            assert!(decode_eqmac(EQMAC_REQUEST_OPCODE, &answer).is_err());
        }
        let list = decode_eqmac(
            EQMAC_STOCK_OPCODE,
            &stock(&[listed(13005, 0, 12), listed(13006, 1, 30)]),
        )
        .unwrap()
        .unwrap();
        let listed_items: Vec<_> = list
            .iter()
            .map(|update| match update {
                MerchantUpdate::Item(item) => {
                    (item.slot, item.price, item.quantity, item.item.details.id)
                }
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(listed_items, [(0, 12, 0, 13005), (1, 30, 0, 13006)]);
        assert_eq!(
            decode_eqmac(EQMAC_DELETE_OPCODE, &[132, 3, 7, 0, 2, 0x40]).unwrap(),
            Some(vec![MerchantUpdate::Removed { slot: 2 }])
        );
        let mut purchase = eqmac_buy(900, 7, 1, 2).unwrap();
        purchase[12..].copy_from_slice(&60u32.to_le_bytes());
        assert_eq!(
            decode_eqmac(EQMAC_BUY_OPCODE, &purchase).unwrap(),
            Some(vec![MerchantUpdate::Bought {
                slot: 1,
                quantity: 2,
                price: 60
            }])
        );
        // A refused purchase: nothing bought.
        assert_eq!(
            decode_eqmac(
                EQMAC_BUY_OPCODE,
                &[132, 3, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
            )
            .unwrap(),
            Some(vec![MerchantUpdate::Bought {
                slot: 0,
                quantity: 0,
                price: 0
            }])
        );
        let mut sale = eqmac_sell(900, 262, 1).unwrap();
        sale[6..8].copy_from_slice(&65535u16.to_le_bytes());
        assert_eq!(
            decode_eqmac(EQMAC_SELL_OPCODE, &sale).unwrap(),
            Some(vec![MerchantUpdate::Sold {
                slot: 262,
                quantity: 1,
                price: 65535
            }])
        );
        assert_eq!(
            decode_eqmac(EQMAC_END_CONFIRM_OPCODE, &[0x0a, 0x66]).unwrap(),
            Some(vec![MerchantUpdate::Closed])
        );
        assert_eq!(decode_eqmac(0xffff, &[]).unwrap(), None);
    }

    #[test]
    fn malformed_merchant_packets_are_refused() {
        for (opcode, size) in [
            (EQMAC_REQUEST_OPCODE, 12),
            (EQMAC_DELETE_OPCODE, 6),
            (EQMAC_BUY_OPCODE, 16),
            (EQMAC_SELL_OPCODE, 16),
        ] {
            assert!(decode_eqmac(opcode, &vec![0; size - 1]).is_err());
            assert!(decode_eqmac(opcode, &vec![0; size + 1]).is_err());
        }
        // A sale from a slot EQMac does not number.
        let mut sale = [0; 16];
        sale[4..6].copy_from_slice(&5000i16.to_le_bytes());
        assert!(decode_eqmac(EQMAC_SELL_OPCODE, &sale).is_err());
        // A record tagged for another class, a negative price, a partial
        // record, and an empty list.
        let mut tagged = listed(13005, 0, 12);
        tagged[0] = 1;
        assert!(decode_eqmac(EQMAC_STOCK_OPCODE, &stock(&[tagged])).is_err());
        assert!(decode_eqmac(EQMAC_STOCK_OPCODE, &stock(&[listed(13005, 0, -1)])).is_err());
        let mut partial = vec![1, 0];
        partial.extend(miniz_oxide::deflate::compress_to_vec_zlib(
            &listed(13005, 0, 12)[..361],
            4,
        ));
        assert!(decode_eqmac(EQMAC_STOCK_OPCODE, &partial).is_err());
        assert!(decode_eqmac(EQMAC_STOCK_OPCODE, &[0, 0]).is_err());
    }
}
