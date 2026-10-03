//! Raids: inviting a player, accepting or declining an invitation, leaving,
//! the raid leader's commands (locking the raid, moving members between raid
//! groups, handing on the lead, removing a member), and the server's word on
//! who is in the player's raid.
//!
//! Layout reference: the Titanium opcodes (`utils/patches/patch_Titanium.conf`),
//! `EQEmu`'s `RaidGeneral_Struct`, `RaidAddMember_Struct`, `RaidCreate_Struct`
//! and `RaidLeadershipUpdate_Struct` (`common/patches/titanium_structs.h`) as
//! the Titanium patch writes them (`ENCODE(OP_RaidUpdate)`,
//! `common/patches/titanium.cpp`), and the action numbers in `common/raid.h`;
//! the rules are `Client::Handle_OP_RaidCommand` (`zone/client_packet.cpp`)
//! and `Raid::AddMember`, `RemoveMember`, `MoveMember`, `LockRaid`,
//! `SendRaidCreate`, `SendMakeLeaderPacketTo` and `SendRaidDisband`
//! (`zone/raids.cpp`). Every
//! command goes on one opcode and every answer on another, told apart by the
//! action at the start.
use crate::command::EncodedCommand;
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_RaidInvite`: every raid command the player sends.
pub const COMMAND_OPCODE: u16 = 0x5891;
/// `OP_RaidUpdate` (and `OP_RaidJoin`, the same number): every raid update
/// the server sends.
pub const UPDATE_OPCODE: u16 = 0x1f21;

/// A command's length: action, two names and a parameter.
const GENERAL: usize = 136;
/// Each name's field, and where the two lie.
const NAME: usize = 64;
const PLAYER: usize = 4;
const LEADER: usize = 68;
/// Where a command's parameter lies, which an added member's raid group
/// takes.
const PARAMETER: usize = 132;

/// The actions (`EQEmu`'s `RaidCommand*` and `raid*` numbers).
const ADD_MEMBER: u32 = 0;
const ACCEPT: u32 = 1;
const REMOVE: u32 = 1;
const INVITE: u32 = 3;
const DISBAND: u32 = 5;
const MOVE: u32 = 6;
const CREATE: u32 = 8;
const LOCK: u32 = 8;
const UNLOCK: u32 = 9;
const LOCKED: u32 = 17;
const UNLOCKED: u32 = 18;
const INVITED: u32 = 20;
const MAKE_LEADER: u32 = 30;

/// A move's parameter for no raid group: out of every group.
const NO_GROUP: u32 = u32::MAX;

/// A member of the player's raid, as the server added them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RaidMember {
    /// Their name.
    pub name: String,
    /// Their raid group, 0 to 11; None outside every group.
    pub group: Option<u8>,
    /// Their class number.
    pub class: u8,
    /// Their level.
    pub level: u8,
    /// Whether they lead their raid group.
    pub group_leader: bool,
}

/// News of raids: the server's word, and what the session sent for the
/// player, which the server does not answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum RaidUpdate {
    /// The session invited a player to the player's raid.
    Inviting {
        /// Who was invited.
        player: String,
    },
    /// The session accepted the inviter's invitation for the player.
    Accepting {
        /// Who invited the player.
        inviter: String,
    },
    /// The player declined the inviter's invitation; the server keeps no
    /// invitations, so nothing is sent.
    Declining {
        /// Who invited the player.
        inviter: String,
    },
    /// The session asked the server to take the player out of their raid.
    Leaving,
    /// The session asked the server to lock the player's raid (true) or
    /// unlock it, which the server answers with [`RaidUpdate::Locked`]
    /// under the leader's name, as it tells a member joining or entering a
    /// zone while the raid is locked.
    Locking {
        /// Lock (true) or unlock.
        locked: bool,
    },
    /// Someone invited the player to their raid.
    Invited {
        /// Who invited them.
        inviter: String,
    },
    /// The player is in a raid this player leads; its members follow.
    Created {
        /// The raid's leader.
        leader: String,
    },
    /// Someone is in the player's raid: one who joined, or one already in
    /// it as the player joins or enters a zone.
    Added(RaidMember),
    /// Someone left the player's raid or was removed, the player among them.
    Removed {
        /// Who.
        member: String,
    },
    /// The player is out of their raid.
    Disbanded,
    /// The raid has this leader.
    Leader {
        /// The leader.
        name: String,
    },
    /// The raid is locked, so that its leader may move members between
    /// raid groups, or unlocked. The server says so to every member as the
    /// leader locks or unlocks it, naming the leader, and again to one
    /// member, naming them, as they join or enter a zone while it is locked.
    Locked {
        /// Whether it is locked.
        locked: bool,
        /// The name the update gives.
        by: String,
    },
    /// A member moved to another raid group, or out of every group. The
    /// server takes them out of the raid and adds them back in their new
    /// place, which the session reads as this one move.
    Moved(RaidMember),
}

/// The text of a name field: its bytes up to the first NUL.
fn name(body: &[u8], at: usize) -> String {
    let field = &body[at..at + NAME];
    let end = field.iter().position(|byte| *byte == 0).unwrap_or(NAME);
    String::from_utf8_lossy(&field[..end]).into_owned()
}

fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

/// A command: its action, then the two names.
fn command(action: u32, player: &str, leader: &str) -> Result<EncodedCommand> {
    with_parameter(action, player, leader, 0)
}

/// A command with its parameter.
fn with_parameter(
    action: u32,
    player: &str,
    leader: &str,
    parameter: u32,
) -> Result<EncodedCommand> {
    let mut body = vec![0; GENERAL];
    body[..4].copy_from_slice(&action.to_le_bytes());
    body[PARAMETER..].copy_from_slice(&parameter.to_le_bytes());
    for (at, text) in [(PLAYER, player), (LEADER, leader)] {
        ensure!(
            !text.is_empty() && text.len() < NAME,
            "a raid command's name must be 1 to 63 bytes"
        );
        body[at..at + text.len()].copy_from_slice(text.as_bytes());
    }
    Ok(EncodedCommand {
        opcode: COMMAND_OPCODE,
        body,
    })
}

/// Invites a player, by name, to the inviter's raid.
///
/// # Errors
/// Rejects a name its field cannot hold.
pub fn invite(player: &str, inviter: &str) -> Result<EncodedCommand> {
    command(INVITE, player, inviter)
}

/// Accepts the inviter's invitation: the inviter's name first, as the
/// invitation gave it, then the player's own.
///
/// # Errors
/// Rejects a name its field cannot hold.
pub fn accept(inviter: &str, player: &str) -> Result<EncodedCommand> {
    command(ACCEPT, inviter, player)
}

/// Takes a member out of the player's raid: the player themself, to leave.
/// The server finds the member by the exact name it gave them.
///
/// # Errors
/// Rejects a name its field cannot hold.
pub fn remove(player: &str, member: &str) -> Result<EncodedCommand> {
    command(DISBAND, player, member)
}

/// Locks the player's raid, or unlocks it.
///
/// # Errors
/// Rejects a name its field cannot hold.
pub fn lock(player: &str, locked: bool) -> Result<EncodedCommand> {
    command(if locked { LOCK } else { UNLOCK }, player, player)
}

/// Moves a member into a raid group, 0 to 11, or out of every group. The
/// server finds the member by the exact name it gave them.
///
/// # Errors
/// Rejects a name its field cannot hold, and a group past the twelfth.
pub fn move_member(player: &str, member: &str, group: Option<u8>) -> Result<EncodedCommand> {
    let parameter = match group {
        Some(group) => {
            ensure!(group < 12, "a raid has twelve groups");
            u32::from(group)
        }
        None => NO_GROUP,
    };
    with_parameter(MOVE, player, member, parameter)
}

/// Hands the lead of the player's raid to a member.
///
/// # Errors
/// Rejects a name its field cannot hold.
pub fn make_leader(player: &str, member: &str) -> Result<EncodedCommand> {
    command(MAKE_LEADER, player, member)
}

/// Decodes a Titanium raid update; None for any other opcode, and for an
/// update this crate does not read (the zone-in marker, leadership
/// abilities, notes and the message of the day).
///
/// # Errors
/// Rejects an update too short for its fields.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<RaidUpdate>> {
    if opcode != UPDATE_OPCODE {
        return Ok(None);
    }
    ensure!(body.len() >= 4, "truncated raid update");
    let general = || {
        ensure!(body.len() >= GENERAL, "truncated raid update");
        Ok(())
    };
    Ok(match word(body, 0) {
        INVITED => {
            general()?;
            Some(RaidUpdate::Invited {
                inviter: name(body, LEADER),
            })
        }
        CREATE => {
            ensure!(body.len() >= PLAYER + NAME, "truncated raid creation");
            Some(RaidUpdate::Created {
                leader: name(body, PLAYER),
            })
        }
        ADD_MEMBER => {
            ensure!(body.len() >= GENERAL + 3, "truncated raid member");
            let group = word(body, PARAMETER);
            Some(RaidUpdate::Added(RaidMember {
                name: name(body, PLAYER),
                group: u8::try_from(group).ok().filter(|group| *group < 12),
                class: body[GENERAL],
                level: body[GENERAL + 1],
                group_leader: body[GENERAL + 2] != 0,
            }))
        }
        REMOVE => {
            general()?;
            Some(RaidUpdate::Removed {
                member: name(body, PLAYER),
            })
        }
        DISBAND => {
            general()?;
            Some(RaidUpdate::Disbanded)
        }
        MAKE_LEADER => {
            general()?;
            Some(RaidUpdate::Leader {
                name: name(body, PLAYER),
            })
        }
        LOCKED | UNLOCKED => {
            general()?;
            Some(RaidUpdate::Locked {
                locked: word(body, 0) == LOCKED,
                by: name(body, PLAYER),
            })
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name in its field, at a place in a body.
    fn put(body: &mut [u8], at: usize, text: &str) {
        body[at..at + text.len()].copy_from_slice(text.as_bytes());
    }

    /// An update of this action and length, naming the player and a leader.
    fn update(action: u32, length: usize, player: &str, leader: &str) -> Vec<u8> {
        let mut body = vec![0; length];
        body[..4].copy_from_slice(&action.to_le_bytes());
        put(&mut body, PLAYER, player);
        put(&mut body, LEADER, leader);
        body
    }

    #[test]
    fn commands_name_the_player_and_the_one_they_concern() {
        let invited = invite("Friend", "Tester").unwrap();
        assert_eq!((invited.opcode, invited.body.len()), (COMMAND_OPCODE, 136));
        assert_eq!(invited.body[..4], 3u32.to_le_bytes());
        assert_eq!(&invited.body[4..10], b"Friend");
        assert_eq!(&invited.body[68..74], b"Tester");
        let accepted = accept("Leader", "Tester").unwrap();
        assert_eq!(accepted.body[..4], 1u32.to_le_bytes());
        assert_eq!(&accepted.body[4..10], b"Leader");
        assert_eq!(&accepted.body[68..74], b"Tester");
        let left = remove("Tester", "Tester").unwrap();
        assert_eq!(left.body[..4], 5u32.to_le_bytes());
        assert!(invite("", "Tester").is_err());
    }

    #[test]
    fn the_leaders_commands_name_the_member_and_the_group() {
        let locked = lock("Tester", true).unwrap();
        assert_eq!(locked.body[..4], 8u32.to_le_bytes());
        assert_eq!(&locked.body[68..74], b"Tester");
        assert_eq!(lock("Tester", false).unwrap().body[..4], 9u32.to_le_bytes());
        let moved = move_member("Tester", "Friend", Some(3)).unwrap();
        assert_eq!(moved.body[..4], 6u32.to_le_bytes());
        assert_eq!(&moved.body[4..10], b"Tester");
        assert_eq!(&moved.body[68..74], b"Friend");
        assert_eq!(moved.body[132..136], 3u32.to_le_bytes());
        let ungrouped = move_member("Tester", "Friend", None).unwrap();
        assert_eq!(ungrouped.body[132..136], u32::MAX.to_le_bytes());
        assert!(move_member("Tester", "Friend", Some(12)).is_err());
        let led = make_leader("Tester", "Friend").unwrap();
        assert_eq!(led.body[..4], 30u32.to_le_bytes());
        assert_eq!(&led.body[68..74], b"Friend");
        let removed = remove("Tester", "Friend").unwrap();
        assert_eq!(&removed.body[68..74], b"Friend");
    }

    #[test]
    fn updates_read_by_their_action() {
        assert_eq!(
            decode(UPDATE_OPCODE, &update(20, 136, "Tester", "Leader")).unwrap(),
            Some(RaidUpdate::Invited {
                inviter: "Leader".into()
            })
        );
        let mut created = vec![0; 72];
        created[..4].copy_from_slice(&8u32.to_le_bytes());
        put(&mut created, PLAYER, "Leader");
        assert_eq!(
            decode(UPDATE_OPCODE, &created).unwrap(),
            Some(RaidUpdate::Created {
                leader: "Leader".into()
            })
        );
        let mut added = update(0, 140, "Friend", "Friend");
        added[132..136].copy_from_slice(&u32::MAX.to_le_bytes());
        added[136] = 2;
        added[137] = 30;
        assert_eq!(
            decode(UPDATE_OPCODE, &added).unwrap(),
            Some(RaidUpdate::Added(RaidMember {
                name: "Friend".into(),
                group: None,
                class: 2,
                level: 30,
                group_leader: false,
            }))
        );
        added[132..136].copy_from_slice(&3u32.to_le_bytes());
        added[138] = 1;
        assert!(matches!(
            decode(UPDATE_OPCODE, &added).unwrap(),
            Some(RaidUpdate::Added(RaidMember {
                group: Some(3),
                group_leader: true,
                ..
            }))
        ));
        assert_eq!(
            decode(UPDATE_OPCODE, &update(1, 136, "Friend", "Friend")).unwrap(),
            Some(RaidUpdate::Removed {
                member: "Friend".into()
            })
        );
        assert_eq!(
            decode(UPDATE_OPCODE, &update(5, 136, "Tester", "Tester")).unwrap(),
            Some(RaidUpdate::Disbanded)
        );
        assert_eq!(
            decode(UPDATE_OPCODE, &update(30, 388, "Leader", "Leader")).unwrap(),
            Some(RaidUpdate::Leader {
                name: "Leader".into()
            })
        );
        assert_eq!(
            decode(UPDATE_OPCODE, &update(17, 136, "Leader", "Leader")).unwrap(),
            Some(RaidUpdate::Locked {
                locked: true,
                by: "Leader".into()
            })
        );
        assert_eq!(
            decode(UPDATE_OPCODE, &update(18, 136, "Tester", "Tester")).unwrap(),
            Some(RaidUpdate::Locked {
                locked: false,
                by: "Tester".into()
            })
        );
        // The zone-in marker and the leadership abilities are not read.
        assert_eq!(
            decode(UPDATE_OPCODE, &update(10, 136, "Tester", "Tester")).unwrap(),
            None
        );
        assert_eq!(
            decode(UPDATE_OPCODE, &update(14, 388, "Tester", "Tester")).unwrap(),
            None
        );
        assert!(decode(UPDATE_OPCODE, &added[..138]).is_err());
        assert!(decode(UPDATE_OPCODE, &update(20, 100, "Tester", "Leader")).is_err());
        assert_eq!(decode(0x1234, &added).unwrap(), None);
    }
}
