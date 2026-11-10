//! The number the client generation puts in the player's own position
//! update: Titanium's offset 2 of `OP_ClientUpdate` (0x14cb).
//!
//! The Titanium program copies its movement-history entry counter there
//! (0x4ac5c1, 0x4aba14), read before the update's own entry is recorded.
//! The counter goes up by one for each entry the history object records
//! (0x4b70db): one for the player's own spawn on every frame (cause 1), one
//! at every update sent (cause 2) and the warp's entries. It is reset to 0
//! as collection starts at zone entry, after the zone-change flush and when
//! the player's own spawn is created (0x4b6ef8). This crate sends no
//! history packet (the per-frame history sender is gated off on public
//! servers by our policy), and the counter counts regardless.
//!
//! The program's spell paths record entries too (causes 3, 7 and 8 at
//! 0x415445, 0x415d65, 0x415ec3 and 0x48ba54). Their triggers in our
//! session are unread, so they are left out (recording checklist item 6.420).
//!
//! `EQMac`'s update counter is unread, so it keeps one more per update.
use std::time::Instant;

/// How a client generation numbers the player's own updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpdateCounter {
    /// One more for each update sent. `EQMac`'s field is unread, so it keeps this.
    PerUpdate,
    /// Titanium's movement-history entry counter.
    HistoryEntries {
        /// How many frames a second the player's own spawn records an
        /// entry on. The program records one each frame it draws, so its
        /// rate is the machine's frame rate; the number here is the rate a
        /// recording of the official client shows (inferred, recording
        /// checklist item 6.420). When a history sender comes, a frame
        /// count the client supplies replaces this rate.
        frames_per_second: u32,
    },
}

/// The frame rate a recording of the official Titanium client shows: its
/// entry counter rose by about 65 a second as the player stood still
/// (inferred, recording checklist item 6.420).
pub const TITANIUM_HISTORY_FRAMES_PER_SECOND: u32 = 65;

/// Titanium's update counter.
pub const TITANIUM_UPDATE_COUNTER: UpdateCounter = UpdateCounter::HistoryEntries {
    frames_per_second: TITANIUM_HISTORY_FRAMES_PER_SECOND,
};

/// The counter's state for one zone admission. The admission is where the
/// program resets it: collection starts at zone entry, and a zone change
/// or a new own spawn makes a new admission.
#[derive(Clone, Copy, Debug)]
pub(super) struct Counter {
    rule: UpdateCounter,
    /// When the frames started counting.
    since: Instant,
    /// The entries other than the frames' own.
    entries: u16,
}

impl Counter {
    pub(super) const fn new(since: Instant) -> Self {
        Self {
            rule: UpdateCounter::PerUpdate,
            since,
            entries: 0,
        }
    }

    pub(super) const fn with_rule(self, rule: UpdateCounter) -> Self {
        Self { rule, ..self }
    }

    /// The number an update sent at `now` carries.
    pub(super) fn value(&self, now: Instant) -> u16 {
        match self.rule {
            UpdateCounter::PerUpdate => self.entries,
            UpdateCounter::HistoryEntries { frames_per_second } => {
                let elapsed = now.saturating_duration_since(self.since).as_nanos();
                let frames = elapsed * u128::from(frames_per_second) / 1_000_000_000;
                // The program's counter is 16 bits and wraps.
                let frames = u16::try_from(frames & 0xffff).unwrap_or_default();
                frames.wrapping_add(self.entries)
            }
        }
    }

    /// An update went out: the history records its entry (cause 2) after
    /// the update copied the counter.
    pub(super) fn sent(&mut self) {
        self.entries = self.entries.wrapping_add(1);
    }

    /// The program's warp (0x48aaa6) records an entry at the old position
    /// (cause 1) before its update copies the counter.
    pub(super) fn before_warp(&mut self) {
        self.history_entry();
    }

    /// Entering a zone-line region records an entry (cause 4, 0x48b60e),
    /// and finding that its zone is the current one another (cause 5,
    /// 0x48b647), before the region's warp.
    pub(super) fn zone_line_region(&mut self) {
        self.history_entry();
        self.history_entry();
    }

    /// After the warp's update, an in-zone warp records the new position
    /// (cause 3, 0x48ac4b).
    pub(super) fn after_warp(&mut self) {
        self.history_entry();
    }

    /// The entries other than the frames': under [`UpdateCounter::PerUpdate`]
    /// the counter itself.
    #[cfg(test)]
    pub(super) const fn entries(&self) -> u16 {
        self.entries
    }

    #[cfg(test)]
    pub(super) const fn set_entries(&mut self, entries: u16) {
        self.entries = entries;
    }

    fn history_entry(&mut self) {
        if matches!(self.rule, UpdateCounter::HistoryEntries { .. }) {
            self.entries = self.entries.wrapping_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn per_update_counts_only_the_updates_sent() {
        // `EQMac`'s rule, as today.
        let start = Instant::now();
        let mut counter = Counter::new(start);
        assert_eq!(counter.value(start + Duration::from_secs(5)), 0);
        counter.before_warp();
        counter.sent();
        counter.after_warp();
        counter.sent();
        assert_eq!(counter.value(start + Duration::from_secs(9)), 2);
    }

    #[test]
    fn history_entries_count_frames_updates_and_the_warp() {
        let start = Instant::now();
        let mut counter = Counter::new(start).with_rule(TITANIUM_UPDATE_COUNTER);
        assert_eq!(counter.value(start), 0);
        // About 65 frames a second, each one entry.
        assert_eq!(counter.value(start + Duration::from_secs(1)), 65);
        assert_eq!(counter.value(start + Duration::from_millis(500)), 32);
        // Each update's own entry is recorded after it copied the counter.
        counter.sent();
        assert_eq!(counter.value(start + Duration::from_secs(1)), 66);
        // A warp: cause 1 before its update, cause 2 after, cause 3 at the
        // new place.
        counter.before_warp();
        let warp = counter.value(start + Duration::from_secs(1));
        counter.sent();
        counter.after_warp();
        assert_eq!(warp, 67);
        assert_eq!(counter.value(start + Duration::from_secs(1)), 69);
    }

    #[test]
    fn history_entries_wrap_at_sixteen_bits() {
        let start = Instant::now();
        let counter = Counter::new(start).with_rule(TITANIUM_UPDATE_COUNTER);
        // 65,536 entries take 1,008.25 seconds at 65 a second.
        let wrapped = start + Duration::from_millis(1_008_250);
        assert_eq!(counter.value(wrapped), 0);
        assert_eq!(counter.value(wrapped + Duration::from_secs(1)), 65);
    }
}
