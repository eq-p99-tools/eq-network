//! The `RoF2` client's messages, as stock `EQEmu` sends them: only those a
//! `RoF2` stage reads (6.508). Every other message stays unlisted, which is
//! true of `RoF2` today, and gets a row with the batch that reads it.
//!
//! The login rows come from `loginserver/login_util/login_opcodes_sod.conf`
//! and the login stage (6.497); the world rows from
//! `utils/patches/patch_RoF2.conf` and the world stage (6.499); the zone
//! rows from the same list, the entry (6.501, 6.502) and the zone decoder
//! (6.510, 6.514, 6.518).

use super::{
    sent, Part,
    Reader::{Admission, Login, World, Zone},
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
/// What the entry and the zone decoder leave unread of a spawn record:
/// the player's own, which the entry reads (6.502), and every other
/// spawn, which stock `EQEmu` sends under the same number
/// (`ENCODE(OP_NewSpawn)` and `ENCODE(OP_ZoneSpawns)` both write
/// `OP_ZoneEntry` records) and the zone decoder reads (6.510).
const SPAWN: &[Part] = &[
    Part("appearance and the /who listing", Planned(Rof2)),
    Part(
        "the motion deltas, the animation and the angle of the packed position",
        Planned(Rof2),
    ),
];

/// What the zone decoder leaves unread of a position update (6.514).
const POSITION: &[Part] = &[Part("the delta heading and the vehicle id", Planned(Rof2))];

/// What the zone decoder leaves unread of a wear change (6.518).
const WEAR_CHANGE: &[Part] = &[Part(
    "the elite material, the hero's forge model and two unknown words",
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
        // The zone's, during the entry (6.501, 6.502), and its spawns
        // (6.510).
        0x1795 => sent("OP_NewZone", ROF2, Partly(&[Admission], DESCRIPTION)),
        0x6506 => sent("OP_PlayerProfile", ROF2, Partly(&[Admission], PROFILE)),
        0x5089 => sent("OP_ZoneEntry", ROF2, Partly(&[Admission, Zone], SPAWN)),
        0x5ae2 => sent("OP_WorldObjectsSent", ROF2, Read(&[Admission])),
        // Its spawns' movement and removal (6.514). The removal's last byte
        // is not a part: `ENCODE(OP_DeleteSpawn)` always writes 1 there.
        // `OP_MobUpdate` (0x2c84) and `OP_NPCMoveUpdate` (0x5892) stay
        // unlisted: stock `EQEmu` sends neither (`common/emu_oplist.h` keeps
        // `OP_MobUpdate` "not used anymore", and nothing in `zone/` sends
        // either), so the gap log tells one if it ever comes.
        0x7dfc => sent("OP_ClientUpdate", ROF2, Partly(&[Zone], POSITION)),
        0x7280 => sent("OP_DeleteSpawn", ROF2, Read(&[Zone])),
        // How they look and how hurt they are (6.518). `OP_ChangeSize` is
        // `ENCODE(OP_SpawnAppearance)`'s rewrite of a size appearance; its
        // last eight bytes are not a part, since the encoder always writes
        // 0 and 1.0 there. `OP_Illusion`, `OP_ManaChange`, `OP_Death`,
        // `OP_Damage` and `OP_Consider` stay unlisted, so the gap log tells
        // them.
        0x0971 => sent("OP_SpawnAppearance", ROF2, Read(&[Zone])),
        0x4707 => sent("OP_ChangeSize", ROF2, Read(&[Zone])),
        0x2828 => sent("OP_HPUpdate", ROF2, Read(&[Zone])),
        0x37b1 => sent("OP_MobHealth", ROF2, Read(&[Zone])),
        0x7994 => sent("OP_WearChange", ROF2, Partly(&[Zone], WEAR_CHANGE)),
        0x7177 => sent("OP_Animation", ROF2, Read(&[Zone])),
        0x559a => sent("OP_PlayerStateAdd", ROF2, Read(&[Zone])),
        0x2d25 => sent("OP_PlayerStateRemove", ROF2, Read(&[Zone])),
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
            .chain(eq_network_game::message::ROF2_READS)
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
