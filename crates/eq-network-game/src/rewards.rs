//! What a quest or a task gives the player.
//!
//! `EQEmu` tells a Titanium client of a quest's reward in `OP_Sound`
//! (`Client::QuestReward`), and of coins alone in `OP_CashReward`
//! (`Client::CashReward`: a task's coins and a quest's `givecash`); TAKP
//! and Quarm tell an `EQMac` client of a quest's reward in `OP_Reward`
//! (`Client::QuestReward`). Each adds the coins to the purse without a money
//! update (`Client::AddMoneyToPP`), so the reward is all the player hears of
//! them; the items and the experience come with messages of their own, and
//! the faction with the server's own lines. `EQEmu`'s list of Titanium's
//! opcodes marks `OP_CashReward`'s as unsure, so whether the official client
//! reads it is not known; the server adds the coins all the same.
//!
//! `EQEmu` also sends `OP_Sound` with a fixed record that gives nothing
//! (`Client::SendSound`): for a quest's `ding`, and as the player earns an
//! ability point where its rule `AA:SoundForAAEarned` says so (off by
//! default). Read as a reward, it would give 9,252 experience from spawn 34,
//! so it reads as [`Sound::Ding`](crate::rewards::Sound::Ding).
//!
//! Layout reference: `QuestReward_Struct` and `CashReward_Struct` (`EQEmu`'s
//! `common/eq_packet_structs.h`), and TAKP's 48-byte `QuestReward_Struct`,
//! whose offset comments are wrong, so the offsets here are counted.
use crate::world::Coins;
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_Sound`: a quest's reward, or `EQEmu`'s sound that gives nothing.
pub const SOUND_OPCODE: u16 = 0x541e;
/// `OP_CashReward`: coins alone.
pub const CASH_REWARD_OPCODE: u16 = 0x4c8c;
/// `EQMac`'s `OP_Reward`: a quest's reward.
pub const EQMAC_REWARD_OPCODE: u16 = 0x8040;

/// How many items a reward record holds.
const ITEMS: usize = 8;

/// `Client::SendSound`'s fixed record, set byte by byte
/// (`zone/client.cpp`): item IDs 1 to 7 are -1, and the rest of what it
/// sets names no spawn, experience or faction the server gives.
pub(crate) const DING: [u8; 68] = {
    let mut body = [0; 68];
    body[0] = 0x22;
    body[4] = 0x02;
    body[5] = 0x02;
    body[8] = 0x24;
    body[9] = 0x24;
    body[12] = 0x01;
    body[13] = 0x01;
    body[16] = 0x05;
    let mut at = 40;
    while at < body.len() {
        body[at] = 0xff;
        at += 1;
    }
    body
};

/// What a quest or a task gives the player.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Reward {
    /// The spawn that gives it; none for coins alone, and when the record
    /// names no spawn (`EQEmu`'s quest reward with no giver).
    pub giver_id: Option<u16>,
    /// The coins the server added to the purse.
    pub coins: Coins,
    /// The experience the server gave, 0 for none.
    pub experience: u32,
    /// The items the server gave, by their IDs, in the record's order.
    pub items: Vec<u32>,
}

/// What a Titanium `OP_Sound` is.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum Sound {
    /// A quest's reward.
    Reward(Reward),
    /// The sound the official client plays for a quest's `ding`:
    /// `EQEmu`'s fixed record, which gives nothing.
    Ding,
}

/// Decodes Titanium's `OP_Sound`: `Client::SendSound`'s fixed record, or a
/// quest's reward (`QuestReward_Struct`, 68 bytes: the giver and the player
/// as 32-bit spawns, the experience at 8, the faction and its modifier, the
/// four coins from 20, copper first, and eight item IDs from 36, 0 or -1
/// for none). The player is always the one the server sends it to, and the
/// faction's change comes in the server's own lines, so neither is read.
///
/// # Errors
/// Rejects any length other than 68 bytes.
pub fn titanium_sound(body: &[u8]) -> Result<Sound> {
    ensure!(body.len() == DING.len(), "invalid Titanium reward length");
    if body == DING {
        return Ok(Sound::Ding);
    }
    Ok(Sound::Reward(Reward {
        giver_id: spawn(word(body, 0)),
        coins: Coins {
            platinum: word(body, 32),
            gold: word(body, 28),
            silver: word(body, 24),
            copper: word(body, 20),
        },
        experience: word(body, 8),
        items: (0..ITEMS)
            .filter_map(|index| item(word(body, 36 + index * 4).cast_signed()))
            .collect(),
    }))
}

/// Decodes `OP_CashReward`: coins alone (`CashReward_Struct`, four 32-bit
/// counts, copper first), with no giver.
///
/// # Errors
/// Rejects any length other than 16 bytes.
pub fn cash_reward(body: &[u8]) -> Result<Reward> {
    ensure!(body.len() == 16, "invalid cash reward length");
    Ok(Reward {
        coins: Coins {
            platinum: word(body, 12),
            gold: word(body, 8),
            silver: word(body, 4),
            copper: word(body, 0),
        },
        ..Reward::default()
    })
}

/// Decodes `EQMac`'s `OP_Reward` (48 bytes: the giver and the player as
/// 16-bit spawns, the experience at 4, the faction and its modifier, the
/// four coins from 16, copper first, and eight 16-bit item IDs from 32).
/// TAKP and Quarm add a count only when it raises the purse's, and
/// experience only above 0 (`Client::AddMoneyToPP`,
/// `Client::QuestReward`), so a negative count or experience reads as 0.
///
/// # Errors
/// Rejects any length other than 48 bytes.
pub fn eqmac_reward(body: &[u8]) -> Result<Reward> {
    ensure!(body.len() == 48, "invalid EQMac reward length");
    let count = |offset| u32::try_from(word(body, offset).cast_signed()).unwrap_or(0);
    Ok(Reward {
        giver_id: spawn(u32::from(short(body, 0))),
        coins: Coins {
            platinum: count(28),
            gold: count(24),
            silver: count(20),
            copper: count(16),
        },
        experience: count(4),
        items: (0..ITEMS)
            .filter_map(|index| item(i32::from(short(body, 32 + index * 2).cast_signed())))
            .collect(),
    })
}

/// The spawn a record names, none for 0 or a number beyond any spawn's.
fn spawn(id: u32) -> Option<u16> {
    u16::try_from(id).ok().filter(|&id| id != 0)
}

/// An item ID, none for 0 or below.
fn item(id: i32) -> Option<u32> {
    u32::try_from(id).ok().filter(|&id| id != 0)
}

fn word(body: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        body[offset..offset + 4]
            .try_into()
            .expect("checked reward length"),
    )
}

fn short(body: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([body[offset], body[offset + 1]])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Titanium reward record from `giver` to spawn 9, with these coins
    /// (copper first), experience and first two item IDs, the rest -1.
    fn titanium(giver: u32, coins: [u32; 4], experience: u32, items: [i32; 2]) -> Vec<u8> {
        let mut body = Vec::new();
        for value in [giver, 9, experience, 1205, 1] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        for value in coins {
            body.extend_from_slice(&value.to_le_bytes());
        }
        for value in items.into_iter().chain([-1; ITEMS - 2]) {
            body.extend_from_slice(&value.to_le_bytes());
        }
        body
    }

    #[test]
    fn a_titanium_reward_reads_the_giver_coins_experience_and_items() {
        let body = titanium(581, [5, 4, 3, 2], 1200, [13_005, 0]);
        assert_eq!(
            titanium_sound(&body).unwrap(),
            Sound::Reward(Reward {
                giver_id: Some(581),
                coins: Coins {
                    platinum: 2,
                    gold: 3,
                    silver: 4,
                    copper: 5,
                },
                experience: 1200,
                items: vec![13_005],
            })
        );
        // No giver, as a task's quest reward has none.
        let body = titanium(0, [0; 4], 0, [0, 0]);
        assert_eq!(
            titanium_sound(&body).unwrap(),
            Sound::Reward(Reward::default())
        );
        // A giver beyond any spawn's 16 bits names none.
        let body = titanium(0x1_0000, [1, 0, 0, 0], 0, [0, 0]);
        let Sound::Reward(reward) = titanium_sound(&body).unwrap() else {
            panic!("a reward");
        };
        assert_eq!((reward.giver_id, reward.coins.copper), (None, 1));
        for length in [67, 69] {
            assert!(titanium_sound(&vec![0; length]).is_err());
        }
    }

    #[test]
    fn eqemus_sound_that_gives_nothing_is_the_ding() {
        assert_eq!(titanium_sound(&DING).unwrap(), Sound::Ding);
        // Any other byte makes it a reward.
        let mut body = DING;
        body[20] = 1;
        assert!(matches!(titanium_sound(&body).unwrap(), Sound::Reward(_)));
    }

    #[test]
    fn a_cash_reward_is_coins_alone() {
        let mut body = Vec::new();
        for value in [7u32, 6, 5, 4] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        assert_eq!(
            cash_reward(&body).unwrap(),
            Reward {
                coins: Coins {
                    platinum: 4,
                    gold: 5,
                    silver: 6,
                    copper: 7,
                },
                ..Reward::default()
            }
        );
        assert!(cash_reward(&body[..15]).is_err());
    }

    #[test]
    fn an_eqmac_reward_reads_its_counted_offsets_and_nothing_below_0() {
        let mut body = Vec::new();
        for value in [581u16, 9] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        for value in [1200i32, 1205, 1, 5, -4, 3, 2] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        for value in [13_005i16, -1, 0, 1001, 0, 0, 0, 0] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        assert_eq!(body.len(), 48);
        assert_eq!(
            eqmac_reward(&body).unwrap(),
            Reward {
                giver_id: Some(581),
                coins: Coins {
                    platinum: 2,
                    gold: 3,
                    silver: 0,
                    copper: 5,
                },
                experience: 1200,
                items: vec![13_005, 1001],
            }
        );
        // Experience below 0 gives none.
        body[4..8].copy_from_slice(&(-50i32).to_le_bytes());
        assert_eq!(eqmac_reward(&body).unwrap().experience, 0);
        assert!(eqmac_reward(&body[..47]).is_err());
    }
}
