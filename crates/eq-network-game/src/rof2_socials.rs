//! `RoF2`'s rolls, assists, yells for help and effects, as stock `EQEmu`
//! sends them, inferred for `RoF2` (6.541).
//!
//! None of these messages has an encoder for Titanium or `RoF2`
//! (`common/patches/titanium_ops.h`, `rof2_ops.h`), and no sender branches
//! on the client's version, so `RoF2` gets the bytes Titanium gets under its
//! own numbers. Each reads through Titanium's reader into Titanium's event,
//! with Titanium's refusals.
//!
//! Whether the `RoF2` program reads the spell effect's field after the delay
//! and its last bytes, and plays the sound file as Titanium does, is not
//! read (6.541).
use crate::world::WorldEvent;
use anyhow::Result;

/// `OP_RandomReply` (`utils/patches/patch_RoF2.conf`; inferred, 6.541).
pub const ROLL_OPCODE: u16 = 0x106b;
/// `OP_Assist` (`utils/patches/patch_RoF2.conf`; inferred, 6.541).
pub const ASSIST_OPCODE: u16 = 0x4478;
/// `OP_YellForHelp` (`utils/patches/patch_RoF2.conf`; inferred, 6.541).
pub const YELL_OPCODE: u16 = 0x4e56;
/// `OP_LevelAppearance` (`utils/patches/patch_RoF2.conf`; inferred, 6.541).
pub const LEVEL_APPEARANCE_OPCODE: u16 = 0x3bc9;
/// `OP_CameraEffect` (`utils/patches/patch_RoF2.conf`; inferred, 6.541).
pub const CAMERA_EFFECT_OPCODE: u16 = 0x127f;
/// `OP_PlayMP3` (`utils/patches/patch_RoF2.conf`; inferred, 6.541).
pub const PLAY_MP3_OPCODE: u16 = 0x5770;
/// `OP_SpellEffect` (`utils/patches/patch_RoF2.conf`; inferred, 6.541).
pub const SPELL_EFFECT_OPCODE: u16 = 0x5936;

/// What one of this module's messages says; None for another opcode.
///
/// - A roll's result (`Client::Handle_OP_RandomRequest`,
///   `zone/client_packet.cpp`), to close clients.
/// - The answer to an assist (`Client::Handle_OP_Assist`,
///   `Client::Handle_OP_AssistGroup`).
/// - A yell for help (`Client::Handle_OP_YellForHelp`), to close clients.
/// - Particles on a spawn (`Mob::SendLevelAppearance`,
///   `Mob::SendAppearanceEffect`, `zone/mob.cpp`).
/// - A camera shake (`Mob::CameraEffect`, `EntityList::CameraEffect`).
/// - A sound file (`Client::PlayMP3`, `zone/client.cpp`).
/// - A spell's nimbus (`Mob::SendSpellEffect`).
///
/// # Errors
/// Refuses what Titanium's reader of the same message refuses.
pub fn read(opcode: u16, body: &[u8]) -> Result<Option<WorldEvent>> {
    use crate::effects;
    Ok(Some(match opcode {
        ROLL_OPCODE => WorldEvent::Roll(crate::socials::roll(body)?),
        ASSIST_OPCODE => WorldEvent::Assisted(crate::socials::assist_answer(body)?),
        YELL_OPCODE => crate::socials::yell(body)?,
        LEVEL_APPEARANCE_OPCODE => WorldEvent::Effect(effects::titanium_particles(body)?),
        CAMERA_EFFECT_OPCODE => WorldEvent::Effect(effects::titanium_camera_shake(body)?),
        PLAY_MP3_OPCODE => WorldEvent::Effect(effects::titanium_sound_file(body)?),
        SPELL_EFFECT_OPCODE => WorldEvent::Effect(effects::titanium_nimbus(body)?),
        _ => return Ok(None),
    }))
}

#[cfg(test)]
mod tests;
