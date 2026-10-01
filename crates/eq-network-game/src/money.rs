//! Coins outside the purse and moving them: the player picks coins up from
//! the purse onto the cursor and puts them down in the purse, the bank or a
//! give or trade window (`OP_MoveCoin`). Servers answer no move; a client
//! keeps the coins where it put them, as the Titanium client does.
//!
//! Layout reference: `EQEmu`'s Titanium `MoveCoin_Struct`
//! (`common/patches/titanium_structs.h`) and `Client::OPMoveCoin`
//! (`zone/client_process.cpp`), which converts between kinds and never takes
//! more than a place holds.
use crate::{command::EncodedCommand, world::Coins};
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_MoveCoin`: coins moved between two places.
pub const MOVE_OPCODE: u16 = 0x7657;

/// A kind of coin.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Coin {
    /// Platinum, worth 1000 copper.
    Platinum,
    /// Gold, worth 100 copper.
    Gold,
    /// Silver, worth 10 copper.
    Silver,
    /// Copper.
    Copper,
}

impl Coin {
    /// Every kind, as the purse lists them.
    pub const ALL: [Self; 4] = [Self::Platinum, Self::Gold, Self::Silver, Self::Copper];

    /// Its worth in copper.
    #[must_use]
    pub const fn copper(self) -> u32 {
        match self {
            Self::Platinum => 1000,
            Self::Gold => 100,
            Self::Silver => 10,
            Self::Copper => 1,
        }
    }

    /// The servers' number for it (`COINTYPE_*`).
    const fn wire(self) -> u32 {
        match self {
            Self::Copper => 0,
            Self::Silver => 1,
            Self::Gold => 2,
            Self::Platinum => 3,
        }
    }
}

/// Where coins are.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
pub enum CoinPlace {
    /// On the cursor.
    Cursor,
    /// In the purse.
    Purse,
    /// In the personal bank, while a banker is near.
    Bank,
    /// In the open give or trade window.
    Trade,
}

impl CoinPlace {
    /// The servers' number for it.
    const fn wire(self) -> u32 {
        match self {
            Self::Cursor => 0,
            Self::Purse => 1,
            Self::Bank => 2,
            Self::Trade => 3,
        }
    }
}

/// Coins moved from one place to another, changing kind on the way as the
/// bank's exchange does.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CoinTransfer {
    /// Where they come from.
    pub from: CoinPlace,
    /// Where they go.
    pub to: CoinPlace,
    /// Their kind where they come from.
    pub coin: Coin,
    /// Their kind where they go.
    pub into: Coin,
    /// How many of `coin` the player moves.
    pub amount: u32,
}

impl CoinTransfer {
    /// What leaves and what arrives, as servers count it: only whole coins of
    /// the destination's kind arrive, and only what they are worth leaves.
    /// Moving 11 gold into platinum takes 10 gold and adds 1 platinum.
    #[must_use]
    pub fn amounts(&self) -> (u32, u32) {
        let (from, to) = (u64::from(self.coin.copper()), u64::from(self.into.copper()));
        let added = u64::from(self.amount) * from / to;
        let taken = added * to / from;
        // Neither is more than the amount's worth in the smaller coin, which a
        // purse never holds past u32.
        (
            u32::try_from(taken).unwrap_or(u32::MAX),
            u32::try_from(added).unwrap_or(u32::MAX),
        )
    }

    /// The packet for the move.
    ///
    /// # Errors
    /// Rejects a move of nothing, one that stays where it is, and amounts the
    /// servers' signed field cannot carry.
    pub fn encode(&self) -> Result<EncodedCommand> {
        ensure!(self.amount > 0, "move at least one coin");
        ensure!(
            self.from != self.to || self.coin != self.into,
            "coins must go somewhere"
        );
        let amount = i32::try_from(self.amount)?;
        let mut body = Vec::with_capacity(20);
        for value in [
            self.from.wire(),
            self.to.wire(),
            self.coin.wire(),
            self.into.wire(),
        ] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        body.extend_from_slice(&amount.to_le_bytes());
        Ok(EncodedCommand {
            opcode: MOVE_OPCODE,
            body,
        })
    }
}

impl Coins {
    /// How many of one kind.
    #[must_use]
    pub const fn of(&self, coin: Coin) -> u32 {
        match coin {
            Coin::Platinum => self.platinum,
            Coin::Gold => self.gold,
            Coin::Silver => self.silver,
            Coin::Copper => self.copper,
        }
    }

    /// The count of one kind, to change.
    pub fn of_mut(&mut self, coin: Coin) -> &mut u32 {
        match coin {
            Coin::Platinum => &mut self.platinum,
            Coin::Gold => &mut self.gold,
            Coin::Silver => &mut self.silver,
            Coin::Copper => &mut self.copper,
        }
    }

    /// Whether there are none of any kind.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.platinum == 0 && self.gold == 0 && self.silver == 0 && self.copper == 0
    }
}

fn word(profile: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        profile[offset],
        profile[offset + 1],
        profile[offset + 2],
        profile[offset + 3],
    ])
}

/// The coins on the cursor and in the bank, from the Titanium player
/// profile: what the player left there at the last logout.
///
/// # Errors
/// Rejects profiles with an unexpected length.
pub fn titanium_elsewhere(profile: &[u8]) -> Result<(Coins, Coins)> {
    ensure!(profile.len() == 19592, "unexpected Titanium profile layout");
    let coins = |start: usize| Coins {
        platinum: word(profile, start),
        gold: word(profile, start + 4),
        silver: word(profile, start + 8),
        copper: word(profile, start + 12),
    };
    Ok((coins(4444), coins(13136)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_is_five_signed_words_in_the_servers_numbering() {
        let transfer = CoinTransfer {
            from: CoinPlace::Purse,
            to: CoinPlace::Cursor,
            coin: Coin::Gold,
            into: Coin::Gold,
            amount: 12,
        };
        let packet = transfer.encode().unwrap();
        assert_eq!(packet.opcode, MOVE_OPCODE);
        assert_eq!(
            packet.body,
            [1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 12, 0, 0, 0]
        );
        let into_trade = CoinTransfer {
            from: CoinPlace::Cursor,
            to: CoinPlace::Trade,
            coin: Coin::Copper,
            into: Coin::Platinum,
            ..transfer
        };
        assert_eq!(
            into_trade.encode().unwrap().body[..16],
            [0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0]
        );
        for refused in [
            CoinTransfer {
                amount: 0,
                ..transfer
            },
            CoinTransfer {
                to: CoinPlace::Purse,
                ..transfer
            },
            CoinTransfer {
                amount: u32::MAX,
                ..transfer
            },
        ] {
            assert!(refused.encode().is_err(), "{refused:?}");
        }
    }

    #[test]
    fn a_change_of_kind_moves_only_whole_coins() {
        let exchange = |coin, into, amount| {
            CoinTransfer {
                from: CoinPlace::Cursor,
                to: CoinPlace::Bank,
                coin,
                into,
                amount,
            }
            .amounts()
        };
        assert_eq!(exchange(Coin::Gold, Coin::Platinum, 11), (10, 1));
        assert_eq!(exchange(Coin::Gold, Coin::Platinum, 9), (0, 0));
        assert_eq!(exchange(Coin::Platinum, Coin::Copper, 2), (2, 2000));
        assert_eq!(exchange(Coin::Silver, Coin::Silver, 7), (7, 7));
    }

    #[test]
    fn the_profile_says_what_is_on_the_cursor_and_in_the_bank() {
        let mut profile = vec![0; 19592];
        profile[4448..4452].copy_from_slice(&7u32.to_le_bytes());
        profile[13148..13152].copy_from_slice(&9u32.to_le_bytes());
        let (cursor, bank) = titanium_elsewhere(&profile).unwrap();
        assert_eq!(cursor.of(Coin::Gold), 7);
        assert_eq!(bank.of(Coin::Copper), 9);
        assert!(!cursor.is_empty() && Coins::default().is_empty());
        assert!(titanium_elsewhere(&profile[1..]).is_err());
    }
}
