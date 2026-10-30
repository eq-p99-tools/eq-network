//! Using a skill as the official client's ability buttons do: the combat
//! abilities (kick, bash, backstab, frenzy and the monk strikes) and taunt
//! at the target, binding wounds on the player or another player, and
//! hiding, sneaking, foraging, fishing, mending, feigning death and sensing
//! heading on the player. Servers answer in chat, in the combat stream, with
//! an item on the cursor or, for a bandaging, with its own packet (see
//! [`crate::bind_wound`]), and keep a recovery timer for each; the combat
//! abilities share one. They end the player's hiding and sneaking with
//! `OP_CancelSneakHide`, which a Titanium client answers
//! ([`titanium_unhide`](crate::abilities::titanium_unhide)). TAKP and Quarm
//! say when a skill's button is ready again before the official client's
//! own wait ends ([`eqmac_reset_skill`](crate::abilities::eqmac_reset_skill)).
//!
//! Layout reference: `EQEmu`'s `CombatAbility_Struct` and
//! `ClientTarget_Struct` (`common/eq_packet_structs.h`) and the Titanium
//! opcodes (`utils/patches/patch_Titanium.conf`); `Client::OPCombatAbility`
//! (`zone/special_attacks.cpp`) and the handlers in `zone/client_packet.cpp`
//! for the rules; `common/features.h` for recovery times;
//! `Client::CancelSneakHide` (`zone/client_process.cpp`) and TAKP's
//! `Mob::CommonBreakSneakHide` (`zone/attack.cpp`) for the end of hiding.
use crate::{command::EncodedCommand, GameDialect};
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

/// `OP_CancelSneakHide`: the server ended the player's hiding and sneaking.
/// It carries nothing.
pub const CANCEL_SNEAK_HIDE_OPCODE: u16 = 0x48c2;
/// `EQMac`'s `OP_CancelSneakHide`, which carries nothing either.
pub const EQMAC_CANCEL_SNEAK_HIDE_OPCODE: u16 = 0x5a41;
/// `EQMac`'s `OP_ResetSkill`: a skill's button is ready again.
pub const EQMAC_RESET_SKILL_OPCODE: u16 = 0x3d42;
/// The appearance kind a Titanium client answers the end of its hiding with
/// (`AppearanceType::Invisibility`).
const INVISIBILITY: u16 = 3;

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
    /// Sense Heading, which only the Titanium client makes wait.
    SenseHeading,
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

    /// The timer it waits on.
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
            Self::SenseHeading => Recovery::SenseHeading,
        })
    }

    /// How long the generation's client keeps the button waiting after a
    /// use, from the official client where it has been read and otherwise
    /// how long `EQEmu` makes its timer wait when nothing shortens it (its
    /// `features.h` time less the second its handlers take off). A
    /// bandaging also holds the next until it ends.
    ///
    /// Titanium, from the program (titanium-program-s8b T7-a, s8c T7): the
    /// client's per-skill table, which it waits out in full since the last
    /// use; Bind Wound waits one second more. The client takes off only the
    /// skill reuse focus (spell effect 227), which this library doesn't
    /// track, so it waits the full time: slower than an official client
    /// with the focus, never faster. Feign Death, Forage, Hide, Sneak and
    /// Sense Heading are read from the table by index (the decompile
    /// thread's 10-30 skill-id read). The client also keeps Hide on a
    /// ready-at time and holds Sneak while its controlled spawn's state is
    /// set (s8c T7); neither is modelled, only the wait. Frenzy keeps the
    /// emulator's time: the client skips the timer for its skill type, so
    /// whether its table value applies is unread. `EQMac` keeps the
    /// emulator's times throughout, Sense Heading none; the TAKP program's
    /// table is unread.
    #[must_use]
    pub const fn reuse(self, dialect: GameDialect) -> Duration {
        if let GameDialect::Titanium = dialect {
            let millis = match self {
                // Unread: the emulator's time below.
                Self::Frenzy => 0,
                Self::SenseHeading => 2_000,
                Self::TigerClaw => 5_500,
                Self::EagleStrike | Self::Taunt => 6_000,
                Self::DragonPunch => 6_200,
                Self::Kick | Self::Bash => 8_000,
                Self::FlyingKick | Self::RoundKick => 8_500,
                Self::Backstab | Self::FeignDeath | Self::Hide | Self::Sneak => 10_000,
                Self::BindWound => 11_000,
                Self::Fishing => 12_000,
                Self::Forage => 100_000,
                Self::Mend => 360_000,
            };
            if millis > 0 {
                return Duration::from_millis(millis);
            }
        }
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

/// The Titanium client's answer to the server's end of its hiding, as
/// `EQEmu`'s source describes it (not yet recorded from the client): the
/// player's own appearance, invisibility 0. `EQEmu` ends the player's
/// sneaking as it sends `OP_CancelSneakHide`, which it sends only to a
/// hidden player, and leaves them hidden until this answer comes; the answer
/// takes them out of hiding, zeroing their invisibility, and `EQEmu` shows
/// it to the zone (`Client::Handle_OP_SpawnAppearance`). TAKP and Quarm end
/// both before they send theirs, and wait for nothing.
///
/// # Errors
/// Rejects a zero spawn ID.
pub fn titanium_unhide(spawn_id: u16) -> anyhow::Result<EncodedCommand> {
    crate::listing::update(spawn_id, INVISIBILITY, 0)
}

/// Reads `EQMac`'s `OP_ResetSkill`: the skill whose button is ready again,
/// by the servers' number for it ([`Ability::skill`]). TAKP and Quarm send
/// it for Mend as their own wait ends, when the player pressed Mend too soon
/// after a zone and so started the official client's full wait anew
/// (`Client::Handle_OP_Mend`), and for each skill with a recovery timer the
/// player has on a GM's `#cleartimers` (`Client::ResetAllSkills`).
///
/// Layout reference: TAKP's `ResetSkill_Struct`, 8 bytes: the skill in 16
/// bits, two bytes of padding and a timer every server leaves 0.
///
/// # Errors
/// Rejects any length other than 8 bytes.
pub fn eqmac_reset_skill(body: &[u8]) -> anyhow::Result<u32> {
    anyhow::ensure!(body.len() == 8, "invalid skill reset length");
    Ok(u32::from(u16::from_le_bytes([body[0], body[1]])))
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
    fn a_titanium_client_answers_the_end_of_its_hiding_with_invisibility_0() {
        assert_eq!(
            titanium_unhide(7).unwrap(),
            EncodedCommand {
                opcode: crate::listing::APPEARANCE_OPCODE,
                body: vec![7, 0, 3, 0, 0, 0, 0, 0],
            }
        );
        assert!(titanium_unhide(0).is_err());
    }

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
        assert_eq!(
            Ability::SenseHeading.recovery(),
            Some(Recovery::SenseHeading)
        );
        assert_eq!(
            Ability::SenseHeading.reuse(GameDialect::EqMac),
            Duration::ZERO
        );
        assert!(Ability::Backstab.strikes() && Ability::Taunt.at_target());
        assert_eq!(
            Ability::Kick.reuse(GameDialect::EqMac),
            Duration::from_secs(4)
        );
        let mut skills = vec![0; 100];
        skills[30] = 1;
        assert!(Ability::Kick.known(&skills, 1) && !Ability::Bash.known(&skills, 1));
        assert!(Ability::Bash.known(&skills, 10), "an Ogre slams");
        assert!(!Ability::Kick.known(&[], 1));
        for dialect in [GameDialect::Titanium, GameDialect::EqMac] {
            assert_eq!(Ability::Mend.reuse(dialect), Duration::from_secs(360));
        }
        // Anyone fishes, and waits ten seconds between casts.
        assert!(Ability::Fishing.known(&[], 1));
        assert_eq!(
            Ability::Fishing.reuse(GameDialect::EqMac),
            Duration::from_secs(10)
        );
        // Anyone bandages, which takes as long as the server's bandaging.
        assert!(Ability::BindWound.known(&[], 1) && !Ability::BindWound.at_target());
        assert_eq!(
            Ability::BindWound.reuse(GameDialect::EqMac),
            crate::bind_wound::DURATION
        );
        assert_eq!(Ability::BindWound.encode(7), crate::bind_wound::encode(7));
    }

    #[test]
    fn titanium_waits_as_long_as_the_official_client() {
        let titanium = |ability: Ability| ability.reuse(GameDialect::Titanium).as_millis();
        for (ability, millis) in [
            (Ability::Backstab, 10_000),
            (Ability::BindWound, 11_000),
            (Ability::Bash, 8_000),
            (Ability::DragonPunch, 6_200),
            (Ability::EagleStrike, 6_000),
            (Ability::FlyingKick, 8_500),
            (Ability::Kick, 8_000),
            (Ability::Mend, 360_000),
            (Ability::RoundKick, 8_500),
            (Ability::TigerClaw, 5_500),
            (Ability::Fishing, 12_000),
            (Ability::Taunt, 6_000),
            (Ability::FeignDeath, 10_000),
            (Ability::Forage, 100_000),
            (Ability::Hide, 10_000),
            (Ability::Sneak, 10_000),
            (Ability::SenseHeading, 2_000),
            // Whether the table's value applies is unread: the emulator's.
            (Ability::Frenzy, 9_000),
        ] {
            assert_eq!(titanium(ability), millis, "{ability:?}");
        }
        for ability in Ability::ALL {
            assert!(ability.reuse(GameDialect::Titanium) >= ability.reuse(GameDialect::EqMac));
        }
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
