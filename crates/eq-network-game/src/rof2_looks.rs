//! How `RoF2`'s spawns look and how hurt they are, as stock `EQEmu` sends
//! them, inferred for `RoF2` (6.518).
//!
//! Each message is a fixed record (`common/patches/rof2_structs.h` and
//! `common/eq_packet_structs.h`, both packed to the byte). Some go out as
//! the server builds them, with no `RoF2` encoder: the health percent and
//! the player states. Others pass through an encoder of their own in
//! `common/patches/rof2.cpp`: `ENCODE(OP_SpawnAppearance)` passes every
//! kind through but the size, which it rewrites as `OP_ChangeSize`;
//! `ENCODE(OP_HPUpdate)` puts the spawn id first; `ENCODE(OP_WearChange)`
//! keeps the material's 32 bits where Titanium's cuts them to 16; and
//! `ENCODE(OP_Animation)` writes the action before the speed.
//!
//! The values are the server's, the same for every generation, so each
//! reader gives the event Titanium's reader gives for the same values and
//! keeps its refusals; where one differs, its doc says why. What the
//! `RoF2` program does with each value is not read (6.518): the
//! invisibility values, the appearance kinds, the wear change's elite
//! material and hero's forge model, the size message's two constants and
//! the animation speed's scale.
use crate::appearance::{AppearanceChange, TextureSlot, WearChange};
use crate::combat::Animation;
use crate::world::{ItemHitPoints, WorldEvent};
use anyhow::{ensure, Context, Result};

/// `OP_SpawnAppearance` (`utils/patches/patch_RoF2.conf`; inferred, 6.518).
pub const APPEARANCE_OPCODE: u16 = 0x0971;
/// `OP_ChangeSize`, the encoder's rewrite of a size appearance
/// (`utils/patches/patch_RoF2.conf`; inferred, 6.518).
pub const SIZE_OPCODE: u16 = 0x4707;
/// `OP_HPUpdate`, the player's own hit points
/// (`utils/patches/patch_RoF2.conf`; inferred, 6.518).
pub const HIT_POINTS_OPCODE: u16 = 0x2828;
/// `OP_MobHealth`, a spawn's health percent
/// (`utils/patches/patch_RoF2.conf`; inferred, 6.518).
pub const HEALTH_OPCODE: u16 = 0x37b1;
/// `OP_WearChange` (`utils/patches/patch_RoF2.conf`; inferred, 6.518).
pub const WEAR_CHANGE_OPCODE: u16 = 0x7994;
/// `OP_Animation` (`utils/patches/patch_RoF2.conf`; inferred, 6.518).
pub const ANIMATION_OPCODE: u16 = 0x7177;
/// `OP_PlayerStateAdd` (`utils/patches/patch_RoF2.conf`; inferred, 6.518).
pub const STATES_ADDED_OPCODE: u16 = 0x559a;
/// `OP_PlayerStateRemove` (`utils/patches/patch_RoF2.conf`; inferred,
/// 6.518).
pub const STATES_REMOVED_OPCODE: u16 = 0x2d25;

/// What one of this module's messages says; None for another opcode, and
/// for an appearance naming no spawn, which says nothing of a look (see
/// [`appearance`]).
///
/// # Errors
/// Refuses a record its reader refuses.
pub fn read(opcode: u16, body: &[u8]) -> Result<Option<WorldEvent>> {
    Ok(Some(match opcode {
        APPEARANCE_OPCODE => return appearance(body),
        SIZE_OPCODE => size(body)?,
        HIT_POINTS_OPCODE => hit_points(body)?,
        HEALTH_OPCODE => health(body)?,
        WEAR_CHANGE_OPCODE => WorldEvent::WearChange(wear_change(body)?),
        ANIMATION_OPCODE => WorldEvent::Animation(animation(body)?),
        STATES_ADDED_OPCODE | STATES_REMOVED_OPCODE => {
            crate::appearance::states(opcode, [STATES_ADDED_OPCODE, STATES_REMOVED_OPCODE], body)?
        }
        _ => return Ok(None),
    }))
}

/// A spawn's appearance: the server's 8-byte `SpawnAppearance_Struct`
/// (`Mob::SendAppearancePacket` in `zone/mob.cpp`), which
/// `ENCODE(OP_SpawnAppearance)` passes through as it is for every kind but
/// the size. It is read by the reader every generation shares, with
/// `RoF2`'s dialect: the invisibility values as Titanium's (inferred,
/// 6.518), and kind 20 through the `/who` listing, as on every generation.
/// A record naming no spawn says nothing of a look and gives None, as
/// Titanium's does.
///
/// # Errors
/// Refuses what the shared reader refuses: a wrong length, a value out of
/// its kind's range, and a listing, visibility, posture or pet update
/// naming no spawn.
pub fn appearance(body: &[u8]) -> Result<Option<WorldEvent>> {
    crate::appearance::spawn_update(body, crate::GameDialect::Rof2)
}

/// A spawn's new size: `ENCODE(OP_SpawnAppearance)` rewrites a size
/// appearance as this 16-byte `ChangeSize_Struct`, the entity in 32 bits,
/// the server's whole size made a float, then a word it always writes as 0
/// and a float it always writes as 1.0, which are not read.
///
/// It differs from Titanium's size appearance in two ways. An entity of
/// zero refuses, where Titanium's record naming no spawn gives nothing:
/// `Mob::SendAppearancePacket` returns before sending anything for a mob
/// without an id, so no zero comes from the server. And the size is a
/// float, so a fraction keeps its fraction; the range is Titanium's, zero
/// to 65535.
///
/// # Errors
/// Refuses any length but 16, an entity of zero or past 16 bits, and a
/// size that is negative, past 65535 or not a number.
pub fn size(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 16, "invalid RoF2 size change length");
    let spawn_id = u16::try_from(word(body, 0))
        .ok()
        .filter(|id| *id != 0)
        .context("invalid RoF2 size change spawn ID")?;
    let size = f32::from_bits(word(body, 4));
    ensure!(
        size.is_finite() && (0.0..=f32::from(u16::MAX)).contains(&size),
        "invalid RoF2 size change size"
    );
    Ok(WorldEvent::Appearance {
        spawn_id,
        change: AppearanceChange::Size(size),
    })
}

/// The player's own hit points: `ENCODE(OP_HPUpdate)` writes the server's
/// `SpawnHPUpdate_Struct` as the spawn id in 16 bits, then the current and
/// the maximum in 32 bits each, 10 bytes. `Mob::SendHPUpdate` sends it to
/// the player alone, with both values less what equipped items add, the
/// current cast to unsigned: it is read signed, as Titanium's is, since it
/// is negative while dying and while below that item bonus.
///
/// # Errors
/// Refuses any length but 10 and a negative maximum.
pub fn hit_points(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 10, "invalid RoF2 hit-point update length");
    let maximum = word(body, 6).cast_signed();
    ensure!(maximum >= 0, "negative RoF2 maximum hit points");
    Ok(WorldEvent::HitPoints {
        spawn_id: u16::from_le_bytes([body[0], body[1]]),
        current: word(body, 2).cast_signed(),
        maximum,
        items: ItemHitPoints::LeftOut,
    })
}

/// A spawn's health percent: the server's 3-byte `SpawnHPUpdate_Struct2`
/// (`Mob::CreateHPPacket`), with no `RoF2` encoder: the spawn id in 16
/// bits and the percent.
///
/// # Errors
/// Refuses any length but 3 and a percent past 100.
pub fn health(body: &[u8]) -> Result<WorldEvent> {
    ensure!(
        body.len() == 3 && body[2] <= 100,
        "invalid RoF2 health update"
    );
    Ok(WorldEvent::HealthPercent {
        spawn_id: u16::from_le_bytes([body[0], body[1]]),
        percent: body[2],
    })
}

/// One slot of a spawn's gear: `ENCODE(OP_WearChange)` writes the 27-byte
/// `WearChange_Struct` field by field (`Mob::SendWearChange` and its
/// neighbours in `zone/mob_appearance.cpp` fill it): the spawn id in 16
/// bits, the material in 32, a word, the elite material, the hero's forge
/// model, another word, the tint and the slot. The material keeps its 32
/// bits. The elite material, the hero's forge model and the two other
/// words are not read (6.518).
///
/// # Errors
/// Refuses any length but 27, a spawn id of zero and a slot past the
/// ninth, as Titanium's reader does.
pub fn wear_change(body: &[u8]) -> Result<WearChange> {
    ensure!(body.len() == 27, "invalid RoF2 wear change length");
    let spawn_id = u16::from_le_bytes([body[0], body[1]]);
    ensure!(spawn_id != 0, "RoF2 wear change without a spawn");
    let slot = TextureSlot::from_index(usize::from(body[26]))
        .context("RoF2 wear change for an unknown slot")?;
    Ok(WearChange {
        spawn_id,
        slot,
        material: word(body, 2),
        tint: crate::appearance::tint(&body[22..26]),
    })
}

/// A spawn's animation: `ENCODE(OP_Animation)` writes the spawn id in 16
/// bits, then the action, then the speed, the other way round from the
/// server's struct and Titanium's record. The speed is over 10, as
/// Titanium's (inferred, 6.518): the server sends both generations the
/// same number, and it only draws other spawns.
///
/// # Errors
/// Refuses any length but 4.
pub fn animation(body: &[u8]) -> Result<Animation> {
    let &[low, high, action, speed] = body else {
        anyhow::bail!("invalid RoF2 animation length");
    };
    Ok(Animation {
        spawn_id: u16::from_le_bytes([low, high]),
        action: action.into(),
        speed: f32::from(speed) / 10.0,
    })
}

/// The little-endian word at `at`, which the caller's length check covers.
fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

#[cfg(test)]
mod tests;
