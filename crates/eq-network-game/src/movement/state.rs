//! A geometry-owning host's current ground motion, independent of interval travel.
use anyhow::{ensure, Result};

/// Current horizontal velocity and Forward input for an ordinary ground sample.
///
/// Velocity is in world X/Y order, in units per second. The host supplies its
/// actual simulation state after its own collision step; this value establishes
/// neither floor support nor collision authority. Position, admission, creation
/// time and heading remain in the accompanying [`super::MovementRequest`].
/// Only the new state-sampling path reads this type. Existing movement requests
/// retain their interval-derived velocity and animation.
///
/// This horizontal-only seam supplies zero vertical report velocity. Applying
/// that choice to a slope is inferred: this API establishes no native slope-zero
/// rule. A host needing owned vertical velocity keeps the existing request path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundState {
    velocity: [f32; 2],
    forward: bool,
}

impl GroundState {
    /// Observed simulation state, with the independently observed Forward input.
    /// A released input may still have the original program's decaying velocity;
    /// a moved endpoint may already have zero current velocity.
    ///
    /// # Errors
    /// Rejects non-finite velocity. The session checks the calibrated cap when
    /// it admits the accompanying position; constructing this value grants none.
    pub fn new(velocity: [f32; 2], forward: bool) -> Result<Self> {
        ensure!(
            velocity.iter().all(|v| v.is_finite()),
            "non-finite ground velocity"
        );
        Ok(Self { velocity, forward })
    }

    /// Horizontal velocity at the sample, in world units per second.
    #[must_use]
    pub const fn velocity(self) -> [f32; 2] {
        self.velocity
    }

    /// Whether the host actually observed Forward input for this state.
    #[must_use]
    pub const fn forward(self) -> bool {
        self.forward
    }

    pub(super) fn check(self, maximum: f32) -> Result<()> {
        ensure!(
            self.velocity[0].hypot(self.velocity[1]) <= maximum,
            "ground velocity exceeds its calibrated cap"
        );
        Ok(())
    }

    pub(super) fn wire(self, scale: f32) -> [f32; 3] {
        [self.velocity[0] * scale, self.velocity[1] * scale, 0.0]
    }
}

/// Whether the new state-sampling path locally admitted a state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GroundSampleResult {
    /// Its genuine creation stamp is inside the existing minimum interval.
    /// No position, accepted time or transport state changed. A changed state
    /// holds stale reports only until the accepted state's existing minimum
    /// interval passes. The next ordinary tick can report that accepted state,
    /// under the unchanged wire gate, even if the host publishes nothing more.
    /// The waiting observation is never accepted by a tick. The host keeps
    /// publishing current states, never restamping an old observation for time.
    WaitingForInterval,
    /// Locally admitted under the usual transaction. The native update gate
    /// may still hold the packet; this is not a wire or server acknowledgement.
    Accepted,
}
