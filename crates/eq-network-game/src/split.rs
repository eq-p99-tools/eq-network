//! Splitting coins with the player's group: `/split`, which shares coins
//! from the purse at once (`OP_Split`), and `/autosplit`, which says
//! whether looted coins are shared (an appearance update of type 28).
//!
//! `EQEmu`'s `Client::Handle_OP_Split` notes that "the client removes the
//! money on its own": the server takes the coins from its own count of the
//! purse, sends no money update, and says only when it cannot split, with
//! "You are not in a group! Keep it all." (12328) or "There is not enough
//! to split, keep it." (13112). The share each member gets arrives as their
//! own money news. `/autosplit` sets the profile's `autosplit` from the
//! update's value, 1 on and 0 off (`Client::Handle_OP_SpawnAppearance`,
//! `AppearanceType::Split`).
//!
//! Layout reference: `EQEmu`'s Titanium `Split_Struct`
//! (`common/patches/titanium_structs.h`), which Titanium's wire carries as
//! built, `AppearanceType::Split` (`common/eq_constants.h`) and the string
//! numbers in `zone/string_ids.h`.
use crate::{command::EncodedCommand, world::Coins};
use anyhow::{ensure, Result};

/// `OP_Split`: coins from the purse to share with the group (Titanium).
pub const SPLIT_OPCODE: u16 = 0x4848;
/// The appearance update that turns automatic splitting on or off.
const AUTO_SPLIT: u16 = 28;
/// The server's line when the player is in no group.
pub const NO_GROUP: u32 = 12328;
/// The server's line when the purse cannot cover the split.
pub const TOO_LITTLE: u32 = 13112;

/// Where the Titanium profile says whether looted coins are shared: a
/// 32-bit `autosplit`, 1 on and 0 off (`EQEmu`'s Titanium
/// `PlayerProfile_Struct`).
const PROFILE_AUTO_SPLIT: usize = 13256;

/// Whether the Titanium profile says looted coins are shared.
///
/// # Errors
/// Rejects a profile too short to say.
pub fn titanium_profile(profile: &[u8]) -> Result<bool> {
    let field = profile
        .get(PROFILE_AUTO_SPLIT..PROFILE_AUTO_SPLIT + 4)
        .ok_or_else(|| anyhow::anyhow!("profile too short for automatic splitting"))?;
    Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]) != 0)
}

/// Shares coins from the purse with the group (16 bytes: platinum, gold,
/// silver, copper).
///
/// # Errors
/// Rejects a split of nothing.
pub fn titanium_split(coins: Coins) -> Result<EncodedCommand> {
    ensure!(!coins.is_empty(), "a split needs some coins");
    let mut body = Vec::with_capacity(16);
    for value in [coins.platinum, coins.gold, coins.silver, coins.copper] {
        body.extend_from_slice(&value.to_le_bytes());
    }
    Ok(EncodedCommand {
        opcode: SPLIT_OPCODE,
        body,
    })
}

/// Turns automatic splitting of looted coins on or off.
///
/// # Errors
/// Rejects a zero spawn ID.
pub fn titanium_auto_split(spawn_id: u16, on: bool) -> Result<EncodedCommand> {
    crate::listing::update(spawn_id, AUTO_SPLIT, u32::from(on))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_split_names_each_kind_and_autosplit_is_appearance_type_28() {
        let split = titanium_split(Coins {
            platinum: 1,
            gold: 2,
            silver: 3,
            copper: 4,
        })
        .unwrap();
        assert_eq!(split.opcode, 0x4848);
        assert_eq!(split.body, [1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0]);
        assert!(titanium_split(Coins::default()).is_err());
        let on = titanium_auto_split(7, true).unwrap();
        assert_eq!(on.opcode, crate::listing::APPEARANCE_OPCODE);
        assert_eq!(on.body, [7, 0, 28, 0, 1, 0, 0, 0]);
        assert_eq!(titanium_auto_split(7, false).unwrap().body[4], 0);
        assert!(titanium_auto_split(0, true).is_err());
        let mut profile = vec![0; 13260];
        assert!(!titanium_profile(&profile).unwrap());
        profile[13256] = 1;
        assert!(titanium_profile(&profile).unwrap());
        assert!(titanium_profile(&profile[..13259]).is_err());
    }
}
