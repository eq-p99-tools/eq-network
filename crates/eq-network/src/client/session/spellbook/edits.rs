//! Admission-local serialization of book edits whose results arrive asynchronously.
use super::BookIntent;
use crate::client::ClientCommand;
use eq_network_game::spells::{BookActionStatus, SpellUpdate};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edit {
    Forget(u8, u32),
    Delete(u16),
    Swap(u16, u16),
    Slot { slot: u32, spell_id: u32, mode: u32 },
}

impl Edit {
    fn command(command: &ClientCommand) -> Option<Self> {
        Some(match *command {
            ClientCommand::ForgetSpell { gem, spell_id, .. } => Self::Forget(gem, spell_id),
            ClientCommand::DeleteSpell { slot, .. } => Self::Delete(slot),
            ClientCommand::SwapSpell { from, to, .. } => Self::Swap(from, to),
            ClientCommand::ScribeSpell { slot, spell_id, .. } => Self::Slot {
                slot: u32::from(slot),
                spell_id,
                mode: 0,
            },
            ClientCommand::MemorizeSpell { gem, spell_id, .. } => Self::Slot {
                slot: u32::from(gem),
                spell_id,
                mode: 1,
            },
            _ => return None,
        })
    }

    /// An empty sentinel can answer any forget of the same gem.
    fn shares_reply(self, other: Self) -> bool {
        match (self, other) {
            (Self::Forget(first, _), Self::Forget(second, _)) => first == second,
            _ => self == other,
        }
    }
}

/// At most one tracked edit owns a reply. An abandoned reply identity stays
/// ambiguous within this admission, even after a matching update arrives.
#[derive(Default)]
pub(super) struct BookEdits {
    active: Option<Outstanding>,
    retired: Vec<Edit>,
}

#[derive(Clone, Copy)]
struct Outstanding {
    edit: Edit,
    notify_at: Instant,
}

impl Outstanding {
    fn new(edit: Edit, now: Instant) -> Self {
        Self {
            edit,
            notify_at: now + Duration::from_secs(30),
        }
    }
}

impl BookEdits {
    /// Whether an edit awaits its result; the shared action table then holds the book.
    pub fn outstanding(&self) -> bool {
        self.active.is_some()
    }

    /// A fresh command needs a reply distinguishable from any timed-out edit.
    pub fn ambiguous(&self, command: &ClientCommand) -> bool {
        Edit::command(command).is_some_and(|edit| {
            self.retired
                .iter()
                .any(|retired| retired.shares_reply(edit))
        })
    }

    /// Whether the shared action table refuses this book mutation right now.
    #[cfg(test)]
    pub fn blocks(&self, command: &ClientCommand) -> bool {
        self.outstanding()
            && super::super::actions::needs(command)
                .contains(&super::super::actions::Resource::Spellbook)
    }

    /// Whether a submitted scribe still awaits its result; the server consumes the
    /// cursor scroll when it answers.
    pub fn scribing(&self) -> bool {
        self.active
            .is_some_and(|pending| matches!(pending.edit, Edit::Slot { mode: 0, .. }))
    }

    /// Called only after an edit has been validated and handed to transport.
    pub fn sent(&mut self, command: &ClientCommand, now: Instant) {
        let Some(edit) = Edit::command(command) else {
            return;
        };
        // Keep the first request and its deadline even if a caller reports it twice.
        self.active.get_or_insert(Outstanding::new(edit, now));
    }

    /// Tracks a prepared scribe/memorize request only after successful submission.
    pub fn prepared_sent(&mut self, intent: &BookIntent, now: Instant) {
        let (slot, spell_id, mode) = match *intent {
            BookIntent::Memorize { gem, spell_id } => (u32::from(gem), spell_id, 1),
            BookIntent::Scribe { slot, spell_id, .. } => (u32::from(slot), spell_id, 0),
        };
        self.active.get_or_insert(Outstanding::new(
            Edit::Slot {
                slot,
                spell_id,
                mode,
            },
            now,
        ));
    }

    /// Successful and failed deletion replies both resolve their exact outstanding slot.
    pub fn observe(&mut self, update: &SpellUpdate) -> Option<BookActionStatus> {
        let matched = match (self.active.map(|pending| pending.edit), update) {
            (Some(Edit::Forget(gem, spell)), update) => update.clears_gem(u32::from(gem), spell),
            (Some(Edit::Delete(expected)), SpellUpdate::BookDeletion { slot, .. }) => {
                expected == *slot
            }
            (Some(Edit::Swap(a, b)), SpellUpdate::BookSwap { from, to }) => a == *from && b == *to,
            (
                Some(Edit::Slot {
                    slot,
                    spell_id,
                    mode,
                }),
                SpellUpdate::Slot {
                    slot: received_slot,
                    spell_id: received_spell,
                    mode: received_mode,
                },
            ) => slot == *received_slot && spell_id == *received_spell && mode == *received_mode,
            _ => false,
        };
        if matched {
            self.active = None;
            Some(
                if matches!(update, SpellUpdate::BookDeletion { success: false, .. }) {
                    BookActionStatus::Rejected("Server rejected spellbook deletion".into())
                } else {
                    BookActionStatus::Confirmed
                },
            )
        } else {
            None
        }
    }

    /// Ends reply ownership on silence without accepting a matching retry.
    pub fn timeout(&mut self, now: Instant) -> Option<BookActionStatus> {
        let pending = self.active.take_if(|pending| now >= pending.notify_at)?;
        self.retired.push(pending.edit);
        Some(BookActionStatus::Unconfirmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_requests_wait_for_exact_slot_spell_and_mode() {
        let now = Instant::now();
        let next = ClientCommand::MemorizeSpell {
            session_id: 7,
            gem: 2,
            spell_id: 73,
            created: now,
        };
        for (intent, mode) in [
            (
                BookIntent::Memorize {
                    gem: 2,
                    spell_id: 73,
                },
                1,
            ),
            (
                BookIntent::Scribe {
                    revision: 1,
                    slot: 2,
                    spell_id: 73,
                },
                0,
            ),
        ] {
            let mut guard = BookEdits::default();
            assert!(!guard.blocks(&next));
            guard.prepared_sent(&intent, now);
            for (slot, spell_id, received_mode) in
                [(3, 73, mode), (2, 74, mode), (2, 73, 3), (2, 73, 1 - mode)]
            {
                let _ = guard.observe(&SpellUpdate::Slot {
                    slot,
                    spell_id,
                    mode: received_mode,
                });
                assert!(guard.blocks(&next));
            }
            let _ = guard.observe(&SpellUpdate::BookDeletion {
                slot: 2,
                success: true,
            });
            assert!(guard.blocks(&next));
            assert_eq!(guard.timeout(now + Duration::from_secs(29)), None);
            let result = guard.observe(&SpellUpdate::Slot {
                slot: 2,
                spell_id: 73,
                mode,
            });
            assert_eq!(result, Some(BookActionStatus::Confirmed));
            assert_eq!(
                guard.observe(&SpellUpdate::Slot {
                    slot: 2,
                    spell_id: 73,
                    mode
                }),
                None
            );
            assert!(!guard.blocks(&next));
            assert_eq!(guard.timeout(now + Duration::from_secs(31)), None);
        }
    }

    #[test]
    fn delayed_edits_release_only_on_matching_reply_without_refreshing_deadline() {
        let now = Instant::now();
        let delete = ClientCommand::DeleteSpell {
            session_id: 7,
            slot: 3,
            spell_id: 42,
            created: now,
        };
        let swap = ClientCommand::SwapSpell {
            session_id: 7,
            from: 3,
            to: 4,
            from_spell: 42,
            to_spell: None,
            created: now,
        };
        let memorize = ClientCommand::MemorizeSpell {
            session_id: 7,
            gem: 0,
            spell_id: 42,
            created: now,
        };
        let mut guard = BookEdits::default();
        guard.sent(&swap, now);
        assert!(guard.blocks(&delete) && guard.blocks(&swap) && guard.blocks(&memorize));
        guard.sent(&delete, now + Duration::from_secs(20));
        let _ = guard.observe(&SpellUpdate::BookDeletion {
            slot: 3,
            success: true,
        });
        let _ = guard.observe(&SpellUpdate::BookSwap { from: 4, to: 3 });
        let _ = guard.observe(&SpellUpdate::Slot {
            slot: 0,
            spell_id: 42,
            mode: 1,
        });
        assert!(guard.blocks(&swap));
        assert_eq!(guard.timeout(now + Duration::from_secs(29)), None);
        let _ = guard.observe(&SpellUpdate::BookSwap { from: 3, to: 4 });
        assert!(!guard.blocks(&swap) && guard.timeout(now + Duration::from_secs(31)).is_none());
        for success in [false, true] {
            guard.sent(&delete, now);
            assert_eq!(
                guard.observe(&SpellUpdate::BookDeletion { slot: 4, success }),
                None
            );
            assert!(guard.blocks(&delete));
            let result = guard.observe(&SpellUpdate::BookDeletion { slot: 3, success });
            assert_eq!(matches!(result, Some(BookActionStatus::Confirmed)), success);
            assert_eq!(
                matches!(result, Some(BookActionStatus::Rejected(_))),
                !success
            );
            assert!(!guard.blocks(&delete));
        }
    }

    #[test]
    fn only_an_outstanding_scribe_counts_as_scribing() {
        let now = Instant::now();
        let mut guard = BookEdits::default();
        assert!(!guard.scribing());
        guard.prepared_sent(
            &BookIntent::Memorize {
                gem: 0,
                spell_id: 42,
            },
            now,
        );
        assert!(!guard.scribing());
        let mut guard = BookEdits::default();
        guard.prepared_sent(
            &BookIntent::Scribe {
                revision: 1,
                slot: 0,
                spell_id: 42,
            },
            now,
        );
        assert!(guard.scribing());
    }

    #[test]
    fn timeout_retires_the_original_edit_without_refreshing_its_deadline() {
        let now = Instant::now();
        let mut edits = BookEdits::default();
        let delete = |slot, spell_id| ClientCommand::DeleteSpell {
            session_id: 7,
            slot,
            spell_id,
            created: now,
        };
        edits.sent(&delete(3, 42), now);
        edits.sent(&delete(4, 73), now + Duration::from_secs(20));
        assert_eq!(edits.timeout(now + Duration::from_secs(29)), None);
        assert_eq!(
            edits.timeout(now + Duration::from_secs(30)),
            Some(BookActionStatus::Unconfirmed)
        );
        assert!(!edits.outstanding());
        assert!(edits.ambiguous(&delete(3, 99)));
        assert!(!edits.ambiguous(&delete(4, 73)));
        assert_eq!(edits.timeout(now + Duration::from_secs(60)), None);
        edits.sent(&delete(4, 73), now + Duration::from_secs(61));
        for success in [true, true, false] {
            assert_eq!(
                edits.observe(&SpellUpdate::BookDeletion { slot: 3, success }),
                None
            );
            assert!(edits.outstanding());
        }
        assert_eq!(
            edits.observe(&SpellUpdate::BookDeletion {
                slot: 4,
                success: true
            }),
            Some(BookActionStatus::Confirmed)
        );
        assert!(!edits.outstanding());
        assert!(edits.ambiguous(&delete(3, 99)));
        assert!(!BookEdits::default().ambiguous(&delete(3, 99)));
    }
}
