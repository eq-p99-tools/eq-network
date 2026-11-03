//! Host-owned movement inputs, separate from permission to move.

/// Complete inputs calculated from the current authoritative character state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeedInputs {
    /// Effective movement modifier in percent; -1000 denotes an actual root.
    pub rate: i16,
    /// Carried weight in whole units (including coins).
    pub weight: u16,
    /// Effective strength. Zero retains the formula's no-encumbrance meaning;
    /// it must never stand for unavailable strength.
    pub capacity: u32,
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
