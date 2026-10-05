//! The player's coins and moving them: the player picks coins up from the
//! purse onto the cursor and puts them down in the purse, the bank or a give
//! or trade window (`OP_MoveCoin`). Servers answer no move, and say what the
//! purse holds only now and then, so a [`Wallet`](crate::money::Wallet)
//! keeps the coins where the player put them, as the Titanium client does.
//!
//! Layout reference: `EQEmu`'s Titanium `MoveCoin_Struct`
//! (`common/patches/titanium_structs.h`) and `Client::OPMoveCoin`
//! (`zone/client_process.cpp`), which converts between kinds and never takes
//! more than a place holds; `Client::TakeMoneyFromPP` and
//! `Client::AddMoneyToPP` (`zone/client.cpp`) for purchases and loot.
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

    /// The kind a server's number (`COINTYPE_*`) names.
    #[must_use]
    pub const fn from_wire(number: u32) -> Option<Self> {
        Some(match number {
            0 => Self::Copper,
            1 => Self::Silver,
            2 => Self::Gold,
            3 => Self::Platinum,
            _ => return None,
        })
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

/// Where the player's coins are. Servers answer no coin move, so whoever
/// sends the moves keeps this, as the Titanium client does; a money update
/// replaces the purse.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Wallet {
    /// The purse, once admission has said what it holds.
    pub purse: Option<Coins>,
    /// On the cursor.
    pub cursor: Coins,
    /// In the bank, once admission has said.
    pub bank: Option<Coins>,
    /// What the player put in the open give or trade window.
    pub given: Coins,
    /// What the other player put in the trade.
    pub offered: Coins,
}

impl Wallet {
    /// The coins in a place, where they are known.
    #[must_use]
    pub const fn get(&self, place: CoinPlace) -> Option<Coins> {
        match place {
            CoinPlace::Purse => self.purse,
            CoinPlace::Cursor => Some(self.cursor),
            CoinPlace::Bank => self.bank,
            CoinPlace::Trade => Some(self.given),
        }
    }

    fn place(&mut self, place: CoinPlace) -> Option<&mut Coins> {
        match place {
            CoinPlace::Purse => self.purse.as_mut(),
            CoinPlace::Cursor => Some(&mut self.cursor),
            CoinPlace::Bank => self.bank.as_mut(),
            CoinPlace::Trade => Some(&mut self.given),
        }
    }

    /// Moves coins as servers count them ([`CoinTransfer::amounts`]).
    ///
    /// # Errors
    /// Refuses a move that moves nothing, or whose source does not hold
    /// what it takes or whose destination is not known yet; servers log
    /// such a move as a possible hack.
    pub fn apply(&mut self, transfer: CoinTransfer) -> Result<(), &'static str> {
        let (taken, added) = transfer.amounts();
        if taken == 0 {
            return Err("Too few coins to change into that kind");
        }
        if self
            .get(transfer.from)
            .is_none_or(|coins| coins.of(transfer.coin) < taken)
        {
            return Err("You do not have that many coins there");
        }
        if self.get(transfer.to).is_none() {
            return Err("Those coins are not known yet");
        }
        if let Some(from) = self.place(transfer.from) {
            *from.of_mut(transfer.coin) -= taken;
        }
        if let Some(to) = self.place(transfer.to) {
            *to.of_mut(transfer.into) = to.of(transfer.into).saturating_add(added);
        }
        Ok(())
    }

    /// Adds coins looted from a corpse to the purse kind by kind, as servers
    /// do without a money update (`EQEmu`'s four-kind
    /// `Client::AddMoneyToPP`).
    pub fn add_to_purse(&mut self, coins: Coins) {
        if let Some(purse) = self.purse.as_mut() {
            for coin in Coin::ALL {
                *purse.of_mut(coin) = purse.of(coin).saturating_add(coins.of(coin));
            }
        }
    }

    /// Takes a purchase's price from the purse as `EQEmu` does without a
    /// money update (`Client::TakeMoneyFromPP`): copper first, then each
    /// larger kind in turn, the coins that cover what is still owed coming
    /// back as change in their own and smaller kinds. The kinds matter: a
    /// coin move the server's purse cannot cover is logged as a hack. A
    /// purse holding less than the price is emptied, as it was wrong already
    /// and too few is safer than coins the server does not have.
    pub fn pay(&mut self, price: u64) {
        const SMALLEST_FIRST: [Coin; 4] = [Coin::Copper, Coin::Silver, Coin::Gold, Coin::Platinum];
        let Some(purse) = self.purse.as_mut() else {
            return;
        };
        let mut owed = price;
        for (index, coin) in SMALLEST_FIRST.into_iter().enumerate() {
            let pile = u64::from(purse.of(coin)) * u64::from(coin.copper());
            *purse.of_mut(coin) = 0;
            if pile > owed {
                let mut change = pile - owed;
                for smaller in SMALLEST_FIRST[..=index].iter().rev() {
                    let worth = u64::from(smaller.copper());
                    let count = u32::try_from(change / worth).unwrap_or(u32::MAX);
                    *purse.of_mut(*smaller) = purse.of(*smaller).saturating_add(count);
                    change %= worth;
                }
                return;
            }
            owed -= pile;
        }
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

/// The coins the player carries, from `EQMac`'s unpacked profile: four
/// signed 32-bit counts, platinum first, at 2924 (TAKP
/// `common/patches/mac_structs.h` `PlayerProfile_Struct`).
///
/// # Errors
/// Rejects a profile of another size and a count below zero.
pub fn eqmac_coins(profile: &[u8]) -> Result<Coins> {
    eqmac_coins_at(profile, 2924)
}

/// The coins on the cursor and in the bank, from `EQMac`'s unpacked
/// profile: the bank's at 2940 and the cursor's at 2956, laid out as the
/// carried ones. `EQMac` has no shared bank.
///
/// # Errors
/// Rejects a profile of another size and a count below zero.
pub fn eqmac_elsewhere(profile: &[u8]) -> Result<(Coins, Coins)> {
    Ok((
        eqmac_coins_at(profile, 2956)?,
        eqmac_coins_at(profile, 2940)?,
    ))
}

/// Four signed counts, platinum first, from `start` in `EQMac`'s profile.
fn eqmac_coins_at(profile: &[u8], start: usize) -> Result<Coins> {
    ensure!(
        profile.len() == crate::quarm::PROFILE_SIZE,
        "unexpected EQMac profile layout"
    );
    let coin = |index: usize| {
        u32::try_from(word(profile, start + index * 4).cast_signed())
            .map_err(|_| anyhow::anyhow!("negative EQMac coin count"))
    };
    Ok(Coins {
        platinum: coin(0)?,
        gold: coin(1)?,
        silver: coin(2)?,
        copper: coin(3)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eqmacs_profile_says_what_is_carried_on_the_cursor_and_in_the_bank() {
        let mut profile = vec![0; crate::quarm::PROFILE_SIZE];
        let mut put = |start: usize, counts: [i32; 4]| {
            for (index, count) in counts.into_iter().enumerate() {
                let at = start + index * 4;
                profile[at..at + 4].copy_from_slice(&count.to_le_bytes());
            }
        };
        put(2924, [1, 2, 3, 4]);
        put(2940, [50, 0, 0, 9]);
        put(2956, [0, 7, 0, 0]);
        let coins = |platinum, gold, silver, copper| Coins {
            platinum,
            gold,
            silver,
            copper,
        };
        assert_eq!(eqmac_coins(&profile).unwrap(), coins(1, 2, 3, 4));
        assert_eq!(
            eqmac_elsewhere(&profile).unwrap(),
            (coins(0, 7, 0, 0), coins(50, 0, 0, 9))
        );
        profile[2936..2940].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(eqmac_coins(&profile).is_err());
        assert!(eqmac_elsewhere(&profile[1..]).is_err());
    }

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
    fn a_wallet_moves_only_what_a_place_holds() {
        let mut wallet = Wallet {
            purse: Some(Coins {
                gold: 12,
                ..Coins::default()
            }),
            bank: Some(Coins::default()),
            ..Wallet::default()
        };
        let pick_up = CoinTransfer {
            from: CoinPlace::Purse,
            to: CoinPlace::Cursor,
            coin: Coin::Gold,
            into: Coin::Gold,
            amount: 11,
        };
        wallet.apply(pick_up).unwrap();
        assert_eq!(wallet.cursor.of(Coin::Gold), 11);
        assert_eq!(wallet.purse.unwrap().of(Coin::Gold), 1);
        assert!(wallet.apply(pick_up).is_err(), "only one gold is left");
        // Into the bank's platinum: one platinum arrives, one gold stays.
        wallet
            .apply(CoinTransfer {
                from: CoinPlace::Cursor,
                to: CoinPlace::Bank,
                into: Coin::Platinum,
                ..pick_up
            })
            .unwrap();
        assert_eq!(wallet.bank.unwrap().of(Coin::Platinum), 1);
        assert_eq!(wallet.cursor.of(Coin::Gold), 1);
        // Too few to change kind, or to a place not known yet.
        let mut unknown = Wallet::default();
        assert!(unknown
            .apply(CoinTransfer {
                amount: 1,
                ..pick_up
            })
            .is_err());
    }

    #[test]
    fn purchases_and_loot_change_the_purse_kind_by_kind_as_servers_do() {
        let coins = |platinum, gold, silver, copper| Coins {
            platinum,
            gold,
            silver,
            copper,
        };
        let paid = |purse, price| {
            let mut wallet = Wallet {
                purse: Some(purse),
                ..Wallet::default()
            };
            wallet.pay(price);
            wallet.purse.unwrap()
        };
        // Copper first: the live check's purchase at a local server.
        assert_eq!(paid(coins(0, 2, 17, 62), 10), coins(0, 2, 17, 52));
        // Then silver, the change coming back in copper.
        assert_eq!(paid(coins(0, 1, 3, 0), 25), coins(0, 1, 0, 5));
        // A platinum covering the rest comes back as every smaller kind.
        assert_eq!(paid(coins(2, 0, 0, 4), 129), coins(1, 8, 7, 5));
        assert_eq!(paid(coins(1, 0, 0, 5), 5), coins(1, 0, 0, 0));
        // A purse that cannot cover the price was wrong; it empties.
        assert_eq!(paid(coins(0, 0, 9, 9), 500), Coins::default());
        let mut wallet = Wallet {
            purse: Some(coins(0, 2, 17, 52)),
            ..Wallet::default()
        };
        wallet.add_to_purse(coins(0, 0, 12, 3));
        assert_eq!(wallet.purse, Some(coins(0, 2, 29, 55)));
        // Nothing is known before admission, and nothing is guessed.
        let mut unknown = Wallet::default();
        unknown.add_to_purse(coins(0, 1, 0, 0));
        unknown.pay(1);
        assert_eq!(unknown.purse, None);
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
