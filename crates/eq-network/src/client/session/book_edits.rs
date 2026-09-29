//! Admission-local serialization of book edits whose results arrive asynchronously.
use super::ClientCommand;
use eq_network_game::spells::{BookActionStatus, SpellUpdate};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
enum Edit {
    Delete(u16),
    Swap(u16, u16),
    Slot { slot: u32, spell_id: u32, mode: u32 },
}

/// A submitted edit is never retried or released by an unrelated spell notification.
#[derive(Default)]
pub(super) struct BookEdits(Option<(Edit, Instant)>);

impl BookEdits {
    /// Whether an edit awaits its result; the shared action table then holds the book.
    pub fn outstanding(&self) -> bool {
        self.0.is_some()
    }

    /// Whether the shared action table refuses this book mutation right now.
    #[cfg(test)]
    pub fn blocks(&self, command: &ClientCommand) -> bool {
        self.outstanding()
            && super::actions::needs(command).contains(&super::actions::Resource::Spellbook)
    }

    /// Whether a submitted scribe still awaits its result; the server consumes the
    /// cursor scroll when it answers.
    pub fn scribing(&self) -> bool {
        matches!(self.0, Some((Edit::Slot { mode: 0, .. }, _)))
    }

    /// Called only after an edit has been validated and handed to transport.
    pub fn sent(&mut self, command: &ClientCommand, now: Instant) {
        let edit = match command {
            ClientCommand::DeleteSpell { slot, .. } => Edit::Delete(*slot),
            ClientCommand::SwapSpell { from, to, .. } => Edit::Swap(*from, *to),
            _ => return,
        };
        // Keep the first request and its deadline even if a caller reports it twice.
        self.0.get_or_insert((edit, now));
    }

    /// Tracks a prepared scribe/memorize request only after successful submission.
    pub fn prepared_sent(&mut self, intent: &super::spellbook::BookIntent, now: Instant) {
        use super::spellbook::BookIntent;
        let (slot, spell_id, mode) = match *intent {
            BookIntent::Memorize { gem, spell_id } => (u32::from(gem), spell_id, 1),
            BookIntent::Scribe { slot, spell_id, .. } => (u32::from(slot), spell_id, 0),
        };
        self.0.get_or_insert((
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
        let matched = match (self.0, update) {
            (Some((Edit::Delete(expected), _)), SpellUpdate::BookDeletion { slot, .. }) => {
                expected == *slot
            }
            (Some((Edit::Swap(a, b), _)), SpellUpdate::BookSwap { from, to }) => {
                a == *from && b == *to
            }
            (
                Some((
                    Edit::Slot {
                        slot,
                        spell_id,
                        mode,
                    },
                    _,
                )),
                SpellUpdate::Slot {
                    slot: received_slot,
                    spell_id: received_spell,
                    mode: received_mode,
                },
            ) => slot == *received_slot && spell_id == *received_spell && mode == *received_mode,
            _ => false,
        };
        if matched {
            self.0 = None;
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

    /// Unknown outcomes require a fresh admission snapshot, never speculative retry.
    pub fn expired(&self, now: Instant) -> bool {
        self.0
            .is_some_and(|(_, sent)| now.saturating_duration_since(sent) >= Duration::from_secs(30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_requests_wait_for_exact_slot_spell_and_mode() {
        use super::super::spellbook::BookIntent;
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
            guard.prepared_sent(&intent, now + Duration::from_secs(20));
            assert!(guard.blocks(&next));
            assert!(!guard.expired(now + Duration::from_secs(29)));
            assert!(guard.expired(now + Duration::from_secs(30)));
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
            assert!(!guard.expired(now + Duration::from_secs(31)));
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
        assert!(!guard.expired(now + Duration::from_secs(29)));
        assert!(guard.expired(now + Duration::from_secs(30)));
        let _ = guard.observe(&SpellUpdate::BookSwap { from: 3, to: 4 });
        assert!(!guard.blocks(&swap) && !guard.expired(now + Duration::from_secs(31)));
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
        use super::super::spellbook::BookIntent;
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
}
