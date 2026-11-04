//! Host-owned movement inputs, separate from permission to move.

/// Complete inputs calculated from the current authoritative character state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SpeedInputs {
    /// Effective movement modifier in percent; -1000 denotes an actual root.
    pub rate: i16,
    /// Carried weight in whole units (including coins).
    pub weight: u16,
    /// Effective strength. Zero retains the formula's no-encumbrance meaning;
    /// it must never stand for unavailable strength. On `EQMac` with a fatigue
    /// component this is a stable conservative fallback, not a second authority.
    pub capacity: u32,
    /// Proven `EQMac` prerequisites, used only by that generation to apply the
    /// session's authoritative fatigue. None retains the legacy capacity and
    /// host-report refresh path; it never supplies guessed strength/stamina.
    pub eqmac_fatigue: Option<EqMacFatigueInputs>,
}

/// Host prerequisites for `EQMac`'s session-owned fatigue calculation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct EqMacFatigueInputs {
    /// Effective signed strength before fatigue and the final stat cap.
    pub strength: i64,
    /// Effective stamina, capped to 1..=255. None or invalid zero keeps the
    /// established conservative maximum fatigue penalty.
    pub stamina: Option<u8>,
}

impl EqMacFatigueInputs {
    /// Apply fatigue before the final stat cap. Unknown fatigue or stamina
    /// retains the maximum ten-point penalty; negative strength never becomes
    /// the movement formula's zero/no-encumbrance sentinel.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Final 1..=255 bound.
    pub fn capacity(self, fatigue: Option<u8>) -> u32 {
        let penalty = fatigue
            .zip(self.stamina.filter(|value| *value != 0))
            .map_or(10, |(fatigue, stamina)| {
                i64::from(fatigue.saturating_sub(stamina)).min(10)
            });
        self.strength.saturating_sub(penalty).clamp(1, 255) as u32
    }
}

impl SpeedInputs {
    /// Effective `EQMac` capacity, leaving legacy capacity (including its zero
    /// sentinel) unchanged when no fatigue prerequisites were reported.
    #[must_use]
    pub fn eqmac_capacity(self, fatigue: Option<u8>) -> u32 {
        self.eqmac_fatigue
            .map_or(self.capacity, |inputs| inputs.capacity(fatigue))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eqmac_fatigue_preserves_signed_bounds_unknowns_and_legacy_capacity() {
        let base = EqMacFatigueInputs {
            strength: 100,
            stamina: Some(100),
        };
        for (strength, expected) in [
            (i64::MIN, 1),
            (-1, 1),
            (0, 1),
            (1, 1),
            (255, 245),
            (260, 250),
            (265, 255),
            (i64::MAX, 255),
        ] {
            assert_eq!(
                EqMacFatigueInputs { strength, ..base }.capacity(Some(110)),
                expected
            );
        }
        assert_eq!(base.capacity(Some(100)), 100);
        assert_eq!(base.capacity(Some(105)), 95);
        assert_eq!(base.capacity(None), 90);
        for stamina in [None, Some(0)] {
            assert_eq!(EqMacFatigueInputs { stamina, ..base }.capacity(Some(1)), 90);
        }
        let legacy = SpeedInputs {
            rate: 0,
            weight: 100,
            capacity: 0,
            eqmac_fatigue: None,
        };
        assert_eq!(legacy.eqmac_capacity(Some(255)), 0);
        assert_eq!(
            SpeedInputs {
                capacity: 99,
                ..legacy
            }
            .eqmac_capacity(Some(255)),
            99
        );
    }
}

/// The local session accepted one exact host report and evaluated its grant.
/// This is host/session bookkeeping, not a remote server acknowledgement.
/// Withdrawals, new input revisions and admissions invalidate this receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SpeedInputAcceptance {
    /// Zone admission that accepted the report.
    pub session_id: u64,
    /// Opaque session revision echoed from the report.
    pub revision: u64,
    /// Opaque host report identity, also distinguishing identical-value recovery.
    pub report_id: u64,
    /// Exact ready inputs accepted for this report.
    pub inputs: SpeedInputs,
    /// Whether these accepted inputs currently have a positive movement grant.
    /// False includes an actual root or unavailable calibration; it grants nothing.
    pub granted: bool,
}

/// Why the host cannot yet establish current movement inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeedInputUnavailable {
    /// Character attributes have not arrived or cannot be interpreted.
    Attributes,
    /// No complete inventory, or an invalidated/correcting inventory.
    Inventory,
    /// A prediction changes weight or effective equipment.
    Prediction,
    /// Carried coin state is unavailable.
    Coins,
    /// Buff membership, spell data or an affecting formula is unresolved.
    Effects,
}

/// An explicit grant prerequisite. Unknown state never means numeric zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeedInputState {
    /// Complete inputs for the revision requested by the session.
    Ready(SpeedInputs),
    /// Withdraw player input until a complete report is available.
    Unavailable(SpeedInputUnavailable),
}
