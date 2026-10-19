//! A quest's word on the player's base stats (`OP_IncreaseStats`), which
//! `EQEmu` alone sends, from a quest's `setstat` and `incstat`
//! (`Client::SetStats`, `Client::IncStats`).
//!
//! Each record names one stat, and only with a value above 0, but the same
//! bytes carry the stat's new value from `setstat` and how far it rises
//! from `incstat`, by which the server's own copy rises twice; nothing in
//! the record tells the two apart, so the next profile has the stats as the
//! server keeps them. A value above 255 arrives as its low byte, and a
//! quest that names a resist sends a record that names nothing. The record
//! has room for the five resists and a second copy of every field, which
//! `EQEmu` never sets.
//!
//! Layout reference: `IncreaseStat_Struct` (`EQEmu`'s
//! `common/eq_packet_structs.h`), 141 bytes: a byte the server leaves 0, the
//! seven stats from 1 (strength, stamina, agility, dexterity, intelligence,
//! wisdom and charisma) and the five resists, one byte each, and the copies
//! from 129.
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_IncreaseStats`.
pub const INCREASE_STATS_OPCODE: u16 = 0x5b7b;
/// The record's length.
const LENGTH: usize = 141;

/// A quest's word on the player's base stats: each stat's value as the
/// server named it, 0 for a stat it left alone. A value is the stat's new
/// base value or how far it rose, which the record does not say (see
/// [`crate::stats`]).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct StatChange {
    /// Strength.
    pub strength: u8,
    /// Stamina.
    pub stamina: u8,
    /// Agility.
    pub agility: u8,
    /// Dexterity.
    pub dexterity: u8,
    /// Intelligence.
    pub intelligence: u8,
    /// Wisdom.
    pub wisdom: u8,
    /// Charisma.
    pub charisma: u8,
}

/// Decodes `OP_IncreaseStats`: the seven stats, one byte each from 1.
///
/// # Errors
/// Rejects any length other than 141 bytes.
pub fn titanium_change(body: &[u8]) -> Result<StatChange> {
    ensure!(body.len() == LENGTH, "invalid stat change length");
    Ok(StatChange {
        strength: body[1],
        stamina: body[2],
        agility: body[3],
        dexterity: body[4],
        intelligence: body[5],
        wisdom: body[6],
        charisma: body[7],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quest_names_one_stat_and_a_resist_names_none() {
        // A quest's `setstat` or `incstat` of 85 for wisdom.
        let mut body = [0; LENGTH];
        body[6] = 85;
        assert_eq!(
            titanium_change(&body).unwrap(),
            StatChange {
                wisdom: 85,
                ..StatChange::default()
            }
        );
        // One for a resist: the record names nothing.
        assert_eq!(
            titanium_change(&[0; LENGTH]).unwrap(),
            StatChange::default()
        );
        for length in [0, LENGTH - 1, LENGTH + 1] {
            assert!(titanium_change(&vec![0; length]).is_err());
        }
    }
}
