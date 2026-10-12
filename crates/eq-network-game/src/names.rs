//! The parts of a character's name around the first: a title before it, and
//! a last name and a suffix after it, which the official client's
//! `/shownames` adds over heads; the titles a player earns; and a spawn's
//! renaming.
//!
//! `EQEmu` tells the player of new titles to choose from
//! (`OP_NewTitlesAvailable`) as an alternate advancement, foraging or a
//! tradeskill earns them; TAKP, for a tradeskill, and Quarm, for foraging
//! or a tradeskill, send an empty `OP_SetTitle` instead. As a character's
//! title or suffix is set or cleared, by the player's choice, a game
//! master's `#set title` or `#set title_suffix`, or a quest, `EQEmu` tells
//! the whole zone, the character included, what it now is
//! (`OP_SetTitleReply`), as it does for a bot's; TAKP and Quarm send the
//! zone an 8-byte `OP_SetTitle` naming the character, which says neither
//! which changed nor to what. The session's spawn table keeps the titles,
//! suffixes and last names the zone hears of.
//!
//! Layout reference: `EQEmu`'s `GMLastName_Struct`, `MobRename_Struct` and
//! `SetTitleReply_Struct` (`common/eq_packet_structs.h`), which Titanium's
//! wire carries as built; and TAKP's and Quarm's `SetTitle_Struct`
//! (`common/eq_packet_structs.h`). The titles come from
//! `Client::NotifyNewTitlesAvailable` (`zone/client.cpp`) and
//! `Client::SetAATitle` and `Client::SetTitleSuffix` (`zone/titles.cpp`).

use crate::world::WorldEvent;
use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;

/// Titanium's `OP_GMLastName`: a character's new last name, sent to
/// everyone in the zone, the character included, when it changes.
pub const LAST_NAME_OPCODE: u16 = 0x23a1;
/// Titanium's `OP_MobRename`: a spawn's new name.
pub const RENAME_OPCODE: u16 = 0x0498;
/// Titanium's `OP_NewTitlesAvailable`: titles the player has newly earned,
/// to choose from.
pub const NEW_TITLES_OPCODE: u16 = 0x4eca;
/// Titanium's `OP_SetTitleReply`: a character's new title or suffix.
pub const TITLE_OPCODE: u16 = 0x5eab;
/// `EQMac`'s `OP_SetTitle`: empty, titles the player has newly earned; or
/// a character's title or suffix changed, without saying which or to what.
pub const EQMAC_TITLE_OPCODE: u16 = 0xd440;

/// Titanium's title record.
const TITLE_LENGTH: usize = 40;
/// Its title or suffix field.
const TITLE: usize = 32;

/// A character's title, last name and suffix; each empty where the server
/// sends none or the dialect does not report it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct NameParts {
    /// A title before the name, such as one an alternate advancement grants.
    pub title: String,
    /// The last name, such as one a player chose with `/surname`.
    pub last_name: String,
    /// A suffix after the name, such as "of Veeshan".
    pub suffix: String,
}

/// A change to a character's title or suffix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum TitleChange {
    /// The title before the name; empty when it was cleared.
    Title(String),
    /// The suffix after the name; empty when it was cleared.
    Suffix(String),
    /// A title or a suffix: `EQMac`'s record says neither which nor what it
    /// now is.
    Unspecified,
}

/// The name parts of a Titanium spawn record (`Spawn_Struct`, 385 bytes):
/// 32 bytes each of suffix at 157, title at 242 and last name at 292.
pub(crate) fn titanium_spawn(record: &[u8]) -> NameParts {
    NameParts {
        title: text(&record[242..274]),
        last_name: text(&record[292..324]),
        suffix: text(&record[157..189]),
    }
}

/// Decodes `OP_GMLastName` (`GMLastName_Struct`, 200 bytes): whose last
/// name changed, by the name they spawned with, and the new last name,
/// empty when it was taken away.
///
/// # Errors
/// Rejects a malformed length or a change for no one.
pub(crate) fn last_name(body: &[u8]) -> Result<(String, String)> {
    ensure!(body.len() == 200, "invalid last name length");
    let name = text(&body[..64]);
    ensure!(!name.is_empty(), "last name for no one");
    Ok((name, text(&body[128..192])))
}

/// Decodes `OP_MobRename` (`MobRename_Struct`, 200 bytes): the old name at
/// 0, again at 64, and the new name at 128. `EQEmu` sends it to the zone
/// (`Mob::SendRename`) as an NPC's corpse takes the server's name, from the
/// one the client gives it (`'s_corpse` for `` `s_corpse ``), as a game
/// master, a quest or a pet's owner gives a spawn a temporary name, and as
/// a game master changes a player's first name. It names no spawn.
///
/// # Errors
/// Rejects a malformed length and a missing name.
pub(crate) fn titanium_rename(body: &[u8]) -> Result<(String, String)> {
    ensure!(body.len() == 200, "invalid rename length");
    let (old_name, new_name) = (text(&body[..64]), text(&body[128..192]));
    ensure!(
        !old_name.is_empty() && !new_name.is_empty(),
        "rename without a name"
    );
    Ok((old_name, new_name))
}

/// Decodes Titanium's `OP_SetTitleReply` (40 bytes): 0 for a title or 1
/// for a suffix, 32 bits; the title or suffix in 32 bytes, ended by a NUL
/// within them and empty when it was cleared; and whose, a spawn in 32
/// bits.
///
/// # Errors
/// Rejects a wrong length, another kind, text that fills its field, and a
/// spawn that is zero or past 16 bits.
fn titanium_title(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == TITLE_LENGTH, "invalid title length");
    let field = &body[4..4 + TITLE];
    ensure!(field.contains(&0), "title without its end");
    let change = match word(body, 0) {
        0 => TitleChange::Title(text(field)),
        1 => TitleChange::Suffix(text(field)),
        _ => bail!("invalid title kind"),
    };
    Ok(WorldEvent::TitleChanged {
        spawn_id: spawn(body, 4 + TITLE)?,
        change,
    })
}

/// Decodes `EQMac`'s `OP_SetTitle`: empty, titles the player has newly
/// earned; or 8 bytes, a word TAKP and Quarm leave zero, then whose title
/// or suffix changed, a spawn in 32 bits.
///
/// # Errors
/// Rejects another length and a spawn that is zero or past 16 bits.
pub(crate) fn eqmac_title(body: &[u8]) -> Result<WorldEvent> {
    if body.is_empty() {
        return Ok(WorldEvent::NewTitles);
    }
    ensure!(body.len() == 8, "invalid title length");
    Ok(WorldEvent::TitleChanged {
        spawn_id: spawn(body, 4)?,
        change: TitleChange::Unspecified,
    })
}

/// What Titanium's messages on names say: last names, renames, new titles
/// and changes to titles and suffixes; None for other messages.
///
/// # Errors
/// Rejects malformed records of those messages.
pub(crate) fn titanium(opcode: u16, body: &[u8]) -> Result<Option<WorldEvent>> {
    Ok(match opcode {
        LAST_NAME_OPCODE => {
            let (name, last_name) = last_name(body)?;
            Some(WorldEvent::LastName { name, last_name })
        }
        RENAME_OPCODE => {
            let (old_name, new_name) = titanium_rename(body)?;
            Some(WorldEvent::Renamed {
                spawn_id: None,
                old_name,
                new_name,
            })
        }
        NEW_TITLES_OPCODE => {
            ensure!(body.is_empty(), "invalid new titles length");
            Some(WorldEvent::NewTitles)
        }
        TITLE_OPCODE => Some(titanium_title(body)?),
        _ => None,
    })
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(crate::world::until_nul(bytes)).into_owned()
}

/// A little-endian word of a fixed-length record.
fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

/// A spawn named by a 32-bit word: never zero, and within 16 bits.
fn spawn(body: &[u8], at: usize) -> Result<u16> {
    let spawn_id = u16::try_from(word(body, at)).context("title spawn out of range")?;
    ensure!(spawn_id != 0, "title without a spawn");
    Ok(spawn_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_carry_the_old_name_and_the_new() {
        let mut body = [0u8; 200];
        body[..15].copy_from_slice(b"a_rat's_corpse8");
        body[64..79].copy_from_slice(b"a_rat's_corpse8");
        body[128..143].copy_from_slice(b"a_rat`s_corpse8");
        body[196] = 1;
        assert_eq!(
            crate::world::titanium_update(RENAME_OPCODE, &body).unwrap(),
            Some(WorldEvent::Renamed {
                spawn_id: None,
                old_name: "a_rat's_corpse8".into(),
                new_name: "a_rat`s_corpse8".into(),
            })
        );
        assert!(titanium_rename(&body[..199]).is_err());
        let mut nameless = body;
        nameless[128..143].fill(0);
        assert!(titanium_rename(&nameless).is_err());
    }

    #[test]
    fn spawn_records_carry_title_last_name_and_suffix() {
        let mut record = [0_u8; 385];
        record[157..168].copy_from_slice(b"of Veeshan\0");
        record[242..246].copy_from_slice(b"Lord");
        record[292..300].copy_from_slice(b"Exemplum");
        assert_eq!(
            titanium_spawn(&record),
            NameParts {
                title: "Lord".into(),
                last_name: "Exemplum".into(),
                suffix: "of Veeshan".into(),
            }
        );
        assert_eq!(titanium_spawn(&[0; 385]), NameParts::default());
    }

    #[test]
    fn last_name_changes_name_whose_and_what() {
        let mut body = [0_u8; 200];
        body[..8].copy_from_slice(b"Examplar");
        body[64..72].copy_from_slice(b"Examplar");
        body[128..136].copy_from_slice(b"Exemplum");
        body[192..200].copy_from_slice(&[1, 0, 1, 0, 1, 0, 1, 0]);
        assert_eq!(
            last_name(&body).unwrap(),
            ("Examplar".to_owned(), "Exemplum".to_owned())
        );
        // A last name taken away is an empty one.
        body[128..136].fill(0);
        assert_eq!(last_name(&body).unwrap().1, "");
        assert!(last_name(&body[..199]).is_err());
        assert!(last_name(&[0; 200]).is_err());
    }

    #[test]
    fn titanium_titles_say_whose_changed_and_to_what() {
        let mut body = [0_u8; 40];
        body[4..13].copy_from_slice(b"Exemplary");
        body[36..40].copy_from_slice(&7_u32.to_le_bytes());
        let title = |body: &[u8]| crate::world::titanium_update(TITLE_OPCODE, body);
        assert_eq!(
            title(&body).unwrap(),
            Some(WorldEvent::TitleChanged {
                spawn_id: 7,
                change: TitleChange::Title("Exemplary".into()),
            })
        );
        body[0] = 1;
        body[4..17].copy_from_slice(b"the Synthetic");
        assert_eq!(
            title(&body).unwrap(),
            Some(WorldEvent::TitleChanged {
                spawn_id: 7,
                change: TitleChange::Suffix("the Synthetic".into()),
            })
        );
        // A suffix cleared is an empty one.
        body[4..36].fill(0);
        assert_eq!(
            title(&body).unwrap(),
            Some(WorldEvent::TitleChanged {
                spawn_id: 7,
                change: TitleChange::Suffix(String::new()),
            })
        );
        let mut full = body;
        full[4..36].fill(b'a');
        assert!(title(&full).is_err());
        let mut other_kind = body;
        other_kind[0] = 2;
        assert!(title(&other_kind).is_err());
        for spawn in [0_u32, 0x1_0000] {
            let mut unnamed = body;
            unnamed[36..40].copy_from_slice(&spawn.to_le_bytes());
            assert!(title(&unnamed).is_err());
        }
        assert!(title(&body[..39]).is_err());
    }

    #[test]
    fn new_titles_carry_nothing() {
        assert_eq!(
            crate::world::titanium_update(NEW_TITLES_OPCODE, &[]).unwrap(),
            Some(WorldEvent::NewTitles)
        );
        assert!(crate::world::titanium_update(NEW_TITLES_OPCODE, &[0]).is_err());
    }

    #[test]
    fn eqmac_titles_are_new_ones_or_an_unspecified_change() {
        assert_eq!(eqmac_title(&[]).unwrap(), WorldEvent::NewTitles);
        let mut body = [0_u8; 8];
        body[4..8].copy_from_slice(&7_u32.to_le_bytes());
        assert_eq!(
            eqmac_title(&body).unwrap(),
            WorldEvent::TitleChanged {
                spawn_id: 7,
                change: TitleChange::Unspecified,
            }
        );
        assert!(eqmac_title(&body[..7]).is_err());
        assert!(eqmac_title(&[0; 8]).is_err());
    }

    #[test]
    fn zone_updates_carry_last_name_changes() {
        let mut body = [0_u8; 200];
        body[..8].copy_from_slice(b"Examplar");
        body[128..136].copy_from_slice(b"Exemplum");
        assert_eq!(
            crate::world::titanium_update(LAST_NAME_OPCODE, &body).unwrap(),
            Some(crate::world::WorldEvent::LastName {
                name: "Examplar".into(),
                last_name: "Exemplum".into(),
            })
        );
    }
}
