//! The give and trade windows on the `EQMac` wire, as the TAKP program sends
//! and reads them (TAKP program read N4, 10-31; TAKP/EQMac program, this
//! install's own modules present, not read). The opcodes are the program's
//! logical values with their bytes swapped, as every `EQMac` opcode here is.
//!
//! The exchange is the Titanium one in other bodies: spawn IDs are 16 bits,
//! the accept and the cancel name the partner, a refusal carries a reason,
//! and the other player's coins and items have their own opcodes. The
//! program reads no packet's length, so each decoder needs only the bytes
//! the program reads.
//!
//! The player half (another player's request, their coins and items) rides
//! the same packets and is not yet checked live; the program's own tests
//! before it asks or answers (the tradeable test 0x4ef773 and the request
//! setting 0x798ad8) are unread (6.307, 6.308).
use super::ExchangeUpdate;
use crate::{command::EncodedCommand, money::Coin};
use anyhow::{ensure, Context, Result};

/// `OP_TradeRequest` (0x40d1): {target u16, own u16}, both ways.
pub const REQUEST_OPCODE: u16 = 0xd140;
/// `OP_TradeRequestAck` (0x40e6): {requester u16, own u16}; from the server,
/// the window opens on the side at +2.
pub const ACKNOWLEDGE_OPCODE: u16 = 0xe640;
/// `OP_TradeAcceptClick` (0x40da): {side u16, accepted u8, x u8}.
pub const ACCEPT_OPCODE: u16 = 0xda40;
/// `OP_CancelTrade` (0x40db): {side u16, own u16}.
pub const CANCEL_OPCODE: u16 = 0xdb40;
/// `OP_FinishTrade` (0x40dc), whose body the program doesn't read.
pub const FINISH_OPCODE: u16 = 0xdc40;
/// `OP_TradeRefused` (0x40d6): {requester u16, refuser u16, reason u8, x u8}.
pub const REFUSED_OPCODE: u16 = 0xd640;
/// `OP_TradeCoins` (0x40e4): {x u16, kind u8, x u8, amount added u32}.
pub const COINS_OPCODE: u16 = 0xe440;
/// `OP_TradeItemPacket` (0x40df): {x u16, slot u16, kind u8, record}.
pub const ITEM_OPCODE: u16 = 0xdf40;

/// The refusal's reason when the player is busy: the program's own when it
/// can't trade now (inferred busy; the 0x62 and 0x63 reasons come from a
/// trade setting this client doesn't have).
const BUSY: u8 = 1;
/// The item record kinds 0x40df carries, and how much of each the program
/// copies: an item (360 bytes), a bag (276) and a book (264); the names of
/// the last two are inferred.
const RECORDS: [(u8, usize); 3] = [(0, ITEM_RECORD), (1, 276), (2, 264)];
/// A whole item record, which this client's reader takes.
const ITEM_RECORD: usize = 360;
/// The kind that empties the slot.
const CLEARED: u8 = 0xff;
/// The trade slots the other player has, numbered 0 to 7; 30 and up are the
/// contents of their bags, which this codec leaves to item updates.
const THEIR_SLOTS: i16 = 8;

fn pair(opcode: u16, first: u16, second: u16) -> EncodedCommand {
    let mut body = Vec::with_capacity(4);
    body.extend_from_slice(&first.to_le_bytes());
    body.extend_from_slice(&second.to_le_bytes());
    EncodedCommand { opcode, body }
}

fn short(body: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([body[at], body[at + 1]])
}

/// The player asks `with` to trade: {target, own}.
///
/// # Errors
/// Rejects the reserved zero ID and asking oneself.
pub fn request(own_id: u16, with: u16) -> Result<EncodedCommand> {
    ensure!(
        own_id != 0 && with != 0 && own_id != with,
        "a trade needs two characters"
    );
    Ok(pair(REQUEST_OPCODE, with, own_id))
}

/// The player takes another player's request: {requester, own}.
///
/// # Errors
/// Rejects the reserved zero ID and an ID past 16 bits.
pub fn acknowledge(own_id: u16, asker: u32) -> Result<EncodedCommand> {
    let asker = u16::try_from(asker).context("spawn ID past 16 bits")?;
    ensure!(own_id != 0 && asker != 0, "a trade needs two characters");
    Ok(pair(ACKNOWLEDGE_OPCODE, asker, own_id))
}

/// The player can't trade now: {requester, own, reason 1, 0}. The program
/// doesn't write the last byte; 0 is this client's.
///
/// # Errors
/// Rejects the reserved zero ID and an ID past 16 bits.
pub fn busy(own_id: u16, asker: u32) -> Result<EncodedCommand> {
    let asker = u16::try_from(asker).context("spawn ID past 16 bits")?;
    ensure!(own_id != 0 && asker != 0, "a trade needs two characters");
    let mut packet = pair(REFUSED_OPCODE, asker, own_id);
    packet.body.extend_from_slice(&[BUSY, 0]);
    Ok(packet)
}

/// The player clicks Give or Trade: {partner, 1, 0}. The program sends it
/// once and has no un-accept; it doesn't write the last byte, so 0 is this
/// client's.
///
/// # Errors
/// Rejects the reserved zero ID.
pub fn accept(with: u16) -> Result<EncodedCommand> {
    ensure!(with != 0, "the trade has no partner");
    let mut body = with.to_le_bytes().to_vec();
    body.extend_from_slice(&[1, 0]);
    Ok(EncodedCommand {
        opcode: ACCEPT_OPCODE,
        body,
    })
}

/// The player closes the window, or withdraws a request: {partner, own}.
/// The program sends it whenever its window closes with a partner set.
///
/// # Errors
/// Rejects the reserved zero ID.
pub fn cancel(own_id: u16, with: u16) -> Result<EncodedCommand> {
    ensure!(own_id != 0 && with != 0, "a trade needs two characters");
    Ok(pair(CANCEL_OPCODE, with, own_id))
}

/// Decodes the exchange packets; other opcodes are not this codec's.
///
/// - A request names the asker at +2, an acknowledgement the side whose
///   window opens at +2.
/// - An accept with +2 set is that side's click; with +2 zero, both sides'
///   clicks are undone ([`ExchangeUpdate::AcceptsReset`]).
/// - A cancel names who closed it at +0.
/// - A refusal names the refuser at +2 and its reason at +4 (0x62 and 0x63
///   from the refuser's setting, 1 busy); the program says the same line
///   for each.
/// - Coins name their kind at +2 (3 platinum to 0 copper) and the amount
///   added at +4.
/// - An item names its record's kind at +4 and the record from +5, placed
///   by the slot the record names at +0xb8. One in their eight slots is
///   shown; one inside their bag is left to item updates; kind 0xff empties
///   slot +2. The program copies 276 bytes of a bag's record (kind 1) and
///   264 of a book's (kind 2); where the whole 360-byte record came, it is
///   read as any item, and where only the copied part came, no reader here
///   takes it yet: it is decoded as [`TradeItem::Unread`], its slot (+2)
///   known and its record not, which the session logs and drops.
///
/// # Errors
/// Rejects a packet shorter than what the program reads, an unknown coin
/// kind or record kind, and an item record that doesn't read.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<TradeMessage>> {
    let needs = |length: usize| {
        ensure!(body.len() >= length, "short EQMac trade packet");
        Ok(())
    };
    let update = match opcode {
        REQUEST_OPCODE => {
            needs(4)?;
            ExchangeUpdate::Requested {
                from: short(body, 2).into(),
            }
        }
        ACKNOWLEDGE_OPCODE => {
            needs(4)?;
            ExchangeUpdate::Opened {
                with: short(body, 2).into(),
            }
        }
        ACCEPT_OPCODE => {
            needs(3)?;
            if body[2] == 0 {
                ExchangeUpdate::AcceptsReset
            } else {
                ExchangeUpdate::Accepted {
                    by: short(body, 0).into(),
                }
            }
        }
        CANCEL_OPCODE => {
            needs(2)?;
            ExchangeUpdate::Cancelled {
                by: short(body, 0).into(),
            }
        }
        FINISH_OPCODE => ExchangeUpdate::Finished,
        REFUSED_OPCODE => {
            needs(5)?;
            ExchangeUpdate::Busy {
                by: short(body, 2).into(),
                reason: Some(body[4]),
            }
        }
        COINS_OPCODE => {
            needs(8)?;
            ExchangeUpdate::Coins {
                coin: Coin::from_wire(body[2].into()).context("unknown coin kind")?,
                amount: u32::from_le_bytes([body[4], body[5], body[6], body[7]]),
            }
        }
        ITEM_OPCODE => return item(body).map(|item| item.map(TradeMessage::Item)),
        _ => return Ok(None),
    };
    Ok(Some(TradeMessage::Update(update)))
}

/// What one `EQMac` exchange packet says.
#[derive(Clone, Debug, PartialEq)]
pub enum TradeMessage {
    /// What the exchange shares with Titanium's.
    Update(ExchangeUpdate),
    /// An item the other player put in or took out.
    Item(TradeItem),
}

/// An item the other player put in their trade slots, or took out.
#[derive(Clone, Debug, PartialEq)]
pub enum TradeItem {
    /// Read as the exchange shares it: an item shown, or a slot emptied.
    Read(ExchangeUpdate),
    /// A bag or book record no reader here takes yet, in their slot.
    Unread {
        /// Their trade slot, 0 to 7.
        index: u8,
        /// The record's kind: 1 a bag, 2 a book (names inferred).
        kind: u8,
    },
}

/// An item packet: {x u16, slot u16, kind u8, record}.
fn item(body: &[u8]) -> Result<Option<TradeItem>> {
    ensure!(body.len() >= 5, "short EQMac trade packet");
    let kind = body[4];
    if kind == CLEARED {
        let slot = i16::from_le_bytes([body[2], body[3]]);
        return Ok(
            their_slot(slot).map(|index| TradeItem::Read(ExchangeUpdate::Withdrawn { index }))
        );
    }
    let length = RECORDS
        .iter()
        .find(|(known, _)| *known == kind)
        .map(|(_, length)| *length)
        .context("unknown EQMac trade record kind")?;
    ensure!(body.len() >= 5 + length, "short EQMac trade record");
    // A bag or book record the program copies only in part is read as an
    // item where the whole record came; otherwise its slot is all that is
    // known.
    let Some(record) = body.get(5..5 + ITEM_RECORD) else {
        let slot = i16::from_le_bytes([body[2], body[3]]);
        return Ok(their_slot(slot).map(|index| TradeItem::Unread { index, kind }));
    };
    let Some((index, item)) = crate::inventory::eqmac_trade_view(record, their_slot)? else {
        return Ok(None);
    };
    Ok(Some(TradeItem::Read(ExchangeUpdate::Offered {
        index,
        item: Box::new(item),
    })))
}

/// One of the other player's eight trade slots; None for the contents of
/// their bags and anything else.
fn their_slot(slot: i16) -> Option<u8> {
    (0..THEIR_SLOTS)
        .contains(&slot)
        .then(|| u8::try_from(slot).ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(opcode: u16, body: &[u8]) -> Option<TradeMessage> {
        decode(opcode, body).unwrap()
    }

    #[test]
    fn the_player_asks_takes_clicks_and_closes_in_sixteen_bit_ids() {
        assert_eq!(
            request(7, 42).unwrap(),
            EncodedCommand {
                opcode: REQUEST_OPCODE,
                body: vec![42, 0, 7, 0],
            }
        );
        assert_eq!(
            acknowledge(7, 50).unwrap(),
            EncodedCommand {
                opcode: ACKNOWLEDGE_OPCODE,
                body: vec![50, 0, 7, 0],
            }
        );
        assert_eq!(
            busy(7, 50).unwrap(),
            EncodedCommand {
                opcode: REFUSED_OPCODE,
                body: vec![50, 0, 7, 0, 1, 0],
            }
        );
        assert_eq!(
            accept(42).unwrap(),
            EncodedCommand {
                opcode: ACCEPT_OPCODE,
                body: vec![42, 0, 1, 0],
            }
        );
        assert_eq!(
            cancel(7, 42).unwrap(),
            EncodedCommand {
                opcode: CANCEL_OPCODE,
                body: vec![42, 0, 7, 0],
            }
        );
        assert!(request(7, 7).is_err() && request(0, 42).is_err());
        assert!(acknowledge(7, 70_000).is_err() && busy(7, 0).is_err());
        assert!(accept(0).is_err() && cancel(7, 0).is_err());
    }

    #[test]
    fn the_server_says_who_asked_took_clicked_refused_and_closed() {
        let shared = |update| Some(TradeMessage::Update(update));
        assert_eq!(
            update(REQUEST_OPCODE, &[7, 0, 50, 0]),
            shared(ExchangeUpdate::Requested { from: 50 })
        );
        assert_eq!(
            update(ACKNOWLEDGE_OPCODE, &[7, 0, 42, 0]),
            shared(ExchangeUpdate::Opened { with: 42 })
        );
        assert_eq!(
            update(ACCEPT_OPCODE, &[50, 0, 1, 0xcc]),
            shared(ExchangeUpdate::Accepted { by: 50 })
        );
        assert_eq!(
            update(ACCEPT_OPCODE, &[50, 0, 0]),
            shared(ExchangeUpdate::AcceptsReset)
        );
        assert_eq!(
            update(CANCEL_OPCODE, &[42, 0, 7, 0]),
            shared(ExchangeUpdate::Cancelled { by: 42 })
        );
        assert_eq!(update(FINISH_OPCODE, &[]), shared(ExchangeUpdate::Finished));
        assert_eq!(
            update(REFUSED_OPCODE, &[7, 0, 50, 0, 0x62, 0]),
            shared(ExchangeUpdate::Busy {
                by: 50,
                reason: Some(0x62)
            })
        );
        // Platinum is 3, and the amount is what was added.
        assert_eq!(
            update(COINS_OPCODE, &[0, 0, 3, 0, 15, 0, 0, 0]),
            shared(ExchangeUpdate::Coins {
                coin: Coin::Platinum,
                amount: 15,
            })
        );
        assert!(decode(COINS_OPCODE, &[0, 0, 9, 0, 15, 0, 0, 0]).is_err());
        assert!(decode(REQUEST_OPCODE, &[7, 0, 50]).is_err());
        assert!(decode(COINS_OPCODE, &[0; 7]).is_err());
        assert_eq!(update(0x1234, &[]), None);
    }

    #[test]
    fn their_slots_empty_and_unread_records_keep_their_place() {
        // Kind 0xff empties slot +2.
        assert_eq!(
            update(ITEM_OPCODE, &[0, 0, 3, 0, 0xff]),
            Some(TradeMessage::Item(TradeItem::Read(
                ExchangeUpdate::Withdrawn { index: 3 }
            )))
        );
        // Inside their bag, it's not this codec's.
        assert_eq!(update(ITEM_OPCODE, &[0, 0, 31, 0, 0xff]), None);
        // A bag record: the slot is known, the record unread.
        let mut bag = vec![0, 0, 2, 0, 1];
        bag.extend_from_slice(&[0; 276]);
        assert_eq!(
            update(ITEM_OPCODE, &bag),
            Some(TradeMessage::Item(TradeItem::Unread { index: 2, kind: 1 }))
        );
        assert!(decode(ITEM_OPCODE, &bag[..200]).is_err());
        assert!(decode(ITEM_OPCODE, &[0, 0, 2, 0, 7]).is_err());
        // A whole item record in their third slot.
        let mut record = vec![0; ITEM_RECORD];
        record[..14].copy_from_slice(b"Synthetic item");
        record[175] = 1;
        record[176] = 1;
        record[180..182].copy_from_slice(&1001i16.to_le_bytes());
        record[184..186].copy_from_slice(&2i16.to_le_bytes());
        let mut offered = vec![0, 0, 2, 0, 0];
        offered.extend_from_slice(&record);
        let Some(TradeMessage::Item(TradeItem::Read(ExchangeUpdate::Offered { index, item }))) =
            update(ITEM_OPCODE, &offered)
        else {
            panic!("an offered item");
        };
        assert_eq!(
            (index, item.slot),
            (
                2,
                crate::inventory::InventorySlot(super::super::THEIR_FIRST_SLOT + 2)
            )
        );
    }
}
