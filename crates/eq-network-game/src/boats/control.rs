//! Correlation only: callers must separately establish vessel eligibility,
//! current support and successful transmission. This does not authorize sends.
use super::{Action, ControlBoat};
use serde::Serialize;
use std::{collections::BTreeSet, time::Instant};

/// A vessel incarnation, supplied from the admission's received spawn state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Vessel {
    /// Zone-local spawn ID.
    pub spawn_id: u16,
    /// Identity of the received spawn incarnation, not a wire request nonce.
    pub incarnation: u64,
}

/// An explicit control intent. Unknown received flags cannot become requests.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Intent {
    /// Ask to take control.
    Acquire,
    /// Stop local control and ask to release it.
    Release,
}

/// Local knowledge, never inferred from silence or a position update.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub enum ControlState {
    /// Nothing establishes control in this admission.
    #[default]
    Unknown,
    /// An acquire was transmitted; no control is established yet.
    Acquiring {
        /// Exact incarnation the caller selected.
        vessel: Vessel,
    },
    /// An unambiguous acquire echo matched the outstanding request.
    Acquired(Vessel),
    /// Local control stopped on release intent; the outcome is unconfirmed.
    Releasing(Vessel),
    /// A release echo arrived. It does not establish prior ownership.
    Released(Vessel),
    /// A timeout, cancellation, support loss or retirement withdrew local
    /// authority; the server's final control state remains unknown.
    Unconfirmed(Vessel),
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    vessel: Vessel,
    intent: Intent,
    expires: Instant,
}

/// Admission-scoped correlation foundation for a future `EQEmu` control owner.
/// It is consumed by the `EQEmu` session feature; it grants no boat motion.
#[derive(Debug)]
pub struct Control {
    session_id: u64,
    state: ControlState,
    pending: Option<Pending>,
    /// Unresolved wire IDs remain fenced even if a spawn ID is reused.
    attempted: BTreeSet<u16>,
    /// Releases awaiting a reliable echo, including expired local waits.
    release_fences: BTreeSet<u16>,
}

impl Control {
    /// Starts an empty admission. A new zone connection must use a new owner.
    #[must_use]
    pub const fn new(session_id: u64) -> Self {
        Self {
            session_id,
            state: ControlState::Unknown,
            pending: None,
            attempted: BTreeSet::new(),
            release_fences: BTreeSet::new(),
        }
    }

    /// Current local knowledge; only `Acquired` is a correlated acquire echo.
    #[must_use]
    pub const fn state(&self) -> ControlState {
        self.state
    }

    /// Whether a same-ID acquire has no older unresolved echo. A completed
    /// reliable, ordered release clears this fence; spawn replacement alone
    /// does not. A sender must check this before transmitting an acquisition.
    #[must_use]
    pub fn may_acquire(&self, vessel: Vessel) -> bool {
        !self.attempted.contains(&vessel.spawn_id)
    }

    /// Records an already-transmitted, independently validated intent.
    /// `expires` is a caller's local deadline, not an `EQEmu` acknowledgement
    /// timer. Replacing an intent retires its response before tracking another.
    ///
    /// Returns false for another admission, zero IDs or an expired deadline.
    pub fn submitted(
        &mut self,
        session_id: u64,
        vessel: Vessel,
        intent: Intent,
        now: Instant,
        expires: Instant,
    ) -> bool {
        if session_id != self.session_id || vessel.spawn_id == 0 || expires <= now {
            return false;
        }
        if intent == Intent::Acquire && !self.may_acquire(vessel) {
            return false;
        }
        if intent == Intent::Acquire {
            self.attempted.insert(vessel.spawn_id);
        } else {
            self.release_fences.insert(vessel.spawn_id);
        }
        self.pending = Some(Pending {
            vessel,
            intent,
            expires,
        });
        self.state = match intent {
            Intent::Acquire => ControlState::Acquiring { vessel },
            Intent::Release => ControlState::Releasing(vessel),
        };
        true
    }

    /// Applies an echo only while its exact admission, ID, intent and local
    /// deadline remain outstanding. The unread trailer is never a nonce.
    /// An observed release echo fences earlier acquires on the same reliable,
    /// ordered `EQEmu` connection. Unfenced same-ID retries are not submitted.
    pub fn received(&mut self, session_id: u64, packet: ControlBoat, now: Instant) {
        if session_id != self.session_id {
            return;
        }
        self.expire(now);
        if packet.unknown != [0; 3] {
            return;
        }
        // A late release still fences older acquires. It grants no control
        // and cannot replace another vessel's currently outstanding intent.
        if packet.action == Action::Release {
            if let Ok(id) = u16::try_from(packet.boat_id) {
                if self.release_fences.remove(&id) {
                    self.attempted.remove(&id);
                    if let Some(vessel) = self.vessel().filter(|vessel| vessel.spawn_id == id) {
                        if !matches!(
                            self.state,
                            ControlState::Acquiring { .. } | ControlState::Acquired(_)
                        ) {
                            self.state = ControlState::Released(vessel);
                            self.pending = None;
                        }
                    }
                }
            }
            return;
        }
        let Some(pending) = self.pending else { return };
        if packet.boat_id != u32::from(pending.vessel.spawn_id) || packet.unknown != [0; 3] {
            return;
        }
        match (pending.intent, packet.action) {
            (Intent::Acquire, Action::Acquire) => {
                self.state = ControlState::Acquired(pending.vessel);
            }
            _ => return,
        }
        self.pending = None;
    }

    /// Withdraws an unanswered request at its local deadline without
    /// asserting that the server refused it.
    pub fn expire(&mut self, now: Instant) {
        if self.pending.is_some_and(|pending| now >= pending.expires) {
            self.cancel();
        }
    }

    /// Stops local control and retires outstanding echoes. This is not a
    /// server release; the session feature separately issues that intent.
    pub fn cancel(&mut self) {
        if let Some(vessel) = self.vessel() {
            self.state = ControlState::Unconfirmed(vessel);
        }
        self.pending = None;
    }

    /// Losing the selected floor link stops local control immediately. The
    /// returned vessel needs a release request while its connection is usable.
    #[must_use]
    pub fn support_changed(&mut self, support: Option<Vessel>) -> Option<Vessel> {
        let vessel = self.vessel()?;
        if support == Some(vessel) || !self.needs_release() {
            return None;
        }
        self.cancel();
        Some(vessel)
    }

    /// Despawn or replacement retires the exact incarnation. Numeric ID reuse
    /// cannot inherit a pending request or an earlier acquired state.
    pub fn retired(&mut self, vessel: Vessel) {
        if self.vessel() == Some(vessel) {
            self.cancel();
        }
    }

    /// Whether local control or an outstanding acquisition needs releasing.
    #[must_use]
    pub const fn needs_release(&self) -> bool {
        matches!(
            self.state,
            ControlState::Acquiring { .. } | ControlState::Acquired(_)
        )
    }

    /// The selected incarnation, including an unconfirmed outcome.
    #[must_use]
    pub const fn vessel(&self) -> Option<Vessel> {
        match self.state {
            ControlState::Unknown => None,
            ControlState::Acquiring { vessel, .. }
            | ControlState::Acquired(vessel)
            | ControlState::Releasing(vessel)
            | ControlState::Released(vessel)
            | ControlState::Unconfirmed(vessel) => Some(vessel),
        }
    }
}

#[cfg(test)]
mod tests;
