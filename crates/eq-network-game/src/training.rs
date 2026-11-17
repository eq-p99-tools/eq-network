//! Training skills at a guildmaster, as the official client's training
//! window does. Asking a guildmaster of the player's own class opens
//! training, and the guildmaster answers with how far each skill can be
//! trained; each practice raises one skill by a point for a practice point
//! and, past a skill of 10, coins; leaving says goodbye. An ordinary skill
//! update has no receipt fields. `EQEmu` retains the existing local point
//! and purse projection. P99 waits for a fresh balance after a practice.
//!
//! Layout reference: `EQEmu`'s `GMTrainee_Struct`, `GMSkillChange_Struct` and
//! `GMTrainEnd_Struct` (`common/eq_packet_structs.h`) and the Titanium
//! opcodes (`utils/patches/patch_Titanium.conf`); `Client::OPGMTraining`,
//! `OPGMTrainSkill` and `OPGMEndTraining` (`zone/client_process.cpp`) for
//! the rules.
use crate::command::EncodedCommand;
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_GMTraining`: asking a guildmaster to train, which the server answers
/// with the same packet filled in.
pub const OFFER_OPCODE: u16 = 0x238f;
/// `OP_GMTrainSkill`: one practice.
pub const TRAIN_OPCODE: u16 = 0x11d2;
/// `OP_GMEndTraining`: leaving training.
pub const END_OPCODE: u16 = 0x613d;

/// How far from a guildmaster the server lets the player train: within 200
/// units (`USE_NPC_RANGE2`), up and down as well as across.
pub const RANGE: f32 = 200.0;

/// The skill numbers a practice may name; languages are trained apart.
pub const SKILLS: u32 = 100;

/// `EQEmu`'s trainer accepts language IDs zero through 25.
pub const LANGUAGES: u8 = 26;
/// The maximum language proficiency.
pub const LANGUAGE_CAP: u32 = 100;

/// The separate skill banks named by a trainer request.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum TrainingSubject {
    /// An ordinary skill, in bank zero.
    Skill(u32),
    /// A language, in bank one; this is not an ordinary skill number.
    Language(u8),
}

impl TrainingSubject {
    /// The ID used by received skill updates, rather than training requests.
    #[must_use]
    pub const fn update_id(self) -> u32 {
        match self {
            Self::Skill(skill) => skill,
            Self::Language(language) => crate::world::LANGUAGE_SKILLS + language as u32,
        }
    }

    /// The received value in the corresponding profile bank.
    #[must_use]
    pub fn value(self, player: &crate::world::PlayerState) -> Option<u32> {
        match self {
            Self::Skill(skill) if skill < SKILLS => player
                .skills
                .as_ref()?
                .get(usize::try_from(skill).ok()?)
                .copied(),
            Self::Language(language) if language < LANGUAGES => player
                .languages
                .as_ref()?
                .get(usize::from(language))
                .copied()
                .map(u32::from),
            _ => None,
        }
    }
}

/// Native Titanium language eligibility (eqgame 0x4bbeda), including the
/// table at 0x78f010. `EQEmu`'s server has its own, separate trainer rules.
#[must_use]
pub fn titanium_language_trainable(language: u8, race: u32, class: Option<u32>, level: u8) -> bool {
    let column = match race {
        1..=12 => usize::try_from(race - 1).ok(),
        128 => Some(12),
        130 => Some(13),
        330 => Some(14),
        _ => None,
    };
    let Some(minimum) = column.and_then(|column| {
        LANGUAGE_LEVELS
            .get(usize::from(language))
            .map(|row| row[column])
    }) else {
        return false;
    };
    level >= minimum
        || matches!((class, language),
            (Some(11), 12) if level > 7
        )
        || matches!((class, language), (Some(11), 17) if level > 19)
        || (language == 11 && (class == Some(14) || (matches!(class, Some(12 | 13)) && level > 7)))
}

// Rows: languages 0..24. Columns: races 1..12, Iksar, Vah Shir, Froglok.
const LANGUAGE_LEVELS: [[u8; 15]; 25] = [
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [10, 1, 15, 20, 20, 10, 15, 25, 15, 15, 10, 20, 15, 30, 30],
    [5, 20, 1, 35, 35, 23, 25, 35, 20, 20, 20, 35, 20, 10, 30],
    [20, 25, 30, 1, 1, 25, 1, 10, 25, 25, 25, 10, 25, 30, 30],
    [25, 10, 25, 15, 15, 1, 15, 15, 30, 30, 15, 15, 30, 30, 30],
    [20, 25, 30, 10, 10, 25, 10, 1, 20, 20, 25, 10, 20, 30, 30],
    [15, 15, 25, 25, 25, 13, 20, 25, 1, 5, 20, 30, 15, 30, 10],
    [15, 15, 25, 25, 25, 13, 20, 25, 5, 1, 20, 30, 15, 30, 20],
    [20, 25, 15, 10, 10, 25, 10, 15, 25, 25, 30, 1, 20, 30, 30],
    [15, 10, 20, 20, 20, 10, 15, 20, 20, 20, 1, 25, 25, 30, 30],
    [99; 15],
    [12; 15],
    [25, 30, 30, 15, 15, 30, 15, 30, 30, 30, 15, 20, 30, 30, 30],
    [15, 20, 20, 20, 20, 20, 20, 20, 15, 15, 20, 20, 15, 20, 1],
    [20, 15, 20, 15, 15, 15, 15, 20, 20, 20, 15, 15, 15, 20, 20],
    [10, 15, 10, 10, 10, 10, 10, 10, 10, 10, 10, 15, 15, 15, 15],
    [20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 1, 20],
    [12, 30, 30, 15, 15, 12, 15, 30, 12, 12, 15, 20, 12, 30, 30],
    [15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 1, 15, 15],
    [20; 15],
    [20; 15],
    [30; 15],
    [40; 15],
    [99; 15],
    [30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 1, 30],
];

/// The length of the request and of the guildmaster's answer.
const OFFER_LENGTH: usize = 448;

/// What the player asks of a guildmaster.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrainingRequest {
    /// Opens training with this guildmaster.
    Open {
        /// The guildmaster's spawn.
        trainer: u16,
    },
    /// Practices a skill once.
    Train {
        /// The skill's number.
        skill: u32,
    },
    /// Practices against the trainer shown in the host's window.
    TrainAt {
        /// The guildmaster shown in the open offer.
        trainer: u16,
        /// The selected skill.
        skill: u32,
    },
    /// Practices a typed subject from the currently displayed offer.
    PracticeAt {
        /// The guildmaster shown in the open offer.
        trainer: u16,
        /// The skill bank and entry selected in that offer.
        subject: TrainingSubject,
    },
    /// Leaves training.
    End,
}

/// The guildmaster's answer to an open request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TrainingOffer {
    /// The guildmaster's spawn.
    pub trainer: u16,
    /// By skill number, the most the guildmaster trains the skill to over
    /// the class's whole career; 0 for a skill they do not teach the player.
    /// A skill may stop below this at the player's level, which the server
    /// says when the player tries.
    pub caps: Vec<u32>,
    /// Language limits projected from received nonzero availability flags
    /// and the session's source-backed eligibility rules. The wire carries
    /// flags rather than numeric language caps (native eqgame 0x56b897).
    pub language_caps: Vec<u32>,
}

impl TrainingOffer {
    /// The current offer's limit in the selected bank; absent entries cannot train.
    #[must_use]
    pub fn subject_cap(&self, subject: TrainingSubject) -> u32 {
        match subject {
            TrainingSubject::Skill(skill) if skill < SKILLS => self.cap(skill),
            TrainingSubject::Language(language) if language < LANGUAGES => self
                .language_caps
                .get(usize::from(language))
                .copied()
                .unwrap_or(0),
            _ => 0,
        }
    }

    /// The most a skill can be trained to, 0 when it cannot be.
    #[must_use]
    pub fn cap(&self, skill: u32) -> u32 {
        usize::try_from(skill)
            .ok()
            .and_then(|skill| self.caps.get(skill))
            .copied()
            .unwrap_or(0)
    }
}

/// News of training.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum TrainingUpdate {
    /// A guildmaster opened training.
    Offered(TrainingOffer),
    /// `EQEmu`'s existing projection after a matching received increase. The
    /// ID is the update ID (100 + language ID for languages); cost and
    /// point changes are projections, not receipt fields.
    Trained {
        /// The received update ID: ordinary skill or 100 plus language ID.
        skill: u32,
        /// Its value now.
        value: u32,
        /// What the practice cost, in copper.
        cost: u64,
    },
    /// A Titanium practice and the limits of what can be inferred from its updates.
    Practice(PracticeUpdate),
    /// Training ended.
    Ended,
}

/// Authority of the point count displayed in the training window.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PracticePoints {
    /// No current balance is known.
    #[default]
    Unknown,
    /// Balance from the profile or the existing `EQEmu` policy.
    Balance(u32),
}

/// Progress and uncertainty of a Titanium practice. A regular skill update does not
/// contain a practice receipt, a point balance or a price.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum PracticeUpdate {
    /// One request was sent, making the current point balance unknown.
    Requested {
        /// The requested skill.
        skill: u32,
        /// The client-side price estimate in copper.
        estimated_cost: u64,
    },
    /// The requested skill rose during observation. This carries no spend receipt.
    SkillRaised {
        /// The requested skill.
        skill: u32,
        /// Its received new value.
        value: u32,
    },
    /// Observation ended without a confirmed practice outcome.
    Unanswered {
        /// The requested skill.
        skill: u32,
    },
    /// An `EQEmu` language operation was retired without a received outcome.
    /// The estimate only identifies potentially spent coins; it is not a receipt.
    OutcomeUnknown {
        /// The received-update ID of the requested language.
        skill: u32,
        /// The request's estimated cost in copper.
        estimated_cost: u64,
    },
    /// The profile point count can no longer be projected.
    PointsUncertain,
}

/// A Titanium training reply before admission and policy checks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrainingReply {
    /// The player spawn named by the reply.
    pub trainee: u32,
    /// Whether the trainer admitted the player.
    pub accepted: bool,
    /// Raw availability flags for the 32 language slots, from bytes 413..445.
    /// Nonzero permits the native fixed cap of 100; the bytes are not caps.
    pub language_flags: [u8; 32],
    /// The trainer and their skill limits.
    pub offer: TrainingOffer,
}

/// Whether a guildmaster of this class trains a player of that one: each
/// class's guildmaster class is the class's number plus 19, from the
/// warrior's 20 to the berserker's 35.
#[must_use]
pub fn trains(trainer_class: u8, player_class: u32) -> bool {
    (1..=16).contains(&player_class) && u32::from(trainer_class) == player_class + 19
}

/// What a practice costs, in copper, from the skill's value before it:
/// nothing for a new skill or one up to 10, and then the cube of the value
/// over 10, divided by 100 (`Client::OPGMTrainSkill`).
#[must_use]
pub fn practice_cost(value: u32) -> u64 {
    let over = u64::from(value.saturating_sub(10));
    u64::try_from(u128::from(over).pow(3) / 100).unwrap_or(u64::MAX)
}

/// Decodes a Titanium training packet from the server: the guildmaster's
/// answer to an open request; None for any other opcode.
///
/// # Errors
/// Rejects an answer [`titanium_offer`] cannot read.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<TrainingUpdate>> {
    Ok(match opcode {
        OFFER_OPCODE => Some(TrainingUpdate::Offered(titanium_offer(body)?)),
        _ => None,
    })
}

/// Encodes asking a guildmaster to train: the guildmaster's and the player's
/// spawns, and room for the answer.
#[must_use]
pub fn titanium_open(trainer: u16, player: u16) -> EncodedCommand {
    let mut body = vec![0; OFFER_LENGTH];
    body[..4].copy_from_slice(&u32::from(trainer).to_le_bytes());
    body[4..8].copy_from_slice(&u32::from(player).to_le_bytes());
    EncodedCommand {
        opcode: OFFER_OPCODE,
        body,
    }
}

/// Decodes the guildmaster's answer: the guildmaster, then the most each of
/// the hundred skills can be trained to.
///
/// # Errors
/// Rejects an answer of the wrong length, and one without a guildmaster.
pub fn titanium_offer(body: &[u8]) -> Result<TrainingOffer> {
    ensure!(body.len() == OFFER_LENGTH, "invalid training offer length");
    let word = |offset: usize| {
        u32::from_le_bytes([
            body[offset],
            body[offset + 1],
            body[offset + 2],
            body[offset + 3],
        ])
    };
    let trainer = u16::try_from(word(0))?;
    ensure!(trainer != 0, "training offer without a guildmaster");
    Ok(TrainingOffer {
        trainer,
        caps: (0..100).map(|skill| word(8 + skill * 4)).collect(),
        language_caps: Vec::new(),
    })
}

/// Reads the admission flag and intended player alongside the ordinary offer.
///
/// # Errors
/// Rejects a malformed training record or invalid trainer.
pub fn titanium_reply(body: &[u8]) -> Result<TrainingReply> {
    Ok(TrainingReply {
        offer: titanium_offer(body)?,
        trainee: u32::from_le_bytes(body[4..8].try_into()?),
        accepted: body[412] != 0,
        language_flags: body[413..445].try_into()?,
    })
}

/// Applies the native skill-bank display rules to a received P99 offer.
#[must_use]
pub fn p99_offer(mut offer: TrainingOffer) -> TrainingOffer {
    for (skill, cap) in offer.caps.iter_mut().enumerate() {
        if matches!(skill, 15 | 33 | 72) || i32::try_from(*cap).is_err() {
            *cap = 0;
        }
    }
    offer
}

/// Encodes one practice of a skill with a guildmaster.
///
/// # Errors
/// Rejects a number that is not a skill's.
pub fn titanium_train(trainer: u16, skill: u32) -> Result<EncodedCommand> {
    titanium_practice(trainer, TrainingSubject::Skill(skill))
}

/// Encodes a practice without borrowing the other bank's ID namespace.
///
/// # Errors
/// Rejects ordinary skill IDs outside 0..100 and language IDs outside 0..26.
pub fn titanium_practice(trainer: u16, subject: TrainingSubject) -> Result<EncodedCommand> {
    let (bank, id) = match subject {
        TrainingSubject::Skill(skill) => {
            ensure!(skill < SKILLS, "not a skill a guildmaster trains");
            (0u16, u16::try_from(skill)?)
        }
        TrainingSubject::Language(language) => {
            ensure!(language < LANGUAGES, "not a language a guildmaster trains");
            (1, u16::from(language))
        }
    };
    let mut body = vec![0; 12];
    body[..2].copy_from_slice(&trainer.to_le_bytes());
    body[4..6].copy_from_slice(&bank.to_le_bytes());
    body[8..10].copy_from_slice(&id.to_le_bytes());
    Ok(EncodedCommand {
        opcode: TRAIN_OPCODE,
        body,
    })
}

/// Encodes leaving training with a guildmaster.
#[must_use]
pub fn titanium_end(trainer: u16, player: u16) -> EncodedCommand {
    let mut body = vec![0; 8];
    body[..4].copy_from_slice(&u32::from(trainer).to_le_bytes());
    body[4..].copy_from_slice(&u32::from(player).to_le_bytes());
    EncodedCommand {
        opcode: END_OPCODE,
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_training_banks_do_not_share_request_ids() {
        let ordinary = titanium_train(42, 5).unwrap();
        let language = titanium_practice(42, TrainingSubject::Language(5)).unwrap();
        assert_eq!(ordinary.body[4..6], [0, 0]);
        assert_eq!(language.body, [42, 0, 0, 0, 1, 0, 0, 0, 5, 0, 0, 0]);
        assert_eq!(TrainingSubject::Language(5).update_id(), 105);
        assert!(titanium_practice(42, TrainingSubject::Language(25)).is_ok());
        assert!(titanium_practice(42, TrainingSubject::Language(26)).is_err());
        assert!(titanium_practice(42, TrainingSubject::Skill(100)).is_err());
        let offer = titanium_offer(&titanium_open(42, 7).body).unwrap();
        assert_eq!(
            offer.language_caps,
            [],
            "the numeric skill-cap decoder does not invent language policy"
        );
        assert_eq!(offer.subject_cap(TrainingSubject::Language(5)), 0);
        let mut body = titanium_open(42, 7).body;
        body[413 + 5] = 0xc9;
        let reply = titanium_reply(&body).unwrap();
        assert_eq!(reply.language_flags[5], 0xc9);
        assert_eq!(reply.language_flags[4], 0);
    }

    #[test]
    fn native_language_training_keeps_racial_levels_and_class_overrides() {
        assert!(!titanium_language_trainable(1, 1, Some(1), 9));
        assert!(titanium_language_trainable(1, 1, Some(1), 10));
        for (race, language) in [(128, 18), (130, 24), (330, 13)] {
            assert!(titanium_language_trainable(language, race, Some(1), 1));
        }
        for (class, language, level) in [(11, 12, 8), (11, 17, 20), (12, 11, 8), (13, 11, 8)] {
            assert!(!titanium_language_trainable(
                language,
                1,
                Some(class),
                level - 1
            ));
            assert!(titanium_language_trainable(language, 1, Some(class), level));
        }
        assert!(titanium_language_trainable(11, 1, Some(14), 1));
        assert!(!titanium_language_trainable(25, 1, Some(14), 100));
        assert!(!titanium_language_trainable(1, 999, Some(1), 100));
        // Every racial column has its own exact boundary, without importing it into EQEmu.
        for (column, race) in (1..=12).chain([128, 130, 330]).enumerate() {
            for (language, row) in LANGUAGE_LEVELS.iter().enumerate() {
                let language = u8::try_from(language).unwrap();
                assert!(!titanium_language_trainable(
                    language,
                    race,
                    Some(1),
                    row[column] - 1
                ));
                assert!(titanium_language_trainable(
                    language,
                    race,
                    Some(1),
                    row[column]
                ));
            }
        }
    }

    #[test]
    fn opening_asks_with_both_spawns_and_the_answer_names_each_cap() {
        let open = titanium_open(42, 7);
        assert_eq!(open.opcode, OFFER_OPCODE);
        assert_eq!(open.body.len(), 448);
        assert_eq!(open.body[..8], [42, 0, 0, 0, 7, 0, 0, 0]);
        assert!(open.body[8..].iter().all(|byte| *byte == 0));
        let mut answer = open.body;
        answer[8 + 10 * 4] = 200;
        answer[8 + 30 * 4] = 5;
        let offer = titanium_offer(&answer).unwrap();
        assert_eq!(offer.trainer, 42);
        assert_eq!((offer.cap(10), offer.cap(30), offer.cap(0)), (200, 5, 0));
        assert_eq!(offer.cap(100), 0);
        assert_eq!(
            decode(OFFER_OPCODE, &answer).unwrap(),
            Some(TrainingUpdate::Offered(offer))
        );
        assert_eq!(decode(TRAIN_OPCODE, &answer).unwrap(), None);
        assert!(titanium_offer(&answer[..447]).is_err());
        assert!(titanium_offer(&[0; 448]).is_err(), "no guildmaster");
    }

    #[test]
    fn a_practice_names_the_guildmaster_and_the_skill_and_leaving_both_spawns() {
        let train = titanium_train(42, 30).unwrap();
        assert_eq!(train.opcode, TRAIN_OPCODE);
        assert_eq!(train.body, [42, 0, 0, 0, 0, 0, 0, 0, 30, 0, 0, 0]);
        assert!(titanium_train(42, 100).is_err());
        let end = titanium_end(42, 7);
        assert_eq!(end.opcode, END_OPCODE);
        assert_eq!(end.body, [42, 0, 0, 0, 7, 0, 0, 0]);
    }

    #[test]
    fn native_reply_carries_trainee_admission_and_positive_visible_caps() {
        let mut body = titanium_open(42, 7).body;
        assert!(!titanium_reply(&body).unwrap().accepted);
        body[412] = 1;
        for (skill, cap) in [(15, 200u32), (30, 200), (31, u32::MAX), (72, 200)] {
            let at = 8 + skill * 4;
            body[at..at + 4].copy_from_slice(&cap.to_le_bytes());
        }
        let reply = titanium_reply(&body).unwrap();
        assert!(reply.accepted);
        assert_eq!(reply.trainee, 7);
        let offer = p99_offer(reply.offer);
        assert_eq!(
            (offer.cap(15), offer.cap(30), offer.cap(31), offer.cap(72)),
            (0, 200, 0, 0)
        );
        assert!(titanium_reply(&body[..412]).is_err());
    }

    #[test]
    fn guildmasters_train_their_own_class_for_a_price_past_ten() {
        assert!(trains(20, 1), "a warrior guildmaster trains warriors");
        assert!(trains(35, 16));
        assert!(!trains(21, 1) && !trains(41, 22) && !trains(19, 0));
        assert_eq!((practice_cost(0), practice_cost(10)), (0, 0));
        assert_eq!(practice_cost(11), 0, "one cubed is under a hundred");
        assert_eq!(practice_cost(15), 1);
        assert_eq!(practice_cost(110), 10_000);
    }
}
