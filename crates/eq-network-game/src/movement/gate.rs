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

/// How often the Titanium program's character tick marks an update due:
/// more than 1000 ms after the last tick, every tick (0x4231c2; R13 a).
pub const TITANIUM_UPDATE_TICK: Duration = Duration::from_millis(1000);

/// A heading change, in units of a 512 circle, that marks an update due in
/// the Titanium program (0x4a2ef2; R13 a). The change wraps: 511 to 1 is 2.
pub const TITANIUM_HEADING_STEP: f32 = 2.0;

/// A speed change that marks an update due in the Titanium program: 0.2 of
/// its speed (spawn +0x48; 0x4a2f3f; R13 a). That is 8 in the update's
/// forward field, which carries the speed × 40, and about 8.53 units a
/// second on the 42.667 a second per unit the speeds use.
pub const TITANIUM_SPEED_STEP: i16 = 8;

/// What marks a generation's own update due, and when a due one goes out.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[allow(clippy::struct_excessive_bools)] // Independent switches of each program's rule.
pub struct FlagRule {
    /// The least time, more than which goes between two updates.
    pub spacing: Duration,
    /// How often, at most, the program's own timer marks an update due.
    pub tick: Duration,
    /// A heading change, from the last heading that marked one due, that
    /// marks an update due.
    pub heading_step: f32,
    /// A change in the update's speed field, from the last speed that
    /// marked one due, that marks an update due.
    pub speed_step: i16,
    /// Every strafing sample marks an update due.
    pub strafing: bool,
    /// A turn rate marks an update due, as a held turn key does.
    pub held_turn: bool,
    /// How many times an unchanged update is repeated before it is held
    /// until something changes; None repeats it whenever it is due.
    pub repeats: Option<u8>,
    /// The program's resets clear the last update's time, so the first
    /// update after zoning in isn't held back.
    pub first_unheld: bool,
    /// While the server holds the player frozen ([`Hold::Frozen`]), only the
    /// timer marks an update due, and one goes out more than this after the
    /// last, carrying the last counter written and recording no history
    /// entry. None keeps the ordinary rule.
    pub frozen_spacing: Option<Duration>,
    /// While stunned ([`Hold::Stunned`]) and not frozen, nothing goes out.
    pub stun_silent: bool,
}

/// The server's hold on the player, as the update gate sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Hold {
    /// No hold.
    #[default]
    None,
    /// The server holds the player frozen (Titanium posture 102).
    Frozen,
    /// The player is stunned, and moves only by a departure that lets them.
    Stunned,
}

/// The TAKP program's rule (TAKP gate read Q1, path A): the 1 s timer, a
/// speed change of 0.2 or more, a heading change of 2.0 or more, a held turn
/// or a jump, then more than 150 ms after the last; an update whose packed
/// bytes match the last sent is repeated 6 times and then held.
pub const EQMAC_FLAGS: FlagRule = FlagRule {
    spacing: EQMAC_UPDATE_SPACING,
    tick: EQMAC_UPDATE_TICK,
    heading_step: EQMAC_HEADING_STEP,
    speed_step: EQMAC_SPEED_STEP,
    strafing: false,
    held_turn: true,
    repeats: Some(EQMAC_REPEATS),
    first_unheld: true,
    // Path C (stand-state 0x66) is unread past the server's freeze.
    frozen_spacing: None,
    stun_silent: false,
};

/// The Titanium program's rule (read R13 a): the 1 s character tick, a
/// heading change of 2 or more, a speed change of 0.2 or more, every
/// strafing frame or a jump, then more than 280 ms after the last. Running
/// straight with nothing else changing goes out once a second. The spell and
/// contact pushes that also mark one due are the server's moves here, and a
/// recording's few short straight gaps they don't explain are open
/// (recording checklist 6.421).
///
/// The program clears its "can act" flag while the character is stunned
/// (character byte +0x104 above 2) or frozen (posture 0x66). Then only a
/// frozen player's update goes out (0x4ac842): from the flag, so in practice
/// the 1 s tick, more than 560 ms after the last, with no history record and
/// the counter field not written, so it repeats the last one. A further
/// condition there, [0x92478c] being 0, is unread and taken to hold; that
/// +0x104 is the stun is inferred too (recording checklist 6.420).
pub const TITANIUM_FLAGS: FlagRule = FlagRule {
    spacing: super::TITANIUM_POSITION_SPACING,
    tick: TITANIUM_UPDATE_TICK,
    heading_step: TITANIUM_HEADING_STEP,
    speed_step: TITANIUM_SPEED_STEP,
    strafing: true,
    held_turn: false,
    repeats: None,
    first_unheld: false,
    frozen_spacing: Some(TITANIUM_FROZEN_SPACING),
    stun_silent: true,
};

/// The least time, more than which goes between two of a frozen Titanium
/// player's updates (0x230 ms at 0x4ac842; read R13 e).
pub const TITANIUM_FROZEN_SPACING: Duration = Duration::from_millis(560);

/// How the generation gates the player's own update.
#[derive(Clone, Copy, Debug)]
pub enum UpdateGate {
    /// An update goes out only after a change, and at least `spacing` after
    /// the last; a sample accepted sooner is held, and the latest goes out
    /// once the spacing passes, with a stationary heartbeat. Titanium used
    /// this before its flag rule was read ([`TITANIUM_FLAGS`]); a host may
    /// still choose it.
    Changed {
        /// The least time between two updates.
        spacing: Duration,
    },
    /// An update is due when the generation's [`FlagRule`] marks it, and
    /// goes out on the next pass more than its spacing after the last
    /// ([`EQMAC_FLAGS`], [`TITANIUM_FLAGS`]). For `EQMac`, paths B (a spawn
    /// the player controls) and C (stand-state 0x66) don't apply: this
    /// client controls no other spawn, and the setters of 0x66 other than
    /// the server's freeze are unread.
    Flagged {
        /// The generation's own packing of the update, whose bytes after the
        /// spawn ID the repeat test compares.
        pack: fn(&PositionPacket) -> Result<EncodedCommand>,
        /// What marks an update due.
        rule: FlagRule,
    },
}

/// A flagged gate's state: the pending flag and what it compares against.
#[derive(Clone, Debug, Default)]
pub(super) struct Flags {
    /// An update is due (`EQMac` 0x799775, Titanium [0x923d79]).
    pending: bool,
    /// The heading and speed when the input last marked an update due
    /// (`EQMac` 0x7f95cc and 0x7f95c8): thresholds compare against these,
    /// not against what was last sent, so a slow drift adds up.
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
    /// sample at `now`; `strafing` says the sample strafes.
    pub(super) fn observe(
        &mut self,
        sample: &PositionPacket,
        now: Instant,
        rule: &FlagRule,
        strafing: bool,
    ) {
        self.observe_tick(now, rule);
        let heading = sample.position.heading;
        let turned = (heading - self.heading + 256.0).rem_euclid(512.0) - 256.0;
        if turned.abs() >= rule.heading_step {
            self.heading = heading;
            self.pending = true;
        }
        if (sample.animation - self.speed).abs() >= rule.speed_step {
            self.speed = sample.animation;
            self.pending = true;
        }
        // A turn rate stands for the `EQMac` program's held turn key
        // (inferred, checklist 6.310): the host sends one only while the
        // player turns.
        if rule.held_turn && sample.delta_heading != 0 {
            self.pending = true;
        }
        if rule.strafing && strafing {
            self.pending = true;
        }
    }

    /// Marks an update due as the program's timer would at `now`.
    pub(super) fn observe_tick(&mut self, now: Instant, rule: &FlagRule) {
        if self
            .tick
            .is_none_or(|tick| now.saturating_duration_since(tick) > rule.tick)
        {
            self.tick = Some(now);
            self.pending = true;
        }
    }

    /// Takes a due update for the frozen rule: due, and more than `spacing`
    /// `since` the last update.
    pub(super) fn release_frozen(&mut self, since: Duration, spacing: Duration) -> bool {
        let released = self.pending && since > spacing;
        if released {
            self.pending = false;
        }
        released
    }

    /// Whether `sample` goes out now, `since` the last update: it must be
    /// due, more than the rule's spacing after the last, and, where the
    /// rule limits repeats, either changed or still within them. A sample
    /// held back stays due.
    ///
    /// # Errors
    /// Propagates the packing's refusal.
    pub(super) fn release(
        &mut self,
        sample: &PositionPacket,
        since: Duration,
        pack: fn(&PositionPacket) -> Result<EncodedCommand>,
        rule: &FlagRule,
    ) -> Result<bool> {
        if !self.pending || since <= rule.spacing {
            return Ok(false);
        }
        if let Some(limit) = rule.repeats {
            let packed = pack(sample)?.body.get(2..).unwrap_or_default().to_vec();
            if self.packed.as_ref() == Some(&packed) {
                self.repeats = self.repeats.saturating_add(1).min(EQMAC_REPEAT_CAP);
                if self.repeats > limit {
                    return Ok(false);
                }
            } else {
                self.packed = Some(packed);
                self.repeats = 0;
            }
        }
        self.pending = false;
        Ok(true)
    }
}
