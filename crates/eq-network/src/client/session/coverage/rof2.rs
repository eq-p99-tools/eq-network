//! The `RoF2` client's messages, as stock `EQEmu` sends them: only those a
//! `RoF2` stage reads (6.508). Every other message stays unlisted, which is
//! true of `RoF2` today, and gets a row with the batch that reads it.
//!
//! The login rows come from `loginserver/login_util/login_opcodes_sod.conf`
//! and the login stage (6.497); the world rows from
//! `utils/patches/patch_RoF2.conf` and the world stage (6.499); the zone
//! rows from the same list and the entry (6.501, 6.502).

use super::{
    sent, Part,
    Reader::{Admission, Login, World},
    Reading::{Partly, Read, Unread},
    Reason::{Needless, Planned},
    ServerMessage,
    Step::Rof2,
    ROF2,
};

/// What the entry leaves unread of `RoF2`'s zone description (6.501).
const DESCRIPTION: &[Part] = &[
    Part(
        "the far clip, the sky and the rest of the description",
        Planned(Rof2),
    ),
    Part(
        "the character's name",
        Needless("the session has it from its entry"),
    ),
];
/// What the entry leaves unread of `RoF2`'s profile (6.502).
const PROFILE: &[Part] = &[Part(
    "gems 9 to 12, the spellbook, coins, buffs, alternate advancement, bandoliers and the \
     rest of the profile",
    Planned(Rof2),
)];
/// What the entry leaves unread of the player's own spawn (6.502).
const OWN_SPAWN: &[Part] = &[Part(
    "appearance, the /who listing and the packed position",
    Planned(Rof2),
)];

/// What the session does with a message stock `EQEmu` sends the `RoF2`
/// client; nothing for one no stage reads yet.
pub(in crate::client) fn rof2(opcode: u16) -> Option<ServerMessage> {
    Some(match opcode {
        // The login server's, for SoD and later clients (6.497).
        0x0017 => sent("OP_ChatMessage", ROF2, Read(&[Login])),
        0x0018 => sent("OP_LoginAccepted", ROF2, Read(&[Login])),
        0x0019 => sent("OP_ServerListResponse", ROF2, Read(&[Login])),
        0x0022 => sent("OP_PlayEverquestResponse", ROF2, Read(&[Login])),
        0x0031 => sent(
            "OP_LoginExpansionPacketData",
            ROF2,
            Unread(Needless(
                "the login needs nothing from it to play (loginserver/client.cpp \
                 Client::SendExpansionPacketData)",
            )),
        ),
        // The world's (6.499).
        0x00d2 => sent("OP_SendCharInfo", ROF2, Read(&[World])),
        0x578f => sent("OP_EnterWorld", ROF2, Read(&[World])),
        0x6259 => sent("OP_PostEnterWorld", ROF2, Read(&[World])),
        0x4c44 => sent("OP_ZoneServerInfo", ROF2, Read(&[World])),
        0x4cb4 => sent("OP_ZoneUnavail", ROF2, Read(&[World])),
        // The zone's, during the entry (6.501, 6.502).
        0x1795 => sent("OP_NewZone", ROF2, Partly(&[Admission], DESCRIPTION)),
        0x6506 => sent("OP_PlayerProfile", ROF2, Partly(&[Admission], PROFILE)),
        0x5089 => sent("OP_ZoneEntry", ROF2, Partly(&[Admission], OWN_SPAWN)),
        0x5ae2 => sent("OP_WorldObjectsSent", ROF2, Read(&[Admission])),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::session::{admission, login, world};
    use std::collections::BTreeSet;

    #[test]
    fn rof2s_table_lists_exactly_what_its_stages_read() {
        let (login, skipped) = login::rof2_reads();
        let read: BTreeSet<u16> = login
            .into_iter()
            .chain(world::rof2::READS)
            .chain(admission::ROF2_READS)
            .collect();
        let listed: BTreeSet<u16> = (0..=u16::MAX)
            .filter(|opcode| {
                rof2(*opcode).is_some_and(|message| !matches!(message.reading, Unread(_)))
            })
            .collect();
        // Every opcode a stage reads has a row, and every row read is read.
        assert_eq!(listed, read);
        // The one unread row is the expansion list the login skips.
        let unread: Vec<u16> = (0..=u16::MAX)
            .filter(|opcode| {
                rof2(*opcode).is_some_and(|message| matches!(message.reading, Unread(_)))
            })
            .collect();
        assert_eq!(unread, skipped.into_iter().collect::<Vec<_>>());
    }
}
