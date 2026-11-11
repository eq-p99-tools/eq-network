//! Explicit cancellation of an ordinary beneficial buff. Slot identity is a
//! local safety check: Titanium's server request identifies the spell. The
//! slot is a received one, or the one the program would have given a
//! landing the server named no slot for.
use super::Buff;
use crate::command::EncodedCommand;
use anyhow::{ensure, Result};
use serde::Serialize;
use std::{collections::BTreeMap, time::Instant};

/// Where the selected buff's slot comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Placement {
    /// A slot the server gave: the admission's buff table or a slot write.
    Received,
    /// The slot the Titanium program gives a landing it was not told the
    /// slot of ([`super::placement`]), with the record the program would
    /// hold there, as far as it is read (R20 (d)); its unread fields are 0.
    /// The record and the slot are inferred where no server has acted on
    /// them yet (recording checklist 6.457 local, 6.458 on a public server).
    Modelled(Buff),
}

/// A selection made from a current own-player buff slot.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// Zone admission that owns the selection.
    pub session_id: u64,
    /// Revision of received buff slots, independent of countdowns and icons.
    pub revision: u64,
    /// The slot, received or modelled, never a displayed ordinal for an
    /// effect without one.
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
    /// Where the slot comes from.
    pub placement: Placement,
}

/// What happened to a request. Receipt of a fade does not establish its cause.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Progress {
    /// Transport submission only; received membership is unchanged.
    Submitted,
    /// The first packet was submitted, but the second could not be sent.
    PartiallySubmitted,
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
        if let Placement::Modelled(buff) = &self.placement {
            ensure!(
                buff.spell_id == self.spell_id
                    && self.slot < super::placement::TITANIUM_PLAYER_SLOTS,
                "Invalid modelled buff slot"
            );
            ensure!(
                !slots.contains_key(&self.slot)
                    && !slots.values().any(|held| held.spell_id == self.spell_id),
                "The selected buff slot changed or is unavailable"
            );
            return Ok(());
        }
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

    /// The record the cancellation names: the received one in its slot, or
    /// the modelled one.
    #[must_use]
    pub fn record<'a>(&'a self, slots: Option<&'a BTreeMap<u32, Buff>>) -> Option<&'a Buff> {
        match &self.placement {
            Placement::Received => slots?.get(&self.slot),
            Placement::Modelled(buff) => Some(buff),
        }
    }
}

/// `EQEmu`'s initialized Titanium C2S `OP_Buff` after slot validation.
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

/// Builds P99's Titanium cancellation pair in send order, after slot validation.
///
/// The first record names the old spell in the cleared buff fields. Its
/// unwritten trailing word is initialized to zero here. The second names an
/// empty slot with its explicit clear flag.
///
/// With a modelled slot ([`Placement::Modelled`]) the pair names the slot
/// the program would hold; whether that server takes it is unchecked until
/// the attended click-off (recording checklist 6.458), so it is inferred.
///
/// # Errors
/// Refuses a missing own-player identity, a non-player slot or a sentinel spell.
pub fn project1999_packets(own_id: u16, slot: u32, spell_id: u32) -> Result<[EncodedCommand; 2]> {
    ensure!(
        own_id != 0 && slot < 38 && !matches!(spell_id, 0 | 0xffff | u32::MAX),
        "Invalid P99 buff cancellation"
    );
    let packet = |spell: u32, clear: u32| {
        let mut body = vec![0; 32];
        body[..4].copy_from_slice(&u32::from(own_id).to_le_bytes());
        body[8..12].copy_from_slice(&spell.to_le_bytes());
        body[24..28].copy_from_slice(&slot.to_le_bytes());
        body[28..].copy_from_slice(&clear.to_le_bytes());
        EncodedCommand {
            opcode: 0x6a53,
            body,
        }
    };
    Ok([packet(spell_id, 0), packet(u32::MAX, 1)])
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
            placement: Placement::Received,
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
    fn a_modelled_slot_is_cancellable_only_while_nothing_received_contradicts_it() {
        let mut request = request();
        request.placement = Placement::Modelled(buff(42));
        let mut slots = BTreeMap::from([(1, buff(43))]);
        assert!(request.check(7, Some(&slots)).is_ok());
        assert_eq!(request.record(Some(&slots)), Some(&buff(42)));
        // A received record in its slot, or of its spell elsewhere.
        slots.insert(3, buff(44));
        assert!(request.check(7, Some(&slots)).is_err());
        slots.remove(&3);
        slots.insert(5, buff(42));
        assert!(request.check(7, Some(&slots)).is_err());
        slots.remove(&5);
        // A record of another spell, a slot past the table, no snapshot.
        request.placement = Placement::Modelled(buff(43));
        assert!(request.check(7, Some(&slots)).is_err());
        request.placement = Placement::Modelled(buff(42));
        request.slot = 38;
        assert!(request.check(7, Some(&slots)).is_err());
        request.slot = 3;
        assert!(request.check(7, None).is_err());
        // The same refusals as a received slot's.
        request.beneficial = None;
        assert!(request.check(7, Some(&slots)).is_err());
    }

    #[test]
    fn p99_cancellation_sends_a_named_fade_then_the_sentinel_clear() {
        let [fade, clear] = project1999_packets(7, 3, 42).unwrap();
        assert_eq!((fade.opcode, clear.opcode), (0x6a53, 0x6a53));
        assert_eq!(
            fade.body,
            [
                7, 0, 0, 0, 0, 0, 0, 0, 42, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0,
                0, 0, 0, 0, 0,
            ]
        );
        assert_eq!(
            clear.body,
            [
                7, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3,
                0, 0, 0, 1, 0, 0, 0,
            ]
        );
        assert!(project1999_packets(0, 3, 42).is_err());
        assert!(project1999_packets(7, 38, 42).is_err());
        for spell in [0, 0xffff, u32::MAX] {
            assert!(project1999_packets(7, 3, spell).is_err());
        }
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
