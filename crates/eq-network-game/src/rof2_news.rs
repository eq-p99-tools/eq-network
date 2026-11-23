//! The player's own news and the combat records `RoF2` gets, as stock
//! `EQEmu` sends them, inferred for `RoF2` (6.519).
//!
//! Six go out as the server builds them, with no `RoF2` encoder and none
//! for Titanium: a death (`Mob::Death` and `Client::Death` in
//! `zone/attack.cpp`), a level (`Client::SetLevel` in `zone/exp.cpp`),
//! experience, food and water, coins and the time of day. Their bytes are
//! the ones a Titanium client gets, so Titanium's readers read them under
//! `RoF2`'s numbers. `OP_ManaChange` also goes out unencoded, but
//! Titanium's encoder cuts it to 16 bytes and `RoF2` gets the server's 20.
//! The damage, consider and skill records pass through encoders of their
//! own in `common/patches/rof2.cpp`.
//!
//! What the `RoF2` program does with the fields not read here is not read
//! (6.519): the death record's words 12 and 20, the mana change's slot,
//! the damage's special word and byte 25, the consider's `PvP` byte and the
//! skill update's constants; and the time of day's hours are taken as
//! Titanium's (1 to 24).
use crate::combat::{Consideration, Damage};
use crate::world::WorldEvent;
use anyhow::{ensure, Context, Result};

/// `OP_Death` (`utils/patches/patch_RoF2.conf`; inferred, 6.519).
pub const DEATH_OPCODE: u16 = 0x6517;
/// `OP_LevelUpdate` (`utils/patches/patch_RoF2.conf`; inferred, 6.519).
pub const LEVEL_OPCODE: u16 = 0x1eec;
/// `OP_ExpUpdate` (`utils/patches/patch_RoF2.conf`; inferred, 6.519).
pub const EXPERIENCE_OPCODE: u16 = 0x20ed;
/// `OP_Stamina` (`utils/patches/patch_RoF2.conf`; inferred, 6.519).
pub const STAMINA_OPCODE: u16 = 0x2a79;
/// `OP_MoneyUpdate` (`utils/patches/patch_RoF2.conf`; inferred, 6.519).
pub const MONEY_OPCODE: u16 = 0x640c;
/// `OP_TimeOfDay` (`utils/patches/patch_RoF2.conf`; inferred, 6.519).
pub const TIME_OPCODE: u16 = 0x5070;
/// `OP_ManaChange` (`utils/patches/patch_RoF2.conf`; inferred, 6.519).
pub const MANA_OPCODE: u16 = 0x5467;
/// `OP_Damage` (`utils/patches/patch_RoF2.conf`; inferred, 6.519).
pub const DAMAGE_OPCODE: u16 = 0x6f15;
/// `OP_Consider` (`utils/patches/patch_RoF2.conf`; inferred, 6.519).
pub const CONSIDER_OPCODE: u16 = 0x742b;
/// `OP_SkillUpdate`, written `0x04c` in `utils/patches/patch_RoF2.conf`
/// (inferred, 6.519).
pub const SKILL_OPCODE: u16 = 0x004c;

/// What one of this module's messages says: one event, or two for a mana
/// change; None for another opcode.
///
/// # Errors
/// Refuses a record its reader refuses.
pub fn read(opcode: u16, body: &[u8]) -> Result<Option<Vec<WorldEvent>>> {
    Ok(Some(vec![match opcode {
        DEATH_OPCODE => WorldEvent::Death(crate::zoning::death(body)?),
        LEVEL_OPCODE => crate::world::level_update(body)?,
        EXPERIENCE_OPCODE => {
            // The server's struct is 8 bytes; Titanium's reader takes a
            // longer body, which no stock server sends RoF2.
            ensure!(body.len() == 8, "invalid RoF2 experience update length");
            crate::world::experience_update(body)?
        }
        STAMINA_OPCODE => WorldEvent::Nourishment(
            crate::food::decode(crate::food::STAMINA_OPCODE, body)?
                .context("stamina reader declined its own opcode")?,
        ),
        MONEY_OPCODE => WorldEvent::Coins(crate::world::money_update(body)?),
        // The server's struct, with the year in 32 bits: `rof2_structs.h`
        // declares a 16-bit year and two bytes after it, but no `RoF2`
        // encoder writes that struct, so it is not what arrives. The hours
        // are Titanium's, 1 to 24 (inferred, 6.519).
        TIME_OPCODE => WorldEvent::TimeOfDay(crate::clock::decode(body)?),
        MANA_OPCODE => return mana_change(body).map(Some),
        DAMAGE_OPCODE => WorldEvent::Damage(damage(body)?),
        CONSIDER_OPCODE => WorldEvent::Consideration(consideration(body)?),
        SKILL_OPCODE => skill(body)?,
        _ => return Ok(None),
    }]))
}

/// The player's mana and endurance, and the spell they came with: the
/// server's 20-byte `ManaChange_Struct` (`Client::CheckManaEndUpdate` in
/// `zone/client.cpp`): mana, endurance and the spell in 32 bits each, the
/// keep-casting byte, three padding bytes, and the slot in 32 bits, which
/// Titanium's encoder drops and `RoF2` gets. It gives Titanium's two
/// events, the spell notice first. The padding and the slot are not read
/// (6.519).
///
/// # Errors
/// Refuses any length but 20 and a keep-casting byte past 1.
pub fn mana_change(body: &[u8]) -> Result<Vec<WorldEvent>> {
    ensure!(
        body.len() == 20 && body[12] <= 1,
        "invalid RoF2 mana change"
    );
    Ok(vec![
        WorldEvent::Spell(crate::spells::SpellUpdate::Mana {
            spell_id: word(body, 8),
            keep_casting: body[12] != 0,
        }),
        WorldEvent::Resources {
            mana: word(body, 0),
            endurance: word(body, 4),
        },
    ])
}

/// One damage record: `ENCODE(OP_Damage)` writes the 30-byte
/// `CombatDamage_Struct` field by field: the target and source in 16 bits
/// each, the kind, the spell in 32 bits at 5, the damage at 9, and the
/// push's force, heading and pitch at 13. Byte 25 is the struct's off-hand
/// flag, which the encoder never writes, so the server leaves it zero; the
/// special word (2 a rampage, 1 a wild rampage) is at 26. Neither is read
/// (6.519). The spell is the server's 16-bit number widened, so one past
/// 16 bits cannot come from it and refuses. Otherwise it gives what
/// Titanium's reader gives for the same values.
///
/// # Errors
/// Refuses any length but 30 and a spell past 16 bits.
pub fn damage(body: &[u8]) -> Result<Damage> {
    ensure!(body.len() == 30, "invalid RoF2 damage length");
    let spell_id = u16::try_from(word(body, 5)).context("invalid RoF2 damage spell ID")?;
    Ok(crate::combat::damage_from(
        body,
        spell_id,
        word(body, 9).cast_signed(),
        crate::combat::push(body, 13)?,
    ))
}

/// The answer to a consider: `ENCODE(OP_Consider)` writes the player, the
/// target, the faction and the color in 32 bits each, the `PvP` byte and
/// three bytes, 20 in all. The encoder copies no hit points, so the
/// server's current and maximum never reach `RoF2` and the answer carries
/// none. The `PvP` byte is not read, as on
/// Titanium.
///
/// # Errors
/// Refuses any length but 20 and a target that is zero or past 16 bits.
pub fn consideration(body: &[u8]) -> Result<Consideration> {
    ensure!(body.len() == 20, "invalid RoF2 consider length");
    let target_id = u16::try_from(word(body, 4))
        .ok()
        .filter(|id| *id != 0)
        .context("invalid RoF2 consider target")?;
    Ok(Consideration {
        target_id,
        faction: word(body, 8),
        color: word(body, 12).into(),
        hit_points: None,
    })
}

/// A skill's new value: `ENCODE(OP_SkillUpdate)` writes the skill and the
/// value in 32 bits each, then four bytes it always sets to 1, 80, 136 and
/// 54, which are not read.
///
/// # Errors
/// Refuses any length but 12.
pub fn skill(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 12, "invalid RoF2 skill update length");
    Ok(WorldEvent::Skill {
        skill_id: word(body, 0),
        value: word(body, 4),
    })
}

/// The little-endian word at `at`, which the caller's length check covers.
fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

#[cfg(test)]
mod tests;
