//! The player's guild: its member list, its members' changes and
//! invitations.
//!
//! `EQEmu` sends a player their guild's whole member list
//! (`OP_GuildMemberList`) as they enter a zone, when their list is out of
//! date, as they join, and as they reload their interface; after the
//! reload, it also sends them a record for each guild member online then,
//! with the zone they are in (`OP_GuildMemberUpdate`). As the world
//! tells of a change, it sends the guild's members in the zone a member
//! entering a zone or leaving the game (`OP_GuildMemberUpdate`), joining
//! (`OP_GuildMemberAdd`, or the whole list where a member's is out of
//! date), and leaving (`OP_GuildMemberDelete`); a member's new level,
//! public note, and rank and flags (`OP_GuildMemberLevel`,
//! `OP_GuildMemberPublicNote` and `OP_GuildMemberRankAltBanker`); and the
//! guild's end (`OP_GuildDeleteGuild`). In the guild hall, it sends a
//! player their own banker flag as it changes (`OP_SetGuildRank`). Nothing
//! in `EQEmu` sends `OP_GuildMemberDetails`, `OP_GuildMemberRename` or
//! `OP_GuildRenameGuild` today, `#guild rename` included; they are read as
//! their layouts say.
//!
//! Every server passes an invitation (`OP_GuildInvite`) from the inviter's
//! client to the player invited: to join, to a player in no guild, or to
//! take a higher rank, to a member. Nothing here answers one yet. TAKP and
//! Quarm send the guild's members in the zone a member's removal
//! (`OP_GuildRemove`) only when the removal fails, with a line saying it
//! succeeded; it reads as what it says, so it is no proof of the roster.
//!
//! Guild numbers name guilds from the guild list ([`crate::listing`]). The
//! session keeps no roster: each update passes to the host.
//!
//! Ranks are Titanium's three ([`GuildRank`](crate::listing::GuildRank)),
//! onto which `EQEmu`'s encoders map its own eight; TAKP and Quarm number
//! theirs the same.
//!
//! Layout reference: `EQEmu`'s Titanium `GuildMembers_Struct`,
//! `GuildMemberEntry_Struct`, `GuildMemberAdd_Struct`,
//! `GuildMemberRank_Struct` and `GuildSetRank_Struct`
//! (`common/patches/titanium_structs.h`), which Titanium's encoders fill
//! (`common/patches/titanium.cpp`); and `GuildMemberUpdate_Struct`,
//! `GuildMemberDetails_Struct`, `GuildMemberDelete_Struct`,
//! `GuildMemberLevel_Struct`, `GuildMemberPublicNote_Struct`,
//! `GuildRenameMember_Struct`, `GuildRenameGuild_Struct` and
//! `GuildDelete_Struct` (`common/eq_packet_structs.h`), which Titanium's
//! wire carries as built, as it does `GuildCommand_Struct`; and TAKP's and
//! Quarm's `GuildCommand_Struct` and `GuildRemove_Struct`
//! (`common/eq_packet_structs.h`). The member list comes from
//! `ZoneGuildManager::MakeGuildMembers` (`zone/guild_mgr.cpp`) by
//! `Client::SendGuildMembersList` and `Client::SendGuildMembers`, the
//! changes by the senders in `zone/guild.cpp` from
//! `ZoneGuildManager::ProcessWorldPacket`, and `OP_SetGuildRank` by
//! `Client::RefreshGuildInfo`; the invitations and removals pass through
//! `Client::Handle_OP_GuildInvite` and TAKP's and Quarm's
//! `Client::Handle_OP_GuildRemove` (`zone/client_packet.cpp`).
use crate::{listing::GuildRank, world::until_nul};
use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;

/// `OP_GuildMemberList`: the guild's members (Titanium).
pub const MEMBER_LIST_OPCODE: u16 = 0x147d;
/// `OP_GuildMemberUpdate`: a member entered a zone or left the game; and
/// `OP_GuildMemberDetails`, which shares its number (Titanium).
pub const MEMBER_UPDATE_OPCODE: u16 = 0x0f4d;
/// `OP_GuildMemberAdd`: a player joined the guild (Titanium).
pub const MEMBER_ADD_OPCODE: u16 = 0x754e;
/// `OP_GuildMemberDelete`: a member left the guild (Titanium).
pub const MEMBER_DELETE_OPCODE: u16 = 0x712a;
/// `OP_GuildMemberLevel`: a member's new level (Titanium).
pub const MEMBER_LEVEL_OPCODE: u16 = 0x461a;
/// `OP_GuildMemberPublicNote`: a member's new public note (Titanium).
pub const MEMBER_NOTE_OPCODE: u16 = 0x32cf;
/// `OP_GuildMemberRankAltBanker`: a member's rank and flags; and
/// `OP_SetGuildRank`, the player's own, which shares its number (Titanium).
pub const MEMBER_RANK_OPCODE: u16 = 0x6966;
/// `OP_GuildMemberRename`: a member's new name (Titanium).
pub const MEMBER_RENAME_OPCODE: u16 = 0x7c59;
/// `OP_GuildRenameGuild`: the guild's new name (Titanium).
pub const RENAME_OPCODE: u16 = 0x4cc7;
/// `OP_GuildDeleteGuild`: the guild was disbanded (Titanium).
pub const DISBAND_OPCODE: u16 = 0x3230;
/// `OP_GuildInvite`: an invitation to join the guild or take a higher
/// rank in it (Titanium).
pub const INVITE_OPCODE: u16 = 0x18b7;
/// `EQMac`'s `OP_GuildInvite`.
pub const EQMAC_INVITE_OPCODE: u16 = 0x1741;
/// `EQMac`'s `OP_GuildRemove`: a member's removal, which TAKP and Quarm
/// send only when it fails.
pub const EQMAC_REMOVE_OPCODE: u16 = 0x1941;

/// The smallest member in the list: an empty name and note.
const MEMBER_LEAST: usize = 42;
/// A name's field.
const NAME: usize = 64;
/// A public note's field.
const NOTE: usize = 256;
/// `OP_GuildMemberUpdate`'s record, and `OP_GuildMemberDetails`'.
const UPDATE_LENGTH: usize = 76;
const DETAILS_LENGTH: usize = 80;
/// `OP_GuildMemberAdd`'s record after Titanium's encoder.
const ADD_LENGTH: usize = 92;
/// The record of both messages numbered 0x6966 after Titanium's encoders.
const RANK_LENGTH: usize = 80;
/// The invitation's record on both generations, and `EQMac`'s removal's.
const COMMAND_LENGTH: usize = 136;
/// The banker word's bits: the low one for a banker, the next for an alt.
const BANKER: u32 = 1;
const ALT: u32 = 2;

/// News of the player's guild.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum GuildUpdate {
    /// The guild's members, in full.
    Members(Vec<GuildMember>),
    /// Where a member is: one who entered a zone or left the game, or after
    /// an interface reload each member online, from `OP_GuildMemberUpdate`;
    /// or from `OP_GuildMemberDetails`, which nothing in `EQEmu` sends
    /// today.
    MemberLocation {
        /// The guild.
        guild_id: u32,
        /// The member.
        name: String,
        /// The zone they entered or are in; None as they leave the game.
        zone_id: Option<u16>,
        /// When they were last seen, in seconds since 1970.
        last_on: u32,
    },
    /// A player joined the guild.
    MemberAdded {
        /// The guild.
        guild_id: u32,
        /// The new member.
        name: String,
        /// Their level.
        level: u32,
        /// Their class number.
        class: u32,
        /// Their rank.
        rank: GuildRank,
        /// The zone they are in; None while offline.
        zone_id: Option<u16>,
        /// When they were last on, in seconds since 1970.
        last_on: u32,
    },
    /// A member left the guild, or was removed from it. On `EQMac`, TAKP
    /// and Quarm send it only when a removal fails, with a line saying it
    /// succeeded: it says what the official client is told, which is no
    /// proof of the roster.
    MemberRemoved {
        /// The guild.
        guild_id: u32,
        /// The member.
        name: String,
    },
    /// A member's new level.
    MemberLevel {
        /// The guild.
        guild_id: u32,
        /// The member.
        name: String,
        /// Their level.
        level: u32,
    },
    /// A member's new public note.
    MemberNote {
        /// The guild.
        guild_id: u32,
        /// The member.
        name: String,
        /// Their note.
        note: String,
    },
    /// A member's rank and flags, from `OP_GuildMemberRankAltBanker`; or
    /// the player's own, from `OP_SetGuildRank`, which names no guild and
    /// sets no alt flag.
    MemberRank {
        /// The guild; None from `OP_SetGuildRank`.
        guild_id: Option<u32>,
        /// The member.
        name: String,
        /// Their rank.
        rank: GuildRank,
        /// Whether they may use the guild bank.
        banker: bool,
        /// Whether they are another member's alternate character.
        alt: bool,
    },
    /// A member's new name, which nothing in `EQEmu` sends today.
    MemberRenamed {
        /// The guild.
        guild_id: u32,
        /// Their name before.
        old_name: String,
        /// Their name now.
        new_name: String,
    },
    /// The guild's new name, which nothing in `EQEmu` sends today.
    Renamed {
        /// The guild.
        guild_id: u32,
        /// Its name now.
        name: String,
    },
    /// The guild was disbanded.
    Disbanded {
        /// The guild.
        guild_id: u32,
    },
    /// An invitation to join the guild, to a player in no guild; or, to a
    /// member, to take a higher rank in it. Nothing answers it yet.
    Invite {
        /// Who invites.
        inviter: String,
        /// The guild.
        guild_id: u32,
        /// The rank offered.
        rank: GuildRank,
    },
}

/// A member of the player's guild, as the member list gives them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GuildMember {
    /// Their name.
    pub name: String,
    /// Their level.
    pub level: u32,
    /// Their class number.
    pub class: u32,
    /// Their rank, as sent: `EQEmu` maps it twice when the member's
    /// character number is also the spawn number of a client older than
    /// `RoF` in the zone, so an officer can read as the leader and the leader
    /// as an officer (`ZoneGuildManager::MakeGuildMembers`).
    pub rank: GuildRank,
    /// Whether they may use the guild bank.
    pub banker: bool,
    /// Whether they are another member's alternate character.
    pub alt: bool,
    /// When they were last on, in seconds since 1970.
    pub last_on: u32,
    /// What they give the guild in tribute.
    pub tribute: GuildTribute,
    /// Their public note.
    pub note: String,
    /// The zone they are in; None while offline.
    pub zone_id: Option<u16>,
}

/// What a member gives their guild in tribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct GuildTribute {
    /// Whether their tribute goes to the guild.
    pub on: bool,
    /// How much they have given in all.
    pub total: u32,
    /// When they last gave, in seconds since 1970.
    pub last: u32,
}

/// What Titanium's guild messages say; None for other messages.
///
/// The six simplest are read here, every number in them 32 bits and every
/// name in 64 bytes, ended by a NUL within them: `OP_GuildMemberDelete`
/// (68 bytes), the guild and the member's name; `OP_GuildMemberLevel`
/// (72), those and the level; `OP_GuildMemberPublicNote` (324), those and
/// the note in 256 bytes; `OP_GuildMemberRename` (132), the guild, the
/// old name and the new; `OP_GuildRenameGuild` (68), the guild and its new
/// name; and `OP_GuildDeleteGuild` (4), the guild.
///
/// # Errors
/// Rejects malformed records of those messages.
pub(crate) fn titanium(opcode: u16, body: &[u8]) -> Result<Option<GuildUpdate>> {
    Ok(Some(match opcode {
        MEMBER_LIST_OPCODE => GuildUpdate::Members(titanium_members(body)?),
        MEMBER_UPDATE_OPCODE => member_location(body)?,
        MEMBER_ADD_OPCODE => member_added(body)?,
        MEMBER_RANK_OPCODE => member_rank(body)?,
        INVITE_OPCODE => invite(body)?,
        MEMBER_DELETE_OPCODE => {
            sized(body, 4 + NAME, "member removal")?;
            GuildUpdate::MemberRemoved {
                guild_id: word(body, 0),
                name: name(body, 4)?,
            }
        }
        MEMBER_LEVEL_OPCODE => {
            sized(body, 8 + NAME, "member level")?;
            GuildUpdate::MemberLevel {
                guild_id: word(body, 0),
                name: name(body, 4)?,
                level: word(body, 4 + NAME),
            }
        }
        MEMBER_NOTE_OPCODE => {
            sized(body, 4 + NAME + NOTE, "member note")?;
            GuildUpdate::MemberNote {
                guild_id: word(body, 0),
                name: name(body, 4)?,
                note: field(&body[4 + NAME..], "public note")?,
            }
        }
        MEMBER_RENAME_OPCODE => {
            sized(body, 4 + 2 * NAME, "member rename")?;
            GuildUpdate::MemberRenamed {
                guild_id: word(body, 0),
                old_name: name(body, 4)?,
                new_name: name(body, 4 + NAME)?,
            }
        }
        RENAME_OPCODE => {
            sized(body, 4 + NAME, "rename")?;
            GuildUpdate::Renamed {
                guild_id: word(body, 0),
                name: name(body, 4)?,
            }
        }
        DISBAND_OPCODE => {
            sized(body, 4, "disbanding")?;
            GuildUpdate::Disbanded {
                guild_id: word(body, 0),
            }
        }
        _ => return Ok(None),
    }))
}

/// A little-endian word of a record.
fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

/// Checks a record's length.
fn sized(body: &[u8], length: usize, what: &str) -> Result<()> {
    ensure!(body.len() == length, "invalid guild {what} length");
    Ok(())
}

/// A text field's text, which a NUL ends within the field.
fn field(bytes: &[u8], what: &str) -> Result<String> {
    ensure!(bytes.contains(&0), "guild {what} without its end");
    Ok(String::from_utf8_lossy(until_nul(bytes)).into_owned())
}

/// The name in the 64 bytes at `at`.
fn name(body: &[u8], at: usize) -> Result<String> {
    field(&body[at..at + NAME], "name")
}

/// The guild in a 16-bit field.
fn short_guild(body: &[u8], at: usize) -> u32 {
    u32::from(u16::from_le_bytes([body[at], body[at + 1]]))
}

/// A zone, None for 0: the member is offline.
fn zone(zone_id: u16) -> Option<u16> {
    (zone_id != 0).then_some(zone_id)
}

/// A zone sent in 32 bits.
fn wide_zone(body: &[u8], at: usize) -> Result<Option<u16>> {
    Ok(zone(
        u16::try_from(word(body, at)).context("guild member's zone out of range")?,
    ))
}

/// Decodes `OP_GuildMemberUpdate` (76 bytes): the guild, 32 bits; the
/// member's name in 64 bytes, ended by a NUL within them; the zone they
/// entered or are in, 0 as they leave the game, and an instance `EQEmu`
/// sets to 0 for a change and to 1 for a member online after an interface
/// reload, each 16 bits; and when they were last seen, 32 bits.
/// `OP_GuildMemberDetails` shares its number at 80 bytes: the guild, the
/// name, the zone and when they were last on, each number 32 bits, then 1
/// while they are offline.
///
/// # Errors
/// Rejects another length, a name that fills its field, and a zone past
/// 16 bits.
fn member_location(body: &[u8]) -> Result<GuildUpdate> {
    let zone_id = match body.len() {
        UPDATE_LENGTH => zone(u16::from_le_bytes([body[68], body[69]])),
        DETAILS_LENGTH => wide_zone(body, 68)?.filter(|_| word(body, 76) == 0),
        _ => bail!("invalid guild member update length"),
    };
    Ok(GuildUpdate::MemberLocation {
        guild_id: word(body, 0),
        name: name(body, 4)?,
        zone_id,
        last_on: word(body, 72),
    })
}

/// Decodes `OP_GuildMemberAdd` (92 bytes after Titanium's encoder): the
/// guild, a word the encoder leaves zero, and the new member's level,
/// class, rank, zone and when they were last on, each 32 bits; then their
/// name in 64 bytes, ended by a NUL within them.
///
/// # Errors
/// Rejects a wrong length, a name that fills its field, and a zone past
/// 16 bits.
fn member_added(body: &[u8]) -> Result<GuildUpdate> {
    sized(body, ADD_LENGTH, "member add")?;
    Ok(GuildUpdate::MemberAdded {
        guild_id: word(body, 0),
        name: name(body, 28)?,
        level: word(body, 8),
        class: word(body, 12),
        rank: GuildRank::from_wire(word(body, 16)),
        zone_id: wide_zone(body, 20)?,
        last_on: word(body, 24),
    })
}

/// Decodes the two messages numbered 0x6966 (80 bytes after Titanium's
/// encoders), which share one layout: the guild in
/// `OP_GuildMemberRankAltBanker` and 0 in `OP_SetGuildRank`, a word the
/// encoders leave zero, and the rank, each 32 bits; the member's name in
/// 64 bytes, ended by a NUL within them; and a word with 1 for a banker
/// and 2 for an alt, of which `OP_SetGuildRank` sets only the banker's.
///
/// # Errors
/// Rejects a wrong length and a name that fills its field.
fn member_rank(body: &[u8]) -> Result<GuildUpdate> {
    sized(body, RANK_LENGTH, "rank")?;
    let flags = word(body, 76);
    Ok(GuildUpdate::MemberRank {
        guild_id: crate::listing::guild(word(body, 0)),
        name: name(body, 12)?,
        rank: GuildRank::from_wire(word(body, 8)),
        banker: flags & BANKER != 0,
        alt: flags & ALT != 0,
    })
}

/// Reads the member list's fields in order; its numbers are big-endian.
struct Fields<'a>(&'a [u8]);

impl Fields<'_> {
    fn number(&mut self) -> Result<u32> {
        let (bytes, rest) = self
            .0
            .split_first_chunk::<4>()
            .context("guild member list ends inside a number")?;
        self.0 = rest;
        Ok(u32::from_be_bytes(*bytes))
    }

    fn short(&mut self) -> Result<u16> {
        let (bytes, rest) = self
            .0
            .split_first_chunk::<2>()
            .context("guild member list ends inside a number")?;
        self.0 = rest;
        Ok(u16::from_be_bytes(*bytes))
    }

    fn text(&mut self) -> Result<String> {
        let end = self
            .0
            .iter()
            .position(|byte| *byte == 0)
            .context("guild member list ends inside a name or note")?;
        let text = String::from_utf8_lossy(&self.0[..end]).into_owned();
        self.0 = &self.0[end + 1..];
        Ok(text)
    }

    fn member(&mut self) -> Result<GuildMember> {
        let name = self.text()?;
        let level = self.number()?;
        let flags = self.number()?;
        let class = self.number()?;
        let rank = GuildRank::from_wire(self.number()?);
        let last_on = self.number()?;
        let on = self.number()? != 0;
        let total = self.number()?;
        let last = self.number()?;
        // A word EQEmu's encoder always sets to 1.
        self.number()?;
        let note = self.text()?;
        // The instance, which the encoder always sets to 0.
        self.short()?;
        let zone_id = self.short()?;
        Ok(GuildMember {
            name,
            level,
            class,
            rank,
            banker: flags & BANKER != 0,
            alt: flags & ALT != 0,
            last_on,
            tribute: GuildTribute { on, total, last },
            note,
            zone_id: zone(zone_id),
        })
    }
}

/// Decodes Titanium's `OP_GuildMemberList`, every number big-endian: a
/// name and its NUL (the guild's, or the player's own on a reload), the
/// member count, then for each member their name and its NUL; their level,
/// a word with 1 for a banker and 2 for an alt, class, rank, when they
/// were last on, whether their tribute is on, its total and when they last
/// gave, and a word always 1, each 32 bits; their public note and its NUL;
/// and an instance always 0 and their zone, 0 while offline, each 16 bits.
///
/// # Errors
/// Rejects a list that ends inside a member, a name or note without its
/// end, and bytes past the last member.
fn titanium_members(body: &[u8]) -> Result<Vec<GuildMember>> {
    let mut fields = Fields(body);
    fields.text()?;
    let count = usize::try_from(fields.number()?)?;
    ensure!(
        count <= fields.0.len() / MEMBER_LEAST,
        "guild member list shorter than its count"
    );
    let members = (0..count)
        .map(|_| fields.member())
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        fields.0.is_empty(),
        "guild member list longer than its members"
    );
    Ok(members)
}

/// Decodes `OP_GuildInvite`, the same on both generations (136 bytes): the
/// invitee's name and the inviter's, each in 64 bytes and ended by a NUL
/// within them; the guild, 16 bits, which the servers fill in where the
/// inviter's client left it 0, and two bytes; and the rank offered, 32
/// bits.
///
/// # Errors
/// Rejects a wrong length and an inviter's name that fills its field.
pub(crate) fn invite(body: &[u8]) -> Result<GuildUpdate> {
    sized(body, COMMAND_LENGTH, "invitation")?;
    Ok(GuildUpdate::Invite {
        inviter: name(body, NAME)?,
        guild_id: short_guild(body, 2 * NAME),
        rank: GuildRank::from_wire(word(body, 2 * NAME + 4)),
    })
}

/// Decodes `EQMac`'s `OP_GuildRemove` (136 bytes): the remover's name and
/// the removed member's, each in 64 bytes and ended by a NUL within them;
/// the guild, 16 bits, and two bytes; and a rank, 32 bits. TAKP and Quarm
/// leave the remover and the rank empty.
///
/// # Errors
/// Rejects a wrong length and a member's name that fills its field.
pub(crate) fn eqmac_removal(body: &[u8]) -> Result<GuildUpdate> {
    sized(body, COMMAND_LENGTH, "removal")?;
    Ok(GuildUpdate::MemberRemoved {
        guild_id: short_guild(body, 2 * NAME),
        name: name(body, NAME)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{titanium_update, WorldEvent};

    /// A member as `EQEmu`'s Titanium encoder writes them.
    fn member(name: &str, numbers: [u32; 8], note: &str, zone_id: u16) -> Vec<u8> {
        let mut entry = name.as_bytes().to_vec();
        entry.push(0);
        for number in numbers.into_iter().chain([1]) {
            entry.extend_from_slice(&number.to_be_bytes());
        }
        entry.extend_from_slice(note.as_bytes());
        entry.push(0);
        entry.extend_from_slice(&[0, 0]);
        entry.extend_from_slice(&zone_id.to_be_bytes());
        entry
    }

    /// A member list with the guild's name before it.
    fn list(members: &[Vec<u8>]) -> Vec<u8> {
        let mut body = b"Synthetic Guild\0".to_vec();
        body.extend_from_slice(&u32::try_from(members.len()).unwrap().to_be_bytes());
        for entry in members {
            body.extend_from_slice(entry);
        }
        body
    }

    #[test]
    fn member_lists_keep_each_member() {
        let body = list(&[
            // An online officer and banker: level 30, class 2, last on,
            // tribute on, 5,000 given, last given.
            member(
                "Firstmember",
                [30, 1, 2, 1, 1_700_000_000, 1, 5000, 1_700_000_100],
                "Main",
                202,
            ),
            // An offline member who is an alt, with no note.
            member("Secondmember", [5, 2, 11, 0, 1_600_000_000, 0, 0, 0], "", 0),
            // Both flags, and a rank this crate does not know.
            member("Third", [60, 3, 14, 9, 0, 0, 0, 0], "Alt of Firstmember", 1),
        ]);
        let Some(WorldEvent::Guild(GuildUpdate::Members(members))) =
            titanium_update(MEMBER_LIST_OPCODE, &body).unwrap()
        else {
            panic!("no member list");
        };
        assert_eq!(
            members[0],
            GuildMember {
                name: "Firstmember".into(),
                level: 30,
                class: 2,
                rank: GuildRank::Officer,
                banker: true,
                alt: false,
                last_on: 1_700_000_000,
                tribute: GuildTribute {
                    on: true,
                    total: 5000,
                    last: 1_700_000_100,
                },
                note: "Main".into(),
                zone_id: Some(202),
            }
        );
        assert_eq!(
            (members[1].rank, members[1].banker, members[1].alt),
            (GuildRank::Member, false, true)
        );
        assert_eq!((members[1].zone_id, members[1].note.as_str()), (None, ""));
        assert_eq!(
            (members[2].rank, members[2].banker, members[2].alt),
            (GuildRank::Other(9), true, true)
        );
        assert_eq!(members[2].zone_id, Some(1));
        assert_eq!(
            titanium(MEMBER_LIST_OPCODE, &list(&[])).unwrap(),
            Some(GuildUpdate::Members(Vec::new()))
        );
        assert_eq!(titanium(0x1234, &body).unwrap(), None);
    }

    /// Little-endian words, as `EQEmu` builds its records.
    fn le(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    /// A text field as `EQEmu` fills it: the text, then NULs.
    fn text(text: &str, length: usize) -> Vec<u8> {
        let mut field = text.as_bytes().to_vec();
        field.resize(length, 0);
        field
    }

    #[test]
    fn member_updates_say_where_a_member_is() {
        let update = |zone_id: u16| {
            let mut body = le(&[7]);
            body.extend(text("Member", NAME));
            body.extend_from_slice(&zone_id.to_le_bytes());
            body.extend_from_slice(&[0, 0]);
            body.extend(le(&[1_700_000_000]));
            body
        };
        let location = |zone_id| GuildUpdate::MemberLocation {
            guild_id: 7,
            name: "Member".into(),
            zone_id,
            last_on: 1_700_000_000,
        };
        assert_eq!(
            titanium_update(MEMBER_UPDATE_OPCODE, &update(202)).unwrap(),
            Some(WorldEvent::Guild(location(Some(202))))
        );
        assert_eq!(
            titanium(MEMBER_UPDATE_OPCODE, &update(0)).unwrap(),
            Some(location(None))
        );
        // The details record: the zone in 32 bits, and 1 while offline.
        let details = |zone_id: u32, offline: u32| {
            let mut body = le(&[7]);
            body.extend(text("Member", NAME));
            body.extend(le(&[zone_id, 1_700_000_000, offline]));
            body
        };
        assert_eq!(
            titanium(MEMBER_UPDATE_OPCODE, &details(202, 0)).unwrap(),
            Some(location(Some(202)))
        );
        assert_eq!(
            titanium(MEMBER_UPDATE_OPCODE, &details(202, 1)).unwrap(),
            Some(location(None))
        );
        assert!(titanium(MEMBER_UPDATE_OPCODE, &details(0x1_0000, 0)).is_err());
        assert!(titanium(MEMBER_UPDATE_OPCODE, &update(202)[..75]).is_err());
        let mut unended = update(202);
        unended[4..4 + NAME].fill(b'a');
        assert!(titanium(MEMBER_UPDATE_OPCODE, &unended).is_err());
    }

    #[test]
    fn new_members_keep_their_level_class_rank_and_zone() {
        let mut body = le(&[7, 0, 30, 2, 1, 202, 1_700_000_000]);
        body.extend(text("Newmember", NAME));
        assert_eq!(
            titanium_update(MEMBER_ADD_OPCODE, &body).unwrap(),
            Some(WorldEvent::Guild(GuildUpdate::MemberAdded {
                guild_id: 7,
                name: "Newmember".into(),
                level: 30,
                class: 2,
                rank: GuildRank::Officer,
                zone_id: Some(202),
                last_on: 1_700_000_000,
            }))
        );
        assert!(titanium(MEMBER_ADD_OPCODE, &body[..91]).is_err());
        body[20..24].copy_from_slice(&0x1_0000u32.to_le_bytes());
        assert!(titanium(MEMBER_ADD_OPCODE, &body).is_err());
    }

    #[test]
    fn ranks_read_alike_from_both_messages_of_their_number() {
        let rank = |words: [u32; 3], flags: u32| {
            let mut body = le(&words);
            body.extend(text("Member", NAME));
            body.extend(le(&[flags]));
            body
        };
        // A member's rank, as an alt and a banker.
        assert_eq!(
            titanium_update(MEMBER_RANK_OPCODE, &rank([7, 0, 2], 3)).unwrap(),
            Some(WorldEvent::Guild(GuildUpdate::MemberRank {
                guild_id: Some(7),
                name: "Member".into(),
                rank: GuildRank::Leader,
                banker: true,
                alt: true,
            }))
        );
        // The player's own, in the guild hall: no guild, a banker, and the
        // rank Titanium's encoder sends for EQEmu's rank of none.
        assert_eq!(
            titanium(MEMBER_RANK_OPCODE, &rank([0, 0, 9], 1)).unwrap(),
            Some(GuildUpdate::MemberRank {
                guild_id: None,
                name: "Member".into(),
                rank: GuildRank::Other(9),
                banker: true,
                alt: false,
            })
        );
        assert!(titanium(MEMBER_RANK_OPCODE, &rank([7, 0, 0], 2)[..79]).is_err());
    }

    #[test]
    fn members_changes_name_the_guild_and_member() {
        let named = |names: &[&str], tail: &[u8]| {
            let mut body = le(&[7]);
            for name in names {
                body.extend(text(name, NAME));
            }
            body.extend_from_slice(tail);
            body
        };
        let cases = [
            (
                MEMBER_DELETE_OPCODE,
                named(&["Member"], &[]),
                GuildUpdate::MemberRemoved {
                    guild_id: 7,
                    name: "Member".into(),
                },
            ),
            (
                MEMBER_LEVEL_OPCODE,
                named(&["Member"], &le(&[31])),
                GuildUpdate::MemberLevel {
                    guild_id: 7,
                    name: "Member".into(),
                    level: 31,
                },
            ),
            (
                MEMBER_NOTE_OPCODE,
                named(&["Member"], &text("A synthetic note", NOTE)),
                GuildUpdate::MemberNote {
                    guild_id: 7,
                    name: "Member".into(),
                    note: "A synthetic note".into(),
                },
            ),
            (
                MEMBER_RENAME_OPCODE,
                named(&["Member", "Renamed"], &[]),
                GuildUpdate::MemberRenamed {
                    guild_id: 7,
                    old_name: "Member".into(),
                    new_name: "Renamed".into(),
                },
            ),
            (
                RENAME_OPCODE,
                named(&["Synthetic Guild"], &[]),
                GuildUpdate::Renamed {
                    guild_id: 7,
                    name: "Synthetic Guild".into(),
                },
            ),
            (
                DISBAND_OPCODE,
                le(&[7]),
                GuildUpdate::Disbanded { guild_id: 7 },
            ),
        ];
        for (opcode, body, update) in cases {
            assert_eq!(
                titanium_update(opcode, &body).unwrap(),
                Some(WorldEvent::Guild(update)),
                "{opcode:#06x}"
            );
            assert!(titanium(opcode, &body[1..]).is_err(), "{opcode:#06x}");
            assert!(
                titanium(opcode, &[body.as_slice(), &[0]].concat()).is_err(),
                "{opcode:#06x}"
            );
        }
        // A name or note that fills its field.
        for (opcode, at, length) in [
            (MEMBER_DELETE_OPCODE, 4, 4 + NAME),
            (MEMBER_RENAME_OPCODE, 4 + NAME, 4 + 2 * NAME),
            (MEMBER_NOTE_OPCODE, 4 + NAME, 4 + NAME + NOTE),
        ] {
            let mut body = vec![0; length];
            body[at..].fill(b'a');
            assert!(titanium(opcode, &body).is_err(), "{opcode:#06x}");
        }
    }

    /// A record of two names, a 16-bit guild, two bytes and a rank, as
    /// `GuildCommand_Struct` and `GuildRemove_Struct` lay them out.
    fn command(first: &str, second: &str, guild_id: u16, rank: u32) -> Vec<u8> {
        let mut body = text(first, NAME);
        body.extend(text(second, NAME));
        body.extend_from_slice(&guild_id.to_le_bytes());
        body.extend_from_slice(&[0, 0]);
        body.extend(le(&[rank]));
        body
    }

    #[test]
    fn invitations_name_the_inviter_guild_and_rank_on_both_generations() {
        let invite = |rank| GuildUpdate::Invite {
            inviter: "Inviter".into(),
            guild_id: 7,
            rank,
        };
        let body = command("Player", "Inviter", 7, 0);
        assert_eq!(
            titanium_update(INVITE_OPCODE, &body).unwrap(),
            Some(WorldEvent::Guild(invite(GuildRank::Member)))
        );
        assert_eq!(
            crate::quarm::updates(EQMAC_INVITE_OPCODE, &command("Player", "Inviter", 7, 1))
                .unwrap(),
            [WorldEvent::Guild(invite(GuildRank::Officer))]
        );
        let rejected = |body: &[u8]| {
            titanium(INVITE_OPCODE, body).is_err()
                && crate::quarm::updates(EQMAC_INVITE_OPCODE, body).is_err()
        };
        assert!(rejected(&body[..135]));
        assert!(rejected(&[body.as_slice(), &[0]].concat()));
        let mut unended = body;
        unended[NAME..2 * NAME].fill(b'a');
        assert!(rejected(&unended));
    }

    #[test]
    fn eqmac_removals_name_the_member_removed() {
        // TAKP's and Quarm's record: no remover and no rank.
        let body = command("", "Member", 7, 0);
        assert_eq!(
            crate::quarm::updates(EQMAC_REMOVE_OPCODE, &body).unwrap(),
            [WorldEvent::Guild(GuildUpdate::MemberRemoved {
                guild_id: 7,
                name: "Member".into(),
            })]
        );
        assert!(crate::quarm::updates(EQMAC_REMOVE_OPCODE, &body[..135]).is_err());
        let mut unended = body;
        unended[NAME..2 * NAME].fill(b'a');
        assert!(crate::quarm::updates(EQMAC_REMOVE_OPCODE, &unended).is_err());
    }

    #[test]
    fn member_lists_must_hold_what_they_count() {
        let entry = member("Member", [1, 0, 1, 0, 0, 0, 0, 0], "", 0);
        let body = list(&[entry.clone(), entry]);
        // One byte short ends inside the last member, and one byte more is
        // past it.
        assert!(titanium(MEMBER_LIST_OPCODE, &body[..body.len() - 1]).is_err());
        assert!(titanium(MEMBER_LIST_OPCODE, &[body.as_slice(), &[0]].concat()).is_err());
        // A count too high for the bytes left, and one too high to hold.
        let mut counted = body.clone();
        counted[16..20].copy_from_slice(&3u32.to_be_bytes());
        assert!(titanium(MEMBER_LIST_OPCODE, &counted).is_err());
        counted[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(titanium(MEMBER_LIST_OPCODE, &counted).is_err());
        // A name without its end.
        assert!(titanium(MEMBER_LIST_OPCODE, b"Synthetic Guild").is_err());
        assert!(titanium(MEMBER_LIST_OPCODE, &[]).is_err());
    }
}
