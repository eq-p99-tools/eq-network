//! One memorization state for request readiness and the host's gauge.
use std::time::{Duration, Instant};

const POINTS: u8 = 92;
const FALLBACK: Duration = Duration::from_secs(5);

/// Installed class level and optional verified effective Bard inputs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemorizationInputs {
    /// Installed spell level for this class; None is unavailable.
    pub spell_level: Option<u8>,
    /// Known capped native stat getter results and AA state, not base attributes.
    pub bard: Option<BardMemorization>,
}

/// Effective native Bard inputs. Missing precision retains approximate timing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BardMemorization {
    /// Effective capped intelligence.
    pub intelligence: u32,
    /// Effective capped dexterity.
    pub dexterity: u32,
    /// Whether received AA selections contain286; None is unknown.
    pub aa_286: Option<bool>,
}

/// Actual worker gauge, separate from a server-confirmed memorize result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemorizationProgress {
    /// Destination gem, zero based.
    pub gem: u8,
    /// Spell being memorized.
    pub spell_id: u32,
    /// Remaining points, zero through92.
    pub remaining: u8,
    /// Existing five-second fallback because a native rule/input is unavailable.
    pub approximate: bool,
}

impl MemorizationProgress {
    /// Completed fraction, bounded even for an invalid external snapshot.
    #[must_use]
    pub fn fraction(self) -> f32 {
        1.0 - f32::from(self.remaining.min(POINTS)) / f32::from(POINTS)
    }
}

#[derive(Clone, Copy, Debug)]
struct Native {
    step: u8,
    interval: Duration,
    last: Option<Instant>,
}

/// Poll-driven memorization. Missing precision preserves the existing approximate
/// five-second behavior instead of making a previously usable spell unavailable.
#[derive(Clone, Debug)]
pub struct MemorizationCountdown {
    started: Instant,
    remaining: u8,
    native: Option<Native>,
    inputs: MemorizationInputs,
    titanium: bool,
    last_poll: Option<Instant>,
    approximate: bool,
}

impl MemorizationCountdown {
    /// Selects the read Titanium rule when all its inputs are known.
    #[must_use]
    pub fn new(
        titanium: bool,
        inputs: MemorizationInputs,
        class: Option<u32>,
        level: u8,
        meditate: Option<u32>,
        started: Instant,
    ) -> Self {
        let rule = || {
            if !titanium || level == 0 {
                return None;
            }
            let spell_level = inputs
                .spell_level
                .filter(|level| !matches!(level, 0 | 255))?;
            let class = class.filter(|class| (1..=16).contains(class))?;
            let m = || meditate.map(|skill| u64::from(skill.max(10)) / 10);
            let step = if class == 8 {
                let bard = inputs.bard?;
                let base = (u64::from(bard.intelligence) + u64::from(bard.dexterity)) / 30 + 1;
                if bard.aa_286? {
                    base * 2
                } else {
                    base + 2 + m()?
                }
            } else {
                3 + m()?
            };
            let interval = (i64::from(spell_level) - i64::from(level) + 5).max(1) * 100;
            Some(Native {
                // Larger steps have the same clamped result on a92-point gauge.
                step: u8::try_from(step.min(u64::from(POINTS))).unwrap_or(POINTS),
                interval: Duration::from_millis(u64::try_from(interval).ok()?),
                last: None,
            })
        };
        let native = rule();
        Self {
            started,
            remaining: POINTS,
            native,
            inputs,
            titanium,
            last_poll: None,
            approximate: native.is_none(),
        }
    }

    /// Native polls read current level and Meditate. Refresh those received
    /// facts without restarting the gauge or catching up elapsed steps.
    pub fn update_character(&mut self, class: Option<u32>, level: u8, meditate: Option<u32>) {
        let Some(previous) = self.native else { return };
        if let Some(mut native) = Self::new(
            self.titanium,
            self.inputs,
            class,
            level,
            meditate,
            self.started,
        )
        .native
        {
            native.last = previous.last;
            self.native = Some(native);
        } else {
            // Retain the last known rule and disclose lost precision.
            self.approximate = true;
        }
    }

    /// Advances once only after the native strict threshold. First eligible poll
    /// steps immediately; stale clocks and long stalls never produce a catch-up burst.
    pub fn poll(&mut self, now: Instant) -> bool {
        if self.remaining == 0
            || now < self.started
            || self.last_poll.is_some_and(|last| now < last)
        {
            return false;
        }
        self.last_poll = Some(now);
        let before = self.remaining;
        if let Some(native) = &mut self.native {
            if native
                .last
                .is_some_and(|last| now.saturating_duration_since(last) <= native.interval)
            {
                return false;
            }
            native.last = Some(now);
            self.remaining = self.remaining.saturating_sub(native.step);
        } else {
            let left = FALLBACK
                .saturating_sub(now.saturating_duration_since(self.started))
                .as_nanos();
            let points = (left * u128::from(POINTS)).div_ceil(FALLBACK.as_nanos());
            self.remaining = u8::try_from(points).unwrap_or(POINTS);
        }
        before != self.remaining
    }

    /// Zero permits submission, never claims that the server accepted it.
    #[must_use]
    pub const fn ready(&self) -> bool {
        self.remaining == 0
    }

    /// Gauge snapshot from this same state, with no second elapsed-time model.
    #[must_use]
    pub const fn progress(&self, gem: u8, spell_id: u32) -> MemorizationProgress {
        MemorizationProgress {
            gem,
            spell_id,
            remaining: self.remaining,
            approximate: self.approximate,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn clock(skill: u32, level: u8, spell_level: u8, now: Instant) -> MemorizationCountdown {
        MemorizationCountdown::new(
            true,
            MemorizationInputs {
                spell_level: Some(spell_level),
                bard: None,
            },
            Some(2),
            level,
            Some(skill),
            now,
        )
    }

    #[test]
    fn skill_level_strict_threshold_and_stall_have_no_catch_up() {
        let now = Instant::now();
        let mut low = clock(0, 10, 10, now);
        assert!(!low.poll(now - Duration::from_millis(1)));
        assert!(low.poll(now));
        assert_eq!(low.progress(0, 42).remaining, 88);
        assert!(!low.poll(now + Duration::from_millis(500)));
        assert!(low.poll(now + Duration::from_secs(30)));
        assert_eq!(low.progress(0, 42).remaining, 84);
        assert!(!low.ready());
        assert!(!low.poll(now + Duration::from_secs(2)));
        let mut high = clock(200, 10, 10, now);
        for ms in [0, 501, 1002, 1503] {
            high.poll(now + Duration::from_millis(ms));
        }
        assert!(high.ready());
        let mut lower = clock(0, 20, 10, now);
        lower.poll(now);
        assert!(lower.poll(now + Duration::from_millis(101)));
    }

    #[test]
    fn thirty_and_sixty_hertz_share_one_countdown_rule() {
        let start = Instant::now();
        for hz in [30_u64, 60] {
            for (skill, minimum) in [(0, 11_000), (200, 1500)] {
                let mut countdown = clock(skill, 10, 10, start);
                let end = (0..1000)
                    .find_map(|frame| {
                        let elapsed = Duration::from_nanos(frame * 1_000_000_000 / hz);
                        countdown.poll(start + elapsed);
                        countdown.ready().then_some(elapsed)
                    })
                    .unwrap();
                assert!(end >= Duration::from_millis(minimum));
                assert!(end < Duration::from_millis(minimum) + Duration::from_secs(1));
            }
        }
    }

    #[test]
    fn bard_integer_division_aa_and_meditate_branches_are_distinct() {
        let start = Instant::now();
        for (aa, skill, step) in [
            (false, 0, 13),
            (false, 200, 32),
            (true, 0, 20),
            (true, 200, 20),
        ] {
            let mut countdown = MemorizationCountdown::new(
                true,
                MemorizationInputs {
                    spell_level: Some(10),
                    bard: Some(BardMemorization {
                        intelligence: 149,
                        dexterity: 150,
                        aa_286: Some(aa),
                    }),
                },
                Some(8),
                10,
                Some(skill),
                start,
            );
            countdown.poll(start);
            assert_eq!(countdown.progress(0, 42).remaining, POINTS - step);
            assert!(!countdown.progress(0, 42).approximate);
        }
    }

    #[test]
    fn missing_bard_inputs_and_unread_dialect_keep_labeled_fallback() {
        let start = Instant::now();
        for titanium in [false, true] {
            let mut countdown = MemorizationCountdown::new(
                titanium,
                MemorizationInputs {
                    spell_level: Some(1),
                    bard: None,
                },
                Some(8),
                1,
                Some(0),
                start,
            );
            assert!(countdown.progress(0, 42).approximate);
            countdown.poll(start + Duration::from_secs(4));
            assert!(!countdown.ready());
            assert!(countdown.progress(0, 42).fraction() > 0.0);
            let progress = countdown.progress(0, 42);
            assert!(!countdown.poll(start + Duration::from_secs(2)));
            assert_eq!(countdown.progress(0, 42), progress);
            countdown.poll(start + Duration::from_secs(5));
            assert!(countdown.ready());
        }
    }

    #[test]
    fn bard_aa_branch_does_not_require_or_add_meditate() {
        let start = Instant::now();
        let mut countdown = MemorizationCountdown::new(
            true,
            MemorizationInputs {
                spell_level: Some(10),
                bard: Some(BardMemorization {
                    intelligence: 149,
                    dexterity: 150,
                    aa_286: Some(true),
                }),
            },
            Some(8),
            10,
            None,
            start,
        );
        countdown.poll(start);
        assert_eq!(countdown.progress(0, 42).remaining, 72);
        assert!(!countdown.progress(0, 42).approximate);
    }

    #[test]
    fn received_level_and_skill_changes_update_the_next_poll_without_restarting() {
        let now = Instant::now();
        let mut countdown = clock(200, 10, 10, now);
        countdown.poll(now);
        assert_eq!(countdown.progress(0, 42).remaining, 69);
        countdown.update_character(Some(2), 20, Some(0));
        countdown.poll(now + Duration::from_millis(101));
        assert_eq!(countdown.progress(0, 42).remaining, 65);
    }
}
