//! Rebase only a carried-mass bound; never certify predicted placement.
use super::{prediction::same_item, Inventory, InventoryItem, InventorySlot, InventoryUpdate};
use std::collections::BTreeSet;

fn plain_carried(slot: InventorySlot, item: &InventoryItem) -> bool {
    (0..=30).contains(&slot.0)
        && item.stack_count.is_none()
        && item.bag_slots == 0
        // A purse can change the coin contribution when put in a pack slot.
        && !(17201..=17230).contains(&item.details.id)
}

enum Arrival {
    EmptyOrigin,
    Relocated(InventorySlot),
}

impl Inventory {
    /// Whether a usable carried weight conservatively includes a queued arrival
    /// or unresolved root-stack distribution. Not a readiness grant.
    #[must_use]
    pub fn carried_weight_is_conservative(&self) -> bool {
        !self.weight_rebased.is_empty()
            || self
                .stack_weight
                .as_ref()
                .is_some_and(super::stack_weight::StackWeightBound::active)
    }

    /// Whether the reducer can include this normalized cursor arrival in a
    /// conservative mass bound instead of treating it as a correction. Shared
    /// with session resync classification; does not apply or acknowledge a move.
    #[must_use]
    pub fn can_rebase_cursor_arrival(&self, update: &InventoryUpdate) -> bool {
        match update {
            InventoryUpdate::Cursor(items) | InventoryUpdate::Set(items) => {
                self.cursor_arrival(items).is_some()
            }
            _ => false,
        }
    }

    pub(super) fn rebase_cursor_arrival(&mut self, update: &InventoryUpdate) -> bool {
        let (InventoryUpdate::Cursor(items) | InventoryUpdate::Set(items)) = update else {
            return false;
        };
        self.rebase_arriving_items(items)
    }

    pub(super) fn rebase_arriving_items(&mut self, items: &[InventoryItem]) -> bool {
        let Some(arrival) = self.cursor_arrival(items) else {
            return false;
        };
        self.weight_rebased.insert(InventorySlot::CURSOR);
        if let Arrival::Relocated(destination) = arrival {
            self.weight_rebased.insert(destination);
        }
        true
    }

    fn cursor_arrival(&self, items: &[InventoryItem]) -> Option<Arrival> {
        let [arrival] = items else {
            return None;
        };
        if !self.received()
            || self.stale()
            || self.weight_history_uncertain
            || arrival.slot != InventorySlot::CURSOR
            || !plain_carried(arrival.slot, arrival)
            || self.items.contains_key(&InventorySlot::CURSOR)
            || self.items.len() + self.queued.iter().map(Vec::len).sum::<usize>() + items.len()
                > 1024
        {
            return None;
        }
        let origin = self.unconfirmed.get(&InventorySlot::CURSOR)?;
        // Repeated definitions cannot identify an additional arrival.
        if self
            .items
            .values()
            .any(|item| item.details.id == arrival.details.id)
            || self
                .unconfirmed
                .values()
                .flatten()
                .any(|item| item.details.id == arrival.details.id)
        {
            return None;
        }
        if origin.is_none() {
            // The cursor was empty before the chain. Keep the new arrival in
            // the mass comparison instead of replacing it with that old None.
            // No other slot or placement origin is covered by this bound.
            return Some(Arrival::EmptyOrigin);
        }
        let origin = origin.as_ref()?;
        let mut traces = self
            .relocations
            .iter()
            .filter(|(_, origin)| **origin == InventorySlot::CURSOR);
        let (&destination, _) = traces.next()?;
        if traces.next().is_some()
            || !self
                .unconfirmed
                .get(&destination)
                .is_some_and(Option::is_none)
            || !self
                .items
                .get(&destination)
                .is_some_and(|item| plain_carried(destination, item) && same_item(origin, item))
        {
            return None;
        }
        // The old item's possible carried positions all have its full mass.
        // Count it and the new arrival once in the bound, without deciding
        // whether the old item reached this destination or the arrival is queued.
        Some(Arrival::Relocated(destination))
    }

    /// Only Set reports actual slot contents. A Cursor transfer can instead be
    /// queued, so its existing placement origins are deliberately untouched.
    pub(super) fn cover_arrival(&mut self, root: InventorySlot) {
        for slot in self.covered(root) {
            self.unconfirmed.remove(&slot);
        }
        self.resolved();
    }

    pub(super) fn follow_rebased_weight(
        &mut self,
        changed: &BTreeSet<InventorySlot>,
        movement: Option<(InventorySlot, InventorySlot)>,
    ) {
        if !changed
            .iter()
            .any(|slot| self.weight_rebased.contains(slot))
        {
            return;
        }
        // The relocation tracer gives the final slot and original origin, not
        // the immediate source. Find that source in the current changed slots.
        let from = changed.iter().find(|slot| self.items.contains_key(slot));
        if let (Some(&from), Some((to, _))) = (from, movement) {
            if self
                .items
                .get(&from)
                .is_some_and(|item| plain_carried(from, item) && plain_carried(to, item))
            {
                self.weight_rebased.extend([from, to]);
                return;
            }
        }
        // Stack, swap, bag/bank or otherwise ambiguous changes cannot turn
        // the bound into exact knowledge. A full baseline must recover it.
        self.weight_history_uncertain = true;
    }

    pub(super) fn observe_rebased_weight(&mut self, update: &InventoryUpdate, arrival: bool) {
        let root = match update {
            InventoryUpdate::Set(items) if !arrival => items
                .first()
                .and_then(|item| (self.items.get(&item.slot) != Some(item)).then_some(item.slot)),
            InventoryUpdate::Remove(slot) if self.items.contains_key(slot) => Some(*slot),
            InventoryUpdate::Consume { slot, .. }
            | InventoryUpdate::Used(slot)
            | InventoryUpdate::Deduct { slot, .. } => Some(*slot),
            _ => None,
        };
        if root.is_some_and(|root| self.weight_rebased.contains(&root)) {
            self.weight_history_uncertain = true;
        }
    }
}
