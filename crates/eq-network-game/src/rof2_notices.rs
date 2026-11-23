//! `RoF2`'s quest rewards, renames, new titles notices and banker changes,
//! as stock `EQEmu` sends them, inferred for `RoF2` (6.549).
//!
//! None of these messages has an encoder for Titanium or `RoF2`
//! (`common/patches/titanium_ops.h`, `rof2_ops.h`), and no sender branches
//! on the client's version, so `RoF2` gets the bytes Titanium gets under its
//! own numbers. Each reads through Titanium's reader into Titanium's event,
//! with Titanium's refusals.
//!
//! Whether the `RoF2` program reads the reward's faction words and the
//! rename's two words is not read (6.549).
use crate::world::WorldEvent;
use anyhow::{ensure, Result};

/// `OP_Sound` (`utils/patches/patch_RoF2.conf`; inferred, 6.549).
pub const SOUND_OPCODE: u16 = 0x1a30;
/// `OP_MobRename` (`utils/patches/patch_RoF2.conf`; inferred, 6.549).
pub const RENAME_OPCODE: u16 = 0x2c57;
/// `OP_NewTitlesAvailable` (`utils/patches/patch_RoF2.conf`; inferred,
/// 6.549).
pub const NEW_TITLES_OPCODE: u16 = 0x0d32;
/// `OP_BankerChange` (`utils/patches/patch_RoF2.conf`; inferred, 6.549).
pub const BANKER_CHANGE_OPCODE: u16 = 0x791e;

/// What one of this module's messages says; None for another opcode.
///
/// - A quest's reward (`Client::QuestReward`, `zone/client.cpp`) or the
///   fixed sound record (`Client::SendSound`).
/// - A spawn's new name (`Mob::SendRename`, `zone/mob.cpp`), to all
///   clients. The names reach the host in the event only.
/// - New titles to choose from (`Client::NotifyNewTitlesAvailable`).
/// - The banker's change (`Client::Handle_OP_BankerChange`,
///   `zone/client_packet.cpp`).
///
/// # Errors
/// Refuses what Titanium's reader of the same message refuses.
pub fn read(opcode: u16, body: &[u8]) -> Result<Option<WorldEvent>> {
    Ok(Some(match opcode {
        SOUND_OPCODE => match crate::rewards::titanium_sound(body)? {
            crate::rewards::Sound::Reward(reward) => WorldEvent::Reward(reward),
            crate::rewards::Sound::Ding => WorldEvent::Ding,
        },
        RENAME_OPCODE => {
            let (old_name, new_name) = crate::names::titanium_rename(body)?;
            WorldEvent::Renamed {
                spawn_id: None,
                old_name,
                new_name,
            }
        }
        NEW_TITLES_OPCODE => {
            ensure!(body.is_empty(), "invalid new titles length");
            WorldEvent::NewTitles
        }
        BANKER_CHANGE_OPCODE => WorldEvent::CoinsChanged(crate::banker::decode(body)?),
        _ => return Ok(None),
    }))
}

#[cfg(test)]
mod tests;
