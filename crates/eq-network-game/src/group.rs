//! Groups: inviting a player, joining or declining an invitation, leaving or
//! disbanding, and the server's word on who is in the player's group.
//!
//! Layout reference: the Titanium opcodes (`utils/patches/patch_Titanium.conf`)
//! and `EQEmu`'s group structs (`GroupInvite_Struct`, `GroupGeneric_Struct`,
//! `GroupCancel_Struct` in `common/patches/titanium_structs.h`, and
//! `GroupJoin_Struct`, `GroupUpdate_Struct` and `GroupUpdate2_Struct` in
//! `common/eq_packet_structs.h`, which the Titanium patch sends unchanged, so
//! they arrive longer than the Titanium client's own); the rules are
//! `Client::Handle_OP_GroupInvite2`, `Handle_OP_GroupFollow2`,
//! `Handle_OP_GroupCancelInvite` and `Handle_OP_GroupDisband`
//! (`zone/client_packet.cpp`) and `Group::AddMember`, `DelMember`,
//! `DisbandGroup` and `SendUpdate` (`zone/groups.cpp`).
use crate::command::EncodedCommand;
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_GroupInvite`: an invitation, either way.
pub const INVITE_OPCODE: u16 = 0x1b48;
/// `OP_GroupInvite2`, which the server also takes as an invitation.
pub const INVITE2_OPCODE: u16 = 0x12d6;
/// `OP_GroupFollow`: joining the inviter's group; the server repeats it to
/// the inviter.
pub const FOLLOW_OPCODE: u16 = 0x7bc7;
/// `OP_GroupCancelInvite`: declining an invitation; the server passes it on
/// to the inviter.
pub const CANCEL_OPCODE: u16 = 0x1f27;
/// `OP_GroupDisband`: leaving, removing a member or disbanding.
pub const DISBAND_OPCODE: u16 = 0x0e76;
/// `OP_GroupUpdate`: the server's word on the group.
pub const UPDATE_OPCODE: u16 = 0x2dd6;

/// The length of each name's field.
const NAME: usize = 64;
/// Where an update's parts lie: its action, the name of the one it is sent
/// to, the member it names (or the five members of a full update) and, in a
/// full update, the leader.
const ACTION: usize = 0;
const MEMBER: usize = 68;
const LEADER: usize = 388;
/// How many other members a full update lists.
const MEMBERS: usize = 5;

/// The update actions (`EQEmu`'s `groupAct*`).
const JOINED: u32 = 0;
const LEFT: u32 = 1;
const DISBANDED: u32 = 6;
const UPDATED: u32 = 7;
const NEW_LEADER: u32 = 8;
const FIRST_INVITE: u32 = 9;

/// News of groups: the server's word, and what the session sent for the
/// player, which the server does not answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum GroupUpdate {
    /// The session invited a player to the player's group.
    Inviting {
        /// Who was invited.
        player: String,
    },
    /// The session joined the inviter's group for the player.
    Following {
        /// Who invited the player.
        inviter: String,
    },
    /// The session declined the inviter's invitation for the player.
    Declining {
        /// Who invited the player.
        inviter: String,
    },
    /// Someone invited the player to their group.
    Invited {
        /// Who invited them.
        inviter: String,
    },
    /// The one the player invited joined, and with them the player formed a
    /// group, which the player leads.
    Formed,
    /// The one the player invited joined.
    Accepted {
        /// Who joined.
        member: String,
    },
    /// The one the player invited declined.
    Declined {
        /// Who declined.
        member: String,
    },
    /// Someone joined the player's group.
    Joined {
        /// Who joined.
        member: String,
    },
    /// Someone left the player's group, or the player did.
    Left {
        /// Who left.
        member: String,
    },
    /// The player's group as the server lists it.
    Members {
        /// Its leader.
        leader: String,
        /// The other members, without the player.
        members: Vec<String>,
    },
    /// The group has a new leader.
    Leader {
        /// The new leader.
        name: String,
    },
    /// The player's group was disbanded.
    Disbanded,
}

/// The text of a name field: its bytes up to the first NUL.
fn name(body: &[u8], at: usize) -> String {
    let field = &body[at..at + NAME];
    let end = field.iter().position(|byte| *byte == 0).unwrap_or(NAME);
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// Two names, each in its own 64-byte field.
fn names(first: &str, second: &str) -> Result<Vec<u8>> {
    let mut body = vec![0; NAME * 2];
    for (at, text) in [(0, first), (NAME, second)] {
        ensure!(
            !text.is_empty() && text.len() < NAME,
            "a group packet's name must be 1 to 63 bytes"
        );
        body[at..at + text.len()].copy_from_slice(text.as_bytes());
    }
    Ok(body)
}

/// Invites a player, by name, to the inviter's group.
///
/// # Errors
/// Rejects a name its field cannot hold.
pub fn invite(player: &str, inviter: &str) -> Result<EncodedCommand> {
    Ok(EncodedCommand {
        opcode: INVITE_OPCODE,
        body: names(player, inviter)?,
    })
}

/// Joins the inviter's group.
///
/// # Errors
/// Rejects a name its field cannot hold.
pub fn follow(inviter: &str, player: &str) -> Result<EncodedCommand> {
    Ok(EncodedCommand {
        opcode: FOLLOW_OPCODE,
        body: names(inviter, player)?,
    })
}

/// Declines the inviter's invitation. The struct's last byte, which the
/// server passes on unread, is sent as 0; what the official client sends
/// there is not checked.
///
/// # Errors
/// Rejects a name its field cannot hold.
pub fn decline(inviter: &str, player: &str) -> Result<EncodedCommand> {
    let mut body = names(inviter, player)?;
    body.push(0);
    Ok(EncodedCommand {
        opcode: CANCEL_OPCODE,
        body,
    })
}

/// Leaves the group, or, as its leader, removes the member the server has
/// as the player's target or disbands the group when there is none: the
/// server decides by its own idea of the target, and the packet names the
/// player in both fields so that, without one, it is the player who leaves.
///
/// # Errors
/// Rejects a name its field cannot hold.
pub fn disband(player: &str) -> Result<EncodedCommand> {
    Ok(EncodedCommand {
        opcode: DISBAND_OPCODE,
        body: names(player, player)?,
    })
}

/// Decodes a Titanium group packet from the server; None for any other
/// opcode, and for an update this crate does not read (the leader's
/// leadership abilities).
///
/// # Errors
/// Rejects a packet too short for its fields.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<GroupUpdate>> {
    Ok(match opcode {
        INVITE_OPCODE | INVITE2_OPCODE => {
            ensure!(body.len() >= NAME * 2, "truncated group invitation");
            Some(GroupUpdate::Invited {
                inviter: name(body, NAME),
            })
        }
        FOLLOW_OPCODE => {
            ensure!(body.len() >= NAME * 2, "truncated group acceptance");
            Some(GroupUpdate::Accepted {
                member: name(body, NAME),
            })
        }
        CANCEL_OPCODE => {
            ensure!(body.len() >= NAME * 2, "truncated group decline");
            Some(GroupUpdate::Declined {
                member: name(body, NAME),
            })
        }
        UPDATE_OPCODE => update(body)?,
        _ => None,
    })
}

/// An `OP_GroupUpdate`, by its action.
fn update(body: &[u8]) -> Result<Option<GroupUpdate>> {
    ensure!(body.len() >= MEMBER + NAME, "truncated group update");
    let action = u32::from_le_bytes([
        body[ACTION],
        body[ACTION + 1],
        body[ACTION + 2],
        body[ACTION + 3],
    ]);
    Ok(match action {
        JOINED => Some(GroupUpdate::Joined {
            member: name(body, MEMBER),
        }),
        LEFT => Some(GroupUpdate::Left {
            member: name(body, MEMBER),
        }),
        NEW_LEADER => Some(GroupUpdate::Leader {
            name: name(body, MEMBER),
        }),
        DISBANDED => Some(GroupUpdate::Disbanded),
        // Sent to the inviter alone, naming them as the member: the first
        // to join formed the group with them.
        FIRST_INVITE => Some(GroupUpdate::Formed),
        UPDATED => {
            ensure!(body.len() >= LEADER + NAME, "truncated group list");
            Some(GroupUpdate::Members {
                leader: name(body, LEADER),
                members: (0..MEMBERS)
                    .map(|place| name(body, MEMBER + place * NAME))
                    .filter(|member| !member.is_empty())
                    .collect(),
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

    #[test]
    fn requests_name_who_they_are_for() {
        let invited = invite("Friend", "Tester").unwrap();
        assert_eq!((invited.opcode, invited.body.len()), (INVITE_OPCODE, 128));
        assert_eq!(&invited.body[..6], b"Friend");
        assert_eq!(&invited.body[64..70], b"Tester");
        let followed = follow("Leader", "Tester").unwrap();
        assert_eq!(
            (followed.opcode, &followed.body[..6]),
            (FOLLOW_OPCODE, &b"Leader"[..])
        );
        assert_eq!(&followed.body[64..70], b"Tester");
        let declined = decline("Leader", "Tester").unwrap();
        assert_eq!((declined.opcode, declined.body.len()), (CANCEL_OPCODE, 129));
        let left = disband("Tester").unwrap();
        assert_eq!(
            (left.opcode, &left.body[..6], &left.body[64..70]),
            (DISBAND_OPCODE, &b"Tester"[..], &b"Tester"[..])
        );
        assert!(invite("", "Tester").is_err());
        assert!(invite(&"x".repeat(64), "Tester").is_err());
    }

    #[test]
    fn the_servers_word_names_the_group() {
        let mut invitation = vec![0; 128];
        put(&mut invitation, 0, "Tester");
        put(&mut invitation, 64, "Leader");
        assert_eq!(
            decode(INVITE_OPCODE, &invitation).unwrap(),
            Some(GroupUpdate::Invited {
                inviter: "Leader".into()
            })
        );
        let mut accepted = vec![0; 128];
        put(&mut accepted, 0, "Tester");
        put(&mut accepted, 64, "Friend");
        assert_eq!(
            decode(FOLLOW_OPCODE, &accepted).unwrap(),
            Some(GroupUpdate::Accepted {
                member: "Friend".into()
            })
        );
        accepted.push(0);
        assert_eq!(
            decode(CANCEL_OPCODE, &accepted).unwrap(),
            Some(GroupUpdate::Declined {
                member: "Friend".into()
            })
        );
        // A join or a leave, as long as the server's own struct.
        let change = |action: u32| {
            let mut body = vec![0; 452];
            body[..4].copy_from_slice(&action.to_le_bytes());
            put(&mut body, 4, "Tester");
            put(&mut body, MEMBER, "Friend");
            decode(UPDATE_OPCODE, &body).unwrap()
        };
        assert_eq!(
            change(JOINED),
            Some(GroupUpdate::Joined {
                member: "Friend".into()
            })
        );
        assert_eq!(
            change(LEFT),
            Some(GroupUpdate::Left {
                member: "Friend".into()
            })
        );
        assert_eq!(
            change(NEW_LEADER),
            Some(GroupUpdate::Leader {
                name: "Friend".into()
            })
        );
        assert_eq!(change(DISBANDED), Some(GroupUpdate::Disbanded));
        // The first to join forms the group with the inviter.
        assert_eq!(change(FIRST_INVITE), Some(GroupUpdate::Formed));
        // The leader's leadership abilities are not read.
        assert_eq!(change(10), None);
        // A full list: the leader and the other members.
        let mut list = vec![0; 836];
        list[..4].copy_from_slice(&UPDATED.to_le_bytes());
        put(&mut list, 4, "Tester");
        put(&mut list, MEMBER, "Leader");
        put(&mut list, MEMBER + NAME, "Friend");
        put(&mut list, LEADER, "Leader");
        assert_eq!(
            decode(UPDATE_OPCODE, &list).unwrap(),
            Some(GroupUpdate::Members {
                leader: "Leader".into(),
                members: vec!["Leader".into(), "Friend".into()],
            })
        );
        assert!(decode(UPDATE_OPCODE, &list[..300]).is_err());
        assert!(decode(INVITE_OPCODE, &invitation[..100]).is_err());
        assert_eq!(decode(0x1234, &invitation).unwrap(), None);
    }
}
