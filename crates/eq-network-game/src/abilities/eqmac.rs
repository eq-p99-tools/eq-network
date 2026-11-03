//! `EQMac` ability requests and eligibility. TAKP and Quarm use these layouts;
//! server revisions, recovery rules and remaining checks are recorded in
//! `docs/eqmac-abilities-parity.md`.
use super::Ability;
use crate::{command::EncodedCommand, world::Position};
use anyhow::{bail, ensure, Result};
use serde::Serialize;

/// Abilities with implemented `EQMac` requests and session validation.
/// Frenzy has no class dispatch on these servers. Fishing and Bind Wound
/// still need their resource/completion exchanges ported.
pub const SUPPORTED: [Ability; 15] = [
    Ability::Kick,
    Ability::Bash,
    Ability::Backstab,
    Ability::FlyingKick,
    Ability::RoundKick,
    Ability::TigerClaw,
    Ability::EagleStrike,
    Ability::DragonPunch,
    Ability::Taunt,
    Ability::Hide,
    Ability::Sneak,
    Ability::Forage,
    Ability::Mend,
    Ability::FeignDeath,
    Ability::SenseHeading,
];

/// An `EQMac` skill's numeric rank, including an eligible zero rank.
/// Values 254 (untrained) and 255 (unavailable) are sentinels; 253 and
/// larger values are unknown. Callers that need training must also require
/// a positive rank. Other client generations must use their own rules.
#[must_use]
pub const fn skill_rank(value: u32) -> Option<u32> {
    if value <= 252 {
        Some(value)
    } else {
        None
    }
}

/// Whether the profile admits this ability. Values 254 (untrained or below
/// level) and 255 (unavailable) are not ranks; 253 and larger unknown values
/// are also refused. Zero is usable where the handler accepts an unpracticed
/// skill, but Hide, Sneak and Forage explicitly require `HasSkill`.
#[must_use]
pub fn known(ability: Ability, skills: &[u32], race: u32, class: Option<u32>) -> bool {
    if !SUPPORTED.contains(&ability) {
        return false;
    }
    if ability == Ability::Bash && super::SLAMMERS.contains(&race) {
        return true;
    }
    let Some(&rank) = skills.get(ability.skill() as usize) else {
        return false;
    };
    let Some(rank) = skill_rank(rank) else {
        return false;
    };
    match ability {
        Ability::Kick => matches!(class, Some(1 | 4 | 7 | 15)),
        Ability::Bash => matches!(class, Some(1 | 2 | 3 | 5)),
        Ability::Backstab => class == Some(9),
        Ability::FlyingKick
        | Ability::RoundKick
        | Ability::TigerClaw
        | Ability::EagleStrike
        | Ability::DragonPunch
        | Ability::Mend
        | Ability::FeignDeath => class == Some(7),
        Ability::Hide | Ability::Sneak | Ability::Forage => rank > 0,
        Ability::Taunt => matches!(class, Some(1 | 3 | 4 | 5)),
        Ability::SenseHeading => true,
        _ => false,
    }
}

/// Encode an ability without implying that it succeeded. `CombatAbility` has
/// a 16-bit target, two zero reserved bytes, attack 100 and a 32-bit skill.
/// Taunt carries only the 16-bit target. Untargeted handlers read no body.
///
/// # Errors
/// Rejects unimplemented abilities and targeted requests without a target.
pub fn encode(ability: Ability, target: Option<u16>) -> Result<EncodedCommand> {
    ensure!(
        SUPPORTED.contains(&ability),
        "unsupported EQMac ability: {ability:?}"
    );
    let mut body = Vec::new();
    let opcode = match ability {
        ability if ability.at_target() => {
            let target = target
                .filter(|id| *id != 0)
                .ok_or_else(|| anyhow::anyhow!("ability needs a target"))?;
            body.extend(target.to_le_bytes());
            if ability == Ability::Taunt {
                0x3b41
            } else {
                body.extend([0; 2]);
                body.extend(100u32.to_le_bytes());
                body.extend(ability.skill().to_le_bytes());
                0x6041
            }
        }
        Ability::Hide => 0x8641,
        Ability::Sneak => 0x8541,
        Ability::Forage => 0x9440,
        Ability::Mend => 0x9d41,
        Ability::FeignDeath => 0xac40,
        Ability::SenseHeading => 0x8741,
        _ => bail!("unsupported EQMac ability: {ability:?}"),
    };
    Ok(EncodedCommand { opcode, body })
}

/// `EQMac`'s lying appearance is a posture request, not proof of successful
/// feigning. The server accepts it and interrupts casting, but does not echo
/// the sender. Standing, sitting or ducking subsequently clears server feign.
///
/// # Errors
/// Rejects a zero spawn ID.
pub fn feign_posture(spawn_id: u16) -> Result<EncodedCommand> {
    ensure!(spawn_id != 0, "feign posture needs a spawn");
    let mut body = spawn_id.to_le_bytes().to_vec();
    body.extend(14u16.to_le_bytes());
    body.extend(115u32.to_le_bytes());
    Ok(EncodedCommand {
        opcode: crate::quarm::ZONE_SPAWN_APPEARANCE,
        body,
    })
}

/// A conservative subset of `EQMac`'s three-dimensional client melee reach.
/// `Client::CombatRange` guarantees at least 16 units; larger model bounding
/// radii and Z offsets are not available in the session. Do not substitute
/// Titanium's race/size formula or permit targets on another floor.
#[must_use]
pub fn in_melee_range(own: Position, target: Position) -> bool {
    let (dx, dy, dz) = (own.x - target.x, own.y - target.y, own.z - target.z);
    dx * dx + dy * dy + dz * dz <= 16.0 * 16.0
}

/// The complete server reset record. Unknown skill IDs and timer values
/// remain available; only timer zero is established to mean ready.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct SkillReset {
    /// Server skill number, including unknown skills.
    pub skill: u16,
    /// Reserved word, retained as received.
    pub padding: u16,
    /// Raw timer; the inspected servers send zero. Other units are unknown.
    pub timer: u32,
}

/// Decode the packed reset record without inventing units for its timer.
///
/// # Errors
/// Rejects any length other than eight bytes.
pub fn reset_skill(body: &[u8]) -> Result<SkillReset> {
    ensure!(body.len() == 8, "invalid skill reset length");
    Ok(SkillReset {
        skill: u16::from_le_bytes(body[0..2].try_into()?),
        padding: u16::from_le_bytes(body[2..4].try_into()?),
        timer: u32::from_le_bytes(body[4..8].try_into()?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{self, Request, Sender};

    #[test]
    fn eqmac_ability_requests_have_exact_generation_specific_bytes() {
        let sender = Sender {
            name: "Tester",
            spawn_id: Some(7),
        };
        for (ability, skill) in [
            (Ability::Kick, 30),
            (Ability::Bash, 10),
            (Ability::Backstab, 8),
            (Ability::FlyingKick, 26),
            (Ability::RoundKick, 38),
            (Ability::TigerClaw, 52),
            (Ability::EagleStrike, 23),
            (Ability::DragonPunch, 21),
        ] {
            let request = Request::Ability {
                ability,
                target: Some(0x1234),
            };
            assert_eq!(
                request::eqmac(&request, sender).unwrap(),
                EncodedCommand {
                    opcode: 0x6041,
                    body: vec![0x34, 0x12, 0, 0, 100, 0, 0, 0, skill, 0, 0, 0],
                }
            );
            assert_eq!(request::titanium(&request, sender).unwrap().opcode, 0x5ee8);
            for target in [None, Some(0)] {
                assert!(encode(ability, target).is_err());
            }
        }
        assert_eq!(
            encode(Ability::Taunt, Some(0x1234)).unwrap(),
            EncodedCommand {
                opcode: 0x3b41,
                body: vec![0x34, 0x12],
            }
        );
        assert!(encode(Ability::Taunt, None).is_err());
        for (ability, opcode) in [
            (Ability::Mend, 0x9d41),
            (Ability::FeignDeath, 0xac40),
            (Ability::Hide, 0x8641),
            (Ability::Sneak, 0x8541),
            (Ability::Forage, 0x9440),
            (Ability::SenseHeading, 0x8741),
        ] {
            let request = Request::Ability {
                ability,
                target: None,
            };
            assert_eq!(
                request::eqmac(&request, sender).unwrap(),
                EncodedCommand {
                    opcode,
                    body: vec![]
                }
            );
        }
        for ability in [Ability::Frenzy, Ability::Fishing, Ability::BindWound] {
            assert!(request::eqmac(
                &Request::Ability {
                    ability,
                    target: Some(42)
                },
                sender
            )
            .is_err());
        }
        assert_eq!(
            request::eqmac(&Request::FeignDeathPosture, sender).unwrap(),
            EncodedCommand {
                opcode: 0xf540,
                body: vec![7, 0, 14, 0, 115, 0, 0, 0],
            }
        );
        assert!(request::eqmac(
            &Request::FeignDeathPosture,
            Sender {
                spawn_id: None,
                ..sender
            }
        )
        .is_err());
        assert!(request::titanium(&Request::FeignDeathPosture, sender).is_err());
    }

    #[test]
    fn eqmac_eligibility_distinguishes_unpracticed_untrained_and_unavailable() {
        let mut skills = vec![0; 100];
        for ability in SUPPORTED {
            for rank in [253, 254, 255, 256, u32::MAX] {
                skills[ability.skill() as usize] = rank;
                assert!(!known(ability, &skills, 1, Some(7)), "{ability:?} {rank}");
            }
            assert!(!known(ability, &[], 1, Some(7)));
        }
        skills.fill(0);
        assert!(known(Ability::Mend, &skills, 1, Some(7)));
        assert!(!known(Ability::Mend, &skills, 1, Some(1)));
        assert!(!known(Ability::FeignDeath, &skills, 1, None));
        assert!(known(Ability::SenseHeading, &skills, 1, Some(1)));
        for ability in [Ability::Hide, Ability::Sneak, Ability::Forage] {
            assert!(!known(ability, &skills, 1, Some(7)));
            skills[ability.skill() as usize] = 1;
            assert!(known(ability, &skills, 1, Some(7)));
        }
        assert!(known(Ability::Bash, &[], 10, Some(6)), "racial slam");
        skills.fill(100);
        assert!(!known(Ability::Backstab, &skills, 1, Some(7)));
        assert!(known(Ability::Backstab, &skills, 1, Some(9)));
        assert!(!known(Ability::Frenzy, &skills, 1, Some(16)));
    }

    #[test]
    fn eqmac_reset_retains_unknown_fields_without_inventing_readiness() {
        let bytes = [0xfe, 0xff, 0x12, 0x34, 1, 2, 3, 4];
        let raw = SkillReset {
            skill: 65534,
            padding: 0x3412,
            timer: 0x0403_0201,
        };
        assert_eq!(reset_skill(&bytes).unwrap(), raw);
        assert_eq!(
            crate::quarm::updates(0x3d42, &bytes).unwrap(),
            [crate::world::WorldEvent::SkillReset(raw)]
        );
        assert!(super::super::eqmac_reset_skill(&bytes).is_err());
        for length in [0, 1, 7, 9] {
            assert!(reset_skill(&vec![0; length]).is_err());
        }
    }

    #[test]
    fn eqmac_melee_checks_height_and_rejects_non_finite_positions() {
        let own = Position::default();
        for (x, z, allowed) in [
            (16.0, 0.0, true),
            (16.1, 0.0, false),
            (0.0, 16.1, false),
            (12.0, 12.0, false),
            (f32::NAN, 0.0, false),
        ] {
            assert_eq!(in_melee_range(own, Position { x, z, ..own }), allowed);
        }
    }
}
