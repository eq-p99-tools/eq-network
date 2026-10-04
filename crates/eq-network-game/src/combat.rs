//! Titanium consider, auto-attack and combat-damage records.
//!
//! The Titanium client prints melee and spell-damage messages itself from
//! `OP_Damage`; these records keep the server's numeric fields so a consumer
//! can do the same without inventing results.
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_Consider`, sent by the client and echoed with the server's assessment.
pub const CONSIDER_OPCODE: u16 = 0x65ca;
/// `OP_AutoAttack`: a four-byte toggle whose first byte is 1 (on) or 0 (off).
pub const AUTO_ATTACK_OPCODE: u16 = 0x5e55;
/// `EQMac`'s `OP_Consider`: the request and its answer share TAKP's 24-byte
/// `Consider_Struct`.
pub const EQMAC_CONSIDER_OPCODE: u16 = 0x3741;
/// `EQMac`'s `OP_AutoAttack`, the same four-byte toggle as Titanium's.
pub const EQMAC_AUTO_ATTACK_OPCODE: u16 = 0x5141;
/// `EQMac`'s `OP_Damage`: TAKP's 24-byte `Damage_Struct`, with the type and
/// the spell as 16-bit fields.
pub const EQMAC_DAMAGE_OPCODE: u16 = 0x5840;

/// `OP_Damage`: one melee, skill or spell damage record.
pub const DAMAGE_OPCODE: u16 = 0x5c78;

/// Damage kind used for spell damage and beneficial spell landings.
pub const SPELL_DAMAGE_KIND: u8 = 231;

/// The server's answer to a consider request.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Consideration {
    /// Considered spawn.
    pub target_id: u16,
    /// Faction standing, 1 (ally) through 9 (scowls); other values are preserved.
    pub faction: u32,
    /// Level-difference color code chosen by the server.
    pub color: ConColor,
    /// Current and maximum hit points when the server supplies them.
    pub hit_points: Option<(i32, i32)>,
}

/// Level-difference colors in the `EQEmu` con encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ConColor {
    /// Too low to be worth experience.
    Gray,
    /// Much lower level.
    Green,
    /// Lower level.
    LightBlue,
    /// Somewhat lower level.
    Blue,
    /// Even level.
    White,
    /// Somewhat higher level.
    Yellow,
    /// Much higher level.
    Red,
    /// Unrecognized code, kept for diagnostics.
    Other(u32),
}

impl From<u32> for ConColor {
    fn from(value: u32) -> Self {
        match value {
            6 => Self::Gray,
            2 => Self::Green,
            18 => Self::LightBlue,
            4 => Self::Blue,
            10 | 20 => Self::White,
            15 => Self::Yellow,
            13 => Self::Red,
            other => Self::Other(other),
        }
    }
}

/// Outcome carried in the signed damage field.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum DamageOutcome {
    /// Points of damage dealt.
    Hit(u32),
    /// The attack missed.
    Miss,
    /// Blocked by the defender.
    Block,
    /// Parried by the defender.
    Parry,
    /// Riposted by the defender.
    Riposte,
    /// Dodged by the defender.
    Dodge,
    /// The defender is invulnerable.
    Invulnerable,
    /// Absorbed by a rune.
    Rune,
    /// Unrecognized negative code.
    Other(i32),
}

impl From<i32> for DamageOutcome {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::Miss,
            -1 => Self::Block,
            -2 => Self::Parry,
            -3 => Self::Riposte,
            -4 => Self::Dodge,
            -5 => Self::Invulnerable,
            -6 => Self::Rune,
            value => u32::try_from(value).map_or(Self::Other(value), Self::Hit),
        }
    }
}

/// One `OP_Damage` record.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Damage {
    /// Entity receiving the attack.
    pub target_id: u16,
    /// Attacking entity.
    pub source_id: u16,
    /// Skill used, or [`SPELL_DAMAGE_KIND`] for spells.
    pub kind: u8,
    /// Spell responsible for spell damage; None for melee and skills.
    pub spell_id: Option<u16>,
    /// Signed outcome from the damage field.
    pub outcome: DamageOutcome,
}

/// Encodes a consider request for a visible target.
///
/// # Errors
/// Rejects the reserved zero ID for either entity.
pub fn consider_request(own_id: u16, target_id: u16) -> Result<[u8; 28]> {
    ensure!(
        own_id != 0 && target_id != 0,
        "consider requires two entities"
    );
    let mut body = [0; 28];
    body[..4].copy_from_slice(&u32::from(own_id).to_le_bytes());
    body[4..8].copy_from_slice(&u32::from(target_id).to_le_bytes());
    Ok(body)
}

/// Encodes `EQMac`'s consider request: the player's and the target's spawns
/// as 16-bit IDs, then room for the answer.
///
/// # Errors
/// Rejects a request without both entities.
pub fn eqmac_consider_request(own_id: u16, target_id: u16) -> Result<[u8; 24]> {
    ensure!(
        own_id != 0 && target_id != 0,
        "consider requires two entities"
    );
    let mut body = [0; 24];
    body[..2].copy_from_slice(&own_id.to_le_bytes());
    body[2..4].copy_from_slice(&target_id.to_le_bytes());
    Ok(body)
}

/// Decodes `EQMac`'s answer to a consider request: the target, the faction
/// standing, the con level and its hit points.
///
/// # Errors
/// Rejects a malformed answer, and one without a target.
pub fn eqmac_consideration(body: &[u8]) -> Result<Consideration> {
    ensure!(body.len() == 24, "invalid EQMac consider length");
    let target_id = u16::from_le_bytes([body[2], body[3]]);
    ensure!(target_id != 0, "consider response without a target");
    let current = i32::from_le_bytes(word(body, 12)?.to_le_bytes());
    let maximum = i32::from_le_bytes(word(body, 16)?.to_le_bytes());
    Ok(Consideration {
        target_id,
        faction: word(body, 4)?,
        color: word(body, 8)?.into(),
        hit_points: (maximum > 0).then_some((current, maximum)),
    })
}

/// Encodes the auto-attack toggle.
#[must_use]
pub const fn auto_attack(enabled: bool) -> [u8; 4] {
    [enabled as u8, 0, 0, 0]
}

/// Decodes the server's consider response.
///
/// # Errors
/// Rejects any length other than the 28-byte Titanium structure.
pub fn consideration(body: &[u8]) -> Result<Consideration> {
    ensure!(body.len() == 28, "invalid consider length");
    let target_id = u16::try_from(word(body, 4)?)?;
    ensure!(target_id != 0, "consider response without a target");
    let current = i32::from_le_bytes(word(body, 16)?.to_le_bytes());
    let maximum = i32::from_le_bytes(word(body, 20)?.to_le_bytes());
    Ok(Consideration {
        target_id,
        faction: word(body, 8)?,
        color: word(body, 12)?.into(),
        hit_points: (maximum > 0).then_some((current, maximum)),
    })
}

/// Decodes one damage record.
///
/// # Errors
/// Rejects any length other than the 23-byte Titanium structure.
pub fn damage(body: &[u8]) -> Result<Damage> {
    ensure!(body.len() == 23, "invalid damage length");
    let kind = body[4];
    let spell_id = u16::from_le_bytes([body[5], body[6]]);
    let raw = i32::from_le_bytes(word(body, 7)?.to_le_bytes());
    Ok(Damage {
        target_id: u16::from_le_bytes([body[0], body[1]]),
        source_id: u16::from_le_bytes([body[2], body[3]]),
        kind,
        spell_id: (kind == SPELL_DAMAGE_KIND && !matches!(spell_id, 0 | u16::MAX))
            .then_some(spell_id),
        outcome: raw.into(),
    })
}

/// Decodes `EQMac`'s damage record: Titanium's fields, the type and the
/// spell 16 bits wide, then the damage at 8 (TAKP `Damage_Struct`, whose
/// types are Titanium's: `SkillDamageTypes`, and 231 for spells).
///
/// # Errors
/// Rejects a malformed record, and a type beyond any skill's.
pub fn eqmac_damage(body: &[u8]) -> Result<Damage> {
    ensure!(body.len() == 24, "invalid EQMac damage length");
    let kind = u8::try_from(u16::from_le_bytes([body[4], body[5]]))
        .map_err(|_| anyhow::anyhow!("EQMac damage type beyond any skill's"))?;
    let spell_id = u16::from_le_bytes([body[6], body[7]]);
    let raw = i32::from_le_bytes(word(body, 8)?.to_le_bytes());
    Ok(Damage {
        target_id: u16::from_le_bytes([body[0], body[1]]),
        source_id: u16::from_le_bytes([body[2], body[3]]),
        kind,
        spell_id: (kind == SPELL_DAMAGE_KIND && !matches!(spell_id, 0 | u16::MAX))
            .then_some(spell_id),
        outcome: raw.into(),
    })
}

fn word(body: &[u8], offset: usize) -> Result<u32> {
    let bytes = body
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow::anyhow!("truncated combat record"))?;
    Ok(u32::from_le_bytes(bytes.try_into()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consider_request_and_response_use_the_28_byte_layout() {
        let request = consider_request(7, 513).unwrap();
        assert_eq!(&request[..8], &[7, 0, 0, 0, 1, 2, 0, 0]);
        assert!(request[8..].iter().all(|byte| *byte == 0));
        assert!(consider_request(0, 5).is_err());

        let mut response = request;
        response[8..12].copy_from_slice(&9u32.to_le_bytes());
        response[12..16].copy_from_slice(&15u32.to_le_bytes());
        response[16..20].copy_from_slice(&40i32.to_le_bytes());
        response[20..24].copy_from_slice(&50i32.to_le_bytes());
        assert_eq!(
            consideration(&response).unwrap(),
            Consideration {
                target_id: 513,
                faction: 9,
                color: ConColor::Yellow,
                hit_points: Some((40, 50)),
            }
        );
        response[20..24].copy_from_slice(&0i32.to_le_bytes());
        response[12..16].copy_from_slice(&77u32.to_le_bytes());
        let unknown = consideration(&response).unwrap();
        assert_eq!(unknown.hit_points, None);
        assert_eq!(unknown.color, ConColor::Other(77));
        assert!(consideration(&response[..27]).is_err());
    }

    #[test]
    fn eqmac_considerations_read_takps_24_byte_answer() {
        let mut answer = [0; 24];
        answer[..4].copy_from_slice(&[7, 0, 9, 0]);
        answer[4..8].copy_from_slice(&4u32.to_le_bytes());
        answer[8..12].copy_from_slice(&2u32.to_le_bytes());
        answer[12..16].copy_from_slice(&50i32.to_le_bytes());
        answer[16..20].copy_from_slice(&100i32.to_le_bytes());
        let considered = eqmac_consideration(&answer).unwrap();
        assert_eq!(
            (
                considered.target_id,
                considered.faction,
                considered.hit_points
            ),
            (9, 4, Some((50, 100)))
        );
        assert_eq!(considered.color, 2u32.into());
        assert!(eqmac_consideration(&answer[..23]).is_err());
        assert!(eqmac_consider_request(0, 9).is_err());
        assert_eq!(&eqmac_consider_request(7, 9).unwrap()[..4], &[7, 0, 9, 0]);
    }

    #[test]
    fn eqmac_damage_reads_takps_24_byte_record() {
        let mut record = [0u8; 24];
        record[..4].copy_from_slice(&[9, 0, 7, 0]);
        record[4..6].copy_from_slice(&231u16.to_le_bytes());
        record[6..8].copy_from_slice(&202u16.to_le_bytes());
        record[8..12].copy_from_slice(&15i32.to_le_bytes());
        let spell = eqmac_damage(&record).unwrap();
        assert_eq!(
            (spell.target_id, spell.source_id, spell.kind, spell.spell_id),
            (9, 7, SPELL_DAMAGE_KIND, Some(202))
        );
        assert_eq!(spell.outcome, damage(&titanium_record(15)).unwrap().outcome);
        // A melee hit names no spell, whatever the field holds.
        record[4..6].copy_from_slice(&1u16.to_le_bytes());
        record[6..8].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(eqmac_damage(&record).unwrap().spell_id, None);
        record[4..6].copy_from_slice(&256u16.to_le_bytes());
        assert!(eqmac_damage(&record).is_err());
        assert!(eqmac_damage(&record[..23]).is_err());
    }

    /// A Titanium damage record with only the damage set.
    fn titanium_record(raw: i32) -> [u8; 23] {
        let mut body = [0u8; 23];
        body[7..11].copy_from_slice(&raw.to_le_bytes());
        body
    }

    #[test]
    fn damage_keeps_signed_outcomes_and_spell_identity() {
        let mut body = [0u8; 23];
        body[..2].copy_from_slice(&581u16.to_le_bytes());
        body[2..4].copy_from_slice(&553u16.to_le_bytes());
        body[4] = SPELL_DAMAGE_KIND;
        body[5..7].copy_from_slice(&219u16.to_le_bytes());
        let spell = damage(&body).unwrap();
        assert_eq!(spell.spell_id, Some(219));
        assert_eq!(spell.outcome, DamageOutcome::Miss);
        body[4] = 1;
        for (raw, outcome) in [
            (12, DamageOutcome::Hit(12)),
            (-1, DamageOutcome::Block),
            (-2, DamageOutcome::Parry),
            (-3, DamageOutcome::Riposte),
            (-4, DamageOutcome::Dodge),
            (-5, DamageOutcome::Invulnerable),
            (-6, DamageOutcome::Rune),
            (-9, DamageOutcome::Other(-9)),
        ] {
            body[7..11].copy_from_slice(&i32::to_le_bytes(raw));
            let melee = damage(&body).unwrap();
            assert_eq!((melee.spell_id, melee.outcome), (None, outcome));
            assert_eq!(
                (melee.target_id, melee.source_id, melee.kind),
                (581, 553, 1)
            );
        }
        assert!(damage(&body[..22]).is_err());
    }

    #[test]
    fn auto_attack_toggle_is_a_single_flag_byte() {
        assert_eq!(auto_attack(true), [1, 0, 0, 0]);
        assert_eq!(auto_attack(false), [0; 4]);
    }
}
