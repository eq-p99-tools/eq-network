//! Server-owned buff state. Durations are wire ticks, not client countdowns.
use anyhow::{ensure, Result};
use serde::Serialize;

/// `EQMac`'s `OP_Action`, which carries spell landings among other actions
/// (TAKP `utils/patches/patch_Mac.conf` lists 0x4046, its bytes swapped).
pub const EQMAC_ACTION_OPCODE: u16 = 0x4640;
/// `EQMac`'s `OP_Buff`: a buff's fade, or its duration corrected (0x4132
/// swapped).
pub const EQMAC_BUFF_OPCODE: u16 = 0x3241;
/// The slot of a buff update whose server named none. TAKP fades a buff by
/// its spell, as its own client finds it (`Client::MakeBuffFadePacket`), so
/// a consumer finds the buff by [`BuffUpdate::spell_id`].
pub const UNKNOWN_SLOT: u32 = u32::MAX;
/// Where the buffs lie in `EQMac`'s unpacked profile: 15 ten-byte records at
/// 616 (TAKP `common/patches/mac_structs.h` `PlayerProfile_Struct`).
const EQMAC_PROFILE_BUFFS: std::ops::Range<usize> = 616..766;

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

/// Decodes `EQMac`'s spell actions (TAKP `Action_Struct`, 36 bytes),
/// excluding other actions sharing the opcode. TAKP sends one as the spell
/// lands, with flag 0, and again with flag 4 once a buff takes hold
/// (`Mob::SpellOnTarget`). The action carries no spell level, which reads 0.
///
/// # Errors
/// Rejects malformed `EQMac` action lengths.
pub fn eqmac_spell_effect(body: &[u8]) -> Result<Option<SpellEffect>> {
    ensure!(body.len() == 36, "invalid EQMac action length");
    if body[24] != 231 {
        return Ok(None);
    }
    let short = |offset| u16::from_le_bytes([body[offset], body[offset + 1]]);
    Ok(Some(SpellEffect {
        target_id: short(0),
        caster_id: short(2),
        caster_level: short(4),
        instrument_modifier: word(body, 8),
        spell_id: short(30),
        spell_level: 0,
        effect_flag: body[33],
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
    Ok(profile[5008..5508]
        .as_chunks::<20>()
        .0
        .iter()
        .map(|bytes| record(bytes))
        .collect())
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

/// Reads `EQMac`'s 15 admission buff slots from an unpacked profile,
/// retaining holes and their indexes, which are the server's own slots (TAKP
/// `Client::Handle_Connect_OP_ZoneEntry` copies its buffs there in order).
///
/// # Errors
/// Rejects a profile of another size.
pub fn eqmac_profile(profile: &[u8]) -> Result<Vec<Option<Buff>>> {
    ensure!(
        profile.len() == crate::quarm::PROFILE_SIZE,
        "invalid EQMac profile length"
    );
    Ok(profile[EQMAC_PROFILE_BUFFS]
        .as_chunks::<10>()
        .0
        .iter()
        .map(eqmac_record)
        .collect())
}

/// Decodes `EQMac`'s buff notice (TAKP `SpellBuffFade_Struct`, 20 bytes),
/// which its server sends the player alone, about their own buffs. A fade
/// (flag 1) names only the spell, which TAKP's client looks for itself
/// (`Client::MakeBuffFadePacket`), so its slot is [`UNKNOWN_SLOT`] and its
/// entity the 0 the server leaves there. Any other flag corrects a buff's
/// duration and names the player and the server's slot
/// (`Client::SendBuffDurationPacket`).
///
/// # Errors
/// Rejects malformed `EQMac` buff update lengths.
pub fn eqmac_update(body: &[u8]) -> Result<BuffUpdate> {
    ensure!(body.len() == 20, "invalid EQMac buff update length");
    let short = |offset| u16::from_le_bytes([body[offset], body[offset + 1]]);
    let fade = word(body, 16) == 1;
    Ok(BuffUpdate {
        spell_id: u32::from(short(6)),
        entity_id: u32::from(short(0)),
        slot: if fade {
            UNKNOWN_SLOT
        } else {
            u32::from(short(12))
        },
        buff: if fade {
            None
        } else {
            eqmac_record(body[2..12].try_into()?)
        },
    })
}

/// Projects a ten-byte `EQMac` buff; an empty type or spell is no buff. A
/// permanent buff's duration is 0xFFFF (TAKP `CalcBuffDuration_formula`,
/// formula 50), read signed as -1, a buff with no end. The record names no
/// caster.
fn eqmac_record(bytes: &[u8; 10]) -> Option<Buff> {
    let short = |offset: usize| u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
    let spell_id = short(4);
    if bytes[0] == 0 || matches!(spell_id, 0 | u16::MAX) {
        return None;
    }
    Some(Buff {
        spell_id: u32::from(spell_id),
        caster_level: bytes[1],
        effect_type: bytes[0],
        bard_modifier: bytes[2],
        duration_ticks: i32::from(i16::from_le_bytes([bytes[6], bytes[7]])),
        counters: u32::from(short(8)),
        caster_id: 0,
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

    #[test]
    fn eqmacs_action_reads_its_own_layout_and_skips_other_actions() {
        let mut body = [0; 36];
        body[..2].copy_from_slice(&7u16.to_le_bytes());
        body[2..4].copy_from_slice(&8u16.to_le_bytes());
        body[4..6].copy_from_slice(&60u16.to_le_bytes());
        body[8..12].copy_from_slice(&10i32.to_le_bytes());
        body[30..32].copy_from_slice(&42u16.to_le_bytes());
        body[33] = 4;
        assert!(eqmac_spell_effect(&body).unwrap().is_none());
        body[24] = 231;
        assert_eq!(
            eqmac_spell_effect(&body).unwrap(),
            Some(SpellEffect {
                target_id: 7,
                caster_id: 8,
                caster_level: 60,
                instrument_modifier: 10,
                spell_id: 42,
                spell_level: 0,
                effect_flag: 4,
            })
        );
        assert!(eqmac_spell_effect(&body[..35]).is_err());
        assert!(eqmac_spell_effect(&[0; 37]).is_err());
    }

    /// A ten-byte `EQMac` buff of spell 42: visible with a timer, caster
    /// level 12, these ticks left and three counters.
    fn eqmac_buff(ticks: u16) -> [u8; 10] {
        let mut record = [2, 12, 10, 0, 42, 0, 0, 0, 3, 0];
        record[6..8].copy_from_slice(&ticks.to_le_bytes());
        record
    }

    #[test]
    fn eqmacs_profile_keeps_holes_and_reads_a_permanent_buff_as_endless() {
        let mut profile = vec![0; crate::quarm::PROFILE_SIZE];
        profile[616 + 30..616 + 40].copy_from_slice(&eqmac_buff(0xffff));
        let mut empty = eqmac_buff(5);
        empty[4..6].copy_from_slice(&u16::MAX.to_le_bytes());
        profile[616..626].copy_from_slice(&empty);
        // The bytes past the table are no buff of it.
        profile[766..776].copy_from_slice(&eqmac_buff(5));
        let buffs = eqmac_profile(&profile).unwrap();
        assert_eq!(buffs.len(), 15);
        assert_eq!(buffs.iter().flatten().count(), 1);
        assert_eq!(
            buffs[3],
            Some(Buff {
                spell_id: 42,
                caster_level: 12,
                effect_type: 2,
                bard_modifier: 10,
                duration_ticks: -1,
                counters: 3,
                caster_id: 0,
            })
        );
        assert!(eqmac_profile(&profile[1..]).is_err());
    }

    #[test]
    fn an_eqmac_fade_names_only_its_spell_and_a_correction_its_slot() {
        // A fade: the type, the spell and the flag; no entity or slot.
        let mut body = [0; 20];
        body[2] = 2;
        body[6..8].copy_from_slice(&42u16.to_le_bytes());
        body[16] = 1;
        assert_eq!(
            eqmac_update(&body).unwrap(),
            BuffUpdate {
                spell_id: 42,
                entity_id: 0,
                slot: UNKNOWN_SLOT,
                buff: None,
            }
        );
        // A correction names the player, the server's slot and the buff.
        body[..2].copy_from_slice(&7u16.to_le_bytes());
        body[2..12].copy_from_slice(&eqmac_buff(30));
        body[12..14].copy_from_slice(&3u16.to_le_bytes());
        body[16] = 0;
        let update = eqmac_update(&body).unwrap();
        assert_eq!((update.entity_id, update.slot), (7, 3));
        assert_eq!(update.buff.map(|buff| buff.duration_ticks), Some(30));
        assert!(eqmac_update(&body[..19]).is_err());
        assert!(eqmac_update(&[0; 21]).is_err());
    }
}
