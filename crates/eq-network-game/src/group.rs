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
//! (`zone/client_packet.cpp`), `Client::GroupFollow` (`zone/client.cpp`),
//! which tells the inviter they formed the group, `Group::AddMember`,
//! `DelMember`, `ChangeLeader` and `DisbandGroup` (`zone/groups.cpp`), and
//! `ZoneDatabase::RefreshGroupFromDB` (`zone/zonedb.cpp`), which sends the
//! full list.
//!
//! The `EQMac` client's packets have the same layouts but their own opcodes
//! (TAKP `utils/patches/patch_Mac.conf`, whose opcodes are listed with their
//! bytes swapped; its `common/patches/mac.cpp` translates none of these): the
//! same structs in TAKP's `common/eq_packet_structs.h` and the same update
//! actions, but for an invitation 65 bytes longer (`GroupInvite_Struct`,
//! 193 bytes, which TAKP's `Handle_OP_GroupInvite2` requires and passes on
//! to the invitee unchanged).
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

/// `EQMac`'s `OP_GroupInvite` (0x203e swapped).
pub const EQMAC_INVITE_OPCODE: u16 = 0x3e20;
/// `EQMac`'s `OP_GroupInvite2` (0x4040), which TAKP passes on as
/// `OP_GroupInvite`.
pub const EQMAC_INVITE2_OPCODE: u16 = 0x4040;
/// `EQMac`'s `OP_GroupFollow` (0x203d swapped).
pub const EQMAC_FOLLOW_OPCODE: u16 = 0x3d20;
/// `EQMac`'s `OP_GroupCancelInvite` (0x4041 swapped).
pub const EQMAC_CANCEL_OPCODE: u16 = 0x4140;
/// `EQMac`'s `OP_GroupDisband` (0x4044 swapped).
pub const EQMAC_DISBAND_OPCODE: u16 = 0x4440;
/// `EQMac`'s `OP_GroupUpdate` (0x2026 swapped).
pub const EQMAC_UPDATE_OPCODE: u16 = 0x2620;
/// How much longer `EQMac`'s invitation is than its two names.
const EQMAC_INVITE_TAIL: usize = 65;

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
/// `EQMac`'s quiet way out of a group (TAKP's `groupActDisband2`), which no
/// `EQEmu` source sends.
const OUT_QUIETLY: u32 = 5;

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

/// The packets the `EQMac` client sends for the same requests, with its own
/// opcodes; its invitation carries 65 more bytes, zero (inferred: what the
/// official client writes there is unrecorded, and TAKP passes it on
/// unread).
///
/// # Errors
/// Rejects a name its field cannot hold.
pub fn eqmac_invite(player: &str, inviter: &str) -> Result<EncodedCommand> {
    let mut body = names(player, inviter)?;
    body.resize(NAME * 2 + EQMAC_INVITE_TAIL, 0);
    Ok(EncodedCommand {
        opcode: EQMAC_INVITE_OPCODE,
        body,
    })
}

/// `EQMac`'s joining of the inviter's group.
///
/// # Errors
/// Rejects a name its field cannot hold.
pub fn eqmac_follow(inviter: &str, player: &str) -> Result<EncodedCommand> {
    Ok(EncodedCommand {
        opcode: EQMAC_FOLLOW_OPCODE,
        ..follow(inviter, player)?
    })
}

/// `EQMac`'s declining of an invitation.
///
/// # Errors
/// Rejects a name its field cannot hold.
pub fn eqmac_decline(inviter: &str, player: &str) -> Result<EncodedCommand> {
    Ok(EncodedCommand {
        opcode: EQMAC_CANCEL_OPCODE,
        ..decline(inviter, player)?
    })
}

/// `EQMac`'s leaving or disbanding, decided by the server as Titanium's is.
///
/// # Errors
/// Rejects a name its field cannot hold.
pub fn eqmac_disband(player: &str) -> Result<EncodedCommand> {
    Ok(EncodedCommand {
        opcode: EQMAC_DISBAND_OPCODE,
        ..disband(player)?
    })
}

/// Decodes an `EQMac` group packet from the server, read as Titanium's of
/// the same layout; None for any other opcode.
///
/// Two things TAKP sends say nothing new: the empty acceptance it sends as
/// each zone admits a player in no group (`zone/client_packet.cpp`
/// `CompleteConnect`), which the profile's empty group places have already
/// said, and the quiet way out naming no one (`eqmac_update`).
///
/// # Errors
/// Rejects a packet too short for its fields.
pub fn decode_eqmac(opcode: u16, body: &[u8]) -> Result<Option<GroupUpdate>> {
    let titanium = match opcode {
        EQMAC_INVITE_OPCODE | EQMAC_INVITE2_OPCODE => INVITE_OPCODE,
        EQMAC_FOLLOW_OPCODE if body.is_empty() => return Ok(None),
        EQMAC_FOLLOW_OPCODE => FOLLOW_OPCODE,
        EQMAC_CANCEL_OPCODE => CANCEL_OPCODE,
        EQMAC_UPDATE_OPCODE => return eqmac_update(body),
        _ => return Ok(None),
    };
    decode(titanium, body)
}

/// An `EQMac` `OP_GroupUpdate`: Titanium's, and a quiet way out
/// (`GroupGeneric_Struct2`: the member, then a string for the client to
/// show). Naming the player, it removes them from their group with the line
/// this crate's `Left` stands for (TAKP `Group::DelMember`, string 12001).
/// Naming no one, it withdraws an invitation as the player answers it
/// (`Client::ClearGroupInvite`), which the session has already done, or
/// ends a group as it becomes part of a raid (`Group::DisbandGroup(true)`),
/// which waits for raids on `EQMac`.
fn eqmac_update(body: &[u8]) -> Result<Option<GroupUpdate>> {
    ensure!(body.len() >= MEMBER + NAME, "truncated group update");
    if action(body) != OUT_QUIETLY {
        return update(body);
    }
    let member = name(body, MEMBER);
    Ok((!member.is_empty()).then_some(GroupUpdate::Left { member }))
}

/// What an update does, from a body long enough to hold it.
fn action(body: &[u8]) -> u32 {
    u32::from_le_bytes([
        body[ACTION],
        body[ACTION + 1],
        body[ACTION + 2],
        body[ACTION + 3],
    ])
}

/// The player's group as the `EQMac` profile lists it, in its six places of
/// 64 bytes, the player among them: the other members, with the leader
/// unknown, or None outside a group. TAKP fills these places as each zone
/// admits a grouped player (`Group::UpdatePlayer`) and names the leader only
/// afterwards (`ZoneDatabase::RefreshGroupLeaderFromDB`).
#[must_use]
pub fn profile_members(places: &[u8], player: &str) -> Option<GroupUpdate> {
    let members: Vec<String> = places
        .as_chunks::<NAME>()
        .0
        .iter()
        .map(|place| name(place, 0))
        .filter(|member| !member.is_empty() && !member.eq_ignore_ascii_case(player))
        .collect();
    (!members.is_empty()).then(|| GroupUpdate::Members {
        leader: String::new(),
        members,
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
    Ok(match action(body) {
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
    fn eqmac_sends_the_same_requests_with_its_own_opcodes() {
        let invited = eqmac_invite("Friend", "Tester").unwrap();
        assert_eq!(
            (invited.opcode, invited.body.len()),
            (EQMAC_INVITE_OPCODE, 193)
        );
        assert_eq!(
            invited.body[..128],
            invite("Friend", "Tester").unwrap().body
        );
        assert!(invited.body[128..].iter().all(|byte| *byte == 0));
        let followed = eqmac_follow("Leader", "Tester").unwrap();
        assert_eq!(
            (followed.opcode, followed.body),
            (
                EQMAC_FOLLOW_OPCODE,
                follow("Leader", "Tester").unwrap().body
            )
        );
        let declined = eqmac_decline("Leader", "Tester").unwrap();
        assert_eq!(
            (declined.opcode, declined.body.len()),
            (EQMAC_CANCEL_OPCODE, 129)
        );
        let left = eqmac_disband("Tester").unwrap();
        assert_eq!(
            (left.opcode, left.body),
            (EQMAC_DISBAND_OPCODE, disband("Tester").unwrap().body)
        );
        assert!(eqmac_invite("", "Tester").is_err());
    }

    #[test]
    fn eqmac_news_reads_as_titaniums() {
        // TAKP passes the inviter's 193 bytes on to the invitee.
        let mut invitation = vec![0; 193];
        put(&mut invitation, 0, "Tester");
        put(&mut invitation, 64, "Leader");
        let invited = Some(GroupUpdate::Invited {
            inviter: "Leader".into(),
        });
        assert_eq!(
            decode_eqmac(EQMAC_INVITE_OPCODE, &invitation).unwrap(),
            invited
        );
        assert_eq!(
            decode_eqmac(EQMAC_INVITE2_OPCODE, &invitation).unwrap(),
            invited
        );
        // The inviter hears they formed the group: GroupJoin_Struct, 388
        // bytes, action 9.
        let mut formed = vec![0; 388];
        formed[..4].copy_from_slice(&9u32.to_le_bytes());
        put(&mut formed, 4, "Leader");
        put(&mut formed, 68, "Leader");
        assert_eq!(
            decode_eqmac(EQMAC_UPDATE_OPCODE, &formed).unwrap(),
            Some(GroupUpdate::Formed)
        );
        // The full list: GroupUpdate_Struct, the leader at 388.
        let mut list = vec![0; 708];
        list[..4].copy_from_slice(&7u32.to_le_bytes());
        put(&mut list, 68, "Friend");
        put(&mut list, 388, "Leader");
        assert_eq!(
            decode_eqmac(EQMAC_UPDATE_OPCODE, &list).unwrap(),
            Some(GroupUpdate::Members {
                leader: "Leader".into(),
                members: vec!["Friend".into()],
            })
        );
        // Titanium's opcodes mean nothing on this wire, nor EQMac's on
        // Titanium's.
        assert_eq!(decode_eqmac(INVITE_OPCODE, &invitation).unwrap(), None);
        assert_eq!(decode(EQMAC_INVITE_OPCODE, &invitation).unwrap(), None);
        assert!(decode_eqmac(EQMAC_UPDATE_OPCODE, &formed[..100]).is_err());
    }

    #[test]
    fn eqmac_has_its_own_ways_out_and_its_own_list() {
        // The quiet way out (GroupGeneric_Struct2, 136 bytes, action 5)
        // naming the player removes them; naming no one says nothing.
        let mut removed = vec![0; 136];
        removed[..4].copy_from_slice(&5u32.to_le_bytes());
        put(&mut removed, 4, "Tester");
        put(&mut removed, 68, "Tester");
        removed[132..].copy_from_slice(&12001u32.to_le_bytes());
        assert_eq!(
            decode_eqmac(EQMAC_UPDATE_OPCODE, &removed).unwrap(),
            Some(GroupUpdate::Left {
                member: "Tester".into(),
            })
        );
        let mut cleared = vec![0; 136];
        cleared[..4].copy_from_slice(&5u32.to_le_bytes());
        assert_eq!(decode_eqmac(EQMAC_UPDATE_OPCODE, &cleared).unwrap(), None);
        assert!(decode_eqmac(EQMAC_UPDATE_OPCODE, &cleared[..100]).is_err());
        // Titanium's servers send no such action.
        assert_eq!(decode(UPDATE_OPCODE, &removed).unwrap(), None);
        // The empty acceptance as a zone admits a player in no group.
        assert_eq!(decode_eqmac(EQMAC_FOLLOW_OPCODE, &[]).unwrap(), None);
        assert!(decode_eqmac(EQMAC_FOLLOW_OPCODE, &[0; 64]).is_err());
        // The profile's six places, the player among them.
        let mut places = vec![0; 384];
        put(&mut places, 0, "Leader");
        put(&mut places, 64, "tester");
        put(&mut places, 192, "Friend");
        assert_eq!(
            profile_members(&places, "Tester"),
            Some(GroupUpdate::Members {
                leader: String::new(),
                members: vec!["Leader".into(), "Friend".into()],
            })
        );
        let mut alone = vec![0; 384];
        put(&mut alone, 0, "Tester");
        assert_eq!(profile_members(&alone, "Tester"), None);
        assert_eq!(profile_members(&[0; 384], "Tester"), None);
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
        // The first to join forms the group with the inviter: a join naming
        // the inviter twice, with their leadership ranks after the names.
        let mut formed = vec![0; 452];
        formed[..4].copy_from_slice(&FIRST_INVITE.to_le_bytes());
        put(&mut formed, 4, "Tester");
        put(&mut formed, MEMBER, "Tester");
        formed[132] = 1;
        assert_eq!(
            decode(UPDATE_OPCODE, &formed).unwrap(),
            Some(GroupUpdate::Formed)
        );
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
