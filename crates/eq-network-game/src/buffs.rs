//! Server-owned buff state. Durations are wire ticks, not client countdowns.
use anyhow::{ensure, Result};
use serde::Serialize;

/// Server spell action. Its icon flag does not supply a buff slot or duration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SpellEffect {
    /// Target entity in this zone.
    pub target_id: u16,
    /// Caster entity in this zone.
    pub caster_id: u16,
    /// Caster level carried by the action.
    pub caster_level: u16,
    /// Raw instrument modifier; do not assume the profile's byte scale.
    pub instrument_modifier: u32,
    /// The spell producing this effect.
    pub spell_id: u16,
    /// Additional spell-level byte from the action.
    pub spell_level: u8,
    /// Raw flags; the documented icon value is 4.
    pub effect_flag: u8,
}

/// Decodes spell actions, excluding melee actions sharing the same opcode.
///
/// # Errors
/// Rejects malformed Titanium action lengths.
pub fn titanium_spell_effect(body: &[u8]) -> Result<Option<SpellEffect>> {
    ensure!(body.len() == 31, "invalid Titanium action length");
    if body[22] != 231 {
        return Ok(None);
    }
    let short = |offset| u16::from_le_bytes([body[offset], body[offset + 1]]);
    Ok(Some(SpellEffect {
        target_id: short(0),
        caster_id: short(2),
        caster_level: short(4),
        instrument_modifier: word(body, 6),
        spell_id: short(27),
        spell_level: body[29],
        effect_flag: body[30],
    }))
}

/// A buff record supplied by the server, without evaluating its spell effects.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Buff {
    /// Spell identifier.
    pub spell_id: u32,
    /// Caster level used for effect calculations.
    pub caster_level: u8,
    /// Raw effect type; unknown values remain available to consumers.
    pub effect_type: u8,
    /// Raw bard modifier, whose scaling is dialect-specific.
    pub bard_modifier: u8,
    /// Remaining server ticks, including negative special durations.
    pub duration_ticks: i32,
    /// Counters or rune bookkeeping supplied by the server.
    pub counters: u32,
    /// Zone-scoped caster identifier, possibly zero.
    pub caster_id: u32,
}

/// One explicit buff-slot notification; casting success is not a buff update.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BuffUpdate {
    /// Spell ID retained even on a fade, so consumers can remove an unslotted icon.
    pub spell_id: u32,
    /// The affected entity, not necessarily the local player.
    pub entity_id: u32,
    /// Server slot number, preserved without allocating by its value.
    pub slot: u32,
    /// Present for a replacement, absent for an explicit fade or empty record.
    pub buff: Option<Buff>,
}

/// Reads all 25 Titanium admission slots, retaining holes and their indexes.
///
/// # Errors
/// Rejects profiles too short to contain the complete buff table.
pub fn titanium_profile(profile: &[u8]) -> Result<Vec<Option<Buff>>> {
    ensure!(profile.len() >= 5508, "truncated Titanium buff table");
    Ok(profile[5008..5508].chunks_exact(20).map(record).collect())
}

/// Decodes a Titanium server buff notification and its explicit fade flag.
///
/// # Errors
/// Rejects unexpected lengths and unknown fade flags rather than guessing a change.
pub fn titanium_update(body: &[u8]) -> Result<BuffUpdate> {
    ensure!(body.len() == 32, "invalid Titanium buff update length");
    let fade = word(body, 28);
    ensure!(fade <= 1, "unknown Titanium buff fade flag");
    Ok(BuffUpdate {
        spell_id: word(body, 8),
        entity_id: word(body, 0),
        slot: word(body, 24),
        buff: if fade == 1 {
            None
        } else {
            record(&body[4..24])
        },
    })
}

/// Projects a length-checked 20-byte record; empty spell sentinels are not effects.
fn record(bytes: &[u8]) -> Option<Buff> {
    let spell_id = word(bytes, 4);
    if bytes[0] == 0 || matches!(spell_id, 0 | 0xffff | u32::MAX) {
        return None;
    }
    Some(Buff {
        spell_id,
        caster_level: bytes[1],
        effect_type: bytes[0],
        bard_modifier: bytes[2],
        duration_ticks: i32::from_le_bytes(bytes[8..12].try_into().expect("checked record length")),
        counters: word(bytes, 12),
        caster_id: word(bytes, 16),
    })
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("checked buff length"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn action_is_not_a_slot_update_and_melee_does_not_become_a_spell() {
        let mut body = [0; 31];
        body[..2].copy_from_slice(&7u16.to_le_bytes());
        body[2..4].copy_from_slice(&8u16.to_le_bytes());
        body[4..6].copy_from_slice(&60u16.to_le_bytes());
        body[6..10].copy_from_slice(&125u32.to_le_bytes());
        body[27..29].copy_from_slice(&42u16.to_le_bytes());
        body[30] = 4;
        assert!(titanium_spell_effect(&body).unwrap().is_none());
        body[22] = 231;
        let effect = titanium_spell_effect(&body).unwrap().unwrap();
        assert_eq!(
            (effect.target_id, effect.caster_id, effect.caster_level),
            (7, 8, 60)
        );
        assert_eq!(
            (
                effect.spell_id,
                effect.instrument_modifier,
                effect.effect_flag
            ),
            (42, 125, 4)
        );
        assert!(titanium_spell_effect(&body[..30]).is_err());
        assert!(titanium_update(&body).is_err());
    }
    #[test]
    fn profile_holes_signed_duration_and_explicit_fade_are_preserved() {
        let mut profile = vec![0; 5508];
        let start = 5008 + 3 * 20;
        profile[start] = 2;
        profile[start + 1] = 12;
        profile[start + 4..start + 8].copy_from_slice(&42u32.to_le_bytes());
        profile[start + 8..start + 12].copy_from_slice(&(-1i32).to_le_bytes());
        let buffs = titanium_profile(&profile).unwrap();
        assert_eq!(buffs.len(), 25);
        assert_eq!(buffs.iter().flatten().count(), 1);
        assert_eq!(buffs[3].as_ref().unwrap().duration_ticks, -1);
        assert!(titanium_profile(&profile[..5507]).is_err());
        let mut body = [0; 32];
        body[..4].copy_from_slice(&7u32.to_le_bytes());
        body[4..24].copy_from_slice(&profile[start..start + 20]);
        body[24..28].copy_from_slice(&3u32.to_le_bytes());
        let update = titanium_update(&body).unwrap();
        assert_eq!((update.entity_id, update.slot), (7, 3));
        assert_eq!(update.buff, buffs[3]);
        body[28] = 1;
        assert!(titanium_update(&body).unwrap().buff.is_none());
        body[28] = 2;
        assert!(titanium_update(&body).is_err());
        assert!(titanium_update(&body[..31]).is_err());
    }
}
