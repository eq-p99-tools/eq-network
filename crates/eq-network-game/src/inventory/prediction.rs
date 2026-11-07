//! Follow only unambiguous whole-item relocations through local predictions.
use super::{Inventory, InventoryItem, InventorySlot, InventoryUpdate};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn same_item(left: &InventoryItem, right: &InventoryItem) -> bool {
    let mut relocated = left.clone();
    relocated.slot = right.slot;
    InventoryItem::same_slot_contents(Some(&relocated), Some(right))
}

impl Inventory {
    /// Records every changed-slot projection in send order without turning it
    /// into server authority or delaying the next move.
    pub(super) fn predict(&mut self, items: Vec<InventoryItem>) {
        let next: BTreeMap<_, _> = items.into_iter().map(|item| (item.slot, item)).collect();
        // Equal endpoints cannot certify an affecting intermediate prefix.
        let preserved = self.preserves_weight(&next);
        let bounded = self.record_stack_weight(&next, preserved);
        self.weight_history_uncertain |= !preserved && !bounded;
        self.trace_relocation(&next);
        let changed: BTreeSet<_> = self
            .items
            .keys()
            .chain(next.keys())
            .copied()
            .filter(|slot| !InventoryItem::same_slot_contents(self.items.get(slot), next.get(slot)))
            .collect();
        for slot in &changed {
            // Charm through waist: the slots whose items' statistics and worn
            // effects the servers apply (EQEmu `BONUS_BEGIN`..`BONUS_STAT_END`);
            // the ammo slot (21) contributes skill modifiers only.
            if (0..=20).contains(&slot.0)
                && [self.items.get(slot), next.get(slot)]
                    .into_iter()
                    .flatten()
                    .any(|item| {
                        item.details
                            .equipment
                            .is_none_or(|rules| rules.worn.is_some())
                            || item
                                .details
                                .bonuses
                                .is_none_or(crate::items::ItemBonuses::has_modifiers)
                    })
            {
                self.equipment_uncertainty.insert(*slot);
            }
            self.unconfirmed
                .entry(*slot)
                .or_insert_with(|| self.items.get(slot).cloned());
        }
        if !changed.is_empty() {
            self.predictions.push_back(changed);
        }
        self.items = next;
        self.predicted = !self.unconfirmed.is_empty();
    }

    /// Matching replacements preserve a trace; conflicting or quantity-changing
    /// updates break it. Never discard independent traces just because another
    /// item changed. The slot-origin uncertainty itself is reconciled separately.
    pub(super) fn retain_relocations(&mut self, update: &InventoryUpdate) {
        let covers = |root, slot: InventorySlot| {
            slot == root || slot.parent().is_some_and(|(parent, _)| parent == root)
        };
        self.relocations.retain(|slot, origin| match update {
            InventoryUpdate::Prediction(_)
            | InventoryUpdate::DestroyCursor
            | InventoryUpdate::Cursor(_)
            | InventoryUpdate::Consume { .. } => true,
            InventoryUpdate::Set(items) => items.first().is_none_or(|root| {
                !covers(root.slot, *origin)
                    && (!covers(root.slot, *slot)
                        || InventoryItem::same_slot_contents(
                            self.items.get(slot),
                            items.iter().find(|item| item.slot == *slot),
                        ))
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
            .filter(|slot| !InventoryItem::same_slot_contents(self.items.get(slot), next.get(slot)))
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
        self.follow_rebased_weight(&changed, movement);
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
