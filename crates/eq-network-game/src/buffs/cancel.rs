//! Explicit cancellation of an ordinary beneficial buff. Slot identity is a
//! local safety check: Titanium's server request identifies the spell.
use super::Buff;
use crate::command::EncodedCommand;
use anyhow::{ensure, Result};
use serde::Serialize;
use std::{collections::BTreeMap, time::Instant};

/// A selection made from a current, explicit own-player buff slot.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// Zone admission that owns the selection.
    pub session_id: u64,
    /// Revision of received buff slots, independent of countdowns and icons.
    pub revision: u64,
    /// Actual server slot, never a displayed ordinal for an unslotted effect.
    pub slot: u32,
    /// The spell expected in that slot.
    pub spell_id: u32,
    /// The host's current spell provider classification. Unknown stays unknown;
    /// neither the buff record's effect type nor an icon establishes this.
    pub beneficial: Option<bool>,
    /// Current provider confirmation that no native non-cancellable effect is
    /// present. Titanium excludes effect 101 in its ordinary cancel path.
    pub cancellable: Option<bool>,
    /// When the player selected this buff.
    pub created: Instant,
}

/// What happened to a request. Receipt of a fade does not establish its cause.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Progress {
    /// Transport submission only; received membership is unchanged.
    Submitted,
    /// A received fade matched the selected slot and spell during the local
    /// wait. It may be natural expiry; it does not acknowledge cancellation.
    FadeObserved,
    /// The received slot changed; the request never follows its replacement.
    Changed,
    /// No matching change arrived within the local waiting interval.
    Unresolved,
    /// This client did not submit the request.
    Refused(String),
}

/// An admission-scoped cancellation report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Update {
    /// Admission that owns this report.
    pub session_id: u64,
    /// Received slot revision used by the selection.
    pub revision: u64,
    /// Selected server slot.
    pub slot: u32,
    /// Selected spell.
    pub spell_id: u32,
    /// Submission, observation or refusal, with no invented acknowledgement.
    pub progress: Progress,
}

impl Request {
    /// An outcome tied to this exact selection.
    #[must_use]
    pub fn update(&self, progress: Progress) -> Update {
        Update {
            session_id: self.session_id,
            revision: self.revision,
            slot: self.slot,
            spell_id: self.spell_id,
            progress,
        }
    }

    /// Revalidates the selected slot and the host's explicit provider result.
    ///
    /// # Errors
    /// Refuses absent, stale, harmful, unknown, replaced or ambiguous buffs.
    pub fn check(&self, revision: u64, slots: Option<&BTreeMap<u32, Buff>>) -> Result<()> {
        ensure!(
            self.beneficial == Some(true),
            "Only known beneficial buffs can be cancelled"
        );
        ensure!(
            self.cancellable == Some(true),
            "This buff is not known to allow cancellation"
        );
        ensure!(
            self.revision == revision,
            "The buff slots changed; select the buff again"
        );
        ensure!(
            !matches!(self.spell_id, 0 | 0xffff | u32::MAX),
            "Invalid buff spell"
        );
        let slots = slots.ok_or_else(|| anyhow::anyhow!("Buff slots are unavailable"))?;
        ensure!(
            slots
                .get(&self.slot)
                .is_some_and(|buff| buff.spell_id == self.spell_id && buff.effect_type != 0),
            "The selected buff slot changed or is unavailable"
        );
        ensure!(
            slots
                .values()
                .filter(|buff| buff.spell_id == self.spell_id)
                .count()
                == 1,
            "The spell occupies more than one buff slot"
        );
        Ok(())
    }
}

/// Titanium's C2S `OP_Buff`, after identity and provider validation.
///
/// # Errors
/// Refuses a missing own-player identity or a sentinel spell.
pub fn titanium_packet(own_id: u16, slot: u32, buff: &Buff) -> Result<EncodedCommand> {
    ensure!(
        own_id != 0 && !matches!(buff.spell_id, 0 | 0xffff | u32::MAX),
        "Invalid buff cancellation"
    );
    let mut body = vec![0; 32];
    body[..4].copy_from_slice(&u32::from(own_id).to_le_bytes());
    body[4] = buff.effect_type;
    body[5] = buff.caster_level;
    body[6] = buff.bard_modifier;
    body[8..12].copy_from_slice(&buff.spell_id.to_le_bytes());
    body[12..16].copy_from_slice(&buff.duration_ticks.to_le_bytes());
    body[16..20].copy_from_slice(&buff.counters.to_le_bytes());
    body[20..24].copy_from_slice(&buff.caster_id.to_le_bytes());
    body[24..28].copy_from_slice(&slot.to_le_bytes());
    Ok(EncodedCommand {
        opcode: 0x6a53,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buff(spell_id: u32) -> Buff {
        Buff {
            spell_id,
            caster_level: 12,
            effect_type: 2,
            bard_modifier: 10,
            duration_ticks: 30,
            counters: 4,
            caster_id: 9,
        }
    }

    fn request() -> Request {
        Request {
            session_id: 5,
            revision: 7,
            slot: 3,
            spell_id: 42,
            beneficial: Some(true),
            cancellable: Some(true),
            created: Instant::now(),
        }
    }

    #[test]
    fn cancellation_requires_current_explicit_unambiguous_beneficial_membership() {
        let mut slots = BTreeMap::from([(3, buff(42))]);
        let mut request = request();
        assert!(request.check(7, Some(&slots)).is_ok());
        assert!(request.check(8, Some(&slots)).is_err());
        assert!(request.check(7, None).is_err());
        for beneficial in [None, Some(false)] {
            request.beneficial = beneficial;
            assert!(request.check(7, Some(&slots)).is_err());
        }
        request.beneficial = Some(true);
        for cancellable in [None, Some(false)] {
            request.cancellable = cancellable;
            assert!(request.check(7, Some(&slots)).is_err());
        }
        request.cancellable = Some(true);
        slots.insert(3, buff(43));
        assert!(request.check(7, Some(&slots)).is_err());
        slots.insert(3, buff(42));
        slots.insert(4, buff(42));
        assert!(request.check(7, Some(&slots)).is_err());
        slots.clear();
        assert!(request.check(7, Some(&slots)).is_err());
    }

    #[test]
    fn titanium_cancellation_has_its_own_constructed_wire_contract() {
        let packet = titanium_packet(7, 3, &buff(42)).unwrap();
        assert_eq!(packet.opcode, 0x6a53);
        assert_eq!(packet.body.len(), 32);
        assert_eq!(
            packet.body,
            [
                7, 0, 0, 0, 2, 12, 10, 0, 42, 0, 0, 0, 30, 0, 0, 0, 4, 0, 0, 0, 9, 0, 0, 0, 3, 0,
                0, 0, 0, 0, 0, 0,
            ]
        );
        assert!(titanium_packet(0, 3, &buff(42)).is_err());
        for spell in [0, 0xffff, u32::MAX] {
            assert!(titanium_packet(7, 3, &buff(spell)).is_err());
        }
    }
}
