//! How `/who` lists a player beyond their name, level, class and race: their
//! guild and rank in it, whether they hide from `/who`, and the game master,
//! away and looking-for-group flags. A Titanium client lists its own zone's
//! players from what the zone told it: each spawn record, then appearance
//! updates (`OP_SpawnAppearance`) and looking-for-group notices
//! (`OP_LFGAppearance`).
//! Guild numbers name guilds from the guild list (`OP_GuildsList`), which the
//! world sends on entry and a zone sends a guild's members.
//!
//! The player says they are away, anonymous or roleplaying with the same
//! appearance update.
//!
//! Layout reference: `EQEmu`'s Titanium `Spawn_Struct`, `GuildsList_Struct`,
//! `LFG_Appearance_Struct` and `SpawnAppearance_Struct`
//! (`common/patches/titanium_structs.h`), and its `AppearanceType` numbers
//! (`common/eq_constants.h`).
use crate::command::EncodedCommand;
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// `OP_SpawnAppearance`: a spawn's appearance update.
pub const APPEARANCE_OPCODE: u16 = 0x7c32;
/// The appearance updates that say how `/who` lists a player.
const ANONYMITY: u16 = 21;
const AWAY: u16 = 24;

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

    /// Its number on the wire, as [`Anonymity::from_wire`] reads it.
    const fn wire(self) -> u32 {
        match self {
            Self::Open => 0,
            Self::Anonymous => 1,
            Self::Roleplaying => 2,
        }
    }
}

/// The player's own appearance update of a kind, to a value.
pub(crate) fn update(spawn_id: u16, kind: u16, value: u32) -> Result<EncodedCommand> {
    ensure!(
        spawn_id != 0,
        "an appearance update needs the player's spawn ID"
    );
    let mut body = spawn_id.to_le_bytes().to_vec();
    body.extend_from_slice(&kind.to_le_bytes());
    body.extend_from_slice(&value.to_le_bytes());
    Ok(EncodedCommand {
        opcode: APPEARANCE_OPCODE,
        body,
    })
}

/// Says the player is away from the keyboard, or back.
///
/// # Errors
/// Rejects a zero spawn ID.
pub fn titanium_away(spawn_id: u16, away: bool) -> Result<EncodedCommand> {
    update(spawn_id, AWAY, u32::from(away))
}

/// Says how the player hides from `/who`.
///
/// # Errors
/// Rejects a zero spawn ID.
pub fn titanium_anonymity(spawn_id: u16, anonymity: Anonymity) -> Result<EncodedCommand> {
    update(spawn_id, ANONYMITY, anonymity.wire())
}

/// A player's rank in their guild.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum GuildRank {
    /// 0, a member.
    Member,
    /// 1, an officer.
    Officer,
    /// 2, the leader.
    Leader,
    /// A number past those, kept as sent: TAKP sends 3 for a guild it does
    /// not know.
    Other(u32),
}

impl GuildRank {
    /// The rank with this number: Titanium's three, onto which `EQEmu`'s
    /// encoder maps its own ranks, and TAKP's and Quarm's.
    const fn from_wire(value: u32) -> Self {
        match value {
            0 => Self::Member,
            1 => Self::Officer,
            2 => Self::Leader,
            other => Self::Other(other),
        }
    }
}

/// How `/who` lists a player beyond their name, level, class and race.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Listing {
    /// The guild's number; None for no guild.
    pub guild: Option<u32>,
    /// The player's rank in the guild; None without one, and where the
    /// dialect does not report it.
    pub guild_rank: Option<GuildRank>,
    /// Whether the player hides from `/who`.
    pub anonymity: Anonymity,
    /// In game master mode.
    pub game_master: bool,
    /// Away from the keyboard.
    pub away: bool,
    /// Looking for a group.
    pub looking: bool,
}

impl Listing {
    /// Keeps a change, as the session's spawn table does; the level `/who`
    /// shows is the spawn's, beside its listing, so it changes nothing here.
    /// A rank is kept only with a guild: leaving or changing guilds clears
    /// it until the new guild's rank comes, and one coming with no guild, as
    /// `EQEmu` follows a member's removal with, is not kept.
    pub fn apply(&mut self, change: ListingChange) {
        match change {
            ListingChange::Level(_) => (),
            ListingChange::Guild(guild) => {
                if guild != self.guild {
                    self.guild_rank = None;
                }
                self.guild = guild;
            }
            ListingChange::GuildRank(rank) => {
                if self.guild.is_some() {
                    self.guild_rank = Some(rank);
                }
            }
            ListingChange::Anonymity(anonymity) => self.anonymity = anonymity,
            ListingChange::GameMaster(on) => self.game_master = on,
            ListingChange::Away(on) => self.away = on,
            ListingChange::Looking(on) => self.looking = on,
        }
    }
}

/// A change in how `/who` lists a player.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ListingChange {
    /// The level `/who` shows.
    Level(u8),
    /// Joined, left or changed guilds.
    Guild(Option<u32>),
    /// The rank that comes with a guild, or a promotion or demotion.
    GuildRank(GuildRank),
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
    let guild = guild(word(record, 238));
    Listing {
        guild,
        guild_rank: guild.map(|_| GuildRank::from_wire(u32::from(record[193]))),
        anonymity: Anonymity::from_wire(u32::from(record[5])),
        game_master: record[1] != 0,
        away: record[237] != 0,
        looking: record[384] != 0,
    }
}

/// The change an appearance update makes to a listing, by the update's kind
/// and value; None for other kinds.
///
/// # Errors
/// Rejects a `/who` level past 255.
pub(crate) fn appearance(kind: u16, value: u32) -> Result<Option<ListingChange>> {
    Ok(Some(match kind {
        1 => ListingChange::Level(u8::try_from(value).context("who level out of range")?),
        20 => ListingChange::GameMaster(value != 0),
        21 => ListingChange::Anonymity(Anonymity::from_wire(value)),
        22 => ListingChange::Guild(guild(value)),
        23 => ListingChange::GuildRank(GuildRank::from_wire(value)),
        24 => ListingChange::Away(value != 0),
        _ => return Ok(None),
    }))
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
        record[193] = 1;
        assert_eq!(titanium_spawn(&record).guild_rank, None);
        record[238..242].copy_from_slice(&7u32.to_le_bytes());
        record[5] = 2;
        record[1] = 1;
        record[237] = 1;
        record[384] = 1;
        assert_eq!(
            titanium_spawn(&record),
            Listing {
                guild: Some(7),
                guild_rank: Some(GuildRank::Officer),
                anonymity: Anonymity::Roleplaying,
                game_master: true,
                away: true,
                looking: true,
            }
        );
    }

    #[test]
    fn updates_change_the_listing() {
        let change = |kind, value| appearance(kind, value).unwrap();
        assert_eq!(change(1, 51), Some(ListingChange::Level(51)));
        assert!(appearance(1, 300).is_err());
        assert_eq!(change(20, 1), Some(ListingChange::GameMaster(true)));
        assert_eq!(
            change(21, 1),
            Some(ListingChange::Anonymity(Anonymity::Anonymous))
        );
        assert_eq!(change(22, u32::MAX), Some(ListingChange::Guild(None)));
        for (value, rank) in [
            (0, GuildRank::Member),
            (1, GuildRank::Officer),
            (2, GuildRank::Leader),
            (3, GuildRank::Other(3)),
        ] {
            assert_eq!(change(23, value), Some(ListingChange::GuildRank(rank)));
        }
        assert_eq!(change(24, 0), Some(ListingChange::Away(false)));
        assert_eq!(change(14, 110), None);
        let mut body = 12u32.to_le_bytes().to_vec();
        body.extend_from_slice(&[1, 0, 0, 0]);
        assert_eq!(looking(&body).unwrap(), (12, ListingChange::Looking(true)));
        assert!(looking(&body[..7]).is_err());
    }

    #[test]
    fn a_listing_keeps_each_change_and_a_new_guild_waits_for_its_rank() {
        let mut listing = Listing::default();
        for change in [
            ListingChange::Guild(Some(3)),
            ListingChange::GuildRank(GuildRank::Leader),
            ListingChange::Anonymity(Anonymity::Roleplaying),
            ListingChange::GameMaster(true),
            ListingChange::Away(true),
            ListingChange::Looking(true),
            ListingChange::Level(40),
        ] {
            listing.apply(change);
        }
        assert_eq!(
            listing,
            Listing {
                guild: Some(3),
                guild_rank: Some(GuildRank::Leader),
                anonymity: Anonymity::Roleplaying,
                game_master: true,
                away: true,
                looking: true,
            }
        );
        // The same guild again keeps the rank; another or none clears it.
        listing.apply(ListingChange::Guild(Some(3)));
        assert_eq!(listing.guild_rank, Some(GuildRank::Leader));
        listing.apply(ListingChange::Guild(Some(4)));
        assert_eq!((listing.guild, listing.guild_rank), (Some(4), None));
        listing.apply(ListingChange::GuildRank(GuildRank::Member));
        listing.apply(ListingChange::Guild(None));
        assert_eq!((listing.guild, listing.guild_rank), (None, None));
        // A rank with no guild, as EQEmu sends after a member's removal.
        listing.apply(ListingChange::GuildRank(GuildRank::Member));
        assert_eq!((listing.guild, listing.guild_rank), (None, None));
    }

    #[test]
    fn the_player_says_how_who_lists_them_as_others_read_it() {
        let away = titanium_away(7, true).unwrap();
        assert_eq!(away.opcode, APPEARANCE_OPCODE);
        assert_eq!(away.body, [7, 0, 24, 0, 1, 0, 0, 0]);
        for anonymity in [
            Anonymity::Open,
            Anonymity::Anonymous,
            Anonymity::Roleplaying,
        ] {
            let sent = titanium_anonymity(7, anonymity).unwrap();
            assert_eq!(
                appearance(21, word(&sent.body, 4)).unwrap(),
                Some(ListingChange::Anonymity(anonymity))
            );
        }
        assert!(titanium_away(0, false).is_err());
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
