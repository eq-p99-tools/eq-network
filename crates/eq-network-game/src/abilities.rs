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

pub mod eqmac;

/// `OP_CombatAbility`: a strike at the target.
pub const COMBAT_OPCODE: u16 = 0x5ee8;
/// `OP_Taunt`.
pub const TAUNT_OPCODE: u16 = 0x5e48;
/// `CombatAbility_Struct.m_atk` for a melee strike; `EQEmu` handles ranged
/// attacks through the same packet with the range slot instead.
const MELEE_STRIKE: u32 = 100;

/// `OP_Fishing`, which carries nothing.
pub const FISHING_OPCODE: u16 = 0x0b36;
/// `OP_PickPocket`: an 18-byte ask of the target (read R14 section 3).
pub const PICK_POCKET_OPCODE: u16 = 0x2ad8;
/// `OP_Begging`: the same 18-byte ask.
pub const BEGGING_OPCODE: u16 = 0x13e7;
/// The Titanium program's Sense Traps and Disarm Traps at a target of the
/// treasure class, instead of their own messages (read R14 section 3).
const TREASURE_SENSE_TRAPS_OPCODE: u16 = 0x07a8;
const TREASURE_DISARM_TRAPS_OPCODE: u16 = 0x41b9;
/// And its Pick Lock, which sends nothing anywhere else.
const TREASURE_PICK_LOCK_OPCODE: u16 = 0x069d;
/// The class the Titanium program sends its treasure messages for: a class
/// byte (read R15b), which `EQEmu` calls `LDoN` treasure (recording checklist
/// 6.432, inferred).
pub const TREASURE_CLASS: u8 = 62;

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
    /// Pick Pockets, at a target close by.
    PickPockets,
    /// Begging, from an NPC close by.
    Begging,
    /// Sense Traps.
    SenseTraps,
    /// Disarm Traps.
    DisarmTraps,
    /// Pick Lock, at a treasure; a door's lock is picked by clicking it.
    PickLock,
    /// Tracking: the server's list of what is near ([`crate::tracking`]).
    Tracking,
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
    /// Sense Heading.
    SenseHeading,
    /// Pick Pockets.
    PickPockets,
    /// Begging.
    Begging,
    /// Sense Traps.
    SenseTraps,
    /// Disarm Traps.
    DisarmTraps,
    /// Pick Lock.
    PickLock,
    /// Tracking.
    Tracking,
}

impl Ability {
    /// Every ability, strikes first and those anyone has, binding wounds and
    /// fishing, last.
    pub const ALL: [Self; 24] = [
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
        Self::PickPockets,
        Self::Begging,
        Self::SenseTraps,
        Self::DisarmTraps,
        Self::PickLock,
        Self::Tracking,
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
            Self::DisarmTraps => 17,
            Self::PickLock => 35,
            Self::Tracking => 53,
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
            Self::PickPockets => 48,
            Self::Sneak => 42,
            Self::TigerClaw => 52,
            Self::Fishing => 55,
            Self::SenseTraps => 62,
            Self::Begging => 67,
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
            Self::PickPockets => "Pick Pockets",
            Self::Begging => "Begging",
            Self::SenseTraps => "Sense Traps",
            Self::DisarmTraps => "Disarm Traps",
            Self::PickLock => "Pick Lock",
            Self::Tracking => "Tracking",
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
            Self::PickPockets => Recovery::PickPockets,
            Self::Begging => Recovery::Begging,
            Self::SenseTraps => Recovery::SenseTraps,
            Self::DisarmTraps => Recovery::DisarmTraps,
            Self::PickLock => Recovery::PickLock,
            Self::Tracking => Recovery::Tracking,
        })
    }

    /// How long the generation's client keeps the button waiting after a
    /// use: the official client's per-skill table, which it waits out in
    /// full since the last use. A bandaging also holds the next until it
    /// ends.
    ///
    /// Titanium, from the program (titanium-program-s8b T7-a, s8c T7):
    /// Bind Wound waits one second more than its table value. The client
    /// takes off only the skill reuse focus (spell effect 227). Feign
    /// Death, Forage, Hide, Sneak and Sense Heading are read from the table
    /// by index (the decompile thread's 10-30 skill-id read). The client
    /// also keeps Hide on a ready-at time and holds Sneak while its
    /// controlled spawn's state is set (s8c T7); neither is modelled, only
    /// the wait. Frenzy waits its table value too: the client starts the
    /// strikes' shared timer with it when a Frenzy goes (s8d B, s8e).
    ///
    /// `EQMac`, from the TAKP program (program-read-gate-v2 section C, T7):
    /// the strikes share one timer as long as the last one used, which the
    /// client shortens by the player's haste; the rest keep their own and
    /// aren't hasted. Alternate abilities shorten Feign Death and Hide.
    ///
    /// This library tracks neither the focus, haste nor alternate
    /// abilities, so it keeps the unmodified waits. `EQMac` Tiger Claw is
    /// raised to the server's six-second unhasted floor. Server slowing can
    /// extend strike recovery further; the server remains authoritative.
    /// On `EQMac`, Frenzy keeps `EQEmu`'s time
    /// (its `features.h` time less the second its handlers take off), as
    /// the TAKP read doesn't list it.
    ///
    /// Pick Pockets, Begging, Sense Traps, Disarm Traps, Pick Lock and
    /// Tracking each wait ten seconds on their own timer, the program's skill-record
    /// value (read R16c, [0x851200 + id x 4] +4, in ms), which also gives
    /// the waits above for the strikes, Feign Death, Hide and Mend.
    #[must_use]
    pub const fn reuse(self, dialect: GameDialect) -> Duration {
        Duration::from_millis(match self {
            Self::SenseHeading => 2_000,
            Self::TigerClaw => match dialect {
                GameDialect::Titanium => 5_500,
                // The EQMac server's unhasted floor is 7 - 1 seconds.
                GameDialect::EqMac => 6_000,
            },
            Self::EagleStrike | Self::Taunt => 6_000,
            Self::DragonPunch => 6_200,
            Self::Kick | Self::Bash => 8_000,
            Self::FlyingKick | Self::RoundKick => 8_500,
            Self::Frenzy => match dialect {
                GameDialect::Titanium => 15_000,
                GameDialect::EqMac => 9_000,
            },
            Self::Backstab
            | Self::FeignDeath
            | Self::Hide
            | Self::Sneak
            | Self::PickPockets
            | Self::Begging
            | Self::SenseTraps
            | Self::DisarmTraps
            | Self::PickLock
            | Self::Tracking => 10_000,
            Self::BindWound => match dialect {
                GameDialect::Titanium => 11_000,
                GameDialect::EqMac => 10_000,
            },
            Self::Fishing => 12_000,
            Self::Forage => 100_000,
            Self::Mend => 360_000,
        })
    }

    /// Whether its packet names the player and their own skill value as
    /// well as the target ([`titanium_ask`]): Pick Pockets and Begging.
    #[must_use]
    pub const fn asks(self) -> bool {
        matches!(self, Self::PickPockets | Self::Begging)
    }

    /// The packet that uses it; a strike or taunt names the target, which
    /// must be the server's target too, and a bandaging the one bandaged.
    /// None for an ability that [`asks`](Self::asks), whose packet
    /// [`titanium_ask`] builds, and for Pick Lock, which has a message only
    /// at a treasure ([`Self::titanium_at_treasure`]), for Tracking, whose
    /// messages [`crate::tracking`] builds, and for Feign Death, which
    /// carries the player's own id ([`crate::feign::titanium_request`]).
    #[must_use]
    pub fn encode(self, target: u16) -> Option<EncodedCommand> {
        if self == Self::BindWound {
            return Some(crate::bind_wound::encode(target));
        }
        if self.asks() || matches!(self, Self::PickLock | Self::Tracking | Self::FeignDeath) {
            return None;
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
            // `OP_Fishing`, `OP_Mend` and `OP_SenseHeading`.
            Self::Hide => (0x4312, Vec::new()),
            Self::Sneak => (0x74e1, Vec::new()),
            Self::Forage => (0x4796, Vec::new()),
            Self::Fishing => (FISHING_OPCODE, Vec::new()),
            Self::Mend => (0x14ef, Vec::new()),
            // The Titanium program's own messages (read R14 section 3).
            Self::SenseTraps => (0x5666, Vec::new()),
            Self::DisarmTraps => (0x1241, Vec::new()),
            _ => (0x05ac, Vec::new()),
        };
        Some(EncodedCommand { opcode, body })
    }

    /// The Titanium program's message for it at a target of the treasure
    /// class ([`TREASURE_CLASS`]), where it has one: Sense Traps, Disarm
    /// Traps and Pick Lock, which carry nothing (read R14 section 3).
    #[must_use]
    pub const fn titanium_at_treasure(self) -> Option<EncodedCommand> {
        let opcode = match self {
            Self::SenseTraps => TREASURE_SENSE_TRAPS_OPCODE,
            Self::DisarmTraps => TREASURE_DISARM_TRAPS_OPCODE,
            Self::PickLock => TREASURE_PICK_LOCK_OPCODE,
            _ => return None,
        };
        Some(EncodedCommand {
            opcode,
            body: Vec::new(),
        })
    }
}

/// The Titanium program's animation of casting a fishing pole, which it
/// sends before a standing player's Fishing (read R15c): `OP_Animation`, the
/// player's own spawn id and two bytes of 5. The program draws byte +3 as
/// the motion at half speed; `EQEmu` reads +2 as the speed and +3 as the
/// action.
///
/// # Errors
/// Refuses a player with no spawn yet.
pub fn titanium_fishing_cast(own: u16) -> anyhow::Result<EncodedCommand> {
    anyhow::ensure!(own != 0, "The player has no spawn yet");
    let [low, high] = own.to_le_bytes();
    Ok(EncodedCommand {
        opcode: crate::combat::ANIMATION_OPCODE,
        body: vec![low, high, 5, 5],
    })
}

/// The Titanium program's Pick Pockets or Begging (read R14 section 3), 18
/// bytes: the target and the player's own spawn as 32-bit ids, the player's
/// value in the skill, 0xff, and five zero bytes. The program takes the
/// value from one 32-bit-per-skill array: Pick Pockets' at character
/// +0x1030 and Begging's at +0x107c lie (67 - 48) x 4 bytes apart.
///
/// # Errors
/// Refuses another ability, and a player with no spawn yet.
pub fn titanium_ask(
    ability: Ability,
    target: u16,
    own: u16,
    skill_value: u32,
) -> anyhow::Result<EncodedCommand> {
    let opcode = match ability {
        Ability::PickPockets => PICK_POCKET_OPCODE,
        Ability::Begging => BEGGING_OPCODE,
        _ => anyhow::bail!("{ability:?} does not ask of a target"),
    };
    anyhow::ensure!(own != 0, "The player has no spawn yet");
    let mut body = Vec::with_capacity(18);
    body.extend_from_slice(&u32::from(target).to_le_bytes());
    body.extend_from_slice(&u32::from(own).to_le_bytes());
    body.extend_from_slice(&skill_value.to_le_bytes());
    body.push(0xff);
    body.resize(18, 0);
    Ok(EncodedCommand { opcode, body })
}

/// What a beg or a pick brought (read R16d).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum AskOutcome {
    /// Nothing.
    Failed,
    /// Coins of one kind, which the program adds to the purse itself: the
    /// server adds them too and sends no money update.
    Coins {
        /// The kind.
        coin: crate::money::Coin,
        /// How many.
        amount: u32,
    },
    /// An item by name; the item itself comes in its own packet.
    Item(String),
}

/// The server's answer to the player's Begging or Pick Pockets.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AskAnswer {
    /// Begging or Pick Pockets.
    pub ability: Ability,
    /// What it brought.
    pub outcome: AskOutcome,
}

impl AskAnswer {
    /// The coins it brought, to add to the purse.
    #[must_use]
    pub fn coins(&self) -> crate::world::Coins {
        let mut coins = crate::world::Coins::default();
        if let AskOutcome::Coins { coin, amount } = self.outcome {
            *match coin {
                crate::money::Coin::Platinum => &mut coins.platinum,
                crate::money::Coin::Gold => &mut coins.gold,
                crate::money::Coin::Silver => &mut coins.silver,
                crate::money::Coin::Copper => &mut coins.copper,
            } = amount;
        }
        coins
    }

    /// The program's line for it (`eqstr_us.txt` string ID, read R16d),
    /// with what the line names: the amount of coins, or the item.
    #[must_use]
    pub fn line(&self) -> (u32, Vec<String>) {
        // Each ability's failure line, then its four coin lines from
        // platinum down; Pick Pockets' item line follows its copper line.
        let base = if self.ability == Ability::Begging {
            0x325c
        } else {
            0x3262
        };
        match &self.outcome {
            AskOutcome::Failed => (base, Vec::new()),
            AskOutcome::Coins { coin, amount } => {
                let kind = match coin {
                    crate::money::Coin::Platinum => 1,
                    crate::money::Coin::Gold => 2,
                    crate::money::Coin::Silver => 3,
                    crate::money::Coin::Copper => 4,
                };
                (base + kind, vec![amount.to_string()])
            }
            AskOutcome::Item(name) => (base + 5, vec![name.clone()]),
        }
    }
}

/// The length of `EQEmu`'s answer to a beg (`BeggingResponse_Struct`).
const BEGGING_ANSWER_LENGTH: usize = 20;
/// The length of its answer to a pick (`sPickPocket_Struct`): four ids and
/// values, the amount, and a 64-byte item name.
const PICK_POCKET_ANSWER_LENGTH: usize = 84;

/// Reads the server's answer to a beg or a pick, as the Titanium program
/// does (read R16d, 0x455900 and 0x455b05): the result byte at +0xc, the
/// amount at +0x10 and, for an item, its name at +0x14. A result of 0 is
/// nothing, 1 to 4 coins from platinum down to copper, and on a pick 5 an
/// item. The program passes over a pick's -1, which `EQEmu` names the
/// answer to a player being picked; it reads no other result, which this
/// passes over too.
///
/// Layout reference: `EQEmu`'s `BeggingResponse_Struct` and
/// `sPickPocket_Struct` (`common/patches/titanium_structs.h`). The 20- and
/// 84-byte floors are those structs' sizes in the reference server, not
/// lengths the program tests.
///
/// # Errors
/// Rejects a body too short for its fields, and another opcode.
pub fn titanium_answer(opcode: u16, body: &[u8]) -> anyhow::Result<Option<AskAnswer>> {
    let (ability, length) = match opcode {
        BEGGING_OPCODE => (Ability::Begging, BEGGING_ANSWER_LENGTH),
        PICK_POCKET_OPCODE => (Ability::PickPockets, PICK_POCKET_ANSWER_LENGTH),
        _ => anyhow::bail!("opcode {opcode:#06x} answers no ask"),
    };
    anyhow::ensure!(
        body.len() >= length,
        "an answer to {ability:?} is {length} bytes, not {}",
        body.len()
    );
    let amount = u32::from_le_bytes([body[0x10], body[0x11], body[0x12], body[0x13]]);
    let coins = |coin| AskOutcome::Coins { coin, amount };
    let outcome = match body[0xc] {
        0 => AskOutcome::Failed,
        1 => coins(crate::money::Coin::Platinum),
        2 => coins(crate::money::Coin::Gold),
        3 => coins(crate::money::Coin::Silver),
        4 => coins(crate::money::Coin::Copper),
        5 if ability == Ability::PickPockets => {
            let name = &body[0x14..length];
            let end = name
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(name.len());
            AskOutcome::Item(String::from_utf8_lossy(&name[..end]).into_owned())
        }
        _ => return Ok(None),
    };
    Ok(Some(AskAnswer { ability, outcome }))
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
/// Rejects any length other than 8 bytes or a nonzero, unestablished timer.
/// Use [`eqmac::reset_skill`] to retain that complete raw record.
pub fn eqmac_reset_skill(body: &[u8]) -> anyhow::Result<u32> {
    let reset = eqmac::reset_skill(body)?;
    anyhow::ensure!(
        reset.timer == 0,
        "unknown skill reset timer: {}",
        reset.timer
    );
    Ok(u32::from(reset.skill))
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
            Ability::Kick.encode(42).unwrap(),
            EncodedCommand {
                opcode: COMBAT_OPCODE,
                body: vec![42, 0, 0, 0, 100, 0, 0, 0, 30, 0, 0, 0],
            }
        );
        assert_eq!(Ability::DragonPunch.encode(7).unwrap().body[8], 21);
        assert_eq!(
            Ability::Taunt.encode(42).unwrap(),
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
            (Ability::SenseHeading, 0x05ac),
            (Ability::SenseTraps, 0x5666),
            (Ability::DisarmTraps, 0x1241),
        ] {
            assert_eq!(
                ability.encode(42),
                Some(EncodedCommand {
                    opcode,
                    body: Vec::new()
                }),
                "{ability:?}"
            );
            assert!(!ability.at_target());
        }
    }

    #[test]
    fn feign_death_is_built_with_the_players_own_id() {
        // `crate::feign::titanium_request` builds it, as the program does.
        assert_eq!(Ability::FeignDeath.encode(42), None);
        let sender = crate::request::Sender {
            name: "Player",
            spawn_id: Some(7),
        };
        let request = crate::request::Request::Ability {
            ability: Ability::FeignDeath,
            target: Some(42),
        };
        assert_eq!(
            crate::request::titanium(&request, sender).unwrap(),
            crate::feign::titanium_request(7).unwrap()
        );
    }

    #[test]
    fn pick_pockets_and_begging_ask_with_the_players_own_skill_value() {
        assert_eq!(Ability::PickPockets.encode(42), None);
        assert_eq!(Ability::Begging.encode(42), None);
        assert_eq!(
            titanium_ask(Ability::PickPockets, 42, 7, 0x0102_0304).unwrap(),
            EncodedCommand {
                opcode: 0x2ad8,
                body: vec![42, 0, 0, 0, 7, 0, 0, 0, 4, 3, 2, 1, 0xff, 0, 0, 0, 0, 0],
            }
        );
        let begging = titanium_ask(Ability::Begging, 42, 7, 25).unwrap();
        assert_eq!(begging.opcode, 0x13e7);
        assert_eq!(begging.body.len(), 18);
        assert_eq!(begging.body[8], 25);
        assert!(titanium_ask(Ability::Kick, 42, 7, 1).is_err());
        assert_eq!(
            titanium_fishing_cast(0x0107).unwrap(),
            EncodedCommand {
                opcode: 0x2acf,
                body: vec![7, 1, 5, 5],
            }
        );
        assert!(titanium_fishing_cast(0).is_err());
        assert!(titanium_ask(Ability::Begging, 42, 0, 1).is_err());
        // The trap skills have their own messages at a treasure.
        for (ability, opcode) in [
            (Ability::SenseTraps, 0x07a8),
            (Ability::DisarmTraps, 0x41b9),
            (Ability::PickLock, 0x069d),
        ] {
            assert_eq!(
                ability.titanium_at_treasure(),
                Some(EncodedCommand {
                    opcode,
                    body: Vec::new()
                })
            );
        }
        assert_eq!(Ability::Kick.titanium_at_treasure(), None);
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
        assert!(Ability::Backstab.strikes() && Ability::Taunt.at_target());
        let mut skills = vec![0; 100];
        skills[30] = 1;
        assert!(Ability::Kick.known(&skills, 1) && !Ability::Bash.known(&skills, 1));
        assert!(Ability::Bash.known(&skills, 10), "an Ogre slams");
        assert!(!Ability::Kick.known(&[], 1));
        for dialect in [GameDialect::Titanium, GameDialect::EqMac] {
            assert_eq!(Ability::Mend.reuse(dialect), Duration::from_secs(360));
        }
        // Anyone fishes, and waits twelve seconds between casts.
        assert!(Ability::Fishing.known(&[], 1));
        for dialect in [GameDialect::Titanium, GameDialect::EqMac] {
            assert_eq!(Ability::Fishing.reuse(dialect), Duration::from_secs(12));
        }
        // Anyone bandages, which takes as long as the server's bandaging.
        assert!(Ability::BindWound.known(&[], 1) && !Ability::BindWound.at_target());
        assert_eq!(
            Ability::BindWound.reuse(GameDialect::EqMac),
            crate::bind_wound::DURATION
        );
        assert_eq!(
            Ability::BindWound.encode(7),
            Some(crate::bind_wound::encode(7))
        );
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
            (Ability::Frenzy, 15_000),
            (Ability::PickPockets, 10_000),
            (Ability::Begging, 10_000),
            (Ability::SenseTraps, 10_000),
            (Ability::DisarmTraps, 10_000),
            (Ability::PickLock, 10_000),
            (Ability::Tracking, 10_000),
        ] {
            assert_eq!(titanium(ability), millis, "{ability:?}");
        }
    }

    #[test]
    fn eqmac_waits_as_long_as_the_unhasted_official_client() {
        let eqmac = |ability: Ability| ability.reuse(GameDialect::EqMac).as_millis();
        for (ability, millis) in [
            (Ability::Backstab, 10_000),
            (Ability::Bash, 8_000),
            (Ability::DragonPunch, 6_200),
            (Ability::EagleStrike, 6_000),
            (Ability::FlyingKick, 8_500),
            (Ability::Kick, 8_000),
            (Ability::RoundKick, 8_500),
            (Ability::TigerClaw, 6_000),
            (Ability::BindWound, 10_000),
            (Ability::Taunt, 6_000),
            (Ability::FeignDeath, 10_000),
            (Ability::Hide, 10_000),
            (Ability::Sneak, 10_000),
            (Ability::SenseHeading, 2_000),
            (Ability::Forage, 100_000),
            (Ability::Fishing, 12_000),
            (Ability::Mend, 360_000),
            // Not in the TAKP read: the emulator's.
            (Ability::Frenzy, 9_000),
        ] {
            assert_eq!(eqmac(ability), millis, "{ability:?}");
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

    #[test]
    fn a_beg_or_a_pick_is_answered_with_its_result_byte_and_amount() {
        let answer = |opcode, length: usize, result: u8, amount: u32, name: &[u8]| {
            let mut body = vec![0; length];
            body[0xc] = result;
            body[0x10..0x14].copy_from_slice(&amount.to_le_bytes());
            body[0x14..0x14 + name.len()].copy_from_slice(name);
            titanium_answer(opcode, &body).unwrap()
        };
        let begged = answer(BEGGING_OPCODE, 20, 3, 7, b"").unwrap();
        assert_eq!(
            begged.outcome,
            AskOutcome::Coins {
                coin: crate::money::Coin::Silver,
                amount: 7
            }
        );
        assert_eq!(begged.coins().silver, 7);
        assert_eq!(begged.line(), (0x325f, vec!["7".to_owned()]));
        let failed = answer(BEGGING_OPCODE, 20, 0, 0, b"").unwrap();
        assert_eq!(failed.line(), (0x325c, Vec::new()));
        assert_eq!(failed.coins(), crate::world::Coins::default());
        let picked = answer(PICK_POCKET_OPCODE, 84, 1, 2, b"").unwrap();
        assert_eq!(picked.line(), (0x3263, vec!["2".to_owned()]));
        assert_eq!(picked.coins().platinum, 2);
        let item = answer(PICK_POCKET_OPCODE, 84, 5, 0, b"Rusty Dagger\0junk").unwrap();
        assert_eq!(item.outcome, AskOutcome::Item("Rusty Dagger".to_owned()));
        assert_eq!(item.line(), (0x3267, vec!["Rusty Dagger".to_owned()]));
        // A pick of the player (-1) and results the program does not read
        // are passed over; a beg names no item.
        assert_eq!(answer(PICK_POCKET_OPCODE, 84, 0xff, 0, b""), None);
        assert_eq!(answer(BEGGING_OPCODE, 20, 5, 0, b""), None);
        assert!(titanium_answer(BEGGING_OPCODE, &[0; 19]).is_err());
        assert!(titanium_answer(PICK_POCKET_OPCODE, &[0; 28]).is_err());
    }
}
