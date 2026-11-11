//! Correlates a confirmed book insertion with its subsequent cursor removal.
use super::BookIntent;
use eq_network_game::{
    inventory::{Inventory, InventorySlot, InventoryUpdate},
    spells::SpellUpdate,
};

#[derive(Default)]
pub(super) struct ScribeConsumption(Option<Receipt>);

struct Receipt {
    revision: u64,
    book_slot: u32,
    spell_id: u32,
    /// When the server confirmed the scribe; its cursor removal follows.
    confirmed: Option<std::time::Instant>,
    /// The scribe's reply did not come in time, so the receipt no longer
    /// holds the cursor; it stays only to read a late answer.
    timed_out: bool,
}

/// How long a confirmed scribe waits for the cursor removal that follows it.
const REMOVAL_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

impl ScribeConsumption {
    /// Records only a validated request that transport has accepted.
    pub fn sent(&mut self, intent: &BookIntent) {
        if let BookIntent::Scribe {
            revision,
            slot,
            spell_id,
        } = *intent
        {
            self.0 = Some(Receipt {
                revision,
                book_slot: u32::from(slot),
                spell_id,
                confirmed: None,
                timed_out: false,
            });
        }
    }

    /// Requires both the exact book update and the outstanding edit's
    /// confirmation, or, for a scribe that timed out, the exact book update
    /// alone: the server's late word that its cursor removal follows.
    pub fn observe(&mut self, update: &SpellUpdate, edit_confirmed: bool) {
        if let Some(receipt) = self.0.as_mut() {
            if (edit_confirmed || receipt.timed_out)
                && receipt.confirmed.is_none()
                && matches!(update, SpellUpdate::Slot { slot, spell_id, mode: 0 }
                if *slot == receipt.book_slot && *spell_id == receipt.spell_id)
            {
                receipt.confirmed = Some(std::time::Instant::now());
            }
        }
    }

    /// The server refused the scribe before touching the cursor, so its
    /// scroll stays where it is and nothing will consume it.
    pub fn refused(&mut self) {
        self.0.take_if(|receipt| receipt.confirmed.is_none());
    }

    /// The scribe of this spell into this book slot went unanswered: its
    /// receipt stops holding the cursor (recording checklist 6.462). Any
    /// other custody is left as it is.
    pub fn timed_out(&mut self, book_slot: u32, spell_id: u32) {
        if let Some(receipt) = self.0.as_mut().filter(|receipt| {
            receipt.confirmed.is_none()
                && receipt.book_slot == book_slot
                && receipt.spell_id == spell_id
        }) {
            receipt.timed_out = true;
        }
    }

    /// Whether a confirmed scribe still waits for its cursor removal. The
    /// server confirms first and removes the scroll next, so moving items in
    /// between would move a scroll it has already consumed.
    pub fn awaiting_cursor(&self, now: std::time::Instant) -> bool {
        self.0.as_ref().is_some_and(|receipt| {
            receipt
                .confirmed
                .is_some_and(|at| now.saturating_duration_since(at) < REMOVAL_WINDOW)
        })
    }

    /// An unconfirmed scribe can still affect the cursor after the book is
    /// free, until its reply times out.
    pub fn unconfirmed(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(|receipt| receipt.confirmed.is_none() && !receipt.timed_out)
    }

    /// An uncertain scribe keeps its custody hold until received cursor evidence
    /// accounts for the scroll. An attributed removal needs an exact book
    /// confirmation and inventory revision. A queued refill may precede it.
    pub fn reconcile(&mut self, inventory: &Inventory, update: InventoryUpdate) -> InventoryUpdate {
        let Some(mut receipt) = self.0.take() else {
            return update;
        };
        let cursor = InventorySlot(30);
        if receipt.confirmed.is_none() {
            // Whole-slot evidence can account for the old scroll. A consume
            // can decrement its stack, and a cursor addition can queue behind it.
            let replaces_scroll = |item: &eq_network_game::inventory::InventoryItem| {
                item.scroll_spell != Some(receipt.spell_id)
            };
            let cursor_changed = match &update {
                InventoryUpdate::Remove(slot) if *slot == cursor => true,
                InventoryUpdate::Snapshot(items) => items
                    .iter()
                    .find(|item| item.slot == cursor)
                    .is_none_or(replaces_scroll),
                InventoryUpdate::Set(items) => items
                    .iter()
                    .any(|item| item.slot == cursor && replaces_scroll(item)),
                _ => false,
            };
            if !cursor_changed {
                self.0 = Some(receipt);
            }
            return update;
        }
        if inventory.revision() == receipt.revision
            && !inventory.stale()
            && inventory.items().get(&cursor).is_some_and(|item| {
                item.scroll_spell == Some(receipt.spell_id)
                    && item.stack_count.is_none()
                    && item.bag_slots == 0
            })
        {
            match update {
                InventoryUpdate::Remove(slot) if slot == cursor => InventoryUpdate::Consume {
                    slot: cursor,
                    charge: false,
                },
                InventoryUpdate::Cursor(_) => {
                    receipt.revision = receipt.revision.wrapping_add(1);
                    self.0 = Some(receipt);
                    update
                }
                _ => update,
            }
        } else if receipt.timed_out && update == InventoryUpdate::Remove(cursor) {
            // A late answer to a scribe whose custody was released: the
            // server used up the scroll, but the player may have moved items
            // since, so the model cannot say where it went.
            InventoryUpdate::Invalidated
        } else {
            update
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use eq_network_game::{inventory::InventoryItem, items::ItemDetails};

    /// A scroll for spell 73 moved from the inventory to the cursor.
    pub(in crate::client::session::spellbook) fn scroll_on_cursor() -> Inventory {
        let mut scroll = InventoryItem {
            activation: eq_network_game::inventory::ItemActivation::default(),
            scroll_spell: Some(73),
            book: None,
            rules: eq_network_game::inventory::ItemPlacement::default(),
            slot: InventorySlot(22),
            stack_count: None,
            charges: 0,
            bag_slots: 0,
            details: ItemDetails {
                base_held_model: None,
                equipment: None,
                bonuses: None,
                id: 42,
                name: "Synthetic scroll".into(),
                lore: String::new(),
                weight_tenths: 0,
                slots: 0,
                classes: 0,
                races: 0,
                flags: vec![],
                stats: vec![],
                price: None,
                icon: None,
            },
        };
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![scroll.clone()]));
        scroll.slot = InventorySlot(30);
        inventory.apply(InventoryUpdate::Prediction(vec![scroll]));
        inventory
    }

    fn tracker(inventory: &Inventory) -> ScribeConsumption {
        let mut tracker = ScribeConsumption::default();
        tracker.sent(&BookIntent::Scribe {
            revision: inventory.revision(),
            slot: 2,
            spell_id: 73,
        });
        tracker
    }

    fn confirmation() -> SpellUpdate {
        SpellUpdate::Slot {
            slot: 2,
            spell_id: 73,
            mode: 0,
        }
    }

    #[test]
    fn confirmed_scribe_consumes_cursor_in_worker_and_host_without_false_correction() {
        let mut worker = scroll_on_cursor();
        let mut host = worker.clone();
        let mut tracker = tracker(&worker);
        tracker.observe(&confirmation(), true);
        let update = tracker.reconcile(&worker, InventoryUpdate::Remove(InventorySlot(30)));
        assert_eq!(
            update,
            InventoryUpdate::Consume {
                slot: InventorySlot(30),
                charge: false
            }
        );
        worker.apply(update.clone());
        host.apply(update);
        assert_eq!(worker, host);
        assert!(!worker.items().contains_key(&InventorySlot(30)));
        assert!(!worker.stale());
        assert!(!worker.predicted()); // Only the consumed scroll's plain pickup is resolved.
        assert_eq!(
            tracker.reconcile(&worker, InventoryUpdate::Remove(InventorySlot(30))),
            InventoryUpdate::Remove(InventorySlot(30))
        );
    }

    #[test]
    fn unrelated_replies_intervening_changes_and_unconfirmed_scribes_cannot_authorize_consumption()
    {
        for case in 0..7 {
            let mut inventory = scroll_on_cursor();
            let mut tracker = tracker(&inventory);
            match case {
                0 => tracker.observe(&confirmation(), false),
                1 => tracker.observe(
                    &SpellUpdate::Slot {
                        slot: 3,
                        spell_id: 73,
                        mode: 0,
                    },
                    true,
                ),
                2 => tracker.observe(
                    &SpellUpdate::Slot {
                        slot: 2,
                        spell_id: 74,
                        mode: 0,
                    },
                    true,
                ),
                3 => tracker.observe(
                    &SpellUpdate::Slot {
                        slot: 2,
                        spell_id: 73,
                        mode: 1,
                    },
                    true,
                ),
                4 => {
                    tracker.observe(&confirmation(), true);
                    inventory.apply(InventoryUpdate::Invalidated);
                }
                5 => {
                    tracker.observe(&confirmation(), true);
                    let unrelated = InventoryUpdate::Remove(InventorySlot(29));
                    assert_eq!(tracker.reconcile(&inventory, unrelated.clone()), unrelated);
                }
                6 => {
                    tracker.observe(&confirmation(), true);
                    // Even a harmless-looking local round trip invalidates the receipt.
                    inventory.apply(InventoryUpdate::Prediction(
                        inventory.items().values().cloned().collect(),
                    ));
                }
                _ => unreachable!(),
            }
            let removal = InventoryUpdate::Remove(InventorySlot(30));
            assert_eq!(
                tracker.reconcile(&inventory, removal.clone()),
                removal,
                "case {case}"
            );
        }
    }

    #[test]
    fn a_timeout_releases_only_the_unanswered_scribes_own_custody() {
        let inventory = scroll_on_cursor();
        // Another scribe's timeout leaves this one's custody alone.
        let mut tracker = tracker(&inventory);
        tracker.timed_out(3, 73);
        tracker.timed_out(2, 74);
        assert!(tracker.unconfirmed());
        tracker.timed_out(2, 73);
        assert!(!tracker.unconfirmed());
        // A confirmed scribe awaiting its removal keeps its hold.
        let mut tracker = super::tests::tracker(&inventory);
        tracker.observe(&confirmation(), true);
        tracker.timed_out(2, 73);
        assert!(tracker.awaiting_cursor(std::time::Instant::now()));
        // A refusal does not drop a confirmed scribe's custody either.
        tracker.refused();
        assert!(tracker.awaiting_cursor(std::time::Instant::now()));
    }

    #[test]
    fn a_cursor_stack_decrement_does_not_resolve_unconfirmed_scribe_custody() {
        let cursor = InventorySlot(30);
        let mut inventory = scroll_on_cursor();
        let mut stack = inventory.items()[&cursor].clone();
        stack.stack_count = Some(2);
        inventory.apply(InventoryUpdate::Snapshot(vec![stack]));
        let mut tracker = tracker(&inventory);
        let decrement = InventoryUpdate::Consume {
            slot: cursor,
            charge: false,
        };
        let received = tracker.reconcile(&inventory, decrement.clone());
        assert_eq!(received, decrement);
        inventory.apply(received);
        assert_eq!(inventory.items()[&cursor].stack_count, Some(1));
        assert!(tracker.unconfirmed());

        let removal = InventoryUpdate::Remove(cursor);
        let received = tracker.reconcile(&inventory, removal.clone());
        assert_eq!(received, removal);
        inventory.apply(received);
        assert!(!inventory.items().contains_key(&cursor));
        assert!(!tracker.unconfirmed());
    }

    #[test]
    fn stacked_scrolls_and_containers_are_not_treated_as_single_consumed_scrolls() {
        for stack in [true, false] {
            let mut inventory = scroll_on_cursor();
            let mut item = inventory.items()[&InventorySlot(30)].clone();
            if stack {
                item.stack_count = Some(2);
            } else {
                item.bag_slots = 2;
            }
            inventory.apply(InventoryUpdate::Set(vec![item]));
            let mut tracker = tracker(&inventory);
            tracker.observe(&confirmation(), true);
            let removal = InventoryUpdate::Remove(InventorySlot(30));
            assert_eq!(tracker.reconcile(&inventory, removal.clone()), removal);
        }
    }
}
