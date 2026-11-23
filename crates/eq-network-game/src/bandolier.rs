//! The player's bandolier sets and potion belt as the Titanium profile
//! carries them (6.559), and the player's saving and clearing of a set and
//! setting and clearing of a belt slot (6.564), and the swap to a set and
//! what it does to the inventory (6.566, [`crate::bandolier::swap()`]).
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

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;

mod swapping;
pub use swapping::swap;

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

/// Titanium's `OP_Bandolier`: saving, clearing and swapping to a set.
pub const OPCODE: u16 = 0x6f0c;
/// Titanium's `OP_PotionBelt`: setting and clearing a belt slot.
pub const BELT_OPCODE: u16 = 0x0719;
/// The length of every `OP_Bandolier` (`BandolierCreate_Struct` and its
/// like, packed); the server drops any other.
pub const LENGTH: usize = 40;
/// The length of `OP_PotionBelt` (`MovePotionToBelt_Struct`).
pub const BELT_LENGTH: usize = 12;
/// The most bytes of a set's name, its end aside.
pub const MOST_NAME: usize = SET_NAME_LENGTH - 1;
/// `OP_Bandolier`'s save (`bandolierCreate`).
const SAVE: u32 = 0;
/// `OP_Bandolier`'s clear (`bandolierRemove`).
const CLEAR: u32 = 1;
/// `OP_Bandolier`'s swap (`bandolierSet`).
const SWAP: u32 = 2;
/// `OP_PotionBelt`'s set; the server clears for any other action.
const BELT_SET: u32 = 0;
/// The action sent to clear a belt slot (inferred, R49-45 c).
const BELT_CLEAR: u32 = 1;
/// The Titanium slots a set's items come from, in its order: primary,
/// secondary, range and ammo (`BandolierSlotToWeaponSlot`).
pub const WEAPON_SLOTS: [crate::inventory::InventorySlot; SET_ITEMS] = [
    crate::inventory::InventorySlot(13),
    crate::inventory::InventorySlot(14),
    crate::inventory::InventorySlot(11),
    crate::inventory::InventorySlot(21),
];
/// The item type of a potion (`ItemTypePotion`), which the belt takes.
pub const POTION: u8 = 21;

/// Builds the save of set `set` under `name` (`BandolierCreate_Struct`):
/// the server stores the name and what is worn in the four weapon slots.
/// The two words after the name are sent as 0, which the server does not
/// read (what the program sends is owed to R49-45 a, inferred).
///
/// # Errors
/// Refuses a set past 3, and a name that is empty, longer than 31 bytes
/// or not printable ASCII: the server copies it unbounded (`strcpy`).
pub fn save(set: u8, name: &str) -> Result<[u8; LENGTH]> {
    check_set(set)?;
    ensure!(
        !name.is_empty() && name.len() <= MOST_NAME,
        "a set name of 1 to 31 letters"
    );
    ensure!(
        name.bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' '),
        "a set name of plain letters"
    );
    let mut body = [0; LENGTH];
    body[..4].copy_from_slice(&SAVE.to_le_bytes());
    body[4] = set;
    body[5..5 + name.len()].copy_from_slice(name.as_bytes());
    Ok(body)
}

/// Builds the clear of set `set` (`BandolierDelete_Struct`): its action,
/// the set and 35 bytes of 0.
///
/// # Errors
/// Refuses a set past 3: the server does not bound it.
pub fn clear(set: u8) -> Result<[u8; LENGTH]> {
    check_set(set)?;
    let mut body = [0; LENGTH];
    body[..4].copy_from_slice(&CLEAR.to_le_bytes());
    body[4] = set;
    Ok(body)
}

/// Builds the swap to set `set` (`BandolierSet_Struct`): its action, the
/// set and 35 bytes of 0. The server answers with no item updates; see
/// [`swap()`] for what it moves.
///
/// # Errors
/// Refuses a set past 3: the server does not bound it.
pub fn swap_to(set: u8) -> Result<[u8; LENGTH]> {
    check_set(set)?;
    let mut body = [0; LENGTH];
    body[..4].copy_from_slice(&SWAP.to_le_bytes());
    body[4] = set;
    Ok(body)
}

/// Builds the setting of belt slot `slot` to item `item_id`
/// (`MovePotionToBelt_Struct`).
///
/// # Errors
/// Refuses a slot past 3 and item 0.
pub fn set_belt(slot: u8, item_id: u32) -> Result<[u8; BELT_LENGTH]> {
    ensure!(item_id != 0, "an item to put on the belt");
    belt(BELT_SET, slot, item_id)
}

/// Builds the clearing of belt slot `slot`, naming the item it held
/// (action and item inferred, R49-45 c: the server reads neither).
///
/// # Errors
/// Refuses a slot past 3.
pub fn clear_belt(slot: u8, item_id: u32) -> Result<[u8; BELT_LENGTH]> {
    belt(BELT_CLEAR, slot, item_id)
}

fn belt(action: u32, slot: u8, item_id: u32) -> Result<[u8; BELT_LENGTH]> {
    if usize::from(slot) >= BELT_SLOTS {
        bail!("a belt slot from 1 to 4");
    }
    let mut body = [0; BELT_LENGTH];
    body[..4].copy_from_slice(&action.to_le_bytes());
    body[4..8].copy_from_slice(&u32::from(slot).to_le_bytes());
    body[8..].copy_from_slice(&item_id.to_le_bytes());
    Ok(body)
}

fn check_set(set: u8) -> Result<()> {
    ensure!(usize::from(set) < SETS, "a set from 1 to 4");
    Ok(())
}

/// A use of the item a potion belt slot names (6.568): the session casts its
/// click effect from the first slot the player wears or carries it in, by
/// the same checks as an item click
/// ([`crate::command::GameCommand::UseBeltItem`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeltUse {
    /// Caller-generated correlation ID, unique within an admission; its
    /// answer is a [`crate::world::WorldEvent::ItemUseAction`].
    pub request_id: u64,
    /// Owning zone admission.
    pub session_id: u64,
    /// The belt slot, 0 to 3.
    pub belt_slot: u8,
    /// Explicit own or known zone spawn ID.
    pub target_id: u16,
    /// Local creation time; stalls and reconnects do not replay requests.
    pub created: std::time::Instant,
    /// The host's word on whether the item's spell is the player's class's,
    /// as [`crate::inventory::ItemUse::installed_class_spell`].
    pub installed_class_spell: Option<bool>,
}

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

    #[test]
    fn a_save_and_a_clear_have_the_servers_layout() {
        let saved = save(2, "Melee").unwrap();
        assert_eq!(saved.len(), 40);
        assert_eq!(word(&saved, 0), 0);
        assert_eq!(saved[4], 2);
        assert_eq!(&saved[5..10], b"Melee");
        assert!(saved[10..].iter().all(|byte| *byte == 0));
        let longest = save(0, &"a".repeat(31)).unwrap();
        // Its end stays within the 32-byte field.
        assert_eq!(longest[36], 0);
        let cleared = clear(3).unwrap();
        assert_eq!((word(&cleared, 0), cleared[4]), (1, 3));
        assert!(cleared[5..].iter().all(|byte| *byte == 0));
        let swapped = swap_to(1).unwrap();
        assert_eq!((word(&swapped, 0), swapped[4]), (2, 1));
        assert!(swapped[5..].iter().all(|byte| *byte == 0));
        assert!(swap_to(4).is_err());
    }

    #[test]
    fn a_set_or_name_out_of_bounds_is_refused() {
        assert!(save(4, "Melee").is_err());
        assert!(clear(4).is_err());
        assert!(save(0, "").is_err());
        assert!(save(0, &"a".repeat(32)).is_err());
        assert!(save(0, "Me\0lee").is_err());
        assert!(save(0, "Mêlée").is_err());
        assert!(save(0, "Two Hand").is_ok());
    }

    #[test]
    fn a_belt_slot_is_set_and_cleared_in_twelve_bytes() {
        let set = set_belt(3, 13_005).unwrap();
        assert_eq!(
            [word(&set, 0), word(&set, 4), word(&set, 8)],
            [0, 3, 13_005]
        );
        let cleared = clear_belt(0, 13_005).unwrap();
        assert_eq!(
            [word(&cleared, 0), word(&cleared, 4), word(&cleared, 8)],
            [1, 0, 13_005]
        );
        assert!(set_belt(4, 13_005).is_err());
        assert!(set_belt(0, 0).is_err());
        assert!(clear_belt(4, 1).is_err());
    }
}
