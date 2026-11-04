//! A property bound for conserved units, including independent refused moves.
use super::{Inventory, InventoryItem, InventorySlot, InventoryUpdate};
use std::collections::{BTreeMap, BTreeSet};

/// A successful, partial or refused root move cannot create units or leave its
/// endpoint footprint. A family occupies at most min(units, footprint slots)
/// stacks, even when an earlier merge failed and later actions were accepted.
/// Whole non-stack instances keep their count. This is independent of the
/// projected endpoint/prefix and adds no penalty for repeating the same edges.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct StackWeightBound {
    baseline: BTreeMap<InventorySlot, InventoryItem>,
    slots: BTreeSet<InventorySlot>,
    active: bool,
}

fn root(slot: InventorySlot) -> bool {
    (0..=30).contains(&slot.0)
}

fn stack_slot(slot: InventorySlot) -> bool {
    (22..=30).contains(&slot.0)
}

fn units(item: &InventoryItem) -> Option<u64> {
    if item.bag_slots != 0
        || (17201..=17230).contains(&item.details.id)
        || item.details.equipment.is_none()
        || item.details.bonuses.is_none()
    {
        return None;
    }
    match item.stack_count {
        Some(count) if count > 0 && count <= item.rules.stack_size => Some(u64::from(count)),
        Some(_) => None,
        None => Some(1),
    }
}

fn same_definition(left: &InventoryItem, right: &InventoryItem) -> bool {
    let mut normalized = left.clone();
    normalized.slot = right.slot;
    normalized.stack_count = normalized.stack_count.map(|_| 1);
    let mut right = right.clone();
    right.stack_count = right.stack_count.map(|_| 1);
    normalized == right
}

fn families<'a>(
    items: impl Iterator<Item = &'a InventoryItem>,
) -> Option<Vec<(&'a InventoryItem, u64)>> {
    let mut result: Vec<(&InventoryItem, u64)> = Vec::new();
    for item in items {
        let count = units(item)?;
        if let Some((_, total)) = result
            .iter_mut()
            .find(|(other, _)| same_definition(item, other))
        {
            *total = total.checked_add(count)?;
        } else {
            result.push((item, count));
        }
    }
    Some(result)
}

impl StackWeightBound {
    pub(super) const fn active(&self) -> bool {
        self.active
    }

    fn extend(
        &mut self,
        before: &BTreeMap<InventorySlot, InventoryItem>,
        after: &BTreeMap<InventorySlot, InventoryItem>,
    ) -> Option<()> {
        let changed: BTreeSet<_> = before
            .keys()
            .chain(after.keys())
            .copied()
            .filter(|slot| before.get(slot) != after.get(slot))
            .collect();
        if changed.len() != 2 || changed.iter().any(|slot| !root(*slot)) {
            return None;
        }
        let previous: Vec<_> = changed.iter().filter_map(|slot| before.get(slot)).collect();
        let next: Vec<_> = changed.iter().filter_map(|slot| after.get(slot)).collect();
        let first = *previous.first()?;
        for item in previous.iter().chain(&next).copied() {
            units(item)?;
        }
        if first.stack_count.is_some() {
            if changed.iter().any(|slot| !stack_slot(*slot))
                || previous
                    .iter()
                    .chain(&next)
                    .any(|item| !same_definition(first, item))
                || previous
                    .iter()
                    .map(|item| u64::from(item.stack_count.unwrap()))
                    .sum::<u64>()
                    != next
                        .iter()
                        .map(|item| u64::from(item.stack_count.unwrap()))
                        .sum::<u64>()
            {
                return None;
            }
        } else if previous.len() != 1 || next.len() != 1 || !same_definition(first, next[0]) {
            // Inter-family swaps and bulk projections are outside this proof.
            return None;
        }
        for slot in &changed {
            if let Some(item) = self.baseline.get(slot) {
                units(item)?;
            }
        }
        let baseline: Vec<_> = self
            .slots
            .union(&changed)
            .filter_map(|slot| self.baseline.get(slot))
            .collect();
        if baseline.iter().any(|item| {
            baseline
                .iter()
                .any(|other| item.details.id == other.details.id && !same_definition(item, other))
        }) {
            // A server merge retains one definition. Differing definitions for
            // one ID could turn cheap units into heavier units on a later split.
            return None;
        }
        self.slots.extend(changed);
        Some(())
    }

    pub(super) fn weight(
        &self,
        current: &BTreeMap<InventorySlot, InventoryItem>,
        total: u64,
    ) -> Option<u64> {
        let baseline = families(self.slots.iter().filter_map(|slot| self.baseline.get(slot)))?;
        let present = families(self.slots.iter().filter_map(|slot| current.get(slot)))?;
        if baseline.len() != present.len()
            || baseline.iter().any(|(item, count)| {
                !present
                    .iter()
                    .any(|(now, units)| count == units && same_definition(item, now))
            })
        {
            return None;
        }
        let slots = u64::try_from(self.slots.len()).ok()?;
        let mut maximum = 0_u64;
        for (item, count) in baseline {
            let occupied = if item.stack_count.is_some() {
                count.min(slots)
            } else {
                count
            };
            maximum = maximum
                .checked_add(occupied.checked_mul(u64::from(item.details.weight_tenths))?)?;
        }
        let here: u64 = self
            .slots
            .iter()
            .filter_map(|slot| current.get(slot))
            .map(|item| u64::from(item.details.weight_tenths))
            .sum();
        if maximum < here {
            return None;
        }
        total.checked_sub(here)?.checked_add(maximum)
    }
}

impl Inventory {
    pub(super) fn record_stack_weight(
        &mut self,
        next: &BTreeMap<InventorySlot, InventoryItem>,
        preserved: bool,
    ) -> bool {
        let previous = self.stack_weight.take();
        let was_active = previous.as_ref().is_some_and(StackWeightBound::active);
        let candidate = previous.or_else(|| {
            (self.received()
                && !self.stale()
                && !self.predicted()
                && !self.weight_history_uncertain
                && self.weight_rebased.is_empty()
                && self.queued.is_empty())
            .then(|| StackWeightBound {
                baseline: self
                    .items
                    .iter()
                    .filter(|(slot, _)| root(**slot))
                    .map(|(&slot, item)| (slot, item.clone()))
                    .collect(),
                slots: BTreeSet::new(),
                active: false,
            })
        });
        if let Some(mut proof) = candidate {
            if proof.extend(&self.items, next).is_some() {
                proof.active |= !preserved;
                let bounded = proof.active;
                self.stack_weight = Some(proof);
                return bounded;
            }
        }
        self.weight_history_uncertain |= was_active;
        false
    }

    pub(super) fn observe_stack_weight(&mut self, update: &InventoryUpdate) {
        if matches!(update, InventoryUpdate::Prediction(_)) {
            return;
        }
        if let Some(proof) = self.stack_weight.take() {
            // Even a matching partial receipt must not be mistaken for evidence
            // covering all independently accepted/refused outcomes of the chain.
            self.weight_history_uncertain |= proof.active;
        }
    }
}

#[cfg(test)]
mod tests;
