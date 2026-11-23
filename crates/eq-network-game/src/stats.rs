//! A word on a spawn's base stats (`OP_IncreaseStats`), as the Titanium
//! program reads it (read R49-6f, 0x455fff): 12 bytes, the spawn id, which
//! stat (1 to 7: strength, stamina, agility, dexterity, intelligence, wisdom
//! and charisma), and the amount, each 32 bits.
//!
//! The program adds an amount above 0 to the player's own base stat and
//! shows no line; for another spawn it shows 0x3273 (amount below 0) or
//! 0x3274, colour 15, with no arguments.
//!
//! `EQEmu` sends something else under this opcode: its 141-byte
//! `IncreaseStat_Struct` (`common/eq_packet_structs.h`), from a quest's
//! `setstat` and `incstat` (`Client::SetStats`, `Client::IncStats`). The
//! program doesn't read that form, so the session doesn't either: it is
//! refused here, and the session counts each one as a gap with its length.
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_IncreaseStats`.
pub const INCREASE_STATS_OPCODE: u16 = 0x5b7b;
/// The length the program reads.
const LENGTH: usize = 12;
/// The length of `EQEmu`'s `IncreaseStat_Struct`, which the program doesn't
/// read.
const EQEMU_LENGTH: usize = 141;

/// A word on a spawn's base stat, as the program reads it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct StatIncrease {
    /// The spawn.
    pub spawn_id: u32,
    /// Which stat, 1 to 7 (strength, stamina, agility, dexterity,
    /// intelligence, wisdom, charisma); the program changes nothing for
    /// another number.
    pub stat: u32,
    /// How far it rises, or falls below 0.
    pub amount: i32,
}

/// Decodes `OP_IncreaseStats` as the Titanium program reads it.
///
/// # Errors
/// Rejects any length but 12 bytes, `EQEmu`'s 141-byte form among them.
pub fn titanium_increase(body: &[u8]) -> Result<StatIncrease> {
    ensure!(
        body.len() != EQEMU_LENGTH,
        "EQEmu's 141-byte IncreaseStat_Struct, which the Titanium program does not read (R49-6f)"
    );
    ensure!(body.len() == LENGTH, "invalid stat increase length");
    let word = |at: usize| [body[at], body[at + 1], body[at + 2], body[at + 3]];
    Ok(StatIncrease {
        spawn_id: u32::from_le_bytes(word(0)),
        stat: u32::from_le_bytes(word(4)),
        amount: i32::from_le_bytes(word(8)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_programs_twelve_bytes_are_read_and_eqemus_form_is_refused() {
        let mut body = Vec::new();
        body.extend_from_slice(&7u32.to_le_bytes());
        body.extend_from_slice(&6u32.to_le_bytes());
        body.extend_from_slice(&(-3i32).to_le_bytes());
        assert_eq!(
            titanium_increase(&body).unwrap(),
            StatIncrease {
                spawn_id: 7,
                stat: 6,
                amount: -3,
            }
        );
        let refused = titanium_increase(&[0; EQEMU_LENGTH]).unwrap_err();
        assert!(refused.to_string().contains("141-byte"), "{refused}");
        for length in [0, LENGTH - 1, LENGTH + 1] {
            assert!(titanium_increase(&vec![0; length]).is_err());
        }
    }
}
