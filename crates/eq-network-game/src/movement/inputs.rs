//! Host-owned movement inputs, separate from permission to move.

/// Complete inputs calculated from the current authoritative character state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SpeedInputs {
    /// Effective movement modifier in percent; -1000 denotes an actual root.
    pub rate: i16,
    /// Carried weight in whole units (including coins).
    pub weight: u16,
    /// Effective strength. Zero retains the formula's no-encumbrance meaning;
    /// it must never stand for unavailable strength.
    pub capacity: u32,
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
