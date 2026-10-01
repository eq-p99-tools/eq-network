//! Handing items to another character: the give window an NPC opens and the
//! trade window between players. One side asks, the other takes the request
//! and both windows open; items go into the trade slots from the cursor, and
//! the exchange goes through once each side has clicked Give or Trade (an NPC
//! takes it at once). Closing the window cancels it, and the server sends
//! what the trade slots held back as ordinary item updates.
//!
//! Between players, each side sees what the other puts in (an item view per
//! trade slot, and the coins added), and anything put in undoes both sides'
//! Trade clicks. What the other player hands over arrives as ordinary item
//! updates and a money update once the trade goes through.
//!
//! Layout reference: `EQEmu`'s Titanium `TradeRequest_Struct`,
//! `TradeAccept_Struct`, `CancelTrade_Struct`, `TradeBusy_Struct` and
//! `TradeCoin_Struct` (`common/patches/titanium_structs.h`), and
//! `zone/trading.cpp` and `zone/client_packet.cpp` for the exchange itself.
use crate::{
    command::EncodedCommand,
    inventory::{InventoryItem, InventorySlot},
    money::Coin,
    world::{Position, SpawnState},
};
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// `OP_TradeRequest`: one character asks another to trade.
pub const REQUEST_OPCODE: u16 = 0x372f;
/// `OP_TradeRequestAck`: the other side took the request, and the windows open.
pub const ACKNOWLEDGE_OPCODE: u16 = 0x4048;
/// `OP_TradeAcceptClick`: one side clicked Give or Trade.
pub const ACCEPT_OPCODE: u16 = 0x0065;
/// `OP_FinishTrade`: the exchange went through.
pub const FINISH_OPCODE: u16 = 0x6014;
/// `OP_CancelTrade`: one side closed the window.
pub const CANCEL_OPCODE: u16 = 0x2dc1;
/// `OP_TradeBusy`: the other player could not trade.
pub const BUSY_OPCODE: u16 = 0x6839;
/// `OP_TradeCoins`: the other player put coins in the trade.
pub const COINS_OPCODE: u16 = 0x34c1;
/// `OP_ItemPacket`, whose `ItemPacketTradeView` kind shows an item the other
/// player put in the trade.
const ITEM_OPCODE: u16 = 0x3397;
/// `ItemPacketTradeView`.
const TRADE_VIEW_KIND: u32 = 0x65;
/// The official client's number for the other player's first trade slot
/// (the skin's `TRDW_TradeSlot8`); their eight run on from it.
pub const THEIR_FIRST_SLOT: i32 = 3008;

/// `CancelTrade_Struct.action` for a closed window. Servers ignore it;
/// `EQEmu` itself sends this value (`groupActUpdate`) when it closes a
/// window at logout.
const CANCEL_ACTION: u32 = 7;

/// How far away another character may stand for the player to ask them to
/// trade, in EQ units. A conservative client policy, like the banker's:
/// `EQEmu` checks no distance, and P99's limit is not measured.
pub const REACH: f32 = 20.0;

/// Who the player is exchanging with, which decides the window and its
/// number of trade slots.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Partner {
    /// An NPC: the give window, with four slots. The NPC takes the items when
    /// the player clicks Give.
    Npc,
    /// Another player: the trade window, with eight slots on each side.
    Player,
}

impl Partner {
    /// How many of the player's trade slots the window has.
    #[must_use]
    pub const fn slots(self) -> u8 {
        match self {
            Self::Npc => 4,
            Self::Player => 8,
        }
    }
}

/// What the server says about an exchange.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum ExchangeUpdate {
    /// Another character asks the player to trade.
    Requested {
        /// The character asking.
        from: u32,
    },
    /// The other side took the player's request: the window opens.
    Opened {
        /// The character the player is exchanging with.
        with: u32,
    },
    /// The other side clicked Trade.
    Accepted {
        /// The side that clicked.
        by: u32,
    },
    /// The exchange went through: the trade slots' items changed hands.
    Finished,
    /// A side closed the window, or the server did (at logout, the player's
    /// own ID). What the player's trade slots held comes back as item updates.
    Cancelled {
        /// The ID the server names: an NPC's own, but this player's when
        /// another player cancelled, as `EQEmu` rewrites the forwarded
        /// packet for its recipient.
        by: u32,
    },
    /// The other player could not trade.
    Busy {
        /// The busy player.
        by: u32,
    },
    /// The player took another player's request, as the official client
    /// does on its own: the window is open. The session says this; no
    /// server packet does.
    Taken {
        /// The other player.
        from: u32,
    },
    /// The other player put an item in their trade slot `index` (0 to 7).
    /// Its slot is the official client's number for that place,
    /// [`THEIR_FIRST_SLOT`] on; what a bag holds is not kept.
    Offered {
        /// Their trade slot.
        index: u8,
        /// The item.
        item: Box<InventoryItem>,
    },
    /// The other player put coins in the trade; the server tells them
    /// nothing else about them until the trade goes through.
    Coins {
        /// Their kind.
        coin: Coin,
        /// How many more.
        amount: u32,
    },
}

fn word(body: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        body[offset],
        body[offset + 1],
        body[offset + 2],
        body[offset + 3],
    ])
}

/// Decodes the exchange packets; other opcodes are not this codec's.
///
/// # Errors
/// Rejects malformed lengths, coin kinds and items.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<ExchangeUpdate>> {
    if opcode == ITEM_OPCODE {
        return offered(body);
    }
    let length = match opcode {
        REQUEST_OPCODE | ACKNOWLEDGE_OPCODE | ACCEPT_OPCODE | CANCEL_OPCODE => 8,
        FINISH_OPCODE => 0,
        BUSY_OPCODE | COINS_OPCODE => 12,
        _ => return Ok(None),
    };
    ensure!(body.len() == length, "invalid trade packet length");
    Ok(Some(match opcode {
        // The request names the player first and the asker second; the
        // acknowledgement the asker first and the side that took it second.
        REQUEST_OPCODE => ExchangeUpdate::Requested {
            from: word(body, 4),
        },
        ACKNOWLEDGE_OPCODE => ExchangeUpdate::Opened {
            with: word(body, 4),
        },
        ACCEPT_OPCODE => ExchangeUpdate::Accepted { by: word(body, 0) },
        CANCEL_OPCODE => ExchangeUpdate::Cancelled { by: word(body, 0) },
        BUSY_OPCODE => ExchangeUpdate::Busy { by: word(body, 4) },
        // `TradeCoin_Struct`: the trader, the kind (one byte) and the amount.
        COINS_OPCODE => ExchangeUpdate::Coins {
            coin: Coin::from_wire(u32::from(body[4])).context("unknown coin kind")?,
            amount: word(body, 8),
        },
        _ => ExchangeUpdate::Finished,
    }))
}

/// An item the other player put in the trade, from its trade view; None for
/// the other item packets. A bag arrives with its contents, which `EQEmu`
/// then also sends one by one, numbered past the eight trade slots; those
/// repeats are left alone.
fn offered(body: &[u8]) -> Result<Option<ExchangeUpdate>> {
    if body.get(..4) != Some(&TRADE_VIEW_KIND.to_le_bytes()) {
        return Ok(None);
    }
    let text = &body[4..];
    // The header's third field is the trade slot, 0 to 7.
    let slot = std::str::from_utf8(text)
        .ok()
        .and_then(|text| text.split('|').nth(2))
        .context("truncated trade view")?;
    let Some(index) = slot.parse::<u8>().ok().filter(|index| *index < 8) else {
        return Ok(None);
    };
    // Parsed where a bag's contents have addresses; only the item is kept.
    let mut items = crate::inventory::parse_at(text, InventorySlot(22))?;
    ensure!(!items.is_empty(), "empty trade view");
    let mut item = items.swap_remove(0);
    item.slot = InventorySlot(THEIR_FIRST_SLOT + i32::from(index));
    Ok(Some(ExchangeUpdate::Offered {
        index,
        item: Box::new(item),
    }))
}

fn pair(opcode: u16, first: u32, second: u32) -> EncodedCommand {
    let mut body = Vec::with_capacity(8);
    body.extend_from_slice(&first.to_le_bytes());
    body.extend_from_slice(&second.to_le_bytes());
    EncodedCommand { opcode, body }
}

/// The player asks `with` to trade.
///
/// # Errors
/// Rejects the reserved zero ID and asking oneself.
pub fn request(own_id: u16, with: u16) -> Result<EncodedCommand> {
    ensure!(
        own_id != 0 && with != 0 && own_id != with,
        "a trade needs two characters"
    );
    Ok(pair(REQUEST_OPCODE, u32::from(with), u32::from(own_id)))
}

/// The player takes another player's request: both windows open.
///
/// # Errors
/// Rejects the reserved zero ID.
pub fn acknowledge(own_id: u16, asker: u32) -> Result<EncodedCommand> {
    ensure!(own_id != 0 && asker != 0, "a trade needs two characters");
    Ok(pair(ACKNOWLEDGE_OPCODE, asker, u32::from(own_id)))
}

/// The player cannot trade now, as when another trade is open.
///
/// # Errors
/// Rejects the reserved zero ID.
pub fn busy(own_id: u16, asker: u32) -> Result<EncodedCommand> {
    ensure!(own_id != 0 && asker != 0, "a trade needs two characters");
    let mut packet = pair(BUSY_OPCODE, asker, u32::from(own_id));
    // `TradeBusy_Struct`'s trailing bytes as `EQEmu` records them.
    packet.body.extend_from_slice(&[1, 0xef, 0xff, 0xff]);
    Ok(packet)
}

/// The player clicks Give or Trade.
///
/// # Errors
/// Rejects the reserved zero ID.
pub fn accept(own_id: u16) -> Result<EncodedCommand> {
    ensure!(own_id != 0, "the player has no spawn ID");
    Ok(pair(ACCEPT_OPCODE, u32::from(own_id), 0))
}

/// The player closes the window.
///
/// # Errors
/// Rejects the reserved zero ID.
pub fn cancel(own_id: u16) -> Result<EncodedCommand> {
    ensure!(own_id != 0, "the player has no spawn ID");
    Ok(pair(CANCEL_OPCODE, u32::from(own_id), CANCEL_ACTION))
}

/// Whether a character stands close enough to the player to be asked to
/// trade, in three dimensions; see [`REACH`].
#[must_use]
pub fn in_reach(position: Position, spawn: &SpawnState) -> bool {
    let distance = (position.x - spawn.position.x)
        .hypot(position.y - spawn.position.y)
        .hypot(position.z - spawn.position.z);
    distance.is_finite() && distance <= REACH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_exchange_packets_say_who_asked_took_clicked_and_closed() {
        let pair = |first: u32, second: u32| {
            let mut body = first.to_le_bytes().to_vec();
            body.extend_from_slice(&second.to_le_bytes());
            body
        };
        // The player is 7; the other side is 42.
        assert_eq!(
            decode(REQUEST_OPCODE, &pair(7, 42)).unwrap(),
            Some(ExchangeUpdate::Requested { from: 42 })
        );
        assert_eq!(
            decode(ACKNOWLEDGE_OPCODE, &pair(7, 42)).unwrap(),
            Some(ExchangeUpdate::Opened { with: 42 })
        );
        assert_eq!(
            decode(ACCEPT_OPCODE, &pair(42, 0xdead_beef)).unwrap(),
            Some(ExchangeUpdate::Accepted { by: 42 })
        );
        assert_eq!(
            decode(CANCEL_OPCODE, &pair(42, 7)).unwrap(),
            Some(ExchangeUpdate::Cancelled { by: 42 })
        );
        assert_eq!(
            decode(FINISH_OPCODE, &[]).unwrap(),
            Some(ExchangeUpdate::Finished)
        );
        let mut busy = pair(7, 42);
        busy.extend_from_slice(&[1, 0xef, 0xff, 0xff]);
        assert_eq!(
            decode(BUSY_OPCODE, &busy).unwrap(),
            Some(ExchangeUpdate::Busy { by: 42 })
        );
        assert_eq!(decode(0x1234, &[]).unwrap(), None);
        assert!(decode(REQUEST_OPCODE, &[0; 7]).is_err());
        assert!(decode(FINISH_OPCODE, &[0]).is_err());
    }

    #[test]
    fn the_other_player_shows_what_they_put_in() {
        use crate::inventory::tests::wire;
        // `TradeCoin_Struct`: the receiver, the kind, filler and how many.
        let mut coins = 7u32.to_le_bytes().to_vec();
        coins.extend_from_slice(&[2, 0xd2, 0x4f, 0]);
        coins.extend_from_slice(&15u32.to_le_bytes());
        assert_eq!(
            decode(COINS_OPCODE, &coins).unwrap(),
            Some(ExchangeUpdate::Coins {
                coin: Coin::Gold,
                amount: 15,
            })
        );
        coins[4] = 9;
        assert!(decode(COINS_OPCODE, &coins).is_err(), "no such coin");
        assert!(decode(COINS_OPCODE, &coins[..11]).is_err());
        // A view of their third slot: a bag holding one item.
        let view = |slot: i32| {
            let mut body = TRADE_VIEW_KIND.to_le_bytes().to_vec();
            body.extend(wire(slot, 100, 4, false, 0, &[(0, wire(0, 42, 0, true, 1, &[]))]).bytes());
            body
        };
        let Some(ExchangeUpdate::Offered { index, item }) = decode(ITEM_OPCODE, &view(2)).unwrap()
        else {
            panic!("an offered item");
        };
        assert_eq!((index, item.slot), (2, InventorySlot(THEIR_FIRST_SLOT + 2)));
        assert_eq!(item.bag_slots, 4);
        // The bag's contents repeated one by one, and other item packets,
        // are not this codec's.
        assert_eq!(decode(ITEM_OPCODE, &view(2031)).unwrap(), None);
        assert_eq!(decode(ITEM_OPCODE, &view(-1)).unwrap(), None);
        let mut trade = view(2);
        trade[0] = 0x67;
        assert_eq!(decode(ITEM_OPCODE, &trade).unwrap(), None);
        assert!(decode(ITEM_OPCODE, &view(2)[..40]).is_err());
    }

    #[test]
    fn the_player_asks_clicks_and_closes_in_the_servers_layout() {
        assert_eq!(
            request(7, 42).unwrap(),
            EncodedCommand {
                opcode: REQUEST_OPCODE,
                body: vec![42, 0, 0, 0, 7, 0, 0, 0],
            }
        );
        assert_eq!(
            accept(7).unwrap(),
            EncodedCommand {
                opcode: ACCEPT_OPCODE,
                body: vec![7, 0, 0, 0, 0, 0, 0, 0],
            }
        );
        assert_eq!(
            cancel(7).unwrap(),
            EncodedCommand {
                opcode: CANCEL_OPCODE,
                body: vec![7, 0, 0, 0, 7, 0, 0, 0],
            }
        );
        assert!(request(7, 7).is_err());
        assert!(request(0, 42).is_err());
        // Taking another player's request, or being too busy to.
        assert_eq!(
            acknowledge(7, 42).unwrap(),
            EncodedCommand {
                opcode: ACKNOWLEDGE_OPCODE,
                body: vec![42, 0, 0, 0, 7, 0, 0, 0],
            }
        );
        assert_eq!(
            busy(7, 42).unwrap(),
            EncodedCommand {
                opcode: BUSY_OPCODE,
                body: vec![42, 0, 0, 0, 7, 0, 0, 0, 1, 0xef, 0xff, 0xff],
            }
        );
        assert!(acknowledge(7, 0).is_err() && busy(0, 42).is_err());
        assert!(accept(0).is_err());
    }

    #[test]
    fn only_a_character_within_reach_can_be_asked() {
        let mut npc = SpawnState {
            class: Some(1),
            spawn_id: 42,
            name: "Synthetic NPC".into(),
            kind: crate::world::SpawnKind::Npc,
            race: 1,
            gender: 0,
            position: Position::default(),
            velocity: [0.0; 3],
            size: 6.0,
            invisible: false,
            appearance: crate::appearance::Appearance::default(),
        };
        let origin = Position::default();
        npc.position.x = REACH;
        assert!(in_reach(origin, &npc));
        npc.position.z = 1.0;
        assert!(!in_reach(origin, &npc));
        npc.position.x = f32::NAN;
        assert!(!in_reach(origin, &npc));
    }
}
