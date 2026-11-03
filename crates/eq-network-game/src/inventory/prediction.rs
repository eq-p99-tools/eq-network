//! Follow only unambiguous whole-item relocations through local predictions.
use super::{Inventory, InventoryItem, InventorySlot, InventoryUpdate};
use std::collections::{BTreeMap, BTreeSet};

fn same_item(left: &InventoryItem, right: &InventoryItem) -> bool {
    let mut relocated = left.clone();
    relocated.slot = right.slot;
    relocated == *right
}

impl Inventory {
    /// Matching replacements preserve a trace; conflicting or quantity-changing
    /// updates break it. Never discard independent traces just because another
    /// item changed. The slot-origin uncertainty itself is reconciled separately.
    pub(super) fn retain_relocations(&mut self, update: &InventoryUpdate) {
        let covers = |root, slot: InventorySlot| {
            slot == root || slot.parent().is_some_and(|(parent, _)| parent == root)
        };
        self.relocations.retain(|slot, origin| match update {
            InventoryUpdate::Prediction(_)
            | InventoryUpdate::Cursor(_)
            | InventoryUpdate::Consume { .. } => true,
            InventoryUpdate::Set(items) => items.first().is_none_or(|root| {
                !covers(root.slot, *origin)
                    && (!covers(root.slot, *slot)
                        || self.items.get(slot) == items.iter().find(|item| item.slot == *slot))
            }),
            InventoryUpdate::Remove(root)
            | InventoryUpdate::Used(root)
            | InventoryUpdate::Deduct { slot: root, .. } => {
                !covers(*root, *slot) && !covers(*root, *origin)
            }
            InventoryUpdate::TradeEmptied => !slot.is_in_trade() && !origin.is_in_trade(),
            InventoryUpdate::WorldEmptied => !slot.is_world() && !origin.is_world(),
            InventoryUpdate::Snapshot(_)
            | InventoryUpdate::Invalidated
            | InventoryUpdate::Settled => false,
        });
    }

    /// A plain move changes exactly two slots: one empties and one fills. Do
    /// not infer identity through swaps, stacks, containers or bulk projections.
    pub(super) fn trace_relocation(&mut self, next: &BTreeMap<InventorySlot, InventoryItem>) {
        let changed: BTreeSet<_> = self
            .items
            .keys()
            .chain(next.keys())
            .copied()
            .filter(|slot| self.items.get(slot) != next.get(slot))
            .collect();
        let movement = if changed.len() == 2 {
            let from = changed
                .iter()
                .find(|slot| self.items.contains_key(slot) && !next.contains_key(slot));
            let to = changed
                .iter()
                .find(|slot| !self.items.contains_key(slot) && next.contains_key(slot));
            from.zip(to).and_then(|(&from, &to)| {
                let item = &self.items[&from];
                (item.stack_count.is_none() && item.bag_slots == 0 && same_item(item, &next[&to]))
                    .then_some((to, self.relocations.get(&from).copied().unwrap_or(from)))
            })
        } else {
            None
        };
        self.relocations.retain(|slot, _| !changed.contains(slot));
        if let Some((to, origin)) = movement {
            self.relocations.insert(to, origin);
        }
    }

    /// Consumption changes the earlier alternative too: the traced item no
    /// longer exists at its origin. Other items moved into that slot, and all
    /// independent predictions, must still await their own evidence.
    pub(super) fn reconcile_consumed(&mut self, slot: InventorySlot, item: Option<&InventoryItem>) {
        let origin = self.relocations.remove(&slot);
        let (Some(origin), Some(item)) = (origin, item) else {
            return;
        };
        if !self.received()
            || self.stale()
            || !self
                .unconfirmed
                .get(&origin)
                .and_then(Option::as_ref)
                .is_some_and(|before| same_item(before, item))
        {
            return;
        }
        if self.items.contains_key(&origin) {
            self.unconfirmed.insert(origin, None);
        } else {
            self.unconfirmed.remove(&origin);
        }
        // An originally empty destination is now authoritatively empty again.
        // A different original occupant still belongs to an unresolved move.
        if self.unconfirmed.get(&slot).is_some_and(Option::is_none) {
            self.unconfirmed.remove(&slot);
        }
        self.resolved();
    }
}
