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
//! wire carries as built; and TAKP's and Quarm's `SetTitle_Struct` and
//! `GMName_Struct` (`common/eq_packet_structs.h`). The titles come from
//! `Client::NotifyNewTitlesAvailable` (`zone/client.cpp`) and
//! `Client::SetAATitle` and `Client::SetTitleSuffix` (`zone/titles.cpp`);
//! `EQMac`'s renames by a game master from `Client::Handle_OP_GMNameChange`
//! (`zone/client_packet.cpp`) and `Client::ChangeFirstName`
//! (`zone/client.cpp`).

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
/// `EQMac`'s `OP_GMNameChange`: a game master renamed a player.
pub const EQMAC_GM_RENAME_OPCODE: u16 = 0xcb40;
/// `EQMac`'s `OP_GMLastName`: a character's new last name, as Titanium's.
pub const EQMAC_LAST_NAME_OPCODE: u16 = 0x6e41;
/// Titanium's `OP_Surname`: the player asks for a last name, and the server
/// answers in the same record.
pub const SURNAME_OPCODE: u16 = 0x4668;
/// `EQMac`'s `OP_Surname`.
pub const EQMAC_SURNAME_OPCODE: u16 = 0xc441;
/// Titanium's `OP_RequestTitles`: the player asks which titles they may
/// choose.
pub const REQUEST_TITLES_OPCODE: u16 = 0x5eba;
/// Titanium's `OP_SendTitleList`: the titles the player may choose.
pub const TITLE_LIST_OPCODE: u16 = 0x3e89;
/// Titanium's `OP_SetTitle`: the player's choice of title or suffix.
pub const SET_TITLE_OPCODE: u16 = 0x1f22;

/// The surname record (`Surname_Struct`), alike on both generations: the
/// player's name in 64 bytes, a word, and the surname in 32.
const SURNAME_LENGTH: usize = 100;
/// Where the surname record's surname starts.
const SURNAME_AT: usize = 68;
/// `EQMac`'s last name record (TAKP's `GMLastName_Struct`): three names of
/// 64 bytes, two bytes and a 16-bit word.
const EQMAC_LAST_NAME_LENGTH: usize = 196;

/// Titanium's title record.
const TITLE_LENGTH: usize = 40;
/// Its title or suffix field.
const TITLE: usize = 32;
/// `EQMac`'s rename by a game master.
const GM_RENAME_LENGTH: usize = 196;
/// Its name fields.
const NAME: usize = 64;

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

/// Decodes `EQMac`'s `OP_GMLastName` (TAKP's `GMLastName_Struct`, 196
/// bytes): whose last name changed at 0 and the new last name at 128, as
/// Titanium's, then two bytes and a word TAKP sets to 1 on what it sends
/// (`Client::ChangeLastName`), which is not read.
///
/// # Errors
/// Rejects a malformed length or a change for no one.
pub(crate) fn eqmac_last_name(body: &[u8]) -> Result<WorldEvent> {
    ensure!(
        body.len() == EQMAC_LAST_NAME_LENGTH,
        "invalid last name length"
    );
    let name = text(&body[..64]);
    ensure!(!name.is_empty(), "last name for no one");
    Ok(WorldEvent::LastName {
        name,
        last_name: text(&body[128..192]),
    })
}

/// The longest surname the record holds, in bytes, its end included.
pub const SURNAME_FIELD: usize = 32;

/// Builds `OP_Surname`'s request (`Surname_Struct`, 100 bytes, alike on
/// both generations): the player's name at 0, a word left 0 (`EQEmu`'s
/// `unknown0064`, TAKP's `success`) and the surname at 68, as given: never
/// cut, and its case left as typed, since `EQEmu` and TAKP set it
/// themselves (`Client::Handle_OP_Surname`). Only what the record cannot
/// carry is refused; which letters a surname may have is the server's to
/// judge, and the program's own rule is not read yet (inferred, program
/// read R49-10).
///
/// # Errors
/// Refuses an empty surname or player name, a surname with a space, a
/// control character or anything outside ASCII, and either one too long
/// for its field.
pub fn surname_request(player: &str, surname: &str) -> Result<Vec<u8>> {
    ensure!(!surname.is_empty(), "a surname needs a word");
    ensure!(
        surname.bytes().all(|byte| byte.is_ascii_graphic()),
        "a surname is one word of plain text"
    );
    ensure!(
        surname.len() < SURNAME_FIELD,
        "a surname is at most {} characters",
        SURNAME_FIELD - 1
    );
    ensure!(
        !player.is_empty() && player.len() < NAME && !player.contains('\0'),
        "a surname request needs the player's name"
    );
    let mut body = vec![0; SURNAME_LENGTH];
    body[..player.len()].copy_from_slice(player.as_bytes());
    body[SURNAME_AT..SURNAME_AT + surname.len()].copy_from_slice(surname.as_bytes());
    Ok(body)
}

/// A title the player may choose, as the server lists it: its number, and
/// the title before the name and the suffix after it it gives, either
/// empty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TitleChoice {
    /// The server's number for it, never 0.
    pub id: u32,
    /// The title before the name it gives; empty for none.
    pub prefix: String,
    /// The suffix after the name it gives; empty for none.
    pub suffix: String,
}

/// The longest title or suffix text the list may carry, in bytes, its end
/// apart: well past the 31 the zone's change of it carries
/// ([`TITLE`]), so any list `EQEmu` builds from its title table fits.
const TITLE_TEXT_LONGEST: usize = 63;

/// Decodes `OP_SendTitleList` (`EQEmu`'s `TitleManager::MakeTitlesPacket`,
/// which Titanium's wire carries as built): a 32-bit count, then for each
/// title its 32-bit number, its prefix and its suffix, each ended by a NUL
/// within 64 bytes. Text is read as [`WorldEvent::TitleChanged`]'s is. A
/// number listed again keeps its first entry.
///
/// # Errors
/// Rejects a short body, a count the body cannot hold, a string with no end
/// within its length, a title numbered 0 and bytes after the last title.
pub(crate) fn title_list(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() >= 4, "title list without its count");
    let count = usize::try_from(word(body, 0)).context("title count out of range")?;
    // Each title takes at least its number and two ends.
    ensure!(
        count <= (body.len() - 4) / 6,
        "title count past the list's end"
    );
    let mut at = 4;
    let string = |at: &mut usize| -> Result<String> {
        let rest = &body[*at..];
        let end = rest
            .iter()
            .take(TITLE_TEXT_LONGEST + 1)
            .position(|byte| *byte == 0)
            .context("title text without its end within its length")?;
        *at += end + 1;
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    };
    let mut titles = Vec::with_capacity(count);
    let mut seen = std::collections::HashSet::with_capacity(count);
    for _ in 0..count {
        ensure!(body.len() >= at + 4, "title without its number");
        let id = word(body, at);
        ensure!(id != 0, "title numbered 0");
        at += 4;
        let prefix = string(&mut at)?;
        let suffix = string(&mut at)?;
        // A number listed again keeps its first entry.
        if seen.insert(id) {
            titles.push(TitleChoice { id, prefix, suffix });
        }
    }
    ensure!(at == body.len(), "bytes after the title list");
    Ok(WorldEvent::TitleList(titles))
}

/// The length of the list's record, as `title_list` reads it.
#[must_use]
pub fn title_list_length(titles: &[TitleChoice]) -> usize {
    4 + titles
        .iter()
        .map(|title| 6 + title.prefix.len() + title.suffix.len())
        .sum::<usize>()
}

/// Builds `OP_SetTitle`'s choice (`SetTitle_Struct`, 8 bytes): 0 for a
/// title or 1 for a suffix, then the title's number, 0 to clear it
/// (`EQEmu`'s `Client::Handle_OP_SetTitle`).
#[must_use]
pub fn set_title(title_id: u32, suffix: bool) -> Vec<u8> {
    let mut body = u32::from(suffix).to_le_bytes().to_vec();
    body.extend_from_slice(&title_id.to_le_bytes());
    body
}

/// Decodes the server's answer in `OP_Surname`: the request's record sent
/// back, its word 1 where the surname was taken (`EQEmu` and TAKP) and 0
/// where TAKP's name filter refused it. `EQEmu` refuses with a line of its
/// own instead and sends no record.
///
/// # Errors
/// Rejects another length, another word and a surname that fills its
/// field.
pub(crate) fn surname_answer(body: &[u8]) -> Result<WorldEvent> {
    ensure!(
        body.len() == SURNAME_LENGTH,
        "invalid surname answer length"
    );
    let accepted = match word(body, 64) {
        0 => false,
        1 => true,
        _ => bail!("invalid surname answer"),
    };
    let field = &body[SURNAME_AT..];
    ensure!(field.contains(&0), "surname answer without its end");
    Ok(WorldEvent::SurnameAnswer {
        accepted,
        last_name: text(field),
    })
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

/// Decodes `EQMac`'s `OP_GMNameChange` (196 bytes): the old name, the game
/// master's and the new name, 64 bytes each and each ended by a NUL within
/// them; a flag for the server to refuse the old name from then on; and
/// three bytes. TAKP and Quarm pass a game master's `/name` on to the whole
/// zone, the player renamed included, the three bytes set to 1. Their
/// `#set name` sends it too, after the rename, so both names are the new
/// one, then sends the player back to character select. It names no spawn.
///
/// # Errors
/// Rejects a wrong length, and an old or new name that is empty or fills
/// its field.
pub(crate) fn eqmac_gm_rename(body: &[u8]) -> Result<WorldEvent> {
    ensure!(
        body.len() == GM_RENAME_LENGTH,
        "invalid game master rename length"
    );
    let name = |at: usize| {
        let field = &body[at..at + NAME];
        ensure!(
            field[0] != 0 && field.contains(&0),
            "game master rename without a name"
        );
        Ok(text(field))
    };
    Ok(WorldEvent::Renamed {
        spawn_id: None,
        old_name: name(0)?,
        new_name: name(2 * NAME)?,
    })
}

/// Decodes Titanium's `OP_SetTitleReply` (40 bytes): 0 for a title or 1
/// for a suffix, 32 bits; the title or suffix in 32 bytes, ended by a NUL
/// within them and empty when it was cleared; and whose, a spawn in 32
/// bits.
///
/// # Errors
/// Rejects a wrong length, another kind, text that fills its field, and a
/// spawn that is zero or past 16 bits.
pub(crate) fn titanium_title(body: &[u8]) -> Result<WorldEvent> {
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
        SURNAME_OPCODE => Some(surname_answer(body)?),
        TITLE_LIST_OPCODE => Some(title_list(body)?),
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
    fn eqmac_game_masters_renames_carry_the_old_name_and_the_new() {
        let mut body = [0_u8; 196];
        body[..8].copy_from_slice(b"Examplar");
        body[64..74].copy_from_slice(b"Gamemaster");
        body[128..136].copy_from_slice(b"Exemplar");
        body[192] = 1;
        body[193..196].fill(1);
        assert_eq!(
            eqmac_gm_rename(&body).unwrap(),
            WorldEvent::Renamed {
                spawn_id: None,
                old_name: "Examplar".into(),
                new_name: "Exemplar".into(),
            }
        );
        assert!(eqmac_gm_rename(&body[..195]).is_err());
        for at in [0, 128] {
            let mut nameless = body;
            nameless[at..at + 64].fill(0);
            assert!(eqmac_gm_rename(&nameless).is_err());
            let mut full = body;
            full[at..at + 64].fill(b'a');
            assert!(eqmac_gm_rename(&full).is_err());
        }
        // The game master's name is not read.
        let mut unnamed_master = body;
        unnamed_master[64..128].fill(0);
        assert!(eqmac_gm_rename(&unnamed_master).is_ok());
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

    #[test]
    fn surname_requests_carry_the_players_name_and_the_surname_as_given() {
        let body = surname_request("Examplar", "exemplum").unwrap();
        assert_eq!(body.len(), 100);
        assert_eq!(&body[..9], b"Examplar\0");
        assert_eq!(&body[64..68], &[0; 4]);
        assert_eq!(&body[68..77], b"exemplum\0");
        assert!(body[77..].iter().all(|byte| *byte == 0));
        assert!(surname_request("Examplar", &"a".repeat(31)).is_ok());
        // Which marks a surname may have is the server's to judge.
        assert!(surname_request("Examplar", "Ex'ample").is_ok());
        assert!(surname_request("Examplar", "Ex-ample").is_ok());
        for refused in [
            "",
            "Ex ample",
            "Ex\tample",
            "Exämple",
            "Ex\0",
            &"a".repeat(32),
        ] {
            assert!(surname_request("Examplar", refused).is_err(), "{refused:?}");
        }
        assert!(surname_request("", "Exemplum").is_err());
        assert!(surname_request(&"a".repeat(64), "Exemplum").is_err());
    }

    #[test]
    fn surname_answers_say_whether_it_was_taken() {
        let mut body = surname_request("Examplar", "Exemplum").unwrap();
        body[64] = 1;
        let answer = |body: &[u8]| crate::world::titanium_update(SURNAME_OPCODE, body);
        assert_eq!(
            answer(&body).unwrap(),
            Some(WorldEvent::SurnameAnswer {
                accepted: true,
                last_name: "Exemplum".into(),
            })
        );
        body[64] = 0;
        assert_eq!(
            surname_answer(&body).unwrap(),
            WorldEvent::SurnameAnswer {
                accepted: false,
                last_name: "Exemplum".into(),
            }
        );
        let mut other = body.clone();
        other[64] = 2;
        assert!(surname_answer(&other).is_err());
        let mut full = body.clone();
        full[68..100].fill(b'a');
        assert!(surname_answer(&full).is_err());
        assert!(surname_answer(&body[..99]).is_err());
    }

    #[test]
    fn eqmac_last_names_name_whose_and_what() {
        let mut body = [0_u8; 196];
        body[..8].copy_from_slice(b"Examplar");
        body[64..72].copy_from_slice(b"Examplar");
        body[128..136].copy_from_slice(b"Exemplum");
        body[194] = 1;
        assert_eq!(
            eqmac_last_name(&body).unwrap(),
            WorldEvent::LastName {
                name: "Examplar".into(),
                last_name: "Exemplum".into(),
            }
        );
        assert_eq!(
            crate::quarm::updates(EQMAC_LAST_NAME_OPCODE, &body).unwrap(),
            [eqmac_last_name(&body).unwrap()]
        );
        let mut answer = surname_request("Examplar", "Exemplum").unwrap();
        answer[64] = 1;
        assert_eq!(
            crate::quarm::updates(EQMAC_SURNAME_OPCODE, &answer).unwrap(),
            [WorldEvent::SurnameAnswer {
                accepted: true,
                last_name: "Exemplum".into(),
            }]
        );
        assert!(eqmac_last_name(&body[..195]).is_err());
        assert!(eqmac_last_name(&[0; 196]).is_err());
        // A 200-byte record is Titanium's, not EQMac's.
        assert!(eqmac_last_name(&[0; 200]).is_err());
    }

    #[test]
    fn title_lists_carry_each_title_and_its_parts() {
        let mut body = 3_u32.to_le_bytes().to_vec();
        for (id, prefix, suffix) in [
            (7_u32, "Lord", ""),
            (9, "", "the Synthetic"),
            (12, "Sir", "of Tests"),
        ] {
            body.extend_from_slice(&id.to_le_bytes());
            body.extend_from_slice(prefix.as_bytes());
            body.push(0);
            body.extend_from_slice(suffix.as_bytes());
            body.push(0);
        }
        let choice = |id, prefix: &str, suffix: &str| TitleChoice {
            id,
            prefix: prefix.into(),
            suffix: suffix.into(),
        };
        assert_eq!(
            crate::world::titanium_update(TITLE_LIST_OPCODE, &body).unwrap(),
            Some(WorldEvent::TitleList(vec![
                choice(7, "Lord", ""),
                choice(9, "", "the Synthetic"),
                choice(12, "Sir", "of Tests"),
            ]))
        );
        assert_eq!(
            title_list(&0_u32.to_le_bytes()).unwrap(),
            WorldEvent::TitleList(Vec::new())
        );
        // Short, cut, a count past the end, bytes after, and a title 0.
        assert!(title_list(&[0; 3]).is_err());
        assert!(title_list(&body[..body.len() - 1]).is_err());
        let mut more = body.clone();
        more[0] = 4;
        assert!(title_list(&more).is_err());
        let mut huge = body.clone();
        huge[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(title_list(&huge).is_err());
        let mut trailing = body.clone();
        trailing.push(0);
        assert!(title_list(&trailing).is_err());
        let mut zero = body.clone();
        zero[4..8].fill(0);
        assert!(title_list(&zero).is_err());
        let WorldEvent::TitleList(read) = title_list(&body).unwrap() else {
            unreachable!()
        };
        assert_eq!(title_list_length(&read), body.len());

        // A text past 63 bytes, and a number listed twice.
        let entry = |id: u32, prefix: &str| {
            let mut entry = id.to_le_bytes().to_vec();
            entry.extend_from_slice(prefix.as_bytes());
            entry.extend([0, 0]);
            entry
        };
        let mut long = 1_u32.to_le_bytes().to_vec();
        long.extend(entry(7, &"a".repeat(64)));
        assert!(title_list(&long).is_err());
        let mut fits = 1_u32.to_le_bytes().to_vec();
        fits.extend(entry(7, &"a".repeat(63)));
        assert!(title_list(&fits).is_ok());
        let mut twice = 2_u32.to_le_bytes().to_vec();
        twice.extend(entry(7, "First"));
        twice.extend(entry(7, "Second"));
        assert_eq!(
            title_list(&twice).unwrap(),
            WorldEvent::TitleList(vec![choice(7, "First", "")])
        );
    }

    #[test]
    fn a_title_choice_names_its_kind_then_its_number() {
        assert_eq!(set_title(7, false), [0, 0, 0, 0, 7, 0, 0, 0]);
        assert_eq!(set_title(9, true), [1, 0, 0, 0, 9, 0, 0, 0]);
        // 0 clears it.
        assert_eq!(set_title(0, true), [1, 0, 0, 0, 0, 0, 0, 0]);
    }
}
