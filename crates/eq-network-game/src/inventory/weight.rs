//! Carried item weight from complete authoritative inventory definitions.
use super::{Inventory, InventoryItem, InventorySlot};
use std::collections::BTreeMap;

/// A weight that cannot be established from this inventory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeightUnavailable {
    /// No complete snapshot, or a stale/correcting one.
    Inventory,
    /// Pending moves change the carried weight.
    Prediction,
    /// Orphaned contents or an invalid container reduction.
    Container,
}

impl Inventory {
    /// Total carried weight in tenths, including this many carried coins. A stack counts
    /// once; `EQMac` reduces the sum of each bag's contents, Titanium reduces
    /// each item (the respective servers' `Client::CalcCurrentWeight`:
    /// `EQMacEmu` `zone/client_mods.cpp` at c2cd265 from line 985, `EQEmu`
    /// `zone/client_mods.cpp` at 4aceae1; see
    /// `docs/movement-input-readiness.md`). This is the servers' rule; the
    /// official programs' own sums are unread. Bank/trade/queued cursor
    /// items do not count. Predictions that preserve the total remain usable.
    ///
    /// # Errors
    /// Rejects missing/stale state, malformed bags and weight-changing predictions.
    pub fn carried_weight_tenths(
        &self,
        generation: crate::GameDialect,
        coins: u64,
    ) -> Result<u64, WeightUnavailable> {
        if !self.received() || self.stale() {
            return Err(WeightUnavailable::Inventory);
        }
        let current: BTreeMap<_, _> = self.items().iter().map(|(&s, i)| (s, i)).collect();
        let weight = total(&current, generation, coins)?;
        if self.predicted() {
            let mut before = current;
            for (slot, origin) in self.prediction_origins() {
                if let Some(item) = origin {
                    before.insert(slot, item);
                } else {
                    before.remove(&slot);
                }
            }
            if total(&before, generation, coins)? != weight {
                return Err(WeightUnavailable::Prediction);
            }
        }
        Ok(weight)
    }
}

fn total(
    items: &BTreeMap<InventorySlot, &InventoryItem>,
    generation: crate::GameDialect,
    coins: u64,
) -> Result<u64, WeightUnavailable> {
    let mut weight = 0;
    let mut contents: BTreeMap<InventorySlot, u64> = BTreeMap::new();
    for (&slot, item) in items {
        if slot.is_equipment() || slot.is_pack() || slot == InventorySlot::CURSOR {
            if item.bag_slots > 0 && item.rules.bag_weight_reduction > 100 {
                return Err(WeightUnavailable::Container);
            }
            weight += u64::from(item.details.weight_tenths);
        } else if slot.is_carried() || slot.is_in_cursor_bag() {
            let (parent, index) = slot.parent().ok_or(WeightUnavailable::Container)?;
            let bag = items.get(&parent).ok_or(WeightUnavailable::Container)?;
            if index >= bag.bag_slots || bag.rules.bag_weight_reduction > 100 {
                return Err(WeightUnavailable::Container);
            }
            let raw = u64::from(item.details.weight_tenths);
            if generation == crate::GameDialect::Titanium {
                weight += raw - raw * u64::from(bag.rules.bag_weight_reduction) / 100;
            } else {
                *contents.entry(parent).or_default() += raw;
            }
        }
    }
    for (parent, raw) in contents {
        let reduction = u64::from(items[&parent].rules.bag_weight_reduction);
        weight += raw - raw * reduction / 100;
    }
    let coin_weight = coins / 4;
    let reduction = if generation == crate::GameDialect::EqMac {
        items
            .iter()
            .filter(|(slot, item)| slot.is_pack() && (17201..=17230).contains(&item.details.id))
            .map(|(_, item)| u64::from(item.rules.bag_weight_reduction))
            .max()
            .unwrap_or(0)
    } else {
        0
    };
    if reduction > 100 {
        return Err(WeightUnavailable::Container);
    }
    Ok(weight + coin_weight - coin_weight * reduction / 100)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        inventory::{InventoryUpdate, ItemPlacement},
        items::{EquipmentRules, ItemBonuses, ItemDetails},
    };

    fn item(slot: i32, weight: u32) -> InventoryItem {
        InventoryItem {
            activation: super::super::ItemActivation::default(),
            scroll_spell: None,
            book: None,
            rules: ItemPlacement::default(),
            slot: InventorySlot(slot),
            stack_count: None,
            charges: 0,
            bag_slots: 0,
            details: ItemDetails {
                equipment: Some(EquipmentRules::default()),
                bonuses: Some(ItemBonuses::default()),
                id: 1,
                name: "Synthetic weight fixture".into(),
                lore: String::new(),
                weight_tenths: weight,
                slots: u32::MAX,
                classes: u32::MAX,
                races: u32::MAX,
                flags: Vec::new(),
                stats: Vec::new(),
                price: None,
                icon: None,
            },
        }
    }

    #[test]
    fn generations_round_bags_separately_and_eqmac_coin_purses_follow_predictions() {
        use crate::GameDialect::{EqMac, Titanium};
        let mut bag = item(22, 10);
        bag.bag_slots = 10;
        bag.rules.bag_weight_reduction = 50;
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![
            bag.clone(),
            item(251, 13),
            item(252, 13),
        ]));
        assert_eq!(inventory.carried_weight_tenths(EqMac, 0), Ok(23));
        assert_eq!(inventory.carried_weight_tenths(Titanium, 0), Ok(24));
        bag.details.id = 17201;
        inventory.apply(InventoryUpdate::Snapshot(vec![bag.clone()]));
        assert_eq!(inventory.carried_weight_tenths(EqMac, 400), Ok(60));
        assert_eq!(inventory.carried_weight_tenths(Titanium, 400), Ok(110));
        bag.slot = InventorySlot::CURSOR;
        inventory.apply(InventoryUpdate::Prediction(vec![bag]));
        assert_eq!(
            inventory.carried_weight_tenths(EqMac, 400),
            Err(WeightUnavailable::Prediction)
        );
    }

    #[test]
    fn absent_empty_stale_and_authoritative_recovery() {
        let mut inventory = Inventory::default();
        assert_eq!(
            inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
            Err(WeightUnavailable::Inventory)
        );
        inventory.apply(InventoryUpdate::Snapshot(vec![]));
        assert_eq!(
            inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
            Ok(0)
        );
        inventory.apply(InventoryUpdate::Set(vec![item(22, 17)]));
        assert_eq!(
            inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
            Ok(17)
        );
        inventory.apply(InventoryUpdate::Invalidated);
        inventory.apply(InventoryUpdate::Remove(InventorySlot(22)));
        assert_eq!(
            inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
            Err(WeightUnavailable::Inventory)
        );
        inventory.apply(InventoryUpdate::Snapshot(vec![]));
        assert_eq!(
            inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
            Ok(0)
        );
    }

    #[test]
    fn stacks_count_once_and_bags_reduce_combined_contents_with_integer_truncation() {
        let mut bag = item(22, 10);
        bag.bag_slots = 10;
        bag.rules.bag_weight_reduction = 50;
        let mut stack = item(251, 13);
        stack.stack_count = Some(20);
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![
            bag,
            stack,
            item(252, 12),
            item(2000, 999),
        ]));
        assert_eq!(
            inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
            Ok(23)
        );
        inventory.apply(InventoryUpdate::Consume {
            slot: InventorySlot(251),
            charge: false,
        });
        assert_eq!(
            inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
            Ok(23)
        );
    }

    #[test]
    fn harmless_prediction_survives_but_banking_and_partial_correction_withdraw() {
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item(22, 20)]));
        inventory.apply(InventoryUpdate::Prediction(vec![item(23, 20)]));
        assert_eq!(
            inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
            Ok(20)
        );
        inventory.apply(InventoryUpdate::Prediction(vec![item(2000, 20)]));
        assert_eq!(
            inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
            Err(WeightUnavailable::Prediction)
        );
        inventory.apply(InventoryUpdate::Set(vec![item(22, 20)]));
        assert!(inventory.awaiting_correction());
        assert_eq!(
            inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
            Err(WeightUnavailable::Inventory)
        );
        inventory.apply(InventoryUpdate::Snapshot(vec![item(22, 20)]));
        assert_eq!(
            inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
            Ok(20)
        );
        inventory.apply(InventoryUpdate::Prediction(vec![item(2000, 20)]));
        inventory.apply(InventoryUpdate::Settled);
        assert_eq!(
            inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
            Ok(0)
        );
    }

    #[test]
    fn consumption_reconciles_the_moved_item_without_settling_an_independent_move() {
        for destination in [22, 25, 2000] {
            let mut scroll = item(22, 30);
            scroll.scroll_spell = Some(73);
            let mut other = item(24, 70);
            other.details.id = 2;
            let mut inventory = Inventory::default();
            inventory.apply(InventoryUpdate::Snapshot(vec![
                scroll.clone(),
                other.clone(),
            ]));
            scroll.slot = InventorySlot::CURSOR;
            inventory.apply(InventoryUpdate::Prediction(vec![
                scroll.clone(),
                other.clone(),
            ]));
            other.slot = InventorySlot(destination);
            inventory.apply(InventoryUpdate::Prediction(vec![scroll, other]));
            inventory.apply(InventoryUpdate::Consume {
                slot: InventorySlot::CURSOR,
                charge: false,
            });
            assert!(inventory.predicted());
            assert!(!inventory.stale());
            assert_eq!(
                inventory
                    .prediction_origins()
                    .map(|(slot, _)| slot.0)
                    .collect::<Vec<_>>(),
                if destination == 22 {
                    vec![22, 24]
                } else {
                    vec![24, destination]
                }
            );
            assert_eq!(
                inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                if destination == 2000 {
                    Err(WeightUnavailable::Prediction)
                } else {
                    Ok(70)
                }
            );
        }
    }

    #[test]
    fn consumption_does_not_guess_through_stacks_swaps_or_ambiguous_bulk_changes() {
        for case in 0..3 {
            let mut first = item(22, 30);
            let mut second = item(23, 70);
            let mut inventory = Inventory::default();
            if case == 0 {
                first.stack_count = Some(1);
            }
            if case == 1 {
                second.slot = InventorySlot::CURSOR;
            }
            if case == 2 {
                second = item(23, 30);
            }
            inventory.apply(InventoryUpdate::Snapshot(vec![
                first.clone(),
                second.clone(),
            ]));
            first.slot = InventorySlot::CURSOR;
            let after = match case {
                0 => vec![first, second],
                1 => {
                    second.slot = InventorySlot(22);
                    vec![first, second]
                }
                _ => vec![first], // Cannot choose which of two identical originals moved.
            };
            inventory.apply(InventoryUpdate::Prediction(after));
            inventory.apply(InventoryUpdate::Consume {
                slot: InventorySlot::CURSOR,
                charge: false,
            });
            assert!(inventory.predicted());
            assert_eq!(
                inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                Err(WeightUnavailable::Prediction)
            );
        }
    }

    #[test]
    fn whole_item_chains_and_known_queue_refills_preserve_weight_without_global_settlement() {
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item(22, 30)]));
        for slot in [30, 23, 30] {
            inventory.apply(InventoryUpdate::Prediction(vec![item(slot, 30)]));
        }
        let mut queued = item(30, 40);
        queued.details.id = 2;
        inventory.apply(InventoryUpdate::Cursor(vec![queued]));
        inventory.apply(InventoryUpdate::Consume {
            slot: InventorySlot::CURSOR,
            charge: false,
        });
        assert_eq!(inventory.items()[&InventorySlot::CURSOR].details.id, 2);
        assert_eq!(inventory.queued().count(), 0);
        assert_eq!(
            inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
            Ok(40)
        );
        // The intermediate slot's history is not a separate server confirmation.
        assert!(inventory.predicted());
        assert_eq!(
            inventory
                .prediction_origins()
                .map(|(slot, _)| slot.0)
                .collect::<Vec<_>>(),
            vec![23]
        );
    }

    #[test]
    fn matching_replacement_preserves_the_trace_but_a_correction_remains_uncertain() {
        for matching in [true, false] {
            let mut inventory = Inventory::default();
            inventory.apply(InventoryUpdate::Snapshot(vec![item(22, 30)]));
            inventory.apply(InventoryUpdate::Prediction(vec![item(30, 30)]));
            inventory.apply(InventoryUpdate::Set(vec![item(
                30,
                if matching { 30 } else { 40 },
            )]));
            inventory.apply(InventoryUpdate::Consume {
                slot: InventorySlot::CURSOR,
                charge: false,
            });
            assert_eq!(inventory.predicted(), !matching);
            assert_eq!(
                inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                if matching {
                    Ok(0)
                } else {
                    Err(WeightUnavailable::Inventory)
                }
            );
            inventory.apply(InventoryUpdate::Snapshot(vec![]));
            assert_eq!(
                inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                Ok(0)
            );
        }
    }
}
