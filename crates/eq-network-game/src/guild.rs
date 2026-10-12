//! The player's guild: its member list.
//!
//! `EQEmu` sends a player their guild's whole member list
//! (`OP_GuildMemberList`) as they enter a zone, when their list is out of
//! date, as they join, and as they reload their interface. Guild numbers
//! name guilds from the guild list ([`crate::listing`]). The session keeps
//! no roster: each update passes to the host.
//!
//! Ranks are Titanium's three ([`GuildRank`](crate::listing::GuildRank)),
//! onto which `EQEmu`'s encoders map its own eight.
//!
//! Layout reference: `EQEmu`'s Titanium `GuildMembers_Struct` and
//! `GuildMemberEntry_Struct` (`common/patches/titanium_structs.h`), which
//! `ENCODE(OP_GuildMemberList)` fills (`common/patches/titanium.cpp`) from
//! `ZoneGuildManager::MakeGuildMembers` (`zone/guild_mgr.cpp`). The senders
//! are `Client::SendGuildMembersList` and `Client::SendGuildMembers`
//! (`zone/guild.cpp`).
use crate::listing::GuildRank;
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// `OP_GuildMemberList`: the guild's members (Titanium).
pub const MEMBER_LIST_OPCODE: u16 = 0x147d;

/// The smallest member in the list: an empty name and note.
const MEMBER_LEAST: usize = 42;
/// The banker word's bits: the low one for a banker, the next for an alt.
const BANKER: u32 = 1;
const ALT: u32 = 2;

/// News of the player's guild.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum GuildUpdate {
    /// The guild's members, in full.
    Members(Vec<GuildMember>),
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
/// # Errors
/// Rejects malformed records of those messages.
pub(crate) fn titanium(opcode: u16, body: &[u8]) -> Result<Option<GuildUpdate>> {
    Ok(Some(match opcode {
        MEMBER_LIST_OPCODE => GuildUpdate::Members(titanium_members(body)?),
        _ => return Ok(None),
    }))
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
            zone_id: (zone_id != 0).then_some(zone_id),
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
