//! Training skills at a guildmaster, as the official client's training
//! window does. Asking a guildmaster of the player's own class opens
//! training, and the guildmaster answers with how far each skill can be
//! trained; each practice raises one skill by a point for a practice point
//! and, past a skill of 10, coins; leaving says goodbye. Servers answer a
//! practice only with the skill's new value, so the cost and the practice
//! point are counted on this side, as the official client counts them.
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
}

impl TrainingOffer {
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
    /// A practice took: the skill's new value, and the coins, in copper, it
    /// cost.
    Trained {
        /// The skill's number.
        skill: u32,
        /// Its value now.
        value: u32,
        /// What the practice cost, in copper.
        cost: u64,
    },
    /// Training ended.
    Ended,
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
    over * over * over / 100
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
    })
}

/// Encodes one practice of a skill with a guildmaster.
///
/// # Errors
/// Rejects a number that is not a skill's.
pub fn titanium_train(trainer: u16, skill: u32) -> Result<EncodedCommand> {
    ensure!(skill < SKILLS, "not a skill a guildmaster trains");
    let mut body = vec![0; 12];
    body[..2].copy_from_slice(&trainer.to_le_bytes());
    // Bytes 4 and 5 say which bank the number is from: 0 for skills, 1 for
    // languages.
    body[8..10].copy_from_slice(&u16::try_from(skill)?.to_le_bytes());
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
