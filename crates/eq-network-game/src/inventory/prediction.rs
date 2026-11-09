//! Follow only unambiguous whole-item relocations through local predictions.
use super::{Inventory, InventoryItem, InventorySlot, InventoryUpdate, TradeCursor};
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
        let changed: BTreeSet<_> = self
            .items
            .keys()
            .chain(next.keys())
            .copied()
            .filter(|slot| !InventoryItem::same_slot_contents(self.items.get(slot), next.get(slot)))
            .collect();
        let cursor = changed.iter().any(|slot| {
            *slot == InventorySlot::CURSOR
                || slot.is_in_cursor_bag()
                || self
                    .projected_custody
                    .get(slot)
                    .is_some_and(|custody| custody.cursor)
        });
        let cursor_slot = InventorySlot::CURSOR;
        if changed.contains(&cursor_slot) && self.trade_cursor != TradeCursor::Clear {
            self.trade_cursor = TradeCursor::Uncertain;
        } else if cursor && changed.iter().any(|slot| slot.is_in_trade()) {
            self.trade_cursor = self
                .trade_offer(&changed, &next)
                .unwrap_or(TradeCursor::Uncertain);
        }
        let station = changed.iter().any(|slot| {
            slot.is_world()
                || self
                    .projected_custody
                    .get(slot)
                    .is_some_and(|custody| custody.station)
        });
        self.track_station_cursor_return(&changed, &next, station);
        // Equal endpoints cannot certify an affecting intermediate prefix.
        // A station route keeps its carried footprint beyond ordinary settlement.
        let preserved = self.preserves_weight(&next);
        let bounded = self.record_stack_weight(&next, preserved);
        self.weight_history_uncertain |= !station && !preserved && !bounded;
        self.trace_relocation(&next);
        let station_carried = station
            || changed
                .iter()
                .any(|slot| self.station_carried.contains(slot));
        if station_carried {
            self.station_carried
                .extend(changed.iter().copied().filter(|&slot| {
                    slot == cursor_slot
                        || slot.is_carried()
                        || slot.is_equipment()
                        || slot.is_in_cursor_bag()
                }));
        }
        for slot in &changed {
            let bandage = (slot.is_equipment() || slot.is_carried())
                && [self.items.get(slot), next.get(slot)]
                    .into_iter()
                    .flatten()
                    .any(|item| item.rules.item_type == crate::bind_wound::BANDAGE);
            let custody = self.projected_custody.entry(*slot).or_default();
            custody.bandage |= bandage;
            custody.cursor |= cursor;
            custody.station |= station;
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
                if station_carried {
                    self.station_equipment_uncertainty.insert(*slot);
                } else {
                    self.equipment_uncertainty.insert(*slot);
                }
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

    /// A received non-stack cursor item moved whole into an empty escrow slot.
    fn trade_offer(
        &self,
        changed: &BTreeSet<InventorySlot>,
        next: &BTreeMap<InventorySlot, InventoryItem>,
    ) -> Option<TradeCursor> {
        let (item, serial) = self.whole_cursor_offer(changed, next, InventorySlot::is_trade)?;
        Some(TradeCursor::Offered {
            id: item.details.id,
            serial,
            weight_tenths: item.details.weight_tenths,
        })
    }

    fn track_station_cursor_return(
        &mut self,
        changed: &BTreeSet<InventorySlot>,
        next: &BTreeMap<InventorySlot, InventoryItem>,
        station: bool,
    ) {
        let cursor = InventorySlot::CURSOR;
        if changed.contains(&cursor) {
            self.station_cursor_return = if station && !self.station_carried.contains(&cursor) {
                self.whole_cursor_offer(changed, next, InventorySlot::is_world)
                    .map(|(item, serial)| (item.details.id, serial))
            } else {
                None
            };
        }
    }

    fn whole_cursor_offer(
        &self,
        changed: &BTreeSet<InventorySlot>,
        next: &BTreeMap<InventorySlot, InventoryItem>,
        accepts: impl Fn(InventorySlot) -> bool,
    ) -> Option<(&InventoryItem, std::num::NonZeroU32)> {
        let cursor = InventorySlot::CURSOR;
        if changed.len() != 2
            || next.contains_key(&cursor)
            || !self.queued.is_empty()
            || !self.custody_known(cursor)
        {
            return None;
        }
        let item = self.items.get(&cursor)?;
        let serial = item.rules.ground_drop_serial?;
        let slot = changed.iter().find(|&&slot| accepts(slot))?;
        (item.bag_slots == 0
            && item.stack_count.is_none()
            && !self.items.contains_key(slot)
            && next.get(slot).is_some_and(|placed| same_item(item, placed)))
        .then_some((item, serial))
    }

    /// Matching replacements preserve a trace; conflicting or quantity-changing
    /// updates break it. Never discard independent traces just because another
    /// item changed. The slot-origin uncertainty itself is reconciled separately.
    pub(super) fn retain_relocations(&mut self, update: &InventoryUpdate) {
        let covers = |root, slot: InventorySlot| {
            slot == root || slot.parent().is_some_and(|(parent, _)| parent == root)
        };
        let station_pending = self.station_moves_unconfirmed();
        self.relocations.retain(|slot, origin| match update {
            InventoryUpdate::BandageStockPending(_)
            | InventoryUpdate::BandageStockUncertain(_)
            | InventoryUpdate::Prediction(_)
            | InventoryUpdate::DestroyCursor
            | InventoryUpdate::GroundDropPending(_)
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
            InventoryUpdate::WorldEmptied | InventoryUpdate::WorldDetached => {
                !slot.is_world() && !origin.is_world()
            }
            InventoryUpdate::Settled => {
                station_pending
                    && [slot, origin].into_iter().any(|key| {
                        self.projected_custody
                            .get(key)
                            .is_some_and(|custody| custody.station)
                    })
            }
            InventoryUpdate::Snapshot(_) | InventoryUpdate::Invalidated => false,
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
