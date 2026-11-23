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
/// `OP_Split` on TAKP and Quarm (Mac 0x4131).
pub const EQMAC_SPLIT_OPCODE: u16 = 0x3141;
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

/// Shares coins with the group on TAKP or Quarm: the same 16-byte
/// `Split_Struct` on the `EQMac` opcode, the only size both servers'
/// `Handle_OP_Split` read (Quarm fc75fcf, TAKP 44afcbf). Each refusal
/// (12328, 13112) gives the coins back as purse additions
/// (`Client::SendClientMoney`), and a split that goes through pays each
/// member's share the same way (`Group::SplitMoney`); Quarm leaves
/// self-found members out of a manual split.
///
/// # Errors
/// Rejects a split of nothing.
pub fn eqmac_split(coins: Coins) -> Result<EncodedCommand> {
    let mut command = titanium_split(coins)?;
    command.opcode = EQMAC_SPLIT_OPCODE;
    Ok(command)
}

/// Turns automatic splitting on or off on TAKP or Quarm: appearance type 28
/// on the `EQMac` appearance update, the same 8 bytes; both servers'
/// `Handle_OP_SpawnAppearance` take 1 as on.
///
/// # Errors
/// Rejects a zero spawn ID.
pub fn eqmac_auto_split(spawn_id: u16, on: bool) -> Result<EncodedCommand> {
    let mut command = titanium_auto_split(spawn_id, on)?;
    command.opcode = crate::quarm::ZONE_SPAWN_APPEARANCE;
    Ok(command)
}

/// Where the unpacked `EQMac` profile says whether looted coins are shared:
/// a byte at 3348 on both servers (`PlayerProfile_Struct::autosplit`,
/// `common/patches/mac_structs.h`).
const EQMAC_PROFILE_AUTO_SPLIT: usize = 3348;

/// Whether the unpacked `EQMac` profile says looted coins are shared.
///
/// # Errors
/// Rejects a profile too short to say.
pub(crate) fn eqmac_profile(profile: &[u8]) -> Result<bool> {
    profile
        .get(EQMAC_PROFILE_AUTO_SPLIT)
        .map(|on| *on != 0)
        .ok_or_else(|| anyhow::anyhow!("EQMac profile too short for automatic splitting"))
}

/// Whether a formatted message is TAKP's or Quarm's refusal of a split,
/// 12328 or 13112 by its string number at +2. Each comes before the
/// additions that give the coins back.
#[must_use]
pub fn eqmac_refused(opcode: u16, body: &[u8]) -> bool {
    opcode == crate::who::eqmac::FORMATTED_OPCODE
        && body
            .get(2..4)
            .map(|id| u32::from(u16::from_le_bytes([id[0], id[1]])))
            .is_some_and(|id| id == NO_GROUP || id == TOO_LITTLE)
}

/// The purse TAKP or Quarm keeps after it takes a split's worth from it,
/// as both servers' `Client::TakeMoneyFromPP(uint64 copper)` takes it for
/// `Handle_OP_Split`: the copper value comes out of copper first, then
/// silver, gold and platinum, and the kind it stops in is broken into
/// change downward, all without a word to the client. None when the purse
/// is worth less, as the server then refuses with 13112.
///
/// So a split takes exactly the coins named only when copper covers it, or
/// when the lower kinds hold nothing; otherwise the server's purse differs
/// from the one the client took the coins from (Quarm fc75fcf, TAKP
/// 44afcbf; what the `EQMac` program shows then is unread, 6.585d).
#[must_use]
pub fn eqmac_take(purse: Coins, worth: u64) -> Option<Coins> {
    let tiers = [
        u64::from(purse.copper),
        u64::from(purse.silver) * 10,
        u64::from(purse.gold) * 100,
        u64::from(purse.platinum) * 1000,
    ];
    if tiers.iter().sum::<u64>() < worth {
        return None;
    }
    let mut kept = purse;
    let mut owed = worth;
    // Copper: what is left stays as copper.
    if tiers[0] > owed {
        kept.copper = narrow(tiers[0] - owed);
        return Some(kept);
    }
    owed -= tiers[0];
    kept.copper = 0;
    // Silver: what is left stays as silver, its odd copper as copper.
    if tiers[1] > owed {
        let left = tiers[1] - owed;
        kept.silver = narrow(left / 10);
        kept.copper += narrow(left % 10);
        return Some(kept);
    }
    owed -= tiers[1];
    kept.silver = 0;
    // Gold: broken into gold, silver and copper.
    if tiers[2] > owed {
        let left = tiers[2] - owed;
        kept.gold = narrow(left / 100);
        kept.silver += narrow(left % 100 / 10);
        kept.copper += narrow(left % 10);
        return Some(kept);
    }
    owed -= tiers[2];
    kept.gold = 0;
    // Platinum: broken into every kind; the lower ones are empty by now.
    let left = tiers[3] - owed;
    kept.platinum = narrow(left / 1000);
    kept.gold += narrow(left % 1000 / 100);
    kept.silver += narrow(left % 100 / 10);
    kept.copper = narrow(left % 10);
    Some(kept)
}

/// A count of coins that came from 32-bit counts.
fn narrow(count: u64) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
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
    }

    #[test]
    fn eqmac_splits_use_the_mac_opcodes_and_the_profile_byte_at_3348() {
        let split = eqmac_split(Coins {
            platinum: 1,
            gold: 2,
            silver: 3,
            copper: 4,
        })
        .unwrap();
        assert_eq!(split.opcode, 0x3141);
        assert_eq!(split.body, [1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0]);
        assert!(eqmac_split(Coins::default()).is_err());
        let on = eqmac_auto_split(7, true).unwrap();
        assert_eq!(on.opcode, 0xf540);
        assert_eq!(on.body, [7, 0, 28, 0, 1, 0, 0, 0]);
        assert_eq!(
            eqmac_auto_split(7, false).unwrap().body,
            [7, 0, 28, 0, 0, 0, 0, 0]
        );
        let mut profile = vec![0; crate::quarm::PROFILE_SIZE];
        assert!(!eqmac_profile(&profile).unwrap());
        profile[3348] = 1;
        assert!(eqmac_profile(&profile).unwrap());
        assert!(eqmac_profile(&profile[..3348]).is_err());
    }

    #[test]
    fn only_the_two_split_refusals_on_the_formatted_opcode_count() {
        let line = |id: u16| {
            let mut body = vec![0, 0];
            body.extend_from_slice(&id.to_le_bytes());
            body.extend_from_slice(&[0; 8]);
            body
        };
        assert!(eqmac_refused(0x3642, &line(12328)));
        assert!(eqmac_refused(0x3642, &line(13112)));
        assert!(!eqmac_refused(0x3642, &line(5029)));
        assert!(!eqmac_refused(0x3d41, &line(12328)));
        assert!(!eqmac_refused(0x3642, &[0, 0, 0x28]));
    }

    fn coins(platinum: u32, gold: u32, silver: u32, copper: u32) -> Coins {
        Coins {
            platinum,
            gold,
            silver,
            copper,
        }
    }

    #[test]
    fn the_mac_servers_take_a_split_from_copper_first_and_make_change() {
        let worth = |c: Coins| c.total_copper();
        // Copper covers it: the rest stays copper.
        assert_eq!(eqmac_take(coins(1, 1, 1, 9), 4), Some(coins(1, 1, 1, 5)));
        // Copper exactly spent goes on to silver with nothing owed.
        assert_eq!(eqmac_take(coins(0, 0, 3, 4), 4), Some(coins(0, 0, 3, 0)));
        // Silver: 25 copper from 3 copper and 4 silver leaves 1 silver 8 copper.
        assert_eq!(eqmac_take(coins(0, 0, 4, 3), 25), Some(coins(0, 0, 1, 8)));
        // Gold: 1 silver from 2 gold leaves 1 gold 9 silver.
        assert_eq!(eqmac_take(coins(0, 2, 0, 0), 10), Some(coins(0, 1, 9, 0)));
        // The reviewer's case: 1 platinum from 500 copper and 2 platinum
        // takes the copper and 500 more from platinum.
        assert_eq!(
            eqmac_take(coins(2, 0, 0, 500), worth(coins(1, 0, 0, 0))),
            Some(coins(1, 5, 0, 0))
        );
        // Platinum broken all the way down.
        assert_eq!(eqmac_take(coins(1, 0, 0, 0), 1), Some(coins(0, 9, 9, 9)));
        // Exactly everything.
        assert_eq!(eqmac_take(coins(1, 1, 1, 1), 1111), Some(coins(0, 0, 0, 0)));
        // Worth more than the purse: the server's 13112.
        assert_eq!(eqmac_take(coins(1, 0, 0, 0), 1001), None);
        // Naming only copper the purse holds takes exactly that.
        assert_eq!(eqmac_take(coins(3, 2, 1, 7), 7), Some(coins(3, 2, 1, 0)));
        assert!(titanium_auto_split(0, true).is_err());
        let mut profile = vec![0; 13260];
        assert!(!titanium_profile(&profile).unwrap());
        profile[13256] = 1;
        assert!(titanium_profile(&profile).unwrap());
        assert!(titanium_profile(&profile[..13259]).is_err());
    }
}
