//! The server's regeneration tick: the timer each character's regeneration
//! and buffs run on. `EQEmu` starts it when the zone takes the character in,
//! so it restarts with every zone, and restarts it from the pass of the
//! zone's loop that it fires on, so each tick lands one period and up to one
//! pass after the last. No packet names a tick; a session learns it from
//! the packets the server sends at it, and tells a front end of each one as
//! [`crate::world::WorldEvent::ServerTick`].
//!
//! Reference: `EQEmu`'s `tic_timer` (`zone/mob.cpp`), the tick's work in
//! `Client::Process` (`zone/client_process.cpp`), `Timer::Check`
//! (`common/timer.cpp`) and the zone loop's pass (`zone/main.cpp`).
use serde::Serialize;
use std::time::Duration;

/// How often a server's tick comes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Cadence {
    /// The timer's period, in milliseconds.
    pub period_ms: u32,
    /// The most a tick lands after one period from the last, in
    /// milliseconds: one pass of the server's loop.
    pub slip_ms: u32,
}

impl Cadence {
    /// The timer's period.
    #[must_use]
    pub fn period(self) -> Duration {
        Duration::from_millis(u64::from(self.period_ms))
    }

    /// The most a tick lands after one period from the last.
    #[must_use]
    pub fn slip(self) -> Duration {
        Duration::from_millis(u64::from(self.slip_ms))
    }

    /// How long a tick takes on average: the period and half a pass.
    #[must_use]
    pub fn mean(self) -> Duration {
        self.period() + self.slip() / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tick_takes_the_period_and_half_a_pass_on_average() {
        let cadence = Cadence {
            period_ms: 6000,
            slip_ms: 32,
        };
        assert_eq!(cadence.period(), Duration::from_secs(6));
        assert_eq!(cadence.slip(), Duration::from_millis(32));
        assert_eq!(cadence.mean(), Duration::from_millis(6016));
    }
}
