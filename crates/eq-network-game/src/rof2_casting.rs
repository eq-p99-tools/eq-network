//! `RoF2`'s casting, stuns and illusions, as stock `EQEmu` sends them,
//! inferred for `RoF2` (6.522).
//!
//! The begin-cast, action, stun and illusion records pass through encoders
//! of their own in `common/patches/rof2.cpp`; the cast interruption and the
//! face change go out as the server builds them. Titanium's encoder rewrites
//! a face change as an illusion of race -1 (`ENCODE(OP_SetFace)` in
//! `titanium.cpp`); `RoF2` gets the server's own `FaceChange_Struct`.
//!
//! Each reader gives the event Titanium's reader gives for the same server
//! values and keeps its refusals; where one differs, its doc says why. What
//! the `RoF2` program does with the fields not read here is not read
//! (6.522): the action's gem, slot and item-cast type and its float scale,
//! the stun's two constants, the illusion's bytes 70 and 71 and its
//! constant at 316, and the Drakkin words.
use crate::appearance::{AppearanceChange, Features, Illusion};
use crate::buffs::{Action, SpellEffect};
use crate::spells::SpellUpdate;
use crate::world::WorldEvent;
use anyhow::{ensure, Context, Result};

/// `OP_BeginCast` (`utils/patches/patch_RoF2.conf`; inferred, 6.522).
pub const BEGIN_CAST_OPCODE: u16 = 0x318f;
/// `OP_Action` (`utils/patches/patch_RoF2.conf`; inferred, 6.522).
pub const ACTION_OPCODE: u16 = 0x744c;
/// `OP_InterruptCast` (`utils/patches/patch_RoF2.conf`; inferred, 6.522).
pub const INTERRUPT_OPCODE: u16 = 0x048c;
/// `OP_Stun` (`utils/patches/patch_RoF2.conf`; inferred, 6.522).
pub const STUN_OPCODE: u16 = 0x36a4;
/// `OP_Illusion` (`utils/patches/patch_RoF2.conf`; inferred, 6.522).
pub const ILLUSION_OPCODE: u16 = 0x312a;
/// `OP_SetFace` (`utils/patches/patch_RoF2.conf`; inferred, 6.522).
pub const FACE_OPCODE: u16 = 0x1af3;

/// What one of this module's messages says; None for another opcode.
///
/// # Errors
/// Refuses a record its reader refuses.
pub fn read(opcode: u16, body: &[u8]) -> Result<Option<Casting>> {
    Ok(Some(match opcode {
        BEGIN_CAST_OPCODE => Casting::Event(WorldEvent::Spell(begin_cast(body)?)),
        ACTION_OPCODE => Casting::Action(action(body)?),
        INTERRUPT_OPCODE => Casting::Event(WorldEvent::Spell(crate::spells::interrupted(body)?)),
        STUN_OPCODE => Casting::Event(WorldEvent::Stun(stun(body)?)),
        ILLUSION_OPCODE => Casting::Event(illusion(body)?),
        FACE_OPCODE => Casting::Event(face_change(body)?),
        _ => return Ok(None),
    }))
}

/// What a casting message gives: an event, or an action, which the zone
/// decoder makes an event of by its kind, as Titanium's.
#[derive(Clone, Debug, PartialEq)]
pub enum Casting {
    /// An event as it is.
    Event(WorldEvent),
    /// An action, by its kind.
    Action(Action),
}

/// A spell beginning: `ENCODE(OP_BeginCast)` writes the spell in 32 bits,
/// the caster in 16 and the cast time in milliseconds, 10 bytes, where the
/// server's struct has the caster first and the spell in 16 bits
/// (`Mob::CastSpell` in `zone/spells.cpp`). A spell past 16 bits cannot
/// come from it and refuses.
///
/// # Errors
/// Refuses any length but 10 and a spell past 16 bits.
pub fn begin_cast(body: &[u8]) -> Result<SpellUpdate> {
    ensure!(body.len() == 10, "invalid RoF2 begin-cast length");
    Ok(SpellUpdate::Began {
        caster_id: u16::from_le_bytes([body[4], body[5]]),
        spell_id: u16::try_from(word(body, 0)).context("invalid RoF2 begin-cast spell ID")?,
        duration_ms: word(body, 6),
    })
}

/// The band in which an action's instrument modifier says the server's
/// integer was below 10: the encoder's `m - 10` is unsigned, so a value of
/// 0 to 9, or a negative focus result that `Mob::GetInstrumentMod`
/// (`zone/client_mods.cpp`) turns into a wrapped `uint32`, goes out as
/// about 2^32 / 10, where the float cannot tell those values apart.
const WRAPPED_MODIFIER: std::ops::RangeInclusive<f32> = 429_000_000.0..=430_000_000.0;

/// The largest modifier read as a whole step: up to here the encoder's
/// single precision keeps the integer, far above what the server computes
/// (10 for an ordinary spell, a bard's instrument within its song cap).
const MODIFIER_LIMIT: f64 = 100_000.0;

/// An action's instrument modifier as the server's integer (6.522).
///
/// The encoder writes `1 + (m - 10) / 10` in single precision, so from 1.0
/// up the integer is `round(value * 10)`, within a tolerance that grows
/// with the value as the encoder's rounding does. The wrap band reads 0:
/// the server's 0 to 9 cannot be told apart there. `Client::SendSpellAnim`
/// (`zone/spells.cpp`) leaves the field 0 and lands in it; Titanium reads
/// that 0 as it is.
fn instrument_modifier(value: f32) -> Result<u32> {
    if WRAPPED_MODIFIER.contains(&value) {
        return Ok(0);
    }
    let scaled = f64::from(value) * 10.0;
    let rounded = scaled.round();
    let tolerance = (scaled * 2.0 * f64::from(f32::EPSILON)).max(0.001);
    ensure!(
        value.is_finite()
            && value >= 1.0
            && rounded <= MODIFIER_LIMIT
            && (scaled - rounded).abs() <= tolerance,
        "invalid RoF2 action instrument modifier"
    );
    // Whole, from 10 to the limit, so inside u32.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(rounded as u32)
}

/// An action: `ENCODE(OP_Action)` writes the 56-byte `ActionAlt_Struct`:
/// the target, source and level in 16 bits each, a 0 word, the instrument
/// modifier as a float at 10, the push's force, heading and pitch at 14,
/// the kind at 26, a 0 damage word and a 0 16-bit word, the spell in 32
/// bits at 33, the spell level and the effect flag, then a 0 gem byte, the
/// 12-byte inventory slot it fills with constants, and a 0 item-cast type.
///
/// The modifier is read back as the server's integer (see
/// `instrument_modifier` below), so [`SpellEffect::instrument_modifier`] keeps
/// the server's number on both generations, except that the server's 0 to
/// 9 all read 0. The spell is the server's 16-bit number widened.
/// Otherwise it gives what Titanium's reader gives for the same values: a
/// spell's effect for kind 231, the spin, or another kind.
///
/// # Errors
/// Refuses any length but 56, a spell past 16 bits, and a modifier that is
/// not a number, is below 1.0 outside the wrap band, is past the limit, or
/// is not a whole step.
pub fn action(body: &[u8]) -> Result<Action> {
    ensure!(body.len() == 56, "invalid RoF2 action length");
    if body[26] != crate::combat::SPELL_DAMAGE_KIND {
        return Ok(crate::buffs::other_action(body, 26));
    }
    let short = |at: usize| u16::from_le_bytes([body[at], body[at + 1]]);
    Ok(Action::Spell(SpellEffect {
        target_id: short(0),
        caster_id: short(2),
        caster_level: short(4),
        instrument_modifier: instrument_modifier(f32::from_bits(word(body, 10)))?,
        spell_id: u16::try_from(word(body, 33)).context("invalid RoF2 action spell ID")?,
        spell_level: body[37],
        effect_flag: body[38],
        push: crate::combat::push(body, 14)?,
    }))
}

/// A stun of the player: `ENCODE(OP_Stun)` writes the duration in
/// milliseconds, then four bytes it fills as 0, 163, 67 and 0
/// (`Client::Stun`), which are not read or compared: only the length
/// holds them. A zero duration ends a stun, as Titanium's.
///
/// # Errors
/// Refuses any length but 8.
pub fn stun(body: &[u8]) -> Result<Option<std::time::Duration>> {
    ensure!(body.len() == 8, "invalid RoF2 stun length");
    crate::control::stun(&body[..4])
}

/// A spawn's illusion: `ENCODE(OP_Illusion)` writes the 336-byte
/// `Illusion_Struct` (`Mob::SendIllusionPacket`): the spawn in 32 bits, the
/// name in 64 bytes, the race in 16 bits at 68, two bytes it copies from the
/// server at 70, the gender, texture and helm texture at 72, 73 and 76, the
/// face in 32 bits at 80, the hair style and color and the beard and its
/// color at 84 to 87, the size as a float at 88, the Drakkin heritage,
/// tattoo and details at 244, and -1 at 316. The name, bytes 70 and 71, the
/// zero word at 240, the Drakkin words and the constant are not read
/// (6.522).
///
/// The race is 16 bits, so it has no -1 of Titanium's face change, which
/// `RoF2` gets as its own message ([`face_change`]).
///
/// # Errors
/// Refuses any length but 336, a spawn that is zero or past 16 bits, a face
/// past 255, and a size that is negative or not a number, as Titanium's
/// reader does.
pub fn illusion(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 336, "invalid RoF2 illusion length");
    let spawn_id = u16::try_from(word(body, 0))
        .ok()
        .filter(|id| *id != 0)
        .context("invalid RoF2 illusion spawn ID")?;
    let size = f32::from_bits(word(body, 88));
    ensure!(
        size.is_finite() && size >= 0.0,
        "invalid RoF2 illusion size"
    );
    let texture = |byte: u8| (byte != u8::MAX).then_some(byte);
    Ok(WorldEvent::Appearance {
        spawn_id,
        change: AppearanceChange::Illusion(Illusion {
            race: u32::from(u16::from_le_bytes([body[68], body[69]])),
            gender: body[72],
            texture: texture(body[73]),
            helm_texture: texture(body[76]),
            size,
            features: Features {
                face: u8::try_from(word(body, 80)).context("invalid RoF2 illusion face")?,
                hair_style: body[84],
                hair_color: body[85],
                beard: body[86],
                beard_color: body[87],
            },
        }),
    })
}

/// A spawn's new face: the server's 24-byte `FaceChange_Struct`
/// (`Mob::SetFaceAppearance` in `zone/mob.cpp`), with no `RoF2` encoder:
/// the hair and beard colors, two eye colors, the hair style, the beard
/// and the face, a padding byte, the three Drakkin words (not read, 6.522),
/// and the spawn in 32 bits at 20.
///
/// # Errors
/// Refuses any length but 24 and a spawn that is zero or past 16 bits.
pub fn face_change(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 24, "invalid RoF2 face change length");
    let spawn_id = u16::try_from(word(body, 20))
        .ok()
        .filter(|id| *id != 0)
        .context("invalid RoF2 face change spawn ID")?;
    Ok(WorldEvent::FaceChanged {
        spawn_id: Some(spawn_id),
        features: Features {
            face: body[6],
            hair_style: body[4],
            hair_color: body[0],
            beard: body[5],
            beard_color: body[1],
        },
        eye_colors: Some([body[2], body[3]]),
    })
}

/// The little-endian word at `at`, which the caller's length check covers.
fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

#[cfg(test)]
mod tests;
