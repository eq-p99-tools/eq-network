//! The player's bandolier sets and potion belt as the Titanium profile
//! carries them, read only.
//!
//! A bandolier set is a name and the four weapon slots' items it puts in
//! place (primary, secondary, range, ammo); the potion belt is four items
//! the player can use from it. Stock `EQEmu` keeps both in the profile and
//! changes them only on the client's word (`Client::CreateBandolier`,
//! `Client::RemoveBandolier`, `Client::Handle_OP_PotionBelt`); it sends no
//! message of its own for them, so the profile at each admission is what
//! the session knows.
//!
//! Layout reference: `EQEmu`'s Titanium `PlayerProfile_Struct`,
//! `Bandolier_Struct`, `BandolierItem_Struct`, `PotionBelt_Struct` and
//! `PotionBeltItem_Struct` (`common/patches/titanium_structs.h`), which its
//! Titanium translator copies as is.

use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// The Titanium profile's length, as the session's other profile readers
/// take it.
const PROFILE_LENGTH: usize = 19592;
/// Where the four sets start (`bandoliers`).
const BANDOLIERS_AT: usize = 6164;
/// Where the potion belt starts (`potionbelt`).
const BELT_AT: usize = 12564;
/// How many sets, and how many potion belt slots.
pub const SETS: usize = 4;
/// How many items a set holds: primary, secondary, range and ammo.
pub const SET_ITEMS: usize = 4;
/// How many potion belt slots.
pub const BELT_SLOTS: usize = 4;
/// A set's name field.
const SET_NAME_LENGTH: usize = 32;
/// An item's record: its number, its icon and its 64-byte name.
const ITEM_LENGTH: usize = 4 + 4 + 64;
/// A set's record: its name, then its items.
const SET_LENGTH: usize = SET_NAME_LENGTH + SET_ITEMS * ITEM_LENGTH;

/// An item a set or the belt names, as the profile keeps it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BeltItem {
    /// The item's number.
    pub item_id: u32,
    /// Its icon.
    pub icon: u32,
    /// Its name.
    pub name: String,
}

/// A bandolier set: its name and the item for each weapon slot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BandolierSet {
    /// The name the player gave it; empty where the profile has none.
    pub name: String,
    /// The primary, secondary, range and ammo items, each `None` where the
    /// set leaves that slot empty.
    pub items: [Option<BeltItem>; SET_ITEMS],
}

/// The four sets, each `None` where it is empty, and the four potion belt
/// slots, each `None` where it is empty.
pub type Profile = ([Option<BandolierSet>; SETS], [Option<BeltItem>; BELT_SLOTS]);

/// Reads the sets and the potion belt from the Titanium profile. An item
/// is empty when its number is 0, whatever its icon and name (`EQEmu`'s
/// remove clears only the number and icon); a set is empty when it has
/// neither a name nor an item.
///
/// # Errors
/// Rejects a profile of another length, and a name with no end within its
/// field.
pub fn titanium_profile(profile: &[u8]) -> Result<Profile> {
    ensure!(
        profile.len() == PROFILE_LENGTH,
        "unexpected Titanium profile layout"
    );
    let mut sets: [Option<BandolierSet>; SETS] = Default::default();
    for (index, set) in sets.iter_mut().enumerate() {
        let record = &profile[BANDOLIERS_AT + index * SET_LENGTH..][..SET_LENGTH];
        let name = text(&record[..SET_NAME_LENGTH]).context("bandolier name without its end")?;
        let mut items: [Option<BeltItem>; SET_ITEMS] = Default::default();
        for (place, item) in items.iter_mut().enumerate() {
            *item = belt_item(&record[SET_NAME_LENGTH + place * ITEM_LENGTH..][..ITEM_LENGTH])?;
        }
        if !name.is_empty() || items.iter().any(Option::is_some) {
            *set = Some(BandolierSet { name, items });
        }
    }
    let mut belt: [Option<BeltItem>; BELT_SLOTS] = Default::default();
    for (index, slot) in belt.iter_mut().enumerate() {
        *slot = belt_item(&profile[BELT_AT + index * ITEM_LENGTH..][..ITEM_LENGTH])?;
    }
    Ok((sets, belt))
}

/// One item record, `None` when its number is 0.
fn belt_item(record: &[u8]) -> Result<Option<BeltItem>> {
    let item_id = word(record, 0);
    if item_id == 0 {
        return Ok(None);
    }
    Ok(Some(BeltItem {
        item_id,
        icon: word(record, 4),
        name: text(&record[8..]).context("bandolier item name without its end")?,
    }))
}

/// A field's text, to its first NUL.
fn text(field: &[u8]) -> Option<String> {
    let end = field.iter().position(|byte| *byte == 0)?;
    Some(String::from_utf8_lossy(&field[..end]).into_owned())
}

fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_item(profile: &mut [u8], at: usize, id: u32, icon: u32, name: &str) {
        profile[at..at + 4].copy_from_slice(&id.to_le_bytes());
        profile[at + 4..at + 8].copy_from_slice(&icon.to_le_bytes());
        profile[at + 8..at + 8 + name.len()].copy_from_slice(name.as_bytes());
    }

    fn set_at(index: usize) -> usize {
        BANDOLIERS_AT + index * SET_LENGTH
    }

    fn item_at(set: usize, place: usize) -> usize {
        set_at(set) + SET_NAME_LENGTH + place * ITEM_LENGTH
    }

    #[test]
    fn the_records_sit_where_the_profile_puts_them() {
        assert_eq!((ITEM_LENGTH, SET_LENGTH), (72, 320));
        assert_eq!(BANDOLIERS_AT + SETS * SET_LENGTH, 7444);
        assert_eq!(BELT_AT + BELT_SLOTS * ITEM_LENGTH, 12852);
    }

    #[test]
    fn each_set_and_belt_slot_is_read_field_for_field() {
        let mut profile = vec![0; PROFILE_LENGTH];
        profile[set_at(0)..set_at(0) + 5].copy_from_slice(b"Melee");
        put_item(
            &mut profile,
            item_at(0, 0),
            5019,
            592,
            "Fine Steel Long Sword",
        );
        put_item(&mut profile, item_at(0, 1), 9006, 678, "Round Shield");
        // Set 2: ammo only, and no name.
        put_item(&mut profile, item_at(2, 3), 8005, 1250, "Arrow");
        put_item(&mut profile, BELT_AT, 14_015, 776, "Distillate of Health");
        put_item(
            &mut profile,
            BELT_AT + 3 * ITEM_LENGTH,
            14_001,
            777,
            "Bandage",
        );
        let (sets, belt) = titanium_profile(&profile).unwrap();
        let item = |item_id, icon, name: &str| {
            Some(BeltItem {
                item_id,
                icon,
                name: name.into(),
            })
        };
        assert_eq!(
            sets[0],
            Some(BandolierSet {
                name: "Melee".into(),
                items: [
                    item(5019, 592, "Fine Steel Long Sword"),
                    item(9006, 678, "Round Shield"),
                    None,
                    None,
                ],
            })
        );
        assert_eq!(sets[1], None);
        assert_eq!(
            sets[2],
            Some(BandolierSet {
                name: String::new(),
                items: [None, None, None, item(8005, 1250, "Arrow")],
            })
        );
        assert_eq!(sets[3], None);
        assert_eq!(
            belt,
            [
                item(14_015, 776, "Distillate of Health"),
                None,
                None,
                item(14_001, 777, "Bandage"),
            ]
        );
    }

    #[test]
    fn an_item_numbered_0_is_empty_whatever_its_name() {
        let mut profile = vec![0; PROFILE_LENGTH];
        // EQEmu's remove leaves the name.
        put_item(&mut profile, item_at(1, 0), 0, 0, "Rusty Short Sword");
        put_item(&mut profile, BELT_AT + ITEM_LENGTH, 0, 0, "Old Potion");
        let (sets, belt) = titanium_profile(&profile).unwrap();
        assert_eq!(sets[1], None);
        assert_eq!(belt[1], None);
    }

    #[test]
    fn a_name_without_its_end_or_another_length_is_refused() {
        let mut profile = vec![0; PROFILE_LENGTH];
        profile[set_at(3)..set_at(3) + SET_NAME_LENGTH].fill(b'a');
        assert!(titanium_profile(&profile).is_err());
        let mut profile = vec![0; PROFILE_LENGTH];
        put_item(&mut profile, item_at(0, 2), 1, 1, "");
        profile[item_at(0, 2) + 8..item_at(0, 2) + ITEM_LENGTH].fill(b'b');
        assert!(titanium_profile(&profile).is_err());
        let mut profile = vec![0; PROFILE_LENGTH];
        put_item(&mut profile, BELT_AT, 1, 1, "");
        profile[BELT_AT + 8..BELT_AT + ITEM_LENGTH].fill(b'c');
        assert!(titanium_profile(&profile).is_err());
        assert!(titanium_profile(&vec![0; PROFILE_LENGTH - 1]).is_err());
    }
}
