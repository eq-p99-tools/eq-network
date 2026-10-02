//! How `/who` lists a player beyond their name, level, class and race: their
//! guild, whether they hide from `/who`, and the game master, away and
//! looking-for-group flags. A Titanium client lists its own zone's players
//! from what the zone told it: each spawn record, then appearance updates
//! (`OP_SpawnAppearance`) and looking-for-group notices (`OP_LFGAppearance`).
//! Guild numbers name guilds from the guild list (`OP_GuildsList`), which the
//! world sends on entry and a zone sends a guild's members.
//!
//! Layout reference: `EQEmu`'s Titanium `Spawn_Struct`, `GuildsList_Struct`
//! and `LFG_Appearance_Struct` (`common/patches/titanium_structs.h`), and its
//! `AppearanceType` numbers (`common/eq_constants.h`).
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_LFGAppearance`: a player started or stopped looking for a group.
pub const LOOKING_OPCODE: u16 = 0x1a85;
/// `OP_GuildsList`: every guild's name, by number.
pub const GUILDS_OPCODE: u16 = 0x6957;
/// Each name in the guild list, and the heading before them.
const GUILD_NAME_LENGTH: usize = 64;

/// Whether a player hides from `/who`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub enum Anonymity {
    /// Listed in full.
    #[default]
    Open,
    /// `/anonymous`: level, class, race, guild and zone hidden.
    Anonymous,
    /// `/roleplay`: as anonymous, but the guild shows.
    Roleplaying,
}

impl Anonymity {
    /// The servers' number for it: 1 anonymous, 2 roleplaying.
    const fn from_wire(value: u32) -> Self {
        match value {
            1 => Self::Anonymous,
            2 => Self::Roleplaying,
            _ => Self::Open,
        }
    }
}

/// How `/who` lists a player beyond their name, level, class and race.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Listing {
    /// The guild's number; None for no guild.
    pub guild: Option<u32>,
    /// Whether the player hides from `/who`.
    pub anonymity: Anonymity,
    /// In game master mode.
    pub game_master: bool,
    /// Away from the keyboard.
    pub away: bool,
    /// Looking for a group.
    pub looking: bool,
}

/// A change in how `/who` lists a player.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ListingChange {
    /// The level `/who` shows.
    Level(u8),
    /// Joined, left or changed guilds.
    Guild(Option<u32>),
    /// Turned anonymous or roleplaying, or open again.
    Anonymity(Anonymity),
    /// Game master mode on or off.
    GameMaster(bool),
    /// Away from the keyboard, or back.
    Away(bool),
    /// Looking for a group, or no longer.
    Looking(bool),
}

/// A guild number, unless it stands for no guild.
fn guild(number: u32) -> Option<u32> {
    (number != 0 && number != u32::MAX).then_some(number)
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// How a Titanium spawn record (`Spawn_Struct`, 385 bytes) lists the player.
pub(crate) fn titanium_spawn(record: &[u8]) -> Listing {
    Listing {
        guild: guild(word(record, 238)),
        anonymity: Anonymity::from_wire(u32::from(record[5])),
        game_master: record[1] != 0,
        away: record[237] != 0,
        looking: record[384] != 0,
    }
}

/// The change an appearance update makes to a listing, by the update's type
/// and value; None for other updates.
pub(crate) fn appearance(kind: u16, value: u32) -> Option<ListingChange> {
    Some(match kind {
        1 => ListingChange::Level(u8::try_from(value).ok()?),
        20 => ListingChange::GameMaster(value != 0),
        21 => ListingChange::Anonymity(Anonymity::from_wire(value)),
        22 => ListingChange::Guild(guild(value)),
        24 => ListingChange::Away(value != 0),
        _ => return None,
    })
}

/// Decodes `OP_LFGAppearance`: whose flag changed, and whether they now look
/// for a group.
///
/// # Errors
/// Rejects a malformed length or spawn ID.
pub(crate) fn looking(body: &[u8]) -> Result<(u16, ListingChange)> {
    ensure!(body.len() == 8, "invalid looking-for-group length");
    let spawn_id = u16::try_from(word(body, 0))?;
    ensure!(spawn_id != 0, "invalid looking-for-group spawn ID");
    Ok((spawn_id, ListingChange::Looking(body[4] != 0)))
}

/// Decodes Titanium's `OP_GuildsList`: a heading, then each guild's name at
/// its number; numbers without a guild are empty.
///
/// # Errors
/// Rejects a length that is not the heading and whole names.
pub fn titanium_guilds(body: &[u8]) -> Result<Vec<(u32, String)>> {
    ensure!(
        body.len() >= GUILD_NAME_LENGTH && body.len().is_multiple_of(GUILD_NAME_LENGTH),
        "invalid guild list length"
    );
    Ok(body[GUILD_NAME_LENGTH..]
        .chunks(GUILD_NAME_LENGTH)
        .zip(0u32..)
        .filter_map(|(name, number)| {
            let end = name
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(name.len());
            (end > 0).then(|| (number, String::from_utf8_lossy(&name[..end]).into_owned()))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spawn_record_says_how_who_lists_the_player() {
        let mut record = vec![0; 385];
        assert_eq!(titanium_spawn(&record), Listing::default());
        record[238..242].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(titanium_spawn(&record).guild, None);
        record[238..242].copy_from_slice(&7u32.to_le_bytes());
        record[5] = 2;
        record[1] = 1;
        record[237] = 1;
        record[384] = 1;
        assert_eq!(
            titanium_spawn(&record),
            Listing {
                guild: Some(7),
                anonymity: Anonymity::Roleplaying,
                game_master: true,
                away: true,
                looking: true,
            }
        );
    }

    #[test]
    fn updates_change_the_listing() {
        assert_eq!(appearance(1, 51), Some(ListingChange::Level(51)));
        assert_eq!(appearance(1, 300), None);
        assert_eq!(appearance(20, 1), Some(ListingChange::GameMaster(true)));
        assert_eq!(
            appearance(21, 1),
            Some(ListingChange::Anonymity(Anonymity::Anonymous))
        );
        assert_eq!(appearance(22, u32::MAX), Some(ListingChange::Guild(None)));
        assert_eq!(appearance(24, 0), Some(ListingChange::Away(false)));
        assert_eq!(appearance(14, 110), None);
        let mut body = 12u32.to_le_bytes().to_vec();
        body.extend_from_slice(&[1, 0, 0, 0]);
        assert_eq!(looking(&body).unwrap(), (12, ListingChange::Looking(true)));
        assert!(looking(&body[..7]).is_err());
    }

    #[test]
    fn the_guild_list_names_each_guild_at_its_number() {
        let mut body = vec![0; 64 * 4];
        body[128..133].copy_from_slice(b"Riot\0");
        body[192..199].copy_from_slice(b"Seekers");
        assert_eq!(
            titanium_guilds(&body).unwrap(),
            [(1, "Riot".to_owned()), (2, "Seekers".to_owned())]
        );
        assert!(titanium_guilds(&body[..100]).is_err());
    }
}
