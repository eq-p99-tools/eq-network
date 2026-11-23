//! `RoF2`'s corpse money and loot close, the surname answer, the title list
//! and title changes, as stock `EQEmu` sends them, inferred for `RoF2`
//! (6.552).
//!
//! None of these messages has an encoder for Titanium or `RoF2`
//! (`common/patches/titanium_ops.h`, `rof2_ops.h`), so `RoF2` gets the bytes
//! Titanium gets under its own numbers. Each reads through Titanium's reader
//! into Titanium's event, with Titanium's refusals.
//!
//! One sender branches on the client's version: for `SoD` and later,
//! `Corpse::MakeLootRequestPackets` (`zone/corpse.cpp`) ends the listing
//! with a second corpse money record whose response is 6
//! (`LootResponse::LootAll`, "every item has been sent"), which Titanium
//! never gets. It gives no event here and the coverage row names it as a
//! part left unread, so the gap log tells it; the batch that reads the loot
//! listing decides its event, and lifting validate waits on it.
//!
//! How the `RoF2` program shows that record, and whether it reads the money
//! record's two bytes after the response, is not read (6.552).
use crate::world::WorldEvent;
use anyhow::Result;

/// `OP_MoneyOnCorpse` (`utils/patches/patch_RoF2.conf`; inferred, 6.552).
pub const MONEY_OPCODE: u16 = 0x5f44;
/// `OP_LootComplete` (`utils/patches/patch_RoF2.conf`; inferred, 6.552).
pub const LOOT_COMPLETE_OPCODE: u16 = 0x55c4;
/// `OP_Surname` (`utils/patches/patch_RoF2.conf`; inferred, 6.552).
pub const SURNAME_OPCODE: u16 = 0x0423;
/// `OP_SendTitleList` (`utils/patches/patch_RoF2.conf`; inferred, 6.552).
pub const TITLE_LIST_OPCODE: u16 = 0x2d08;
/// `OP_SetTitleReply` (`utils/patches/patch_RoF2.conf`; inferred, 6.552).
pub const TITLE_OPCODE: u16 = 0x4c21;
/// The corpse money record's response that marks the end of a listing for
/// `SoD` and later (`LootResponse::LootAll`, `zone/common.h`).
pub const LOOT_ALL: u8 = 6;
/// The corpse money record's length (`moneyOnCorpseStruct`).
const MONEY_LENGTH: usize = 20;

/// What one of this module's messages says; None for another opcode, and
/// for the Loot All record.
///
/// - The answer to a loot request, with the coins handed over, or a refusal
///   (`Corpse::MakeLootRequestPackets`, `Corpse::SendLootReqErrorPacket`,
///   `zone/corpse.cpp`).
/// - The loot window closed (`Corpse::EndLoot`,
///   `Corpse::SendEndLootErrorPacket`, `Client::Handle_OP_LootItem`).
/// - The answer to a surname request, the request copied back
///   (`Client::Handle_OP_Surname`, `zone/client_packet.cpp`).
/// - The titles the player may choose (`TitleManager::MakeTitlesPacket`,
///   `zone/titles.cpp`).
/// - A character's new title or suffix (`Client::SetAATitle`,
///   `Client::SetTitleSuffix`), to all clients.
///
/// The last name and the titles reach the host in the event only.
///
/// # Errors
/// Refuses what Titanium's reader of the same message refuses.
pub fn read(opcode: u16, body: &[u8]) -> Result<Option<WorldEvent>> {
    let loot =
        |titanium_opcode| Ok(crate::loot::decode(titanium_opcode, body)?.map(WorldEvent::Loot));
    match opcode {
        MONEY_OPCODE if body.len() == MONEY_LENGTH && body[0] == LOOT_ALL => Ok(None),
        MONEY_OPCODE => loot(crate::loot::MONEY_OPCODE),
        LOOT_COMPLETE_OPCODE => loot(crate::loot::COMPLETE_OPCODE),
        SURNAME_OPCODE => crate::names::surname_answer(body).map(Some),
        TITLE_LIST_OPCODE => crate::names::title_list(body).map(Some),
        TITLE_OPCODE => crate::names::titanium_title(body).map(Some),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests;
