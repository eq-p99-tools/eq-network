//! The slot the Titanium program gives a buff landing on the player.
//!
//! A server need not say where a landing buff went: `EQEmu` sends a Titanium
//! client no slot when a buff lands (`Mob::AddBuff` sends its buffs packet
//! to `SoD` and later only), so the program works the slot out itself. Its
//! slot finder 0x414071, called from the action handler 0x414913, is read
//! in R3 (program reads R1-R4): for a player spawn the table holds 38
//! slots; a spell already held keeps its first slot (step 3); otherwise
//! the first empty slot of the spell's band is taken (step 5), a song with
//! no free song slot falling back to the normal band.
//!
//! Step 4, which replaces a stacking conflict in its own slot, is not
//! ported, and needs not be for a landing that follows the server's fade:
//! the program fades by spell, nothing keeps the faded slot, and the slot
//! finder then sees it as a plain empty slot, so the lowest empty slot of
//! the band is taken, not necessarily the faded one (R20 (b), (e), (f)). A
//! landing that comes while a buff it could replace is still held is the
//! caller's to leave unslotted. Step 6, which evicts a song when the band is
//! full, is not ported either: such a landing has no slot here. Step 2's
//! blocker (effect 148) is only partly read, so a landing while a held
//! spell carries it has no slot here.
//!
//! The normal band's top is 15 plus the effect-327 total (R20 (c)); which
//! sources that total sums is unread, so a landing with slots 0-14 full has
//! no slot.
use std::{collections::BTreeMap, ops::Range};

/// How many slots the program's table holds for a player spawn (0x40d933).
pub const TITANIUM_PLAYER_SLOTS: u32 = 38;
/// The song band, count-13 to count-2 for a player (0x40db4e, 0x410db0),
/// for a spell with +0x17d and +0x19b set (0x40d89f; R20 corrects R3).
pub const TITANIUM_SONG_SLOTS: Range<u32> = 25..37;
/// The last slot, which has a band of its own, for a spell with +0x17d and
/// +0x1c4 set (0x41477a-0x414790; R20).
pub const TITANIUM_LAST_SLOT: u32 = 37;
/// The normal band's top without effect 327 (0x410db0, R20 (c)): the band
/// is 15 slots plus the effect-327 total, at most 10 more. Which sources
/// that total sums is unread, so no slot past these 15 is named.
pub const TITANIUM_NORMAL_TOP: u32 = 15;

/// The spell fields that choose a landing's band (R20 (a), from the spells
/// file parser 0x44dbf0; fields counted from 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Band {
    /// Spell +0x17d, field 83: the column the book reads as beneficial.
    pub beneficial: bool,
    /// Spell +0x19b, field 154: with +0x17d, the song band.
    pub short: bool,
    /// Spell +0x1c4, field 168: with +0x17d, the last slot.
    pub last: bool,
}

/// Why a landing was left without a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Unplaced {
    /// The installed spell data lacks a field that chooses the band.
    SpellFieldsUnknown,
    /// The band has no empty slot; the program's eviction (step 6) is not
    /// taken.
    BandFull,
    /// Slots 0-14 are full; whether effect 327 opens more is unread.
    TopUnknown,
    /// A held spell carries effect 148, whose blocking test (step 2) is only
    /// partly read.
    Blocked,
}

/// The slot the program gives `spell` landing on the player, given every
/// slot the player holds and the spell it holds there. `band` is `None`
/// where the spell data lacks the band's fields; `blocker` says whether a
/// held spell carries effect 148.
///
/// # Errors
/// Reports why the slot cannot be named.
pub fn titanium_player(
    held: &BTreeMap<u32, u32>,
    spell: u32,
    band: Option<Band>,
    blocker: bool,
) -> Result<u32, Unplaced> {
    if blocker {
        return Err(Unplaced::Blocked);
    }
    // Step 3: the spell's first slot, wherever it is. Its refusals keep the
    // old record where it is (R20 (g)), so the slot is the same either way.
    if let Some((slot, _)) = held.iter().find(|(_, held)| **held == spell) {
        return Ok(*slot);
    }
    let band = band.ok_or(Unplaced::SpellFieldsUnknown)?;
    let empty = |mut slots: Range<u32>| slots.find(|slot| !held.contains_key(slot));
    // The band (0x41477a-0x414790): the song selector first, then the last
    // slot's.
    let song = band.beneficial && band.short;
    if !song && band.beneficial && band.last {
        return empty(TITANIUM_LAST_SLOT..TITANIUM_PLAYER_SLOTS).ok_or(Unplaced::BandFull);
    }
    if let Some(slot) = song.then(|| empty(TITANIUM_SONG_SLOTS)).flatten() {
        return Ok(slot);
    }
    empty(0..TITANIUM_NORMAL_TOP).ok_or(Unplaced::TopUnknown)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NORMAL: Option<Band> = Some(Band {
        beneficial: false,
        short: false,
        last: false,
    });
    const SONG: Option<Band> = Some(Band {
        beneficial: true,
        short: true,
        last: false,
    });

    fn held(slots: &[(u32, u32)]) -> BTreeMap<u32, u32> {
        slots.iter().copied().collect()
    }

    #[test]
    fn a_new_buff_takes_the_first_empty_normal_slot() {
        assert_eq!(titanium_player(&held(&[]), 278, NORMAL, false), Ok(0));
        // A faded buff's slot is the next landing's, ahead of later ones.
        assert_eq!(
            titanium_player(&held(&[(1, 10), (2, 11)]), 278, NORMAL, false),
            Ok(0)
        );
        assert_eq!(
            titanium_player(&held(&[(0, 9), (2, 11)]), 278, NORMAL, false),
            Ok(1)
        );
        // Songs held do not take normal slots.
        assert_eq!(
            titanium_player(&held(&[(25, 700)]), 278, NORMAL, false),
            Ok(0)
        );
    }

    #[test]
    fn a_held_spell_keeps_its_first_slot() {
        let slots = held(&[(0, 9), (3, 278), (26, 278)]);
        assert_eq!(titanium_player(&slots, 278, NORMAL, false), Ok(3));
        // Even with no spell data: the step needs none.
        assert_eq!(titanium_player(&slots, 278, None, false), Ok(3));
    }

    #[test]
    fn songs_take_the_song_band_then_fall_back_to_the_normal_one() {
        assert_eq!(titanium_player(&held(&[]), 700, SONG, false), Ok(25));
        assert_eq!(
            titanium_player(&held(&[(25, 701), (0, 9)]), 700, SONG, false),
            Ok(26)
        );
        let full: BTreeMap<u32, u32> = TITANIUM_SONG_SLOTS
            .map(|slot| (slot, slot + 1000))
            .collect();
        assert_eq!(titanium_player(&full, 700, SONG, false), Ok(0));
    }

    #[test]
    fn the_last_slot_has_its_own_band_and_each_band_needs_both_its_flags() {
        let last = Some(Band {
            beneficial: true,
            short: false,
            last: true,
        });
        assert_eq!(titanium_player(&held(&[]), 800, last, false), Ok(37));
        assert_eq!(
            titanium_player(&held(&[(37, 801)]), 800, last, false),
            Err(Unplaced::BandFull)
        );
        // +0x17d alone, or +0x19b or +0x1c4 without it: the normal band.
        for band in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let band = Some(Band {
                beneficial: band.0,
                short: band.1,
                last: band.2,
            });
            assert_eq!(titanium_player(&held(&[]), 800, band, false), Ok(0));
        }
    }

    #[test]
    fn the_normal_band_ends_at_15_while_effect_327_is_unread() {
        let fifteen: BTreeMap<u32, u32> = (0..15).map(|slot| (slot, slot + 1000)).collect();
        assert_eq!(
            titanium_player(&fifteen, 278, NORMAL, false),
            Err(Unplaced::TopUnknown)
        );
        // A held effect-148 spell leaves the landing without a slot.
        assert_eq!(
            titanium_player(&held(&[(0, 9)]), 278, NORMAL, true),
            Err(Unplaced::Blocked)
        );
        // Without the band's fields, a new spell has no slot.
        assert_eq!(
            titanium_player(&held(&[]), 278, None, false),
            Err(Unplaced::SpellFieldsUnknown)
        );
    }
}
