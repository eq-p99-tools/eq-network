//! The `RoF2` client's messages, as stock `EQEmu` sends them: only those a
//! `RoF2` stage reads (6.508). Every other message stays unlisted, which is
//! true of `RoF2` today, and gets a row with the batch that reads it.
//!
//! The login rows come from `loginserver/login_util/login_opcodes_sod.conf`
//! and the login stage (6.497); the world rows from
//! `utils/patches/patch_RoF2.conf` and the world stage (6.499); the zone
//! rows from the same list, the entry (6.501, 6.502) and the zone decoder
//! (6.510, 6.514, 6.518, 6.519, 6.522, 6.527, 6.532, 6.538, 6.539, 6.541,
//! 6.544, 6.549), and the chat rows from the chat parser (6.521).

use super::{
    sent, Part,
    Reader::{Admission, Chat, Login, World, Zone},
    Reading::{Partly, Read, Unread},
    Reason::{Needless, Planned},
    ServerMessage,
    Step::Rof2,
    NIMBUS, RENAME, REWARD, ROF2,
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

/// What the zone decoder leaves unread of a mana change (6.519).
const MANA_CHANGE: &[Part] = &[Part(
    "the slot, which Titanium's encoder drops",
    Planned(Rof2),
)];

/// What the zone decoder leaves unread of a damage record (6.519).
const DAMAGE: &[Part] = &[Part(
    "the off-hand flag, which the encoder never writes, and the special word (rampage, wild \
     rampage)",
    Planned(Rof2),
)];

/// What the zone decoder leaves unread of a wear change (6.518).
const WEAR_CHANGE: &[Part] = &[Part(
    "the elite material, the hero's forge model and two unknown words",
    Planned(Rof2),
)];

/// What the zone decoder leaves unread of an illusion (6.522).
const ILLUSION: &[Part] = &[Part(
    "the Drakkin heritage, tattoo and details",
    Planned(Rof2),
)];
/// What the zone decoder leaves unread of a face change (6.522).
const FACE_CHANGE: &[Part] = &[Part(
    "the Drakkin heritage, tattoo and details",
    Planned(Rof2),
)];
/// What the zone decoder leaves unread of a buff update (6.527).
const BUFF: &[Part] = &[Part(
    "the counters, which the encoder does not send, the hit count, the location and the slot \
     data",
    Planned(Rof2),
)];
/// What the zone decoder leaves unread of a ground object (6.539).
const GROUND_OBJECT: &[Part] = &[Part(
    "the zone, the instance, the two tilts and the size, which Titanium's ground object has no \
     field for",
    Planned(Rof2),
)];
/// What the session leaves unread of the weather (6.539).
const WEATHER: &[Part] = &[Part(
    "the 12-byte form sent at zone-in, which Titanium's admission takes as a step",
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
        // 0 and 1.0 there. `OP_Illusion` stays unlisted, so the gap log
        // tells it.
        0x0971 => sent("OP_SpawnAppearance", ROF2, Read(&[Zone])),
        0x4707 => sent("OP_ChangeSize", ROF2, Read(&[Zone])),
        0x2828 => sent("OP_HPUpdate", ROF2, Read(&[Zone])),
        0x37b1 => sent("OP_MobHealth", ROF2, Read(&[Zone])),
        0x7994 => sent("OP_WearChange", ROF2, Partly(&[Zone], WEAR_CHANGE)),
        0x7177 => sent("OP_Animation", ROF2, Read(&[Zone])),
        0x559a => sent("OP_PlayerStateAdd", ROF2, Read(&[Zone])),
        0x2d25 => sent("OP_PlayerStateRemove", ROF2, Read(&[Zone])),
        // The player's own news and the combat records (6.519). The first
        // six go out unencoded, the bytes Titanium gets. The skill
        // update's last four bytes are not a part: `ENCODE(OP_SkillUpdate)`
        // always writes 1, 80, 136 and 54 there. `OP_ManaUpdate` (0x3791)
        // and `OP_EnduranceUpdate` (0x5f42), which stock `EQEmu` sends only
        // to SoD and later with the maxima, stay unlisted until a batch
        // reads them, as do the group and raid percents.
        // Stock `EQEmu` sends its own `Death_Struct`, whose bind zone and
        // attack skill sit the other way round from where the programs'
        // structs declare them, so the bind zone read at word 20 carries
        // the damage type and word 12 the bind zone (`zoning::death`).
        0x6517 => sent("OP_Death", ROF2, Read(&[Zone])),
        0x1eec => sent("OP_LevelUpdate", ROF2, Read(&[Zone])),
        0x20ed => sent("OP_ExpUpdate", ROF2, Read(&[Zone])),
        0x2a79 => sent("OP_Stamina", ROF2, Read(&[Zone])),
        0x640c => sent("OP_MoneyUpdate", ROF2, Read(&[Zone])),
        0x5070 => sent("OP_TimeOfDay", ROF2, Read(&[Zone])),
        0x5467 => sent("OP_ManaChange", ROF2, Partly(&[Zone], MANA_CHANGE)),
        0x6f15 => sent("OP_Damage", ROF2, Partly(&[Zone], DAMAGE)),
        0x742b => sent("OP_Consider", ROF2, Read(&[Zone])),
        0x004c => sent("OP_SkillUpdate", ROF2, Read(&[Zone])),
        // Casting, stuns and illusions (6.522). The action's zero words,
        // gem byte, slot constants and item-cast type, and the stun's four
        // constant bytes, are not parts: the encoder writes them, and they
        // carry nothing from the server. The action's modifier reads the
        // server's 0 to 9 all as 0, since the encoder's unsigned arithmetic
        // wraps them alike. The illusion's name, its two bytes at 70 (which
        // Titanium has no field for), its zero word at 240 and its constant
        // at 316 are not read either.
        0x318f => sent("OP_BeginCast", ROF2, Read(&[Zone])),
        0x744c => sent("OP_Action", ROF2, Read(&[Zone])),
        0x048c => sent("OP_InterruptCast", ROF2, Read(&[Zone])),
        0x36a4 => sent("OP_Stun", ROF2, Read(&[Zone])),
        0x312a => sent("OP_Illusion", ROF2, Partly(&[Zone], ILLUSION)),
        0x1af3 => sent("OP_SetFace", ROF2, Partly(&[Zone], FACE_CHANGE)),
        // Buffs (6.527). The icon messages' and the pet window's zero
        // words are not parts; they carry nothing from the server.
        0x659c => sent("OP_Buff", ROF2, Partly(&[Zone], BUFF)),
        0x3377 => sent("OP_BuffCreate", ROF2, Read(&[Zone])),
        0x4f4b => sent("OP_TargetBuffs", ROF2, Read(&[Zone])),
        0x5882 => sent("OP_PetBuffWindow", ROF2, Read(&[Zone])),
        // The spell book and gems (6.532), unencoded: the server's own
        // structs, with a 720-slot book. The deletion's padding is the
        // client's own request echoed, so it is not a part.
        0x217c => sent("OP_MemorizeSpell", ROF2, Read(&[Zone])),
        0x0efa => sent("OP_SwapSpell", ROF2, Read(&[Zone])),
        0x3358 => sent("OP_DeleteSpell", ROF2, Read(&[Zone])),
        // The mana and endurance pools (6.538), unencoded and sent to SoD
        // and later only.
        0x3791 => sent("OP_ManaUpdate", ROF2, Read(&[Zone])),
        0x5f42 => sent("OP_EnduranceUpdate", ROF2, Read(&[Zone])),
        0x2404 => sent("OP_MobManaUpdate", ROF2, Read(&[Zone])),
        0x1c81 => sent("OP_MobEnduranceUpdate", ROF2, Read(&[Zone])),
        // Doors, ground objects and the weather (6.539). The door record's
        // four bytes at 56 and everything from 68 are not parts: the
        // encoder writes bytes 80 to 86 only, as 0, 1, 0, 1, 0, 0 and 0,
        // and byte 87 and the 12 after it are not written. The ground
        // object's 52-byte tail past its object type is not written either.
        // The door's move, the click and the weather go out unencoded, the
        // bytes Titanium gets.
        0x7291 => sent("OP_SpawnDoor", ROF2, Read(&[Zone])),
        0x08e8 => sent("OP_MoveDoor", ROF2, Read(&[Zone])),
        0x6fca => sent("OP_GroundSpawn", ROF2, Partly(&[Zone], GROUND_OBJECT)),
        0x4aa1 => sent("OP_ClickObject", ROF2, Read(&[Zone])),
        0x661e => sent("OP_Weather", ROF2, Partly(&[Zone], WEATHER)),
        // Rolls, assists, yells for help and effects (6.541), unencoded for
        // both generations, the bytes Titanium gets. The spell effect's
        // unread field and bytes are Titanium's part.
        0x106b => sent("OP_RandomReply", ROF2, Read(&[Zone])),
        0x4478 => sent("OP_Assist", ROF2, Read(&[Zone])),
        0x4e56 => sent("OP_YellForHelp", ROF2, Read(&[Zone])),
        0x3bc9 => sent("OP_LevelAppearance", ROF2, Read(&[Zone])),
        0x127f => sent("OP_CameraEffect", ROF2, Read(&[Zone])),
        0x5770 => sent("OP_PlayMP3", ROF2, Read(&[Zone])),
        0x5936 => sent("OP_SpellEffect", ROF2, Partly(&[Zone], NIMBUS)),
        // Targets, charms, cash rewards, linked reuse and the answers to
        // begging, pick pocketing and bandaging (6.544), unencoded for both
        // generations, the bytes Titanium gets.
        0x58e2 => sent("OP_TargetCommand", ROF2, Read(&[Zone])),
        0x0272 => sent("OP_TargetHoTT", ROF2, Read(&[Zone])),
        0x5d92 => sent("OP_Charm", ROF2, Read(&[Zone])),
        0x5f7a => sent("OP_CashReward", ROF2, Read(&[Zone])),
        0x1619 => sent("OP_LinkedReuse", ROF2, Read(&[Zone])),
        0x6703 => sent("OP_Begging", ROF2, Read(&[Zone])),
        0x39e8 => sent("OP_PickPocket", ROF2, Read(&[Zone])),
        0x0386 => sent("OP_Bind_Wound", ROF2, Read(&[Zone])),
        // Quest rewards and their sound, renames, the new titles notice and
        // the banker's change (6.549), unencoded for both generations, the
        // bytes Titanium gets, with Titanium's parts.
        0x1a30 => sent("OP_Sound", ROF2, Partly(&[Zone], REWARD)),
        0x2c57 => sent("OP_MobRename", ROF2, Partly(&[Zone], RENAME)),
        0x0d32 => sent("OP_NewTitlesAvailable", ROF2, Read(&[Zone])),
        0x791e => sent("OP_BankerChange", ROF2, Read(&[Zone])),
        // Its chat and the server's own lines (6.521). The channel
        // message's two words and the byte the encoder writes as 0, and its
        // 15-byte zero tail, are not parts: they carry nothing from the
        // server. The others keep Titanium's shape; the guild's message of
        // the day has no encoder for either generation.
        0x0c22 => sent("OP_MOTD", ROF2, Read(&[Chat])),
        0x2b2d => sent("OP_ChannelMessage", ROF2, Read(&[Chat])),
        0x373b => sent("OP_Emote", ROF2, Read(&[Chat])),
        0x0083 => sent("OP_SpecialMesg", ROF2, Read(&[Chat])),
        0x1024 => sent("OP_FormattedMessage", ROF2, Read(&[Chat])),
        0x213f => sent("OP_SimpleMessage", ROF2, Read(&[Chat])),
        0x3e13 => sent("OP_GuildMOTD", ROF2, Read(&[Chat])),
        0x4f1f => sent("OP_GetGuildMOTDReply", ROF2, Read(&[Chat])),
        0x43af => sent("OP_ColoredText", ROF2, Read(&[Chat])),
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
            .chain(eq_network_game::chat::ROF2_READS)
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
