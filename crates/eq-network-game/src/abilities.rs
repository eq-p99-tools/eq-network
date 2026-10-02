//! Using a skill as the official client's ability buttons do: the combat
//! abilities (kick, bash, backstab, frenzy and the monk strikes) and taunt
//! at the target, binding wounds on the player or another player, and
//! hiding, sneaking, foraging, fishing, mending, feigning death and sensing
//! heading on the player. Servers answer in chat, in the combat stream, with
//! an item on the cursor or, for a bandaging, with its own packet (see
//! [`crate::bind_wound`]), and keep a recovery timer for each; the combat
//! abilities share one.
//!
//! Layout reference: `EQEmu`'s `CombatAbility_Struct` and
//! `ClientTarget_Struct` (`common/eq_packet_structs.h`) and the Titanium
//! opcodes (`utils/patches/patch_Titanium.conf`); `Client::OPCombatAbility`
//! (`zone/special_attacks.cpp`) and the handlers in `zone/client_packet.cpp`
//! for the rules; `common/features.h` for recovery times.
use crate::command::EncodedCommand;
use serde::Serialize;
use std::time::Duration;

/// `OP_CombatAbility`: a strike at the target.
pub const COMBAT_OPCODE: u16 = 0x5ee8;
/// `OP_Taunt`.
pub const TAUNT_OPCODE: u16 = 0x5e48;
/// `CombatAbility_Struct.m_atk` for a melee strike; `EQEmu` handles ranged
/// attacks through the same packet with the range slot instead.
const MELEE_STRIKE: u32 = 100;

/// `OP_Fishing`, which carries nothing.
pub const FISHING_OPCODE: u16 = 0x0b36;

/// Races that bash without the skill or a shield (`EQEmu`'s "slam"):
/// Barbarian, Troll and Ogre.
const SLAMMERS: [u32; 3] = [2, 9, 10];

/// A skill the player uses on demand.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Ability {
    /// Kick.
    Kick,
    /// Bash: with a shield, or for an Ogre, Troll or Barbarian without one.
    Bash,
    /// Backstab.
    Backstab,
    /// Frenzy.
    Frenzy,
    /// Flying Kick.
    FlyingKick,
    /// Round Kick.
    RoundKick,
    /// Tiger Claw.
    TigerClaw,
    /// Eagle Strike.
    EagleStrike,
    /// Dragon Punch, Tail Rake for an Iksar.
    DragonPunch,
    /// Taunt.
    Taunt,
    /// Hide.
    Hide,
    /// Sneak.
    Sneak,
    /// Forage.
    Forage,
    /// Fishing, with a fishing pole in the primary hand and bait carried.
    Fishing,
    /// Bind Wound, with a bandage carried, on the player or another player.
    BindWound,
    /// Mend.
    Mend,
    /// Feign Death.
    FeignDeath,
    /// Sense Heading.
    SenseHeading,
}

/// The recovery timer an ability waits on. Servers keep one for all the
/// combat abilities, so a kick also delays a bash.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Recovery {
    /// Kick, bash, backstab, frenzy and the monk strikes.
    Strike,
    /// Taunt.
    Taunt,
    /// Hide.
    Hide,
    /// Sneak.
    Sneak,
    /// Forage.
    Forage,
    /// Fishing.
    Fishing,
    /// A bandaging under way.
    BindWound,
    /// Mend.
    Mend,
    /// Feign Death.
    FeignDeath,
}

impl Ability {
    /// Every ability, strikes first and those anyone has, binding wounds and
    /// fishing, last.
    pub const ALL: [Self; 18] = [
        Self::Kick,
        Self::Bash,
        Self::Backstab,
        Self::Frenzy,
        Self::FlyingKick,
        Self::RoundKick,
        Self::TigerClaw,
        Self::EagleStrike,
        Self::DragonPunch,
        Self::Taunt,
        Self::Hide,
        Self::Sneak,
        Self::Forage,
        Self::Mend,
        Self::FeignDeath,
        Self::SenseHeading,
        Self::BindWound,
        Self::Fishing,
    ];

    /// The servers' number for its skill (`EQ::skills::SkillType`), which
    /// the profile and skill updates count.
    #[must_use]
    pub const fn skill(self) -> u32 {
        match self {
            Self::Backstab => 8,
            Self::BindWound => 9,
            Self::Bash => 10,
            Self::DragonPunch => 21,
            Self::EagleStrike => 23,
            Self::FeignDeath => 25,
            Self::FlyingKick => 26,
            Self::Forage => 27,
            Self::Hide => 29,
            Self::Kick => 30,
            Self::Mend => 32,
            Self::RoundKick => 38,
            Self::SenseHeading => 40,
            Self::Sneak => 42,
            Self::TigerClaw => 52,
            Self::Fishing => 55,
            Self::Taunt => 73,
            Self::Frenzy => 74,
        }
    }

    /// Whether a character with these skill values (by skill number) and
    /// this race has it: a skill above zero, as servers require, or the
    /// slam a Barbarian, Troll or Ogre bashes with. Anyone can fish, as a
    /// tradeskill is used from nothing, and anyone can bandage; the server
    /// checks the pole and bait, and the session the bandage.
    #[must_use]
    pub fn known(self, skills: &[u32], race: u32) -> bool {
        let skill = usize::try_from(self.skill()).unwrap_or(usize::MAX);
        matches!(self, Self::Fishing | Self::BindWound)
            || skills.get(skill).is_some_and(|value| *value > 0)
            || (self == Self::Bash && SLAMMERS.contains(&race))
    }

    /// The ability for a skill, if the skill is one.
    #[must_use]
    pub fn from_skill(skill: u32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|ability| ability.skill() == skill)
    }

    /// Its name, as the official client's skill list shows it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Kick => "Kick",
            Self::Bash => "Bash",
            Self::Backstab => "Backstab",
            Self::Frenzy => "Frenzy",
            Self::FlyingKick => "Flying Kick",
            Self::RoundKick => "Round Kick",
            Self::TigerClaw => "Tiger Claw",
            Self::EagleStrike => "Eagle Strike",
            Self::DragonPunch => "Dragon Punch",
            Self::Taunt => "Taunt",
            Self::Hide => "Hide",
            Self::Sneak => "Sneak",
            Self::Forage => "Forage",
            Self::Fishing => "Fishing",
            Self::BindWound => "Bind Wound",
            Self::Mend => "Mend",
            Self::FeignDeath => "Feign Death",
            Self::SenseHeading => "Sense Heading",
        }
    }

    /// Whether it strikes the target in melee.
    #[must_use]
    pub const fn strikes(self) -> bool {
        matches!(self.recovery(), Some(Recovery::Strike))
    }

    /// Whether it is used on the target, who must be in melee range: the
    /// strikes and taunt.
    #[must_use]
    pub const fn at_target(self) -> bool {
        self.strikes() || matches!(self, Self::Taunt)
    }

    /// The timer it waits on; Sense Heading waits on none.
    #[must_use]
    pub const fn recovery(self) -> Option<Recovery> {
        Some(match self {
            Self::Kick
            | Self::Bash
            | Self::Backstab
            | Self::Frenzy
            | Self::FlyingKick
            | Self::RoundKick
            | Self::TigerClaw
            | Self::EagleStrike
            | Self::DragonPunch => Recovery::Strike,
            Self::Taunt => Recovery::Taunt,
            Self::Hide => Recovery::Hide,
            Self::Sneak => Recovery::Sneak,
            Self::Forage => Recovery::Forage,
            Self::Fishing => Recovery::Fishing,
            Self::BindWound => Recovery::BindWound,
            Self::Mend => Recovery::Mend,
            Self::FeignDeath => Recovery::FeignDeath,
            Self::SenseHeading => return None,
        })
    }

    /// How long `EQEmu` makes its timer wait when nothing shortens it: its
    /// `features.h` time less the second its handlers take off. Haste
    /// shortens a strike's, and skill reuse focus any of them, so this is
    /// the longest the server waits. A bandaging holds the next until it
    /// ends, which takes this long unless it fails first.
    #[must_use]
    pub const fn reuse(self) -> Duration {
        Duration::from_secs(match self {
            Self::Kick | Self::Bash | Self::EagleStrike | Self::Taunt => 4,
            Self::TigerClaw | Self::DragonPunch => 5,
            Self::FlyingKick | Self::Sneak => 6,
            Self::Hide => 7,
            Self::Backstab | Self::RoundKick | Self::FeignDeath => 8,
            Self::Frenzy => 9,
            Self::Fishing | Self::BindWound => 10,
            Self::Forage => 49,
            Self::Mend => 360,
            Self::SenseHeading => 0,
        })
    }

    /// The packet that uses it; a strike or taunt names the target, which
    /// must be the server's target too, and a bandaging the one bandaged.
    #[must_use]
    pub fn encode(self, target: u16) -> EncodedCommand {
        if self == Self::BindWound {
            return crate::bind_wound::encode(target);
        }
        let word = |value: u32| value.to_le_bytes();
        let (opcode, body) = match self {
            Self::Taunt => (TAUNT_OPCODE, word(u32::from(target)).to_vec()),
            ability if ability.strikes() => (
                COMBAT_OPCODE,
                [
                    word(u32::from(target)),
                    word(MELEE_STRIKE),
                    word(ability.skill()),
                ]
                .concat(),
            ),
            // The rest carry no body: `OP_Hide`, `OP_Sneak`, `OP_Forage`,
            // `OP_Fishing`, `OP_Mend`, `OP_FeignDeath` and `OP_SenseHeading`.
            Self::Hide => (0x4312, Vec::new()),
            Self::Sneak => (0x74e1, Vec::new()),
            Self::Forage => (0x4796, Vec::new()),
            Self::Fishing => (FISHING_OPCODE, Vec::new()),
            Self::Mend => (0x14ef, Vec::new()),
            Self::FeignDeath => (0x7489, Vec::new()),
            _ => (0x05ac, Vec::new()),
        };
        EncodedCommand { opcode, body }
    }
}

/// Whether a melee ability reaches the target, as `EQEmu` decides
/// (`Mob::CombatRange`): within a reach that grows with the larger body,
/// across the ground; the server allows more against a target that flees.
#[must_use]
pub fn in_melee_range(own: Body, target: Body) -> bool {
    // A few dragons and worms have a fixed size for reach.
    let size = |body: Body| match body.race {
        49 | 158 | 196 => 60.0,
        _ if body.size < 6.0 => 8.0,
        _ => body.size,
    };
    let size = size(own).max(size(target));
    let mut reach = if size > 29.0 {
        size * size
    } else if size > 19.0 {
        size * size * 2.0
    } else {
        size * size * 4.0
    };
    // Velious dragons and the dragon skeleton reach further still.
    match target.race {
        184 => reach *= 1.75,
        122 => reach *= 2.25,
        _ => (),
    }
    if reach > 10_000.0 {
        reach /= 7.0;
    }
    let (dx, dy) = (own.x - target.x, own.y - target.y);
    dx * dx + dy * dy <= reach
}

/// The size `EQEmu` gives a player of a race on entering a zone
/// (`Client::Handle_Connect_OP_ZoneEntry`), which melee reach counts: an
/// Ogre's 9 down to a Gnome's 3, and 0 for a race it does not list.
#[must_use]
pub const fn player_size(race: u32) -> f32 {
    match race {
        10 => 9.0,
        9 => 8.0,
        2 | 130 => 7.0,
        1 | 3 | 5 | 128 | 522 => 6.0,
        7 => 5.5,
        4 | 6 | 330 => 5.0,
        8 => 4.0,
        11 => 3.5,
        12 => 3.0,
        _ => 0.0,
    }
}

/// What melee reach depends on for one side: its race, its size and where
/// it stands on the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    /// Race number.
    pub race: u32,
    /// Size as servers count it; under 6 counts as 8.
    pub size: f32,
    /// East-west position.
    pub x: f32,
    /// North-south position.
    pub y: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strikes_and_taunt_name_the_target_and_the_rest_carry_nothing() {
        assert_eq!(
            Ability::Kick.encode(42),
            EncodedCommand {
                opcode: COMBAT_OPCODE,
                body: vec![42, 0, 0, 0, 100, 0, 0, 0, 30, 0, 0, 0],
            }
        );
        assert_eq!(Ability::DragonPunch.encode(7).body[8], 21);
        assert_eq!(
            Ability::Taunt.encode(42),
            EncodedCommand {
                opcode: TAUNT_OPCODE,
                body: vec![42, 0, 0, 0],
            }
        );
        for (ability, opcode) in [
            (Ability::Hide, 0x4312),
            (Ability::Sneak, 0x74e1),
            (Ability::Forage, 0x4796),
            (Ability::Fishing, 0x0b36),
            (Ability::Mend, 0x14ef),
            (Ability::FeignDeath, 0x7489),
            (Ability::SenseHeading, 0x05ac),
        ] {
            assert_eq!(
                ability.encode(42),
                EncodedCommand {
                    opcode,
                    body: Vec::new()
                },
                "{ability:?}"
            );
            assert!(!ability.at_target());
        }
    }

    #[test]
    fn each_ability_has_its_skill_and_strikes_share_one_timer() {
        for ability in Ability::ALL {
            assert_eq!(Ability::from_skill(ability.skill()), Some(ability));
        }
        assert_eq!(Ability::from_skill(0), None);
        assert_eq!(Ability::Kick.recovery(), Ability::Bash.recovery());
        assert_ne!(Ability::Kick.recovery(), Ability::Taunt.recovery());
        assert_eq!(Ability::SenseHeading.recovery(), None);
        assert!(Ability::Backstab.strikes() && Ability::Taunt.at_target());
        assert_eq!(Ability::Kick.reuse(), Duration::from_secs(4));
        let mut skills = vec![0; 100];
        skills[30] = 1;
        assert!(Ability::Kick.known(&skills, 1) && !Ability::Bash.known(&skills, 1));
        assert!(Ability::Bash.known(&skills, 10), "an Ogre slams");
        assert!(!Ability::Kick.known(&[], 1));
        assert_eq!(Ability::Mend.reuse(), Duration::from_secs(360));
        // Anyone fishes, and waits ten seconds between casts.
        assert!(Ability::Fishing.known(&[], 1));
        assert_eq!(Ability::Fishing.reuse(), Duration::from_secs(10));
        // Anyone bandages, which takes as long as the server's bandaging.
        assert!(Ability::BindWound.known(&[], 1) && !Ability::BindWound.at_target());
        assert_eq!(Ability::BindWound.reuse(), crate::bind_wound::DURATION);
        assert_eq!(Ability::BindWound.encode(7), crate::bind_wound::encode(7));
    }

    #[test]
    fn melee_reaches_as_far_as_the_larger_body_allows() {
        let body = |race, size, x| Body {
            race,
            size,
            x,
            y: 0.0,
        };
        // Small bodies count as size 8: sixteen units.
        assert!(in_melee_range(body(1, 0.0, 0.0), body(1, 6.0 - 1.0, 16.0)));
        assert!(!in_melee_range(body(1, 0.0, 0.0), body(1, 0.0, 16.5)));
        // A size 10 creature: twenty units.
        assert!(in_melee_range(body(1, 0.0, 0.0), body(1, 10.0, 20.0)));
        assert!(!in_melee_range(body(1, 0.0, 0.0), body(1, 10.0, 20.5)));
        // Against a size 6 creature a human, at size 6 too, reaches twelve
        // units, less far than a gnome, whose 3 counts as 8.
        let human = body(1, player_size(1), 0.0);
        assert!(in_melee_range(human, body(1, 6.0, 12.0)));
        assert!(!in_melee_range(human, body(1, 6.0, 12.5)));
        assert!(in_melee_range(
            body(12, player_size(12), 0.0),
            body(1, 6.0, 16.0)
        ));
        assert_eq!(player_size(10), 9.0);
        assert_eq!(player_size(7), 5.5);
        assert_eq!(player_size(999), 0.0);
        // A lava dragon counts as size 60, and huge reaches are cut down.
        assert!(in_melee_range(body(1, 0.0, 0.0), body(49, 1.0, 59.9)));
        assert!(!in_melee_range(body(1, 0.0, 0.0), body(49, 1.0, 60.5)));
        assert!(in_melee_range(body(1, 0.0, 0.0), body(1, 200.0, 75.0)));
        assert!(!in_melee_range(body(1, 0.0, 0.0), body(1, 200.0, 76.0)));
    }
}
