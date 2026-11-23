//! `RoF2`'s targets, charms, cash rewards, linked reuse timers and the
//! answers to begging, pick pocketing and bandaging, as stock `EQEmu` sends
//! them, inferred for `RoF2` (6.544).
//!
//! None of these messages has an encoder for Titanium or `RoF2`
//! (`common/patches/titanium_ops.h`, `rof2_ops.h`), and no sender branches
//! on the client's version, so `RoF2` gets the bytes Titanium gets under its
//! own numbers. Each reads through Titanium's reader into Titanium's event,
//! with Titanium's refusals.
//!
//! Whether the `RoF2` program reads the pick pocket answer's skill word and
//! the begging answer's first three words, and shows the linked reuse timer
//! from its end or its start, is not read (6.544).
use crate::{abilities::Ability, world::WorldEvent};
use anyhow::Result;

/// `OP_TargetCommand` (`utils/patches/patch_RoF2.conf`; inferred, 6.544).
pub const TARGET_OPCODE: u16 = 0x58e2;
/// `OP_TargetHoTT` (`utils/patches/patch_RoF2.conf`; inferred, 6.544).
pub const TARGET_OF_TARGET_OPCODE: u16 = 0x0272;
/// `OP_Charm` (`utils/patches/patch_RoF2.conf`; inferred, 6.544).
pub const CHARM_OPCODE: u16 = 0x5d92;
/// `OP_CashReward` (`utils/patches/patch_RoF2.conf`; inferred, 6.544).
pub const CASH_REWARD_OPCODE: u16 = 0x5f7a;
/// `OP_LinkedReuse` (`utils/patches/patch_RoF2.conf`; inferred, 6.544).
pub const LINKED_REUSE_OPCODE: u16 = 0x1619;
/// `OP_Begging` (`utils/patches/patch_RoF2.conf`; inferred, 6.544).
pub const BEGGING_OPCODE: u16 = 0x6703;
/// `OP_PickPocket` (`utils/patches/patch_RoF2.conf`; inferred, 6.544).
pub const PICK_POCKET_OPCODE: u16 = 0x39e8;
/// `OP_Bind_Wound` (`utils/patches/patch_RoF2.conf`; inferred, 6.544).
pub const BIND_WOUND_OPCODE: u16 = 0x0386;

/// What one of this module's messages says; None for another opcode, and
/// for a begging or pick pocket result Titanium's reader passes over.
///
/// - The target the server sets (`Client::SendTargetCommand`,
///   `zone/client.cpp`) and the target's target (`Client::SetHoTT`).
/// - A charm made or released (the charm arms of `Mob::SpellEffect`,
///   `zone/spell_effects.cpp`).
/// - A cash reward (`Client::CashReward`).
/// - A linked spell's reuse timer (`Client::SetLinkedSpellReuseTimer`,
///   `zone/spells.cpp`).
/// - The answer to a beg (`Client::Handle_OP_Begging`,
///   `zone/client_packet.cpp`) or a pick (`Client::SendPickPocketResponse`).
///   A pick's item name reaches the host in the event only.
/// - The answer about a bandaging (`Client::BindWound`).
///
/// # Errors
/// Refuses what Titanium's reader of the same message refuses.
pub fn read(opcode: u16, body: &[u8]) -> Result<Option<WorldEvent>> {
    Ok(Some(match opcode {
        TARGET_OPCODE => WorldEvent::Target(crate::targets::titanium_target(body)?),
        TARGET_OF_TARGET_OPCODE => {
            WorldEvent::TargetOfTarget(crate::targets::titanium_target(body)?)
        }
        CHARM_OPCODE => WorldEvent::Charm(crate::control::titanium_charm(body)?),
        CASH_REWARD_OPCODE => WorldEvent::Reward(crate::rewards::cash_reward(body)?),
        LINKED_REUSE_OPCODE => {
            WorldEvent::LinkedReuse(crate::disciplines::titanium_linked_reuse(body)?)
        }
        BEGGING_OPCODE => {
            return Ok(
                crate::abilities::ask_answer(Ability::Begging, body)?.map(WorldEvent::AskAnswered)
            );
        }
        PICK_POCKET_OPCODE => {
            return Ok(crate::abilities::ask_answer(Ability::PickPockets, body)?
                .map(WorldEvent::AskAnswered));
        }
        BIND_WOUND_OPCODE => WorldEvent::BindWound(crate::bind_wound::answer(body)?),
        _ => return Ok(None),
    }))
}

#[cfg(test)]
mod tests;
