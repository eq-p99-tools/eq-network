//! What game masters and the world tell the zone of a player: a kick, a
//! kill, and a player made to look like an NPC.
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
//! Nothing here acts on a notice: the session passes each to the host.
//!
//! Layout reference: `GMKick_Struct`, `GMKill_Struct` and
//! `BecomeNPC_Struct` (`common/eq_packet_structs.h`), the same in `EQEmu`,
//! TAKP and Quarm, which both wires carry as built. The world's kick and
//! kill come from `Client::WorldKick` and `Client::GMKill`
//! (`zone/client.cpp`), and a game master's from `Client::Handle_OP_GMKick`,
//! `Client::Handle_OP_GMKill` and `Client::Handle_OP_GMBecomeNPC`
//! (`zone/client_packet.cpp`).
use crate::world::until_nul;
use anyhow::{ensure, Result};
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

/// A name field.
const NAME: usize = 64;
/// The kick and kill record: two names and a byte.
const KICK_LENGTH: usize = 2 * NAME + 1;
/// The become-NPC record.
const BECOME_NPC_LENGTH: usize = 8;

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
    let name = |at: usize| {
        let field = &body[at..at + NAME];
        ensure!(field.contains(&0), "{what} name without its end");
        Ok(String::from_utf8_lossy(until_nul(field)).into_owned())
    };
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
