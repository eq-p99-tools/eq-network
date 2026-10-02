//! Handing items to another character: the give window an NPC opens and the
//! trade window between players. One side asks, the other takes the request
//! and both windows open; items go into the trade slots from the cursor, and
//! the exchange goes through once each side has clicked Give or Trade (an NPC
//! takes it at once). Closing the window cancels it, and the server sends
//! what the trade slots held back as ordinary item updates.
//!
//! Layout reference: `EQEmu`'s Titanium `TradeRequest_Struct`,
//! `TradeAccept_Struct`, `CancelTrade_Struct` and `TradeBusy_Struct`
//! (`common/patches/titanium_structs.h`), and `zone/trading.cpp` for the
//! exchange itself.
use crate::{
    command::EncodedCommand,
    world::{Position, SpawnState},
};
use anyhow::{ensure, Result};
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
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
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
        /// The side the server names.
        by: u32,
    },
    /// The other player could not trade.
    Busy {
        /// The busy player.
        by: u32,
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
/// Rejects malformed lengths.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<ExchangeUpdate>> {
    let length = match opcode {
        REQUEST_OPCODE | ACKNOWLEDGE_OPCODE | ACCEPT_OPCODE | CANCEL_OPCODE => 8,
        FINISH_OPCODE => 0,
        BUSY_OPCODE => 12,
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
        _ => ExchangeUpdate::Finished,
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
            level: 0,
            listing: crate::listing::Listing::default(),
            name_parts: crate::names::NameParts::default(),
            pet_owner: None,
            hp_percent: None,
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
