//! What game masters and the world tell the zone of a player: a kick, a
//! kill, and a player made to look like an NPC; and, to game masters, the
//! petition queue.
//!
//! The world's kick and kill (`OP_GMKick`, `OP_GMKill`) reach only the
//! player they name, with no game master's name; after a kick, the server
//! drops the player. A game master's `/kick` or `/kill` of a player in the
//! same zone is passed on to the whole zone, the game master included, and
//! the server does nothing more: it leaves the named player's client to
//! leave or die. A game master's `/becomenpc` (`OP_GMBecomeNPC`) reaches
//! only the player it names, whom the server then shows with an NPC's name
//! colour and whose tells it stops.
//!
//! TAKP and Quarm send each game master of status 80 and up in the zone
//! the petition queue (`OP_PetitionRefresh`), one record per petition, as
//! the queue changes and as a game master enters the zone; a record
//! numbered 0 says the queue is empty. Quarm changes the queue only under
//! its rule `Petitions:PetitionSystemActive`, off by default.
//!
//! Nothing here acts on a notice: the session passes each to the host.
//!
//! Layout reference: `GMKick_Struct`, `GMKill_Struct` and
//! `BecomeNPC_Struct` (`common/eq_packet_structs.h`), the same in `EQEmu`,
//! TAKP and Quarm, which both wires carry as built; and TAKP's and Quarm's
//! `PetitionUpdate_Struct`. The world's kick and kill come from
//! `Client::WorldKick` and `Client::GMKill` (`zone/client.cpp`), a game
//! master's from `Client::Handle_OP_GMKick`, `Client::Handle_OP_GMKill`
//! and `Client::Handle_OP_GMBecomeNPC` (`zone/client_packet.cpp`), and the
//! petition queue from `EntityList::SendPetitionToAdmins`
//! (`zone/entity.cpp`) by `PetitionList::UpdateGMQueue`
//! (`zone/petitions.cpp`).
use crate::world::until_nul;
use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;

/// `OP_GMKick`: a player is kicked from the game (Titanium).
pub const KICK_OPCODE: u16 = 0x692c;
/// `OP_GMKill`: a player is killed (Titanium).
pub const KILL_OPCODE: u16 = 0x6980;
/// `OP_GMBecomeNPC`: the player is made to look like an NPC (Titanium).
pub const BECOME_NPC_OPCODE: u16 = 0x7864;
/// `EQMac`'s `OP_GMKick`.
pub const EQMAC_KICK_OPCODE: u16 = 0x6d40;
/// `EQMac`'s `OP_GMKill`.
pub const EQMAC_KILL_OPCODE: u16 = 0x6c40;
/// `EQMac`'s `OP_GMBecomeNPC`.
pub const EQMAC_BECOME_NPC_OPCODE: u16 = 0x8c41;
/// `EQMac`'s `OP_PetitionRefresh`: a petition in the queue, for a game
/// master.
pub const EQMAC_PETITION_OPCODE: u16 = 0x1140;

/// A name field.
const NAME: usize = 64;
/// The kick and kill record: two names and a byte.
const KICK_LENGTH: usize = 2 * NAME + 1;
/// The become-NPC record.
const BECOME_NPC_LENGTH: usize = 8;
/// The petition record, from a server whose time is 32 bits.
const PETITION_LENGTH: usize = 180;
/// From a server whose time is 64 bits.
const WIDE_PETITION_LENGTH: usize = 184;
/// The petition's account field.
const ACCOUNT: usize = 32;
/// The status of a petition a game master has checked out.
const CHECKED_OUT: u32 = 0xffff_ffff;

/// What a game master or the world tells the zone of a player.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum GmNotice {
    /// A player is kicked from the game. From the world it names the player
    /// alone, and the server then drops them; a game master's `/kick` of a
    /// player in the zone reaches everyone in it, and the server leaves the
    /// named player's client to leave.
    Kicked {
        /// Whom, by name; the player's own or another's.
        name: String,
        /// The game master who kicked them; None for the world's kick.
        gm_name: Option<String>,
    },
    /// A player is killed. From the world it names the player alone; a
    /// game master's `/kill` of a player in the zone reaches everyone in
    /// it. Neither kills anyone by itself: the server leaves it to the
    /// named player's client.
    Killed {
        /// Whom, by name; the player's own or another's.
        name: String,
        /// The game master who killed them; None for the world's kill.
        gm_name: Option<String>,
    },
    /// A game master's `/becomenpc` made the player look like an NPC: the
    /// server shows them with an NPC's name colour and stops their tells.
    BecameNpc {
        /// The highest level of player the server lets attack them.
        max_level: i32,
    },
    /// A petition in the queue, for a game master; None for the record
    /// that says the queue is empty (`EQMac`).
    Petition(Option<PetitionEntry>),
}

/// A petition in the queue, as TAKP and Quarm send it to a game master.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PetitionEntry {
    /// The petition's number.
    pub number: u32,
    /// How urgent it is, as sent: 0 green, 1 yellow and 2 red by TAKP's
    /// notes; 0 once it is checked out.
    pub urgency: u32,
    /// Whether a game master has checked it out; such a petition carries
    /// no account or character.
    pub checked_out: bool,
    /// When it was sent, in seconds since 1970, as wide as the server's
    /// time: 32 or 64 bits.
    pub sent: u64,
    /// The petitioner's account; empty once it is checked out.
    pub account: String,
    /// The petitioner's character; empty once it is checked out.
    pub character: String,
    /// How many petitions the queue holds.
    pub queue_total: i32,
}

/// What Titanium's notices from game masters and the world say; None for
/// other messages.
///
/// # Errors
/// Rejects malformed records of those messages.
pub(crate) fn titanium(opcode: u16, body: &[u8]) -> Result<Option<GmNotice>> {
    Ok(Some(match opcode {
        KICK_OPCODE => kicked(body)?,
        KILL_OPCODE => killed(body)?,
        BECOME_NPC_OPCODE => became_npc(body)?,
        _ => return Ok(None),
    }))
}

/// What `EQMac`'s notices from game masters and the world say; None for
/// other messages.
///
/// # Errors
/// Rejects malformed records of those messages.
pub(crate) fn eqmac(opcode: u16, body: &[u8]) -> Result<Option<GmNotice>> {
    Ok(Some(match opcode {
        EQMAC_KICK_OPCODE => kicked(body)?,
        EQMAC_KILL_OPCODE => killed(body)?,
        EQMAC_BECOME_NPC_OPCODE => became_npc(body)?,
        EQMAC_PETITION_OPCODE => GmNotice::Petition(eqmac_petition(body)?),
        _ => return Ok(None),
    }))
}

/// Decodes `OP_GMKick`.
fn kicked(body: &[u8]) -> Result<GmNotice> {
    let (name, gm_name) = names(body, "kick")?;
    Ok(GmNotice::Kicked { name, gm_name })
}

/// Decodes `OP_GMKill`.
fn killed(body: &[u8]) -> Result<GmNotice> {
    let (name, gm_name) = names(body, "kill")?;
    Ok(GmNotice::Killed { name, gm_name })
}

/// Reads a kick or kill, the same on both generations (129 bytes): the
/// player's name and the game master's, 64 bytes each and each ended by a
/// NUL within them, the game master's empty from the world; then a byte.
///
/// # Errors
/// Rejects a wrong length, a name that fills its field, and no player's
/// name.
fn names(body: &[u8], what: &str) -> Result<(String, Option<String>)> {
    ensure!(body.len() == KICK_LENGTH, "invalid {what} length");
    let name = |at| field(body, at, NAME).with_context(|| format!("{what} name without its end"));
    let (player, gm_name) = (name(0)?, name(NAME)?);
    ensure!(!player.is_empty(), "{what} for no one");
    Ok((player, (!gm_name.is_empty()).then_some(gm_name)))
}

/// Decodes `OP_GMBecomeNPC`, the same on both generations (8 bytes): the
/// player's own spawn, 32 bits, since the servers send it only to the
/// player it names; then the highest level of player the server lets
/// attack them, a signed 32 bits.
///
/// # Errors
/// Rejects a wrong length.
fn became_npc(body: &[u8]) -> Result<GmNotice> {
    ensure!(body.len() == BECOME_NPC_LENGTH, "invalid become-NPC length");
    Ok(GmNotice::BecameNpc {
        max_level: i32::from_le_bytes([body[4], body[5], body[6], body[7]]),
    })
}

/// Decodes `EQMac`'s `OP_PetitionRefresh`: the petition's number, its
/// urgency and its status, each 32 bits, the status 0xffffffff for a
/// petition checked out and otherwise the time it was sent; the time it was
/// sent, 32 or 64 bits as the server's own time is, so 180 or 184 bytes in
/// all; the account in 32 bytes, ended by a NUL within them; the receiving
/// game master's own name in 64; how many petitions the queue holds, a
/// signed 32 bits; and the character in 64 bytes, ended by a NUL within
/// them. A record numbered 0 says the queue is empty.
///
/// # Errors
/// Rejects another length, and an account or character that fills its
/// field.
fn eqmac_petition(body: &[u8]) -> Result<Option<PetitionEntry>> {
    let (sent, account_at) = match body.len() {
        PETITION_LENGTH => (u64::from(word(body, 12)), 16),
        WIDE_PETITION_LENGTH => (
            u64::from(word(body, 12)) | u64::from(word(body, 16)) << 32,
            20,
        ),
        _ => bail!("invalid petition length"),
    };
    let number = word(body, 0);
    if number == 0 {
        return Ok(None);
    }
    let total_at = account_at + ACCOUNT + NAME;
    Ok(Some(PetitionEntry {
        number,
        urgency: word(body, 4),
        checked_out: word(body, 8) == CHECKED_OUT,
        sent,
        account: field(body, account_at, ACCOUNT).context("petition account without its end")?,
        character: field(body, total_at + 4, NAME).context("petition character without its end")?,
        queue_total: i32::from_le_bytes(word(body, total_at).to_le_bytes()),
    }))
}

/// A little-endian word of a fixed-length record.
fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

/// The text of a field of `len` bytes at `at`; None when no NUL ends it
/// within them.
fn field(body: &[u8], at: usize, len: usize) -> Option<String> {
    let field = &body[at..at + len];
    field
        .contains(&0)
        .then(|| String::from_utf8_lossy(until_nul(field)).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::WorldEvent;

    fn record(name: &[u8], gm_name: &[u8]) -> [u8; KICK_LENGTH] {
        let mut body = [0_u8; KICK_LENGTH];
        body[..name.len()].copy_from_slice(name);
        body[NAME..NAME + gm_name.len()].copy_from_slice(gm_name);
        body
    }

    #[test]
    fn kicks_and_kills_name_whom_and_any_game_master() {
        type Decoder = fn(u16, &[u8]) -> Result<Option<GmNotice>>;
        let relayed = record(b"Examplar", b"Gamemaster");
        let from_world = record(b"Examplar", b"");
        for (kick, kill, decode) in [
            (KICK_OPCODE, KILL_OPCODE, titanium as Decoder),
            (EQMAC_KICK_OPCODE, EQMAC_KILL_OPCODE, eqmac),
        ] {
            assert_eq!(
                decode(kick, &relayed).unwrap(),
                Some(GmNotice::Kicked {
                    name: "Examplar".into(),
                    gm_name: Some("Gamemaster".into()),
                })
            );
            assert_eq!(
                decode(kill, &from_world).unwrap(),
                Some(GmNotice::Killed {
                    name: "Examplar".into(),
                    gm_name: None,
                })
            );
            assert!(decode(kick, &relayed[..128]).is_err());
            assert!(decode(kill, &record(b"", b"Gamemaster")).is_err());
            let mut full = relayed;
            full[NAME..2 * NAME].fill(b'a');
            assert!(decode(kick, &full).is_err());
        }
    }

    #[test]
    fn becoming_an_npc_carries_the_highest_level_that_may_attack() {
        let mut body = [0_u8; 8];
        body[..4].copy_from_slice(&7_u32.to_le_bytes());
        body[4..].copy_from_slice(&30_i32.to_le_bytes());
        for decoded in [
            titanium(BECOME_NPC_OPCODE, &body),
            eqmac(EQMAC_BECOME_NPC_OPCODE, &body),
        ] {
            assert_eq!(
                decoded.unwrap(),
                Some(GmNotice::BecameNpc { max_level: 30 })
            );
        }
        assert!(titanium(BECOME_NPC_OPCODE, &body[..7]).is_err());
        assert_eq!(titanium(0x1234, &body).unwrap(), None);
    }

    /// A petition record as a server whose time is `time` bytes wide
    /// builds it.
    fn petition(time: usize, number: u32, checked_out: bool) -> Vec<u8> {
        let mut body = vec![0_u8; 176 + time];
        body[..4].copy_from_slice(&number.to_le_bytes());
        let sent = 1_700_000_000_u32;
        if checked_out {
            body[8..12].fill(0xff);
        } else {
            body[4..8].copy_from_slice(&2_u32.to_le_bytes());
            body[8..12].copy_from_slice(&sent.to_le_bytes());
            body[12 + time..12 + time + 9].copy_from_slice(b"synthetic");
            body[12 + time + 32..12 + time + 32 + 10].copy_from_slice(b"Gamemaster");
            body[112 + time..112 + time + 8].copy_from_slice(b"Examplar");
        }
        body[12..16].copy_from_slice(&sent.to_le_bytes());
        body[108 + time..112 + time].copy_from_slice(&3_i32.to_le_bytes());
        body
    }

    #[test]
    fn petitions_read_at_either_width_of_time() {
        for time in [4, 8] {
            assert_eq!(
                eqmac(EQMAC_PETITION_OPCODE, &petition(time, 12, false)).unwrap(),
                Some(GmNotice::Petition(Some(PetitionEntry {
                    number: 12,
                    urgency: 2,
                    checked_out: false,
                    sent: 1_700_000_000,
                    account: "synthetic".into(),
                    character: "Examplar".into(),
                    queue_total: 3,
                })))
            );
            assert_eq!(
                eqmac(EQMAC_PETITION_OPCODE, &petition(time, 12, true)).unwrap(),
                Some(GmNotice::Petition(Some(PetitionEntry {
                    number: 12,
                    urgency: 0,
                    checked_out: true,
                    sent: 1_700_000_000,
                    account: String::new(),
                    character: String::new(),
                    queue_total: 3,
                })))
            );
            // The empty queue's record.
            assert_eq!(
                eqmac(EQMAC_PETITION_OPCODE, &petition(time, 0, true)).unwrap(),
                Some(GmNotice::Petition(None))
            );
            let mut full = petition(time, 12, false);
            full[12 + time..12 + time + 32].fill(b'a');
            assert!(eqmac(EQMAC_PETITION_OPCODE, &full).is_err());
            let mut full = petition(time, 12, false);
            full[112 + time..176 + time].fill(b'a');
            assert!(eqmac(EQMAC_PETITION_OPCODE, &full).is_err());
        }
        // A 64-bit time past 32 bits keeps its high word.
        let mut late = petition(8, 12, false);
        late[16..20].copy_from_slice(&1_u32.to_le_bytes());
        let Some(GmNotice::Petition(Some(entry))) = eqmac(EQMAC_PETITION_OPCODE, &late).unwrap()
        else {
            panic!("a petition");
        };
        assert_eq!(entry.sent, 1_700_000_000 + (1 << 32));
        assert!(eqmac(EQMAC_PETITION_OPCODE, &[0; 179]).is_err());
        assert!(eqmac(EQMAC_PETITION_OPCODE, &[0; 188]).is_err());
    }

    #[test]
    fn zone_updates_carry_notices() {
        let body = record(b"Examplar", b"");
        assert_eq!(
            crate::world::titanium_update(KICK_OPCODE, &body).unwrap(),
            Some(WorldEvent::Gm(GmNotice::Kicked {
                name: "Examplar".into(),
                gm_name: None,
            }))
        );
        assert_eq!(
            crate::quarm::updates(EQMAC_KILL_OPCODE, &body).unwrap(),
            vec![WorldEvent::Gm(GmNotice::Killed {
                name: "Examplar".into(),
                gm_name: None,
            })]
        );
    }
}
