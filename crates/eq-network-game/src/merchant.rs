//! Merchant windows: opening, stock, purchases, sales and closing, in
//! Titanium's layouts and, through [`decode_eqmac`](crate::merchant::decode_eqmac),
//! in `EQMac`'s.
//!
//! Prices and stock come from the server; a purchase or sale is only settled by the
//! server's echo, its money update and the resulting inventory packets.
use crate::inventory::InventoryItem;
use anyhow::{ensure, Result};
use serde::Serialize;

mod eqmac;

pub use eqmac::{
    decode_eqmac, eqmac_buy, eqmac_end, eqmac_request, eqmac_sell, EQMAC_BUY_OPCODE,
    EQMAC_DELETE_OPCODE, EQMAC_END_CONFIRM_OPCODE, EQMAC_END_OPCODE, EQMAC_REQUEST_OPCODE,
    EQMAC_SELL_OPCODE, EQMAC_STOCK_OPCODE,
};

/// `OP_ShopRequest`: open (1) or close (0) in both directions.
pub const REQUEST_OPCODE: u16 = 0x45f9;
/// `OP_ShopPlayerBuy`: the player buys from the merchant; echoed on success.
pub const BUY_OPCODE: u16 = 0x221e;
/// `OP_ShopPlayerSell`: the player sells to the merchant; echoed on success.
pub const SELL_OPCODE: u16 = 0x0e13;
/// `OP_ShopEnd`: the player closes the window.
pub const END_OPCODE: u16 = 0x7e03;
/// `OP_ShopEndConfirm`: the server closed the window.
pub const END_CONFIRM_OPCODE: u16 = 0x20b2;
/// `OP_ShopDelItem`: an item left the merchant's stock.
pub const DELETE_OPCODE: u16 = 0x0da9;
/// `ItemPacketMerchant` inside `OP_ItemPacket`.
pub const ITEM_PACKET_KIND: u32 = 0x64;

/// How a client generation's merchant lists quote their prices: with the
/// merchant's rate in them, as `EQEmu`'s Titanium lists do
/// (`Client::BulkSendMerchantInventory` multiplies by `CalcPriceMod`), or
/// before it, as TAKP's `EQMac` lists do, the rate coming with the window's
/// opening.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Quotes {
    /// Each list price is what a unit costs.
    #[default]
    WithRate,
    /// Each list price is before the rate the window opened with.
    BeforeRate,
}

impl Quotes {
    /// How a client generation's lists quote their prices.
    #[must_use]
    pub const fn of(dialect: crate::GameDialect) -> Self {
        match dialect {
            crate::GameDialect::Titanium => Self::WithRate,
            crate::GameDialect::EqMac => Self::BeforeRate,
        }
    }

    /// What a unit costs, from its list price and the window's rate, as the
    /// server charges it. TAKP charges `Price * SellRate * rate` per unit,
    /// in single precision and cut to whole copper
    /// (`Client::Handle_OP_ShopPlayerBuy`), where its list price is
    /// `Price * SellRate` already cut (`BulkSendMerchantInventory`), so for
    /// an item whose `SellRate` is not 1 this can be a copper short; the
    /// most a unit costs is [`Quotes::most_per_unit`].
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )] // As the server computes it: in single precision, cut toward zero.
    pub fn unit_price(self, quote: u32, rate: f32) -> u32 {
        match self {
            Self::WithRate => quote,
            Self::BeforeRate => (quote as f32 * rate) as u32,
        }
    }

    /// The most a unit can cost, never below what the server charges: the
    /// list price where it carries the rate (`EQEmu` charges what it lists,
    /// cut the same way), and otherwise a list price one copper higher than
    /// the cut one, times the rate, rounded up.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Rounded up first; a negative rate costs nothing.
    pub fn most_per_unit(self, quote: u32, rate: f32) -> u64 {
        match self {
            Self::WithRate => u64::from(quote),
            Self::BeforeRate => ((f64::from(quote) + 1.0) * f64::from(rate)).ceil() as u64,
        }
    }
}

/// One item offered by the merchant.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MerchantItem {
    /// Merchant list slot used to buy it.
    pub slot: u32,
    /// Server price in copper for one unit.
    pub price: u32,
    /// Units available; server-specific values mean unlimited.
    pub quantity: u32,
    /// Item definition as sent by the server.
    pub item: InventoryItem,
}

/// Server-driven merchant changes.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum MerchantUpdate {
    /// Response to an open request; `rate` is the server's price multiplier.
    Opened {
        /// Merchant entity.
        merchant_id: u16,
        /// False when the merchant refused to trade.
        accepted: bool,
        /// Price multiplier supplied by the server.
        rate: f32,
    },
    /// One stock entry.
    Item(Box<MerchantItem>),
    /// A stock entry is gone.
    Removed {
        /// Merchant list slot.
        slot: u32,
    },
    /// Echo of a purchase.
    Bought {
        /// Merchant list slot.
        slot: u32,
        /// Units bought.
        quantity: u32,
        /// Total price in copper.
        price: u32,
    },
    /// Echo of a sale.
    Sold {
        /// Inventory slot the item came from.
        slot: i32,
        /// Units sold.
        quantity: u32,
        /// Total price in copper.
        price: u32,
    },
    /// The merchant window closed.
    Closed,
}

fn word(body: &[u8], offset: usize) -> Result<u32> {
    let bytes = body
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow::anyhow!("truncated merchant record"))?;
    Ok(u32::from_le_bytes(bytes.try_into()?))
}

/// Encodes an open (true) or close (false) request.
///
/// # Errors
/// Rejects reserved zero IDs.
pub fn request(merchant_id: u16, player_id: u16, open: bool) -> Result<[u8; 16]> {
    ensure!(
        merchant_id != 0 && player_id != 0,
        "shopping requires two entities"
    );
    let mut body = [0; 16];
    body[..4].copy_from_slice(&u32::from(merchant_id).to_le_bytes());
    body[4..8].copy_from_slice(&u32::from(player_id).to_le_bytes());
    body[8..12].copy_from_slice(&u32::from(open).to_le_bytes());
    body[12..].copy_from_slice(&1.0f32.to_le_bytes());
    Ok(body)
}

/// Encodes a purchase of `quantity` units from a merchant slot.
///
/// # Errors
/// Rejects zero IDs and quantities.
pub fn buy(merchant_id: u16, player_id: u16, slot: u32, quantity: u32) -> Result<[u8; 24]> {
    ensure!(
        merchant_id != 0 && player_id != 0,
        "shopping requires two entities"
    );
    ensure!(quantity != 0, "buy at least one unit");
    let mut body = [0; 24];
    body[..4].copy_from_slice(&u32::from(merchant_id).to_le_bytes());
    body[4..8].copy_from_slice(&u32::from(player_id).to_le_bytes());
    body[8..12].copy_from_slice(&slot.to_le_bytes());
    body[16..20].copy_from_slice(&quantity.to_le_bytes());
    Ok(body)
}

/// Encodes a sale of `quantity` units from an inventory slot; the server sets the price.
///
/// # Errors
/// Rejects zero IDs and quantities.
pub fn sell(merchant_id: u16, slot: i32, quantity: u32) -> Result<[u8; 16]> {
    ensure!(merchant_id != 0, "shopping requires a merchant");
    ensure!(quantity != 0, "sell at least one unit");
    let mut body = [0; 16];
    body[..4].copy_from_slice(&u32::from(merchant_id).to_le_bytes());
    body[4..8].copy_from_slice(&slot.to_le_bytes());
    body[8..12].copy_from_slice(&quantity.to_le_bytes());
    Ok(body)
}

/// Encodes closing the merchant window.
///
/// # Errors
/// Rejects reserved zero IDs.
pub fn end(merchant_id: u16, player_id: u16) -> Result<[u8; 8]> {
    ensure!(
        merchant_id != 0 && player_id != 0,
        "shopping requires two entities"
    );
    let mut body = [0; 8];
    body[..4].copy_from_slice(&u32::from(merchant_id).to_le_bytes());
    body[4..].copy_from_slice(&u32::from(player_id).to_le_bytes());
    Ok(body)
}

/// Decodes merchant packets; other opcodes and item packet kinds return None.
///
/// # Errors
/// Rejects malformed recognized packets.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<MerchantUpdate>> {
    Ok(Some(match opcode {
        REQUEST_OPCODE => {
            ensure!(body.len() >= 16, "invalid merchant response length");
            let rate = f32::from_bits(word(body, 12)?);
            MerchantUpdate::Opened {
                merchant_id: u16::try_from(word(body, 0)?)?,
                accepted: word(body, 8)? == 1,
                rate: if rate.is_finite() { rate } else { 1.0 },
            }
        }
        BUY_OPCODE => {
            ensure!(body.len() == 24, "invalid purchase echo length");
            MerchantUpdate::Bought {
                slot: word(body, 8)?,
                quantity: word(body, 16)?,
                price: word(body, 20)?,
            }
        }
        SELL_OPCODE => {
            ensure!(body.len() == 16, "invalid sale echo length");
            MerchantUpdate::Sold {
                slot: i32::from_le_bytes(word(body, 4)?.to_le_bytes()),
                quantity: word(body, 8)?,
                price: word(body, 12)?,
            }
        }
        DELETE_OPCODE => {
            ensure!(
                matches!(body.len(), 12 | 16),
                "invalid stock removal length"
            );
            MerchantUpdate::Removed {
                slot: word(body, 8)?,
            }
        }
        END_CONFIRM_OPCODE => MerchantUpdate::Closed,
        0x3397 if body.get(..4) == Some(&ITEM_PACKET_KIND.to_le_bytes()) => {
            let header = merchant_header(&body[4..])?;
            let mut items = crate::inventory::parse_items(&body[4..])?;
            ensure!(items.len() == 1, "merchant view must hold one item");
            MerchantUpdate::Item(Box::new(MerchantItem {
                slot: header.0,
                price: header.1,
                quantity: header.2,
                item: items.remove(0),
            }))
        }
        _ => return Ok(None),
    }))
}

/// Reads the merchant slot, price and count from the serialized item header.
fn merchant_header(body: &[u8]) -> Result<(u32, u32, u32)> {
    let text = std::str::from_utf8(&body[..body.len().min(128)])
        .or_else(|error| std::str::from_utf8(&body[..error.valid_up_to()]))?;
    let mut fields = text.split('|');
    let mut next = || -> Result<i64> {
        Ok(fields
            .next()
            .ok_or_else(|| anyhow::anyhow!("truncated merchant item header"))?
            .parse()?)
    };
    next()?;
    next()?;
    let slot = u32::try_from(next()?)?;
    let price = u32::try_from(next()?)?;
    let quantity = u32::try_from(next()?.max(0))?;
    Ok((slot, price, quantity))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_before_the_rate_costs_what_the_server_charges_and_never_more_than_its_most() {
        use crate::GameDialect;
        assert_eq!(Quotes::of(GameDialect::Titanium), Quotes::WithRate);
        assert_eq!(Quotes::of(GameDialect::EqMac), Quotes::BeforeRate);
        // Titanium's lists carry the rate: a unit costs what is listed.
        assert_eq!(Quotes::WithRate.unit_price(125, 1.3), 125);
        assert_eq!(Quotes::WithRate.most_per_unit(125, 1.3), 125);
        // TAKP's do not: 100 at 1.25 costs 125, and at 0.875, 87.
        assert_eq!(Quotes::BeforeRate.unit_price(100, 1.25), 125);
        assert_eq!(Quotes::BeforeRate.unit_price(100, 0.875), 87);
        // A list price cut from up to one copper more costs at most 127.
        assert_eq!(Quotes::BeforeRate.most_per_unit(100, 1.25), 127);
        for (quote, rate) in [(100u32, 1.25f32), (7, 0.9), (1999, 1.1), (0, 1.2)] {
            assert!(
                Quotes::BeforeRate.most_per_unit(quote, rate)
                    >= u64::from(Quotes::BeforeRate.unit_price(quote, rate))
            );
        }
    }

    fn wire(slot: u32, price: u32, count: i32) -> Vec<u8> {
        let mut fields = vec!["0".to_owned(); 159];
        fields[1] = "Test ration".into();
        fields[4] = "13005".into();
        fields[5] = "25".into();
        fields[11] = "500".into();
        fields[154] = "-1".into();
        let mut text = format!(
            "1|0|{slot}|{price}|{count}|0|{slot}|0|0|0|0|\"{}\"",
            fields.join("|")
        );
        text.push_str(&"|".repeat(10));
        text.push('\0');
        let mut body = ITEM_PACKET_KIND.to_le_bytes().to_vec();
        body.extend_from_slice(text.as_bytes());
        body
    }

    #[test]
    fn shop_requests_use_titanium_layouts() {
        let open = request(900, 7, true).unwrap();
        assert_eq!(&open[..12], &[132, 3, 0, 0, 7, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(open[12..], 1.0f32.to_le_bytes());
        assert_eq!(&request(900, 7, false).unwrap()[8..12], &[0; 4]);
        let purchase = buy(900, 7, 3, 2).unwrap();
        assert_eq!(
            (&purchase[8..12], &purchase[16..20]),
            (&[3, 0, 0, 0][..], &[2, 0, 0, 0][..])
        );
        assert!(buy(900, 7, 3, 0).is_err());
        let sale = sell(900, 23, 1).unwrap();
        assert_eq!(&sale[4..12], &[23, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(end(900, 7).unwrap(), [132, 3, 0, 0, 7, 0, 0, 0]);
        assert!(request(0, 7, true).is_err());
    }

    #[test]
    fn responses_stock_and_settlements_decode() {
        let mut open = request(900, 7, true).unwrap();
        assert_eq!(
            decode(REQUEST_OPCODE, &open).unwrap(),
            Some(MerchantUpdate::Opened {
                merchant_id: 900,
                accepted: true,
                rate: 1.0
            })
        );
        open[8] = 0;
        assert!(matches!(
            decode(REQUEST_OPCODE, &open).unwrap(),
            Some(MerchantUpdate::Opened {
                accepted: false,
                ..
            })
        ));
        let Some(MerchantUpdate::Item(stock)) = decode(0x3397, &wire(4, 12, -1)).unwrap() else {
            panic!("merchant stock");
        };
        assert_eq!((stock.slot, stock.price, stock.quantity), (4, 12, 0));
        assert_eq!(stock.item.details.id, 13005);
        let mut purchase = buy(900, 7, 4, 2).unwrap();
        purchase[20..].copy_from_slice(&24u32.to_le_bytes());
        assert_eq!(
            decode(BUY_OPCODE, &purchase).unwrap(),
            Some(MerchantUpdate::Bought {
                slot: 4,
                quantity: 2,
                price: 24
            })
        );
        let mut sale = sell(900, 23, 1).unwrap();
        sale[12..].copy_from_slice(&5u32.to_le_bytes());
        assert_eq!(
            decode(SELL_OPCODE, &sale).unwrap(),
            Some(MerchantUpdate::Sold {
                slot: 23,
                quantity: 1,
                price: 5
            })
        );
        let mut removal = [0u8; 16];
        removal[8] = 4;
        assert_eq!(
            decode(DELETE_OPCODE, &removal).unwrap(),
            Some(MerchantUpdate::Removed { slot: 4 })
        );
        assert_eq!(
            decode(END_CONFIRM_OPCODE, &[]).unwrap(),
            Some(MerchantUpdate::Closed)
        );
        assert_eq!(decode(0x3397, &0x69u32.to_le_bytes()).unwrap(), None);
    }
}
