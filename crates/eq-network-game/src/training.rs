//! Training skills at a guildmaster, as the official client's training
//! window does. Asking a guildmaster of the player's own class opens
//! training, and the guildmaster answers with how far each skill can be
//! trained; each practice raises one skill by a point for a practice point
//! and, past a skill of 10, coins; leaving says goodbye. An ordinary skill
//! update has no receipt fields. `EQEmu` retains the existing local point
//! and purse projection. P99 follows Titanium press-time spending (6.683).
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
/// `EQMac`'s `OP_GMTraining` (0x409c swapped): asking a guildmaster to
/// train, which TAKP and Quarm answer with the same 244 bytes filled in
/// (`GMTrainee_Struct`, `Client::OPGMTraining` at TAKP 44afcbf and Quarm
/// fc75fcf).
pub const EQMAC_OFFER_OPCODE: u16 = 0x9c40;
/// `EQMac`'s `OP_GMTrainSkill` (0x4140 swapped): one practice, the 12-byte
/// `GMSkillChange_Struct`.
pub const EQMAC_TRAIN_OPCODE: u16 = 0x4041;
/// `EQMac`'s `OP_GMEndTraining` (0x409d swapped): leaving training, the
/// 4-byte `GMTrainEnd_Struct`.
pub const EQMAC_END_OPCODE: u16 = 0x9d40;

/// `EQMac`'s `OP_SkillUpdate2` (0x41be swapped): a skill's new value that
/// prints no line, which TAKP and Quarm send for a practice
/// (`Client::SetSkill` with `silent`), as `SkillUpdate2_Struct`: the spawn,
/// two bytes of padding, then the skill and its value as 32-bit integers.
pub const EQMAC_SKILL_OPCODE: u16 = 0xbe41;

/// The length of `EQMac`'s request and of the guildmaster's answer.
const EQMAC_OFFER_LENGTH: usize = 244;
/// Where the `EQMac` answer keeps the guildmaster's price scale (`greed`,
/// `Client::CalcPriceMod`), whether it admitted the player (`success`), and
/// the 32 language flags.
const EQMAC_GREED: usize = 204;
const EQMAC_SUCCESS: usize = 208;
const EQMAC_LANGUAGES: usize = 209;

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
/// table at 0x78f010. Applied alongside received availability on every server
/// with supported language training; proficiency does not bypass this rule.
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
    level >= minimum || class_learns(language, class, level)
}

/// The languages a class may learn whatever its race's table says, as both
/// the Titanium program (0x4bbeda) and the `EQMac` program (0x4ef555) allow
/// them: a necromancer (11) Dark Speech (12) past level 7 and Elder Elvish
/// (17) past 19, and Thieves' Cant (11) past level 7 for classes 12 and 13
/// and always for class 14.
fn class_learns(language: u8, class: Option<u32>, level: u8) -> bool {
    matches!((class, language), (Some(11), 12) if level > 7)
        || matches!((class, language), (Some(11), 17) if level > 19)
        || (language == 11 && (class == Some(14) || (matches!(class, Some(12 | 13)) && level > 7)))
}

/// How many languages the `EQMac` program's minimum-level table covers.
pub const EQMAC_TABLE_LANGUAGES: usize = 25;
/// The races the table has a column for, in its order: 1 to 12, Iksar and
/// Vah Shir.
pub const EQMAC_TABLE_RACES: [u32; 14] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 128, 130];

/// The `EQMac` program's minimum level for each language and race, which
/// its training gate checks (0x4ef555): a host reads it from the installed
/// program (races 1 to 12 at 0x60b0ac + (language x 14 + race) x 4, Iksar
/// at 0x60b0e0 + language x 0x38, Vah Shir at 0x60b0e4 + language x 0x38)
/// and hands it to the session through its configuration. No values are
/// kept here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageLevels([[u32; 14]; EQMAC_TABLE_LANGUAGES]);

impl LanguageLevels {
    /// Takes a table read from an installed program, by language then by
    /// [`EQMAC_TABLE_RACES`], if it looks like the program's: every race may
    /// speak the first language (0) at level 1, and no race at all learns
    /// languages 10 or 23 below 99. None for any other table, which keeps
    /// language training shut.
    #[must_use]
    pub fn checked(rows: [[u32; 14]; EQMAC_TABLE_LANGUAGES]) -> Option<Self> {
        let all = |row: &[u32; 14], value: u32| row.iter().all(|cell| *cell == value);
        (all(&rows[0], 1) && all(&rows[10], 99) && all(&rows[23], 99)).then_some(Self(rows))
    }

    /// The minimum level for this language and race; None for a language or
    /// race the table has no place for.
    #[must_use]
    pub fn minimum(&self, language: u8, race: u32) -> Option<u32> {
        let column = EQMAC_TABLE_RACES.iter().position(|known| *known == race)?;
        Some(self.0.get(usize::from(language))?[column])
    }
}

/// The `EQMac` program's rule for whether the player may learn a language
/// (0x4ef555, read 6.597): their level at least the table's minimum for
/// the language and race, or one of the class exceptions Titanium has too.
/// A language or race outside the table is refused.
#[must_use]
pub fn eqmac_language_trainable(
    levels: &LanguageLevels,
    language: u8,
    race: u32,
    class: Option<u32>,
    level: u8,
) -> bool {
    levels
        .minimum(language, race)
        .is_some_and(|minimum| u32::from(level) >= minimum || class_learns(language, class, level))
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
    /// `EQMac`'s price scale for this guildmaster, by which TAKP and Quarm
    /// multiply a practice's cost; None on Titanium, whose `EQEmu` prices a
    /// practice without one.
    pub price_scale: Option<PriceScale>,
}

/// A guildmaster's price scale as the `EQMac` answer carries it: a 32-bit
/// float, kept by its bits so offers compare exactly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PriceScale(u32);

impl PriceScale {
    /// The scale from its float.
    #[must_use]
    pub const fn new(scale: f32) -> Self {
        Self(scale.to_bits())
    }

    /// The scale.
    #[must_use]
    pub const fn get(self) -> f32 {
        f32::from_bits(self.0)
    }
}

impl Serialize for PriceScale {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_f32(self.get())
    }
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
    /// Titanium's local accepted Practice press (0x56b48e, read 6.683).
    /// This spends locally; it is not a server acknowledgement.
    Practiced {
        /// The selected bank and entry.
        subject: TrainingSubject,
        /// The value at the press, before any local increment.
        from: u32,
        /// The local price paid, in copper.
        cost: u64,
    },
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
    /// Balance from the profile or a supported native/local projection.
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

/// What a practice costs on TAKP and Quarm, in copper, from the skill's
/// value before it and the guildmaster's price scale: nothing up to 10, and
/// then the cube of the value over 10, times the scale, times 0.0099999998,
/// cut to a whole number, as `Client::OPGMTrainSkill` computes it in double
/// precision (TAKP 44afcbf, Quarm fc75fcf). A new skill costs nothing.
#[must_use]
pub fn eqmac_practice_cost(value: u32, scale: PriceScale) -> u64 {
    let over = f64::from(value.saturating_sub(10));
    let cost = over * over * over * f64::from(scale.get()) * 0.009_999_999_8;
    if cost.is_finite() && cost >= 1.0 {
        // Truncated as the server's cast to int does.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let whole = cost.trunc() as u64;
        whole
    } else {
        0
    }
}

/// What the `EQMac` program asks a practice to cost before it sends one
/// (0x4ef6c7, read 6.597): the value over 10, cubed, times the
/// guildmaster's scale, times 0.01, cut to whole copper; nothing up to 10.
/// The server then takes its own figure ([`eqmac_practice_cost`]), which
/// can be a copper apart.
#[must_use]
pub fn eqmac_program_cost(value: u32, scale: f32) -> u64 {
    let over = f64::from(value.saturating_sub(10));
    let cost = over * over * over * f64::from(scale) * 0.01;
    if cost.is_finite() && cost >= 1.0 {
        // Cut toward zero, as the program's conversion does.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let whole = cost.trunc() as u64;
        whole
    } else {
        0
    }
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
        price_scale: None,
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

/// Encodes asking a guildmaster to train on TAKP and Quarm: the
/// guildmaster's and the player's spawns as 16-bit IDs, and room for the
/// answer. Both servers drop any other length.
#[must_use]
pub fn eqmac_open(trainer: u16, player: u16) -> EncodedCommand {
    let mut body = vec![0; EQMAC_OFFER_LENGTH];
    body[..2].copy_from_slice(&trainer.to_le_bytes());
    body[2..4].copy_from_slice(&player.to_le_bytes());
    EncodedCommand {
        opcode: EQMAC_OFFER_OPCODE,
        body,
    }
}

/// Reads TAKP's and Quarm's answer to an open request: the guildmaster,
/// the player, each skill's most as a 16-bit value, the price scale, whether
/// the guildmaster admitted the player, and the language flags. A refusal
/// for faction is the request sent back with `success` 0.
///
/// # Errors
/// Rejects an answer of the wrong length, and one without a guildmaster.
pub fn eqmac_reply(body: &[u8]) -> Result<TrainingReply> {
    ensure!(
        body.len() == EQMAC_OFFER_LENGTH,
        "invalid EQMac training offer length"
    );
    let short = |at: usize| u16::from_le_bytes([body[at], body[at + 1]]);
    let trainer = short(0);
    ensure!(trainer != 0, "training offer without a guildmaster");
    let scale = f32::from_le_bytes(body[EQMAC_GREED..EQMAC_GREED + 4].try_into()?);
    Ok(TrainingReply {
        trainee: u32::from(short(2)),
        accepted: body[EQMAC_SUCCESS] != 0,
        language_flags: body[EQMAC_LANGUAGES..EQMAC_LANGUAGES + 32].try_into()?,
        offer: TrainingOffer {
            trainer,
            caps: (0..100)
                .map(|skill| u32::from(short(4 + skill * 2)))
                .collect(),
            language_caps: Vec::new(),
            price_scale: Some(PriceScale::new(scale)),
        },
    })
}

/// A skill's new value for a spawn, as `EQMac`'s silent update names it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpawnSkill {
    /// The spawn whose skill it is.
    pub spawn_id: u16,
    /// The skill.
    pub skill_id: u32,
    /// Its value now.
    pub value: u32,
}

/// Reads `EQMac`'s silent skill update.
///
/// # Errors
/// Rejects a body other than 12 bytes, and a negative skill or value.
pub fn eqmac_skill(body: &[u8]) -> Result<SpawnSkill> {
    ensure!(body.len() == 12, "invalid EQMac silent skill update length");
    let int = |at: usize| i32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
    Ok(SpawnSkill {
        spawn_id: u16::from_le_bytes([body[0], body[1]]),
        skill_id: u32::try_from(int(4)).map_err(|_| anyhow::anyhow!("negative skill"))?,
        value: u32::try_from(int(8)).map_err(|_| anyhow::anyhow!("negative skill value"))?,
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

/// Encodes one practice on TAKP and Quarm: Titanium's 12 bytes with the
/// player's spawn at +2, as `GMSkillChange_Struct` names it, on the Mac
/// opcode.
///
/// # Errors
/// As [`titanium_practice`].
pub fn eqmac_practice(
    trainer: u16,
    player: u16,
    subject: TrainingSubject,
) -> Result<EncodedCommand> {
    let mut practice = titanium_practice(trainer, subject)?;
    practice.opcode = EQMAC_TRAIN_OPCODE;
    practice.body[2..4].copy_from_slice(&player.to_le_bytes());
    Ok(practice)
}

/// Encodes leaving training on TAKP and Quarm: the guildmaster's and the
/// player's spawns as 16-bit IDs.
#[must_use]
pub fn eqmac_end(trainer: u16, player: u16) -> EncodedCommand {
    let mut body = vec![0; 4];
    body[..2].copy_from_slice(&trainer.to_le_bytes());
    body[2..].copy_from_slice(&player.to_le_bytes());
    EncodedCommand {
        opcode: EQMAC_END_OPCODE,
        body,
    }
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

    /// A made-up table that passes the shape check: language 0 at 1, 10 and
    /// 23 at 99, everything else at 20 but language 5, which race 2 and Vah
    /// Shir learn at 1.
    fn made_up_levels() -> LanguageLevels {
        let mut rows = [[20; 14]; EQMAC_TABLE_LANGUAGES];
        rows[0] = [1; 14];
        rows[10] = [99; 14];
        rows[23] = [99; 14];
        rows[5][1] = 1;
        rows[5][13] = 1;
        LanguageLevels::checked(rows).unwrap()
    }

    #[test]
    fn a_language_table_must_look_like_the_programs() {
        let good = made_up_levels();
        assert_eq!(good.minimum(5, 2), Some(1));
        assert_eq!(good.minimum(5, 130), Some(1));
        assert_eq!(good.minimum(5, 1), Some(20));
        assert_eq!(good.minimum(25, 1), None);
        assert_eq!(good.minimum(5, 330), None);
        let mut rows = [[20; 14]; EQMAC_TABLE_LANGUAGES];
        rows[0] = [1; 14];
        rows[10] = [99; 14];
        assert_eq!(LanguageLevels::checked(rows), None);
        assert_eq!(
            LanguageLevels::checked([[0; 14]; EQMAC_TABLE_LANGUAGES]),
            None
        );
    }

    #[test]
    fn the_mac_gate_takes_the_tables_level_or_a_class_exception() {
        let levels = made_up_levels();
        assert!(!eqmac_language_trainable(&levels, 5, 1, Some(1), 19));
        assert!(eqmac_language_trainable(&levels, 5, 1, Some(1), 20));
        assert!(eqmac_language_trainable(&levels, 5, 2, Some(1), 1));
        // The class exceptions, the same as Titanium's.
        assert!(!eqmac_language_trainable(&levels, 12, 1, Some(11), 7));
        assert!(eqmac_language_trainable(&levels, 12, 1, Some(11), 8));
        assert!(eqmac_language_trainable(&levels, 17, 1, Some(11), 20));
        assert!(eqmac_language_trainable(&levels, 11, 1, Some(14), 1));
        assert!(eqmac_language_trainable(&levels, 11, 1, Some(12), 8));
        assert!(!eqmac_language_trainable(&levels, 11, 1, Some(12), 7));
        // Outside the table: refused.
        assert!(!eqmac_language_trainable(&levels, 25, 1, Some(14), 60));
        assert!(!eqmac_language_trainable(&levels, 0, 330, Some(1), 60));
    }

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
            Vec::<u32>::new(),
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
        // Use races whose ordinary threshold is above each override. Human
        // Elder Teir'Dal starts at12, so it cannot test the Necromancer20 edge.
        for (race, class, language, level) in [
            (1, 11, 12, 8),
            (2, 11, 17, 20),
            (1, 12, 11, 8),
            (1, 13, 11, 8),
        ] {
            assert!(
                !titanium_language_trainable(language, race, Some(class), level - 1),
                "race {race}, class {class}, language {language}: below override {level}"
            );
            assert!(titanium_language_trainable(
                language,
                race,
                Some(class),
                level
            ));
            assert!(
                !titanium_language_trainable(language, race, Some(1), level),
                "the racial threshold must not supply this class-override fixture"
            );
        }
        // Racial eligibility can precede a class override; the override never
        // removes an earlier grant (native eqgame0x4bbeda, table0x78f010).
        assert!(!titanium_language_trainable(17, 1, Some(11), 11));
        assert!(titanium_language_trainable(17, 1, Some(11), 12));
        assert!(titanium_language_trainable(17, 1, Some(11), 19));
        assert!(titanium_language_trainable(11, 1, Some(14), 1));
        assert!(!titanium_language_trainable(25, 1, Some(14), 100));
        assert!(!titanium_language_trainable(1, 999, Some(1), 100));
        // Every racial column has its own exact boundary.
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

    #[test]
    fn eqmac_training_is_the_mac_program_s_own_bytes() {
        let open = eqmac_open(0x0123, 0x0456);
        assert_eq!(open.opcode, EQMAC_OFFER_OPCODE);
        let mut expected = vec![0; 244];
        expected[..4].copy_from_slice(&[0x23, 0x01, 0x56, 0x04]);
        assert_eq!(open.body, expected);

        let skill = eqmac_practice(0x0123, 0x0456, TrainingSubject::Skill(33)).unwrap();
        assert_eq!(
            (skill.opcode, skill.body),
            (
                EQMAC_TRAIN_OPCODE,
                vec![0x23, 0x01, 0x56, 0x04, 0, 0, 0, 0, 33, 0, 0, 0]
            )
        );
        let language = eqmac_practice(0x0123, 0x0456, TrainingSubject::Language(25)).unwrap();
        assert_eq!(
            language.body,
            [0x23, 0x01, 0x56, 0x04, 1, 0, 0, 0, 25, 0, 0, 0]
        );
        assert!(eqmac_practice(1, 2, TrainingSubject::Skill(100)).is_err());
        assert!(eqmac_practice(1, 2, TrainingSubject::Language(26)).is_err());

        let end = eqmac_end(0x0123, 0x0456);
        assert_eq!(
            (end.opcode, end.body),
            (EQMAC_END_OPCODE, vec![0x23, 0x01, 0x56, 0x04])
        );
    }

    #[test]
    fn an_eqmac_answer_reads_sixteen_bit_caps_its_scale_and_admission() {
        let mut body = eqmac_open(0x0123, 0x0456).body;
        body[4 + 2 * 33..4 + 2 * 33 + 2].copy_from_slice(&200u16.to_le_bytes());
        body[4 + 2 * 99..4 + 2 * 99 + 2].copy_from_slice(&5u16.to_le_bytes());
        body[204..208].copy_from_slice(&1.25f32.to_le_bytes());
        body[208] = 1;
        body[209..241].fill(201);
        let reply = eqmac_reply(&body).unwrap();
        assert_eq!((reply.trainee, reply.accepted), (0x0456, true));
        assert_eq!(reply.language_flags, [201; 32]);
        assert_eq!(reply.offer.trainer, 0x0123);
        assert_eq!(
            (reply.offer.cap(33), reply.offer.cap(99), reply.offer.cap(0)),
            (200, 5, 0)
        );
        assert_eq!(reply.offer.caps.len(), 100);
        assert_eq!(reply.offer.price_scale.map(PriceScale::get), Some(1.25));
        body[208] = 0;
        assert!(!eqmac_reply(&body).unwrap().accepted);
        assert!(eqmac_reply(&body[..243]).is_err());
        let mut nobody = body.clone();
        nobody[..2].fill(0);
        assert!(eqmac_reply(&nobody).is_err());
    }

    #[test]
    fn an_eqmac_practice_costs_the_server_s_scaled_cube() {
        let scale = |value: f32| PriceScale::new(value);
        // Up to 10 is free, whatever the scale.
        assert_eq!(eqmac_practice_cost(0, scale(1.25)), 0);
        assert_eq!(eqmac_practice_cost(10, scale(1.25)), 0);
        // 1 * 0.8 * 0.0099999998 and 1 * 1.25 * 0.0099999998 cut to 0.
        assert_eq!(eqmac_practice_cost(11, scale(0.8)), 0);
        assert_eq!(eqmac_practice_cost(11, scale(1.25)), 0);
        // 40^3 = 64000: 64000 * 0.8 (as f32) * 0.0099999998 = 511.99...
        assert_eq!(eqmac_practice_cost(50, scale(0.8)), 511);
        assert_eq!(eqmac_practice_cost(50, scale(1.25)), 799);
        // 190^3 = 6859000.
        assert_eq!(eqmac_practice_cost(200, scale(0.8)), 54871);
        assert_eq!(eqmac_practice_cost(200, scale(1.25)), 85737);
        assert_eq!(eqmac_practice_cost(200, scale(f32::NAN)), 0);
    }

    #[test]
    fn an_eqmac_silent_skill_update_names_its_spawn() {
        let body = [7, 0, 0xaa, 0xbb, 33, 0, 0, 0, 201, 0, 0, 0];
        assert_eq!(
            eqmac_skill(&body).unwrap(),
            SpawnSkill {
                spawn_id: 7,
                skill_id: 33,
                value: 201
            }
        );
        assert!(eqmac_skill(&body[..11]).is_err());
        let mut negative = body;
        negative[11] = 0x80;
        assert!(eqmac_skill(&negative).is_err());
    }
}
