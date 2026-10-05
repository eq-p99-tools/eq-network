//! Explicit movement gates during death and pending zone transfers.
use anyhow::{ensure, Result};
use eq_network_game::zoning::ZoneOffer;
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
enum Phase {
    #[default]
    Active,
    Dead,
    Transfer {
        offer: ZoneOffer,
        started: Instant,
        dead: bool,
    },
    /// The zone approved the transfer and the player is leaving it, since
    /// this moment.
    Departing {
        since: Instant,
    },
}

/// A transfer denial restores the prior life state, never blindly enables input.
#[derive(Debug, Default)]
pub(super) struct ZoneLifecycle {
    phase: Phase,
}
impl ZoneLifecycle {
    pub(super) fn blocks_motion(&self) -> bool {
        !matches!(self.phase, Phase::Active)
    }
    pub(super) fn is_dead(&self) -> bool {
        matches!(self.phase, Phase::Dead | Phase::Transfer { dead: true, .. })
    }
    pub(super) fn pending(&self) -> Option<&ZoneOffer> {
        if let Phase::Transfer { offer, .. } = &self.phase {
            Some(offer)
        } else {
            None
        }
    }
    pub(super) fn expired(&self, now: Instant) -> bool {
        matches!(&self.phase, Phase::Transfer { started, .. }
            if now.saturating_duration_since(*started) >= Duration::from_secs(45))
    }
    /// Whether the player is leaving the zone that approved their transfer.
    pub(super) fn departing(&self) -> bool {
        matches!(self.phase, Phase::Departing { .. })
    }
    /// Whether the player has been leaving for this long.
    pub(super) fn departed(&self, now: Instant, wait: Duration) -> bool {
        matches!(self.phase, Phase::Departing { since }
            if now.saturating_duration_since(since) >= wait)
    }
    pub(super) fn mark_dead(&mut self) {
        match &mut self.phase {
            Phase::Transfer { dead, .. } => *dead = true,
            Phase::Departing { .. } => (),
            _ => self.phase = Phase::Dead,
        }
    }
    /// Returns true only for the first copy of an offer; conflicting offers fail.
    pub(super) fn offer(&mut self, offer: ZoneOffer, now: Instant) -> Result<bool> {
        if let Some(pending) = self.pending() {
            ensure!(*pending == offer, "conflicting zone transfer offer");
            return Ok(false);
        }
        ensure!(
            !matches!(self.phase, Phase::Departing { .. }),
            "zone already handed off"
        );
        self.phase = Phase::Transfer {
            offer,
            started: now,
            dead: self.is_dead(),
        };
        Ok(true)
    }
    /// Called only after the response has been validated against the pending
    /// offer; an approval starts the player's departure now.
    pub(super) fn finish(&mut self, approved: bool, now: Instant) -> Result<()> {
        ensure!(
            self.pending().is_some(),
            "zone approval without a pending request"
        );
        self.phase = if approved {
            Phase::Departing { since: now }
        } else if self.is_dead() {
            Phase::Dead
        } else {
            Phase::Active
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn offer() -> ZoneOffer {
        ZoneOffer {
            zone_id: 9,
            instance_id: 0,
            position: eq_network_game::world::Position::default(),
            reason: 10,
            to_bind: true,
            solicited: true,
        }
    }
    #[test]
    fn duplicate_offers_do_not_resend_or_extend_the_timeout() {
        let now = Instant::now();
        let mut state = ZoneLifecycle::default();
        assert!(state.offer(offer(), now).unwrap());
        assert!(!state.offer(offer(), now + Duration::from_secs(44)).unwrap());
        assert!(state.expired(now + Duration::from_secs(45)));
        let mut different = offer();
        different.zone_id = 10;
        assert!(state.offer(different, now).is_err());
        assert_eq!(state.pending(), Some(&offer()));
    }
    #[test]
    fn death_during_transfer_survives_denial_and_approval_never_resumes_old_input() {
        let now = Instant::now();
        let mut state = ZoneLifecycle::default();
        assert!(!state.blocks_motion());
        state.offer(offer(), now).unwrap();
        assert!(state.blocks_motion());
        state.mark_dead();
        state.finish(false, now).unwrap();
        assert!(state.is_dead() && state.blocks_motion());
        state.offer(offer(), now).unwrap();
        state.finish(true, now).unwrap();
        assert!(state.blocks_motion() && state.departing());
        assert!(state.offer(offer(), now).is_err());
        assert!(state.finish(true, now).is_err());
        assert!(!ZoneLifecycle::default().blocks_motion());
    }
    #[test]
    fn a_departure_lasts_from_the_approval() {
        let now = Instant::now();
        let mut state = ZoneLifecycle::default();
        let wait = Duration::from_secs(2);
        assert!(!state.departing() && !state.departed(now, wait));
        state.offer(offer(), now).unwrap();
        assert!(!state.departing());
        state.finish(true, now).unwrap();
        assert!(state.departing());
        assert!(!state.departed(now + Duration::from_millis(1999), wait));
        assert!(state.departed(now + wait, wait));
    }
    #[test]
    fn denied_transfer_restores_alive_state_but_unsolicited_approval_is_rejected() {
        let mut state = ZoneLifecycle::default();
        assert!(state.finish(true, Instant::now()).is_err());
        state.offer(offer(), Instant::now()).unwrap();
        state.finish(false, Instant::now()).unwrap();
        assert!(!state.blocks_motion());
        assert!(state.pending().is_none());
    }
}
