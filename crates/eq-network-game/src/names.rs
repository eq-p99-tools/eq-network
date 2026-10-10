//! The parts of a character's name around the first: a title before it, and
//! a last name and a suffix after it, which the official client's
//! `/shownames` adds over heads; and a spawn's renaming.

use crate::world::WorldEvent;
use anyhow::{ensure, Result};
use serde::Serialize;

/// Titanium's `OP_GMLastName`: a character's new last name, sent to
/// everyone in the zone, the character included, when it changes.
pub const LAST_NAME_OPCODE: u16 = 0x23a1;
/// Titanium's `OP_MobRename`: a spawn's new name.
pub const RENAME_OPCODE: u16 = 0x0498;

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

/// What Titanium's messages on names say: last names and renames; None for
/// other messages.
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
        _ => None,
    })
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(crate::world::until_nul(bytes)).into_owned()
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
