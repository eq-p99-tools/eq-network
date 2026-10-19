//! Titanium consider, auto-attack, combat-damage and animation records.
//!
//! The Titanium client prints melee and spell-damage messages itself from
//! `OP_Damage`; these records keep the server's numeric fields so a consumer
//! can do the same without inventing results. `OP_Animation` asks every
//! client near a spawn to play one of its motions, such as a swing, so a
//! consumer can show what the server says happened. Damage records and
//! spell actions carry the push a hit or a spell gives
//! ([`Push`](crate::combat::Push)).
use anyhow::{ensure, Context, Result};
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

/// `OP_Animation`: a spawn's one-shot motion, which `EQEmu` sends to every
/// client near the spawn, the spawn's own included (`Mob::DoAnim`).
pub const ANIMATION_OPCODE: u16 = 0x2acf;
/// `EQMac`'s `OP_Animation`: TAKP's 12-byte `Animation_Struct`, with the
/// action 32 bits wide and the speed a float (TAKP's `0x40a1`, in this
/// library's byte order as [`EQMAC_DAMAGE_OPCODE`] is).
pub const EQMAC_ANIMATION_OPCODE: u16 = 0xa140;

/// Damage kind used for spell damage and beneficial spell landings.
pub const SPELL_DAMAGE_KIND: u8 = 231;
/// The damage kinds of a damage shield's record: decay, chill, freezing,
/// torment, burn and thorns (`EQEmu`'s and TAKP's `DmgShieldType`,
/// `common/spdat.h`), by the shield's spell or its resist. A shield against
/// spells sends the kind of its wearer's other damage shield
/// (`Mob::DamageShield`), so its record reads as one of that kind. On a
/// wearer with no other damage shield it sends kind 0, and its record,
/// negative with the attacker as its target, reads as an avoidance code
/// (past 6 points, an unknown one) that nothing can tell from a real one.
pub const DAMAGE_SHIELD_KINDS: std::ops::RangeInclusive<u8> = 244..=249;

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

/// Outcome carried in the signed damage field, read by the record's kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum DamageOutcome {
    /// Points of damage dealt; for a damage shield's kind, the points the
    /// shield dealt the one who struck its wearer.
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
    /// A spell took hold and dealt no damage: the record `EQEmu` and TAKP
    /// send everyone near as most spells land, and `EQEmu`'s for a spell a
    /// rune absorbed whole.
    Landed,
    /// The points a lifetap took, in the record TAKP and Quarm send its
    /// caster alone, naming the caster as both target and source.
    Tapped(u32),
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

impl DamageOutcome {
    /// Reads a record's damage field by its kind. Both servers send a
    /// damage shield's points negated (`EQEmu`'s and TAKP's
    /// `Mob::DamageShield`), a spell that took hold and dealt nothing as 0
    /// (`Mob::SpellOnTarget`), and TAKP and Quarm a lifetap's points to its
    /// caster negated, the caster as target and source
    /// (`Mob::GenerateDamagePackets`); the rest are damage or the avoidance
    /// codes. A tap a rune absorbs whole sends 0, which reads as
    /// [`Self::Landed`].
    fn read(kind: u8, (target_id, source_id): (u16, u16), raw: i32) -> Self {
        if DAMAGE_SHIELD_KINDS.contains(&kind) {
            return Self::Hit(raw.unsigned_abs());
        }
        if kind == SPELL_DAMAGE_KIND {
            if raw == 0 {
                return Self::Landed;
            }
            if raw < 0 && target_id == source_id {
                return Self::Tapped(raw.unsigned_abs());
            }
        }
        raw.into()
    }
}

/// A push a hit or a spell gives the spawn it lands on, as the server sends
/// it. Nothing here applies it: how far the official client moves for one
/// is not recorded yet.
///
/// `EQEmu` pushes a player on every melee hit that does damage, unless they
/// are rooted, and an NPC by chance, by the skill's force and toward the
/// attacker's heading (`Mob::CommonDamage`); it gives a spell's push back
/// and push up to its action and its landing record
/// (`Mob::SpellOnTarget`). It keeps one melee record and sets its force
/// only for a push, so a later miss or avoidance it sends carries the last
/// push's force; it never sets that record's pitch. TAKP and Quarm give a
/// force only to a melee hit, by the skill, and send the caster's heading
/// with every spell, and no force; neither sets the pitch. So a push in a
/// record of a miss or an avoidance is never one to apply.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Push {
    /// How hard.
    pub force: f32,
    /// Which way, in [`Position`](crate::world::Position)'s 512-unit turn:
    /// the attacker's or the caster's heading, which TAKP and Quarm send
    /// doubled from their own 256-unit turn.
    pub heading: f32,
    /// How far up: Titanium's `hit_pitch`, which `EQEmu` fills with a
    /// spell's push up, and `EQMac`'s `pushup_angle`, an angle whose sine
    /// times the force is the upward push (TAKP's `Damage_Struct`).
    pub pitch: f32,
}

/// Reads the push at `at`: the force, the heading and the pitch, three
/// floats. None when the force and the pitch are both 0, since a heading
/// alone pushes nothing, and when any of the three is not a finite number,
/// which nothing could apply; the record carrying it reads all the same.
///
/// # Errors
/// Rejects a record too short to hold the three.
pub(crate) fn push(body: &[u8], at: usize) -> Result<Option<Push>> {
    let mut values = [0.0f32; 3];
    for (index, value) in values.iter_mut().enumerate() {
        *value = f32::from_bits(word(body, at + index * 4)?);
    }
    let [force, heading, pitch] = values;
    let finite = values.iter().all(|value| value.is_finite());
    Ok((finite && (force != 0.0 || pitch != 0.0)).then_some(Push {
        force,
        heading,
        pitch,
    }))
}

/// One `OP_Damage` record.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Damage {
    /// Entity receiving the attack.
    pub target_id: u16,
    /// Attacking entity.
    pub source_id: u16,
    /// The damage's type, not the skill: the servers' type for the skill
    /// (`EQEmu`'s `SkillDamageTypes`, `common/eq_constants.h`: both blunt
    /// skills send 0 and every kick 30), [`SPELL_DAMAGE_KIND`] for spells,
    /// [`DAMAGE_SHIELD_KINDS`] for damage shields, 252 for a fall (another
    /// client's own record, which `EQEmu` relays to everyone near it), 253
    /// for traps and 255 for a skill with no type of its own.
    pub kind: u8,
    /// Spell responsible for spell damage; None for melee and skills.
    pub spell_id: Option<u16>,
    /// The damage field, read by the kind.
    pub outcome: DamageOutcome,
    /// The push the record gives its target; none when it sends no force
    /// or pitch, or a value that is not a finite number. See [`Push`] for
    /// which records carry one.
    pub push: Option<Push>,
}

/// One `OP_Animation`: a motion the server asks the clients near a spawn to
/// play once, such as a swing as an attack goes out.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Animation {
    /// The spawn that moves.
    pub spawn_id: u16,
    /// The servers' animation number, the same on both client generations
    /// (`EQEmu`'s and TAKP's `DoAnim` arguments): 1 to 11 the weapon swings,
    /// kicks and the bow, 42 to 44 the three casting gestures and 45 to 47
    /// the monk attacks. Every number is kept, known or not.
    pub action: u16,
    /// How fast to play it, as a multiple of the normal speed: Titanium's
    /// byte over 10 and `EQMac`'s float as sent. Inferred from the servers'
    /// defaults, 10 for `EQEmu` and 1.0 for TAKP, which every swing in their
    /// attack code sends.
    pub speed: f32,
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

/// Decodes one damage record: the target and source, the kind, the spell,
/// the damage and the push at 11 (`CombatDamage_Struct`).
///
/// # Errors
/// Rejects any length other than the 23-byte Titanium structure.
pub fn damage(body: &[u8]) -> Result<Damage> {
    ensure!(body.len() == 23, "invalid damage length");
    let kind = body[4];
    let spell_id = u16::from_le_bytes([body[5], body[6]]);
    let raw = i32::from_le_bytes(word(body, 7)?.to_le_bytes());
    let spawns = (
        u16::from_le_bytes([body[0], body[1]]),
        u16::from_le_bytes([body[2], body[3]]),
    );
    Ok(Damage {
        target_id: spawns.0,
        source_id: spawns.1,
        kind,
        spell_id: (kind == SPELL_DAMAGE_KIND && !matches!(spell_id, 0 | u16::MAX))
            .then_some(spell_id),
        outcome: DamageOutcome::read(kind, spawns, raw),
        push: push(body, 11)?,
    })
}

/// Decodes `EQMac`'s damage record: Titanium's fields, the type and the
/// spell 16 bits wide, then the damage at 8 and the push at 12 (TAKP
/// `Damage_Struct`, whose types are Titanium's: `SkillDamageTypes`, and 231
/// for spells).
///
/// # Errors
/// Rejects a malformed record, and a type beyond any skill's.
pub fn eqmac_damage(body: &[u8]) -> Result<Damage> {
    ensure!(body.len() == 24, "invalid EQMac damage length");
    let kind = u8::try_from(u16::from_le_bytes([body[4], body[5]]))
        .map_err(|_| anyhow::anyhow!("EQMac damage type beyond any skill's"))?;
    let spell_id = u16::from_le_bytes([body[6], body[7]]);
    let raw = i32::from_le_bytes(word(body, 8)?.to_le_bytes());
    let spawns = (
        u16::from_le_bytes([body[0], body[1]]),
        u16::from_le_bytes([body[2], body[3]]),
    );
    Ok(Damage {
        target_id: spawns.0,
        source_id: spawns.1,
        kind,
        spell_id: (kind == SPELL_DAMAGE_KIND && !matches!(spell_id, 0 | u16::MAX))
            .then_some(spell_id),
        outcome: DamageOutcome::read(kind, spawns, raw),
        push: push(body, 12)?,
    })
}

/// Decodes Titanium's animation record: the spawn as 16 bits, then the
/// speed byte (10 is normal) and the action byte.
///
/// # Errors
/// Rejects any length other than the 4-byte Titanium structure.
pub fn animation(body: &[u8]) -> Result<Animation> {
    let &[low, high, speed, action] = body else {
        anyhow::bail!("invalid animation length");
    };
    Ok(Animation {
        spawn_id: u16::from_le_bytes([low, high]),
        action: action.into(),
        speed: f32::from(speed) / 10.0,
    })
}

/// Decodes `EQMac`'s animation record: the spawn as 16 bits, two bytes of
/// padding, the action as a signed 32-bit number and the speed as a float
/// (TAKP `Animation_Struct`).
///
/// # Errors
/// Rejects any length other than the 12-byte `EQMac` structure, an action
/// outside 0 to 65535, and a speed that is negative or not a number.
pub fn eqmac_animation(body: &[u8]) -> Result<Animation> {
    ensure!(body.len() == 12, "invalid EQMac animation length");
    let action = i32::from_le_bytes(word(body, 4)?.to_le_bytes());
    let speed = f32::from_bits(word(body, 8)?);
    ensure!(
        speed.is_finite() && speed >= 0.0,
        "invalid EQMac animation speed"
    );
    Ok(Animation {
        spawn_id: u16::from_le_bytes([body[0], body[1]]),
        action: u16::try_from(action).context("EQMac animation beyond any action")?,
        speed,
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
        assert_eq!(spell.outcome, DamageOutcome::Landed);
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
    fn each_kind_reads_its_damage_field_its_own_way() {
        const SPELL: u8 = SPELL_DAMAGE_KIND;
        // Target, source, kind and value: what each generation's record
        // reads as.
        let cases = [
            // A damage shield's points come negated, its wearer the source;
            // either sign reads as the points.
            ((553, 581), 249, -3, DamageOutcome::Hit(3)),
            ((553, 581), 244, -40, DamageOutcome::Hit(40)),
            ((553, 581), 246, 7, DamageOutcome::Hit(7)),
            // A spell that took hold and dealt nothing.
            ((581, 553), SPELL, 0, DamageOutcome::Landed),
            ((553, 553), SPELL, 0, DamageOutcome::Landed),
            // A lifetap's points to its caster, the caster target and source.
            ((553, 553), SPELL, -5, DamageOutcome::Tapped(5)),
            ((553, 553), SPELL, -120, DamageOutcome::Tapped(120)),
            // A spell on someone else keeps the codes.
            ((581, 553), SPELL, -5, DamageOutcome::Invulnerable),
            ((581, 553), SPELL, -6, DamageOutcome::Rune),
            ((581, 553), SPELL, 30, DamageOutcome::Hit(30)),
            // Melee, a fall and a trap read as before.
            ((581, 553), 0, 0, DamageOutcome::Miss),
            ((581, 581), 30, -1, DamageOutcome::Block),
            ((581, 581), 252, 160, DamageOutcome::Hit(160)),
            ((581, 0), 253, 25, DamageOutcome::Hit(25)),
        ];
        // A shield of 1 to 6 points reads as its points, never an avoidance.
        let shields = DAMAGE_SHIELD_KINDS.flat_map(|kind| {
            (1..=6i32).map(move |points| {
                (
                    (553, 581),
                    kind,
                    -points,
                    DamageOutcome::Hit(points.unsigned_abs()),
                )
            })
        });
        for ((target, source), kind, raw, outcome) in cases.into_iter().chain(shields) {
            let mut titanium = titanium_record(raw);
            titanium[..2].copy_from_slice(&u16::to_le_bytes(target));
            titanium[2..4].copy_from_slice(&u16::to_le_bytes(source));
            titanium[4] = kind;
            let mut eqmac = [0u8; 24];
            eqmac[..2].copy_from_slice(&u16::to_le_bytes(target));
            eqmac[2..4].copy_from_slice(&u16::to_le_bytes(source));
            eqmac[4..6].copy_from_slice(&u16::from(kind).to_le_bytes());
            eqmac[8..12].copy_from_slice(&raw.to_le_bytes());
            let case = format!("{target} {source} {kind} {raw}");
            assert_eq!(damage(&titanium).unwrap().outcome, outcome, "{case}");
            assert_eq!(eqmac_damage(&eqmac).unwrap().outcome, outcome, "{case}");
        }
    }

    #[test]
    fn damage_records_carry_their_push() {
        let mut titanium = titanium_record(12);
        let mut eqmac = [0u8; 24];
        eqmac[8..12].copy_from_slice(&12i32.to_le_bytes());
        // No force and no pitch: no push, whatever the heading.
        titanium[15..19].copy_from_slice(&256.0f32.to_le_bytes());
        eqmac[16..20].copy_from_slice(&256.0f32.to_le_bytes());
        assert_eq!(damage(&titanium).unwrap().push, None);
        assert_eq!(eqmac_damage(&eqmac).unwrap().push, None);
        // A force, a heading and a pitch, each where its generation keeps it.
        let push = Push {
            force: 0.3,
            heading: 256.0,
            pitch: 0.5,
        };
        for (body, at) in [(&mut titanium[..], 11), (&mut eqmac[..], 12)] {
            for (offset, value) in [(0, push.force), (4, push.heading), (8, push.pitch)] {
                body[at + offset..at + offset + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        assert_eq!(damage(&titanium).unwrap().push, Some(push));
        assert_eq!(eqmac_damage(&eqmac).unwrap().push, Some(push));
        // A pitch alone is a push too.
        titanium[11..15].copy_from_slice(&0.0f32.to_le_bytes());
        assert_eq!(damage(&titanium).unwrap().push.unwrap().force, 0.0);
        // A value that is not a finite number is no push, and the record
        // still reads.
        titanium[19..23].copy_from_slice(&f32::NAN.to_le_bytes());
        eqmac[12..16].copy_from_slice(&f32::INFINITY.to_le_bytes());
        let (titanium, eqmac) = (damage(&titanium).unwrap(), eqmac_damage(&eqmac).unwrap());
        assert_eq!((titanium.push, eqmac.push), (None, None));
        assert_eq!(
            (titanium.outcome, eqmac.outcome),
            (DamageOutcome::Hit(12), DamageOutcome::Hit(12))
        );
    }

    #[test]
    fn titanium_animations_read_the_spawn_speed_and_action() {
        let swing = animation(&[0x2d, 0x01, 10, 5]).unwrap();
        assert_eq!(
            swing,
            Animation {
                spawn_id: 301,
                action: 5,
                speed: 1.0,
            }
        );
        // Every action and speed byte is kept, known or not.
        let other = animation(&[7, 0, 25, 200]).unwrap();
        assert_eq!((other.action, other.speed), (200, 2.5));
        for body in [&[][..], &[0; 3], &[0; 5]] {
            assert!(animation(body).is_err());
        }
    }

    #[test]
    fn eqmac_animations_read_takps_12_byte_record() {
        let mut record = [0u8; 12];
        record[..2].copy_from_slice(&301u16.to_le_bytes());
        // The padding is not read.
        record[2..4].copy_from_slice(&[0xcd, 0xcd]);
        record[4..8].copy_from_slice(&8i32.to_le_bytes());
        record[8..].copy_from_slice(&1.0f32.to_le_bytes());
        assert_eq!(
            eqmac_animation(&record).unwrap(),
            Animation {
                spawn_id: 301,
                action: 8,
                speed: 1.0,
            }
        );
        record[4..8].copy_from_slice(&65535i32.to_le_bytes());
        assert_eq!(eqmac_animation(&record).unwrap().action, u16::MAX);
        for action in [-1, 65536] {
            record[4..8].copy_from_slice(&i32::to_le_bytes(action));
            assert!(eqmac_animation(&record).is_err());
        }
        record[4..8].copy_from_slice(&5i32.to_le_bytes());
        for speed in [f32::NAN, f32::INFINITY, -1.0] {
            record[8..].copy_from_slice(&speed.to_le_bytes());
            assert!(eqmac_animation(&record).is_err());
        }
        record[8..].copy_from_slice(&0.5f32.to_le_bytes());
        assert_eq!(eqmac_animation(&record).unwrap().speed, 0.5);
        assert!(eqmac_animation(&record[..11]).is_err());
        assert!(eqmac_animation(&[0; 13]).is_err());
        // Titanium's four bytes are not `EQMac`'s record.
        assert!(eqmac_animation(&[0x2d, 0x01, 10, 5]).is_err());
    }

    #[test]
    fn auto_attack_toggle_is_a_single_flag_byte() {
        assert_eq!(auto_attack(true), [1, 0, 0, 0]);
        assert_eq!(auto_attack(false), [0; 4]);
    }
}
