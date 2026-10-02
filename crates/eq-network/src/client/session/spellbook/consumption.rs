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
}

/// How long a confirmed scribe waits for the cursor removal that follows it.
const REMOVAL_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

impl ScribeConsumption {
    /// Records only a validated request that transport has accepted.
    pub fn sent(&mut self, intent: &BookIntent) {
        self.0 = match *intent {
            BookIntent::Scribe {
                revision,
                slot,
                spell_id,
            } => Some(Receipt {
                revision,
                book_slot: u32::from(slot),
                spell_id,
                confirmed: None,
            }),
            BookIntent::Memorize { .. } => None,
        };
    }

    /// Requires both the exact book update and the outstanding edit's confirmation.
    pub fn observe(&mut self, update: &SpellUpdate, edit_confirmed: bool) {
        if let Some(receipt) = self.0.as_mut() {
            if edit_confirmed
                && matches!(update, SpellUpdate::Slot { slot, spell_id, mode: 0 }
                if *slot == receipt.book_slot && *spell_id == receipt.spell_id)
            {
                receipt.confirmed = Some(std::time::Instant::now());
            }
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

    /// Consumes the receipt on the next inventory update, never on a later retry.
    /// A changed inventory, stackable scroll, or unmatched response stays conservative.
    pub fn reconcile(&mut self, inventory: &Inventory, update: InventoryUpdate) -> InventoryUpdate {
        let Some(receipt) = self.0.take() else {
            return update;
        };
        let cursor = InventorySlot(30);
        if receipt.confirmed.is_some()
            && inventory.revision() == receipt.revision
            && !inventory.stale()
            && update == InventoryUpdate::Remove(cursor)
            && inventory.items().get(&cursor).is_some_and(|item| {
                item.scroll_spell == Some(receipt.spell_id)
                    && item.stack_count.is_none()
                    && item.bag_slots == 0
            })
        {
            InventoryUpdate::Consume {
                slot: cursor,
                charge: false,
            }
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
            icon: 0,
            stack_count: None,
            charges: 0,
            bag_slots: 0,
            details: ItemDetails {
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
        assert!(worker.predicted()); // No invented confirmation of preceding storage moves.
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
