//! When the client generation lets the player's own position update out.
//!
//! Each generation's program gates its own update in its own way; the
//! movement session keeps the host's samples at their own pace and lets
//! one out only when the generation's gate would.
use super::PositionPacket;
use crate::command::EncodedCommand;
use anyhow::Result;
use std::time::{Duration, Instant};

/// The least time, more than which the TAKP program leaves between two of
/// the player's own updates (path A: 150 ms at 0x53ea6a; TAKP gate read Q1).
pub const EQMAC_UPDATE_SPACING: Duration = Duration::from_millis(150);

/// How often, at most, the TAKP program's own timer marks an update as due:
/// more than 1000 ms after the last time it did (0x4c131d; Q1 6a).
pub const EQMAC_UPDATE_TICK: Duration = Duration::from_millis(1000);

/// How many times the TAKP program repeats an unchanged update before it
/// stops: it sends while its repeat counter is 1 to 6 (Q1 4f).
pub const EQMAC_REPEATS: u8 = 6;

/// The repeat counter's cap in the TAKP program (Q1 4f).
const EQMAC_REPEAT_CAP: u8 = 200;

/// A heading change, in heading units of a 512 circle, that marks an update
/// as due at once in the TAKP program (2.0 at 0x540376; Q1 6b).
pub const EQMAC_HEADING_STEP: f32 = 2.0;

/// A speed change that marks an update as due at once in the TAKP program:
/// 0.2 (Q1 6b), which is 2 in the update's speed field (speed × 10).
pub const EQMAC_SPEED_STEP: i16 = 2;

/// How the generation gates the player's own update.
#[derive(Clone, Copy, Debug)]
pub enum UpdateGate {
    /// Titanium (program read p1b, T1): an update goes out only after a
    /// change, and more than `spacing` (280 ms) after the last; a sample
    /// accepted sooner is held, and the latest goes out once the spacing
    /// passes. The standing repeat stays at the heartbeat recorded on P99,
    /// since whether a timer sets the change flag is unsure.
    Changed {
        /// The least time between two updates.
        spacing: Duration,
    },
    /// `EQMac` (TAKP gate read Q1, path A): an update is due on the 1 s
    /// timer, a speed change of 0.2 or more, a heading change of 2.0 or
    /// more, a held turn or a jump, and goes out on the next pass more than
    /// 150 ms after the last. One whose packed bytes match the last sent is
    /// repeated 6 times and then held until something changes. Paths B (a
    /// spawn the player controls) and C (stand-state 0x66) don't apply: this
    /// client controls no other spawn, and the setters of 0x66 other than
    /// the server's freeze are unread.
    Flagged {
        /// The generation's own packing of the update, whose bytes after the
        /// spawn ID the change test compares.
        pack: fn(&PositionPacket) -> Result<EncodedCommand>,
    },
}

/// The `EQMac` gate's state: the pending flag and what it compares against.
#[derive(Clone, Debug, Default)]
pub(super) struct Flags {
    /// An update is due (0x799775).
    pending: bool,
    /// The heading and speed when the input last marked an update due
    /// (0x7f95cc and 0x7f95c8): thresholds compare against these, not
    /// against what was last sent.
    heading: f32,
    speed: i16,
    /// When the timer last marked an update due.
    tick: Option<Instant>,
    /// The packed bytes last sent, after the spawn ID.
    packed: Option<Vec<u8>>,
    /// How many unchanged updates in a row the change test has seen.
    repeats: u8,
}

impl Flags {
    /// Marks an update due, as the program's jump does.
    pub(super) fn raise(&mut self) {
        self.pending = true;
    }

    /// Marks an update due as the program's input and timer would for this
    /// sample at `now`.
    pub(super) fn observe(&mut self, sample: &PositionPacket, now: Instant) {
        if self
            .tick
            .is_none_or(|tick| now.saturating_duration_since(tick) > EQMAC_UPDATE_TICK)
        {
            self.tick = Some(now);
            self.pending = true;
        }
        let heading = sample.position.heading;
        let turned = (heading - self.heading + 256.0).rem_euclid(512.0) - 256.0;
        if turned.abs() >= EQMAC_HEADING_STEP {
            self.heading = heading;
            self.pending = true;
        }
        if (sample.animation - self.speed).abs() >= EQMAC_SPEED_STEP {
            self.speed = sample.animation;
            self.pending = true;
        }
        // A turn rate stands for the program's held turn key (inferred,
        // checklist 6.310): the host sends one only while the player turns.
        if sample.delta_heading != 0 {
            self.pending = true;
        }
    }

    /// Whether `sample` goes out now, `since` the last update: it must be
    /// due, more than [`EQMAC_UPDATE_SPACING`] after the last, and either
    /// changed or still within its repeats. A sample held back stays due.
    ///
    /// # Errors
    /// Propagates the packing's refusal.
    pub(super) fn release(
        &mut self,
        sample: &PositionPacket,
        since: Duration,
        pack: fn(&PositionPacket) -> Result<EncodedCommand>,
    ) -> Result<bool> {
        if !self.pending || since <= EQMAC_UPDATE_SPACING {
            return Ok(false);
        }
        let packed = pack(sample)?.body.get(2..).unwrap_or_default().to_vec();
        if self.packed.as_ref() == Some(&packed) {
            self.repeats = self.repeats.saturating_add(1).min(EQMAC_REPEAT_CAP);
            if self.repeats > EQMAC_REPEATS {
                return Ok(false);
            }
        } else {
            self.packed = Some(packed);
            self.repeats = 0;
        }
        self.pending = false;
        Ok(true)
    }
}
