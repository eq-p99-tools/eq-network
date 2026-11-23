//! `RoF2`'s mana and endurance pools, as stock `EQEmu` sends them, inferred
//! for `RoF2` (6.538).
//!
//! None of these messages has a `RoF2` encoder (`common/patches/rof2_ops.h`),
//! so `RoF2` gets the server's own structs. The server sends them to `SoD` and
//! later clients only; Titanium gets none, learning its own pools from
//! `OP_ManaChange` and a group member's from the hit point update.
//!
//! How the `RoF2` program shows a pool above its maximum, and whether it
//! uses the extended-target pool update for its target windows, is not
//! read (6.538).
use crate::world::WorldEvent;
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_ManaUpdate` (`utils/patches/patch_RoF2.conf`; inferred, 6.538).
pub const MANA_OPCODE: u16 = 0x3791;
/// `OP_EnduranceUpdate` (`utils/patches/patch_RoF2.conf`; inferred, 6.538).
pub const ENDURANCE_OPCODE: u16 = 0x5f42;
/// `OP_MobManaUpdate` (`utils/patches/patch_RoF2.conf`; inferred, 6.538).
pub const MANA_PERCENT_OPCODE: u16 = 0x2404;
/// `OP_MobEnduranceUpdate` (`utils/patches/patch_RoF2.conf`; inferred,
/// 6.538).
pub const ENDURANCE_PERCENT_OPCODE: u16 = 0x1c81;

/// Which of a spawn's pools a message gives.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Pool {
    /// Mana.
    Mana,
    /// Endurance.
    Endurance,
}

/// What one of this module's messages says; None for another opcode.
///
/// # Errors
/// Refuses a record its reader refuses.
pub fn read(opcode: u16, body: &[u8]) -> Result<Option<WorldEvent>> {
    Ok(Some(match opcode {
        MANA_OPCODE => pool(Pool::Mana, body)?,
        ENDURANCE_OPCODE => pool(Pool::Endurance, body)?,
        MANA_PERCENT_OPCODE => percent(Pool::Mana, body)?,
        ENDURANCE_PERCENT_OPCODE => percent(Pool::Endurance, body)?,
        _ => return Ok(None),
    }))
}

/// A pool with its maximum: the server's 10-byte `ManaUpdate_Struct` or
/// `EnduranceUpdate_Struct`, the current value, the maximum and the spawn
/// (`Client::SendManaUpdate`, `Client::SendEnduranceUpdate` and
/// `Client::CheckManaEndUpdate` in `zone/client.cpp`, to the player and to
/// clients holding the player on an extended target). Both values are kept
/// as sent: a current above the maximum is not clamped.
///
/// The spawn is the player's own from `SendManaUpdate` and
/// `SendEnduranceUpdate`; `CheckManaEndUpdate`'s extended-target copy gives
/// another player's exact values to those holding them on an extended
/// target, and is handed on as sent. Nothing logs the numbers.
///
/// # Errors
/// Refuses any length but 10 and a spawn of 0, since the server sends its
/// own id.
pub fn pool(pool: Pool, body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 10, "invalid RoF2 pool update length");
    let spawn_id = u16::from_le_bytes([body[8], body[9]]);
    ensure!(spawn_id != 0, "invalid RoF2 pool update spawn ID");
    let word = |at: usize| u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
    Ok(WorldEvent::ResourcePool {
        spawn_id,
        pool,
        current: word(0),
        maximum: word(4),
    })
}

/// A pool as a percent: the server's 3-byte `MobManaUpdate_Struct` or
/// `MobEnduranceUpdate_Struct`, the spawn and the percent
/// (`Group::SendHPManaEndPacketsTo`, `SendManaPacketFrom` and the like in
/// `zone/groups.cpp`, and the raid's own). The percent is the server's
/// own rounding (`GetManaPercent`, `GetEndurancePercent`), and reads as
/// `OP_MobHealth`'s percent does ([`crate::rof2_looks::health`]).
///
/// # Errors
/// Refuses any length but 3, a spawn of 0 and a percent over 100.
pub fn percent(pool: Pool, body: &[u8]) -> Result<WorldEvent> {
    ensure!(
        body.len() == 3 && body[2] <= 100,
        "invalid RoF2 pool percent update"
    );
    let spawn_id = u16::from_le_bytes([body[0], body[1]]);
    ensure!(spawn_id != 0, "invalid RoF2 pool percent spawn ID");
    Ok(WorldEvent::ResourcePercent {
        spawn_id,
        pool,
        percent: body[2],
    })
}

#[cfg(test)]
mod tests;
