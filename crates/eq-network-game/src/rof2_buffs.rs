//! `RoF2`'s buffs, as stock `EQEmu` sends them, inferred for `RoF2`
//! (6.527).
//!
//! The server's buff slots are `RoF2`'s own (`emu_constants.h` takes
//! `RoF2::spells::LONG_BUFFS` and `SHORT_BUFFS`), so `ServerToRoF2BuffSlot`
//! in `common/patches/rof2.cpp` returns each slot as it is and every slot
//! read here is the server's number. The buff update, the icon messages and
//! the pet's window pass through encoders of their own in that file.
//!
//! What the `RoF2` program does with the update's fade words, its location,
//! hit count and slot data, and how it combines the update with the icon
//! messages, is not read (6.527).
use crate::buffs::{Applies, Buff, BuffIcon, BuffIcons, BuffUpdate, BuffWindow};
use crate::pets::{PetBuff, PetBuffs, BUFF_SLOTS};
use crate::world::WorldEvent;
use anyhow::{ensure, Context, Result};

/// `OP_Buff` (`utils/patches/patch_RoF2.conf`; inferred, 6.527).
pub const UPDATE_OPCODE: u16 = 0x659c;
/// `OP_BuffCreate` (`utils/patches/patch_RoF2.conf`; inferred, 6.527).
pub const ICONS_OPCODE: u16 = 0x3377;
/// `OP_TargetBuffs` (`utils/patches/patch_RoF2.conf`; inferred, 6.527).
pub const TARGET_ICONS_OPCODE: u16 = 0x4f4b;
/// `OP_PetBuffWindow` (`utils/patches/patch_RoF2.conf`; inferred, 6.527).
pub const PET_OPCODE: u16 = 0x5882;

/// The most buff slots any spawn has: `RoF2`'s `NPC_BUFFS`
/// (`common/patches/rof2_limits.h`), which a target NPC's window can fill;
/// a player has 63.
const MOST_SLOTS: usize = 97;

/// The longest caster name the server writes: `strn0cpy` into 64 bytes.
const NAME_LIMIT: usize = 63;

/// What one of this module's messages says; None for another opcode.
///
/// # Errors
/// Refuses a record its reader refuses.
pub fn read(opcode: u16, body: &[u8]) -> Result<Option<WorldEvent>> {
    Ok(Some(match opcode {
        UPDATE_OPCODE => WorldEvent::Buff(update(body)?),
        ICONS_OPCODE | TARGET_ICONS_OPCODE => WorldEvent::BuffIcons(icons(body)?),
        PET_OPCODE => WorldEvent::PetBuffs(pet_buffs(body)?),
        _ => return Ok(None),
    }))
}

/// A buff update: `ENCODE(OP_Buff)` writes the 100-byte
/// `SpellBuffPacket_Struct`: the entity, the 88-byte record at 4, the slot
/// at 92 and the fade word at 96, which is 1 for the server's 1 and 2 for
/// anything else. The server sends 0 for a duration
/// (`Client::SendBuffDurationPacket` in `zone/spells.cpp`) and 1 for a fade
/// (`Client::MakeBuffFadePacket`).
///
/// It gives Titanium's [`BuffUpdate`] for the same server values: 1 is
/// [`Applies::FadeBySpell`] and 2 [`Applies::BySpell`], Titanium's
/// readings of the server's 1 and 0 (inferred, 6.527), so
/// [`Applies::BySlot`] never comes from `RoF2`. The record is None under
/// Titanium's rule. It differs from Titanium's in two ways:
/// - [`Buff::counters`] reads 0: the encoder copies no counter word, and
///   leaves the record's slot data zero.
/// - The slot is the server's own number, where Titanium's encoder
///   renumbers it.
///
/// The hit count, the location and the slot data are not read.
///
/// # Errors
/// Refuses any length but 100, a fade word other than 1 or 2, and a bard
/// modifier that is not the server's byte over 10.
pub fn update(body: &[u8]) -> Result<BuffUpdate> {
    ensure!(body.len() == 100, "invalid RoF2 buff update length");
    let applies = match word(body, 96) {
        1 => Applies::FadeBySpell,
        2 => Applies::BySpell,
        _ => anyhow::bail!("invalid RoF2 buff update fade word"),
    };
    let bard_modifier = bard_modifier(f32::from_bits(word(body, 8)))?;
    let spell_id = word(body, 12);
    let empty = body[4] == 0 || matches!(spell_id, 0 | 0xffff | u32::MAX);
    Ok(BuffUpdate {
        spell_id,
        entity_id: word(body, 0),
        slot: word(body, 92),
        applies,
        record_type: body[4],
        buff: (applies != Applies::FadeBySpell && !empty).then(|| Buff {
            spell_id,
            caster_level: body[5],
            effect_type: body[4],
            bard_modifier,
            duration_ticks: word(body, 16).cast_signed(),
            counters: 0,
            caster_id: word(body, 20),
        }),
    })
}

/// A buff's bard modifier as the server's byte (6.527): the encoder writes
/// exactly 1.0 for 10 and `m / 10.0f` for any other byte, so the byte is
/// `round(value * 10)`, within the tolerance the encoder's single
/// precision leaves.
fn bard_modifier(value: f32) -> Result<u8> {
    let scaled = f64::from(value) * 10.0;
    let rounded = scaled.round();
    let tolerance = (scaled * 2.0 * f64::from(f32::EPSILON)).max(0.001);
    ensure!(
        value.is_finite()
            && (0.0..=f64::from(u8::MAX)).contains(&rounded)
            && (scaled - rounded).abs() <= tolerance,
        "invalid RoF2 buff bard modifier"
    );
    // Whole, from 0 to 255.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(rounded as u8)
}

/// The icon messages: `ENCODE(OP_BuffCreate)`, which `OP_TargetBuffs`
/// forwards to, writes the entity, the tick timer, the all-buffs byte and a
/// 16-bit count, then each entry's slot, spell, ticks, hits and caster
/// name, NUL-ended, and the window's type byte last
/// (`Mob::MakeBuffsPacket`, `Client::SendBuffNumHitPacket`). After each
/// fade the update's encoder also sends one of its own: one entry with the
/// fade's slot, spell -1 and no name, which reads as that slot emptied.
///
/// # Errors
/// Refuses a spawn that is zero or past 16 bits, a count past 97, a slot
/// past 96 or named twice, a name that does not end inside the body, is
/// longer than 63 bytes or is not ASCII, and a body that does not end with
/// the type byte right after the last entry. The count bounds the walk.
pub fn icons(body: &[u8]) -> Result<BuffIcons> {
    let header = body.get(..11).context("invalid RoF2 buff icons length")?;
    let spawn_id = u16::try_from(word(header, 0))
        .ok()
        .filter(|id| *id != 0)
        .context("invalid RoF2 buff icons spawn ID")?;
    let count = usize::from(u16::from_le_bytes([header[9], header[10]]));
    ensure!(count <= MOST_SLOTS, "invalid RoF2 buff icons count");
    let mut icons: Vec<BuffIcon> = Vec::new();
    let mut at = 11;
    for _ in 0..count {
        let entry = Entry::read(body, at)?;
        ensure!(
            usize::try_from(entry.slot).is_ok_and(|slot| slot < MOST_SLOTS)
                && !icons.iter().any(|icon| icon.slot == entry.slot),
            "invalid RoF2 buff icons slot"
        );
        at = entry.end;
        icons.push(BuffIcon {
            slot: entry.slot,
            spell_id: (entry.spell_id != u32::MAX).then_some(entry.spell_id),
            ticks: entry.ticks,
            hits: entry.hits,
            caster_name: (!entry.name.is_empty())
                .then(|| String::from_utf8_lossy(entry.name).into_owned()),
        });
    }
    ensure!(body.len() == at + 1, "invalid RoF2 buff icons length");
    Ok(BuffIcons {
        spawn_id,
        window: BuffWindow::from_wire(body[at]),
        all: header[8] != 0,
        tick_timer_ms: word(header, 4).cast_signed(),
        icons,
    })
}

/// The pet's buffs: `ENCODE(OP_PetBuffWindow)` writes the icon messages'
/// shape (`Mob::SendPetBuffsToClient`): the pet, a 0 word, the all-buffs
/// byte 1 and the count, then each filled slot below 30 with its spell,
/// ticks, a 0 hit count and no name, and a type byte. It gives Titanium's
/// [`PetBuffs`] for the same server values, each buff in its slot.
///
/// The encoder sizes the body by the server's count but writes an entry
/// for each nonzero spell, so where the two differ the body is short, or
/// ends in zero padding that would read as entries of spell 0. Both refuse,
/// so a buff is never placed in another slot. A spell of -1 leaves its
/// slot empty, as Titanium's.
///
/// # Errors
/// Refuses a pet past 16 bits, an all-buffs byte other than 1, a slot past
/// 29 or named twice, a spell of 0, a hit count or a name, a count past
/// 30, and a body that does not end with the type byte right after the
/// last entry.
pub fn pet_buffs(body: &[u8]) -> Result<PetBuffs> {
    let header = body.get(..11).context("invalid RoF2 pet buffs length")?;
    let pet = u16::try_from(word(header, 0)).context("invalid RoF2 pet buffs pet")?;
    ensure!(header[8] == 1, "invalid RoF2 pet buffs list");
    let count = usize::from(u16::from_le_bytes([header[9], header[10]]));
    ensure!(count <= BUFF_SLOTS, "invalid RoF2 pet buffs count");
    let mut slots = vec![None; BUFF_SLOTS];
    let mut named = [false; BUFF_SLOTS];
    let mut at = 11;
    for _ in 0..count {
        let entry = Entry::read(body, at)?;
        let slot = usize::try_from(entry.slot)
            .ok()
            .filter(|slot| *slot < BUFF_SLOTS && !named[*slot])
            .context("invalid RoF2 pet buffs slot")?;
        ensure!(
            entry.spell_id != 0 && entry.hits == 0 && entry.name.is_empty(),
            "invalid RoF2 pet buffs entry"
        );
        named[slot] = true;
        slots[slot] = (entry.spell_id != u32::MAX).then_some(PetBuff {
            spell_id: entry.spell_id,
            ticks: entry.ticks,
        });
        at = entry.end;
    }
    ensure!(body.len() == at + 1, "invalid RoF2 pet buffs length");
    Ok(PetBuffs { pet, slots })
}

/// One entry of the icon messages' shape.
struct Entry<'a> {
    slot: u32,
    spell_id: u32,
    ticks: i32,
    hits: u32,
    name: &'a [u8],
    /// Where the next entry starts.
    end: usize,
}

impl<'a> Entry<'a> {
    /// The entry at `at`: four words, then a NUL-ended name of at most 63
    /// ASCII bytes inside the body.
    fn read(body: &'a [u8], at: usize) -> Result<Self> {
        let fixed = body
            .get(at..at + 16)
            .context("invalid RoF2 buff entry length")?;
        let rest = &body[at + 16..];
        let length = rest
            .iter()
            .take(NAME_LIMIT + 1)
            .position(|byte| *byte == 0)
            .context("invalid RoF2 buff entry name")?;
        let name = &rest[..length];
        ensure!(name.is_ascii(), "invalid RoF2 buff entry name");
        Ok(Self {
            slot: word(fixed, 0),
            spell_id: word(fixed, 4),
            ticks: word(fixed, 8).cast_signed(),
            hits: word(fixed, 12),
            name,
            end: at + 16 + length + 1,
        })
    }
}

/// The little-endian word at `at`, which the caller's length check covers.
fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

#[cfg(test)]
mod tests;
