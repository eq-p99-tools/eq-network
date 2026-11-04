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
    /// Coin-free weight establishes the item contribution; exactly one hundred
    /// coin-weight units distinguishes every decoded purse reduction percent.
    /// Together these prove equality for every coin count, in both dialects.
    pub(super) fn preserves_weight(&self, next: &BTreeMap<InventorySlot, InventoryItem>) -> bool {
        let before: BTreeMap<_, _> = self
            .items
            .iter()
            .map(|(&slot, item)| (slot, item))
            .collect();
        let after: BTreeMap<_, _> = next.iter().map(|(&slot, item)| (slot, item)).collect();
        [crate::GameDialect::EqMac, crate::GameDialect::Titanium]
            .into_iter()
            .all(|generation| {
                [0, 400].into_iter().all(|coins| {
                    total(&before, generation, coins)
                        .is_ok_and(|weight| total(&after, generation, coins) == Ok(weight))
                })
            })
    }

    /// Total carried weight in tenths, including this many carried coins. A stack counts
    /// once; `EQMac` reduces the sum of each bag's contents, Titanium reduces
    /// each item (the respective servers' `Client::CalcCurrentWeight`:
    /// `EQMacEmu` `zone/client_mods.cpp` at c2cd265 from line 985, `EQEmu`
    /// `zone/client_mods.cpp` at 4aceae1; see
    /// `docs/movement-input-readiness.md`). The Titanium program's own sum
    /// (0x41bb06, stored by the stat recompute 0x4212b3) agrees: slots 0
    /// through 30 only, a stack's weight once, each bag content at the
    /// bag's reduction with the bag itself in full, coins at 1/40 each less
    /// the best purse in a general slot, held to 32,766. It also takes off
    /// a weight-reduction effect (221), which this sum does not, so the
    /// client reads heavier, the slower side. The `EQMac` program's sum
    /// (0x4bbb78) is the same but for the effect and augments: a stack once,
    /// each bag content at the bag's reduction in floating point (the exact
    /// value this tenths sum reaches by reducing the bag's aggregate), coins
    /// at 1/40 less the best purse, held to 32,766. Bank/trade/queued cursor
    /// items do not count. Predictions that preserve the total remain usable.
    /// A new cursor arrival after a plain carried relocation retains placement
    /// uncertainty while using a conservative mass bound: an alternative where
    /// that arrival is still queued weighs less, never more. This does not
    /// establish the relocated item's endpoint or acknowledge a move.
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
        if self.weight_history_uncertain {
            return Err(WeightUnavailable::Prediction);
        }
        if self.predicted() {
            let mut before = current;
            for (slot, origin) in self.prediction_origins() {
                if self.weight_rebased.contains(&slot) {
                    continue;
                }
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

    fn planned_move(inventory: &mut Inventory, rules: super::super::MoveRules, from: i32, to: i32) {
        planned_move_with_bank(inventory, rules, from, to, false);
    }

    fn planned_move_with_bank(
        inventory: &mut Inventory,
        rules: super::super::MoveRules,
        from: i32,
        to: i32,
        bank_access: bool,
    ) {
        let request = super::super::InventoryMove {
            session_id: 1,
            revision: inventory.revision(),
            sequence: None,
            from: InventorySlot(from),
            to: InventorySlot(to),
            quantity: super::super::MoveQuantity::Whole,
            created: std::time::Instant::now(),
        };
        let actor = super::super::InventoryActor {
            bank_access,
            deity: None,
            dual_wield: None,
            class: Some(1),
            race: 1,
            level: 60,
            trade_slots: 0,
            trade_no_drop: false,
            world_container: false,
        };
        let update = inventory.plan_move_for(rules, &request, actor).unwrap();
        inventory.apply(update);
    }

    #[test]
    fn arrival_after_cursor_history_keeps_the_new_item_in_the_weight_baseline() {
        use super::super::MoveRules;
        let mut outcomes = Vec::new();
        for rules in [MoveRules::Takp, MoveRules::Quarm] {
            for cursor in [false, true] {
                let mut inventory = Inventory::default();
                inventory.apply(InventoryUpdate::Snapshot(vec![item(30, 37)]));
                for (from, to) in [(30, 8), (8, 30), (30, 24)] {
                    planned_move(&mut inventory, rules, from, to);
                    assert_eq!(
                        inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                        Ok(37)
                    );
                }
                let mut arrival = item(30, 50);
                arrival.details.id = 2;
                inventory.apply(if cursor {
                    InventoryUpdate::Cursor(vec![arrival])
                } else {
                    InventoryUpdate::Set(vec![arrival])
                });
                let weight = inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0);
                eprintln!(
                    "{rules:?} cursor={cursor}: weight={weight:?}, stale={}, origins={:?}",
                    inventory.stale(),
                    inventory.prediction_origins().collect::<Vec<_>>()
                );
                assert!(inventory.predicted());
                outcomes.push((weight, inventory.stale()));
            }
        }
        assert_eq!(outcomes, vec![(Ok(87), false); 4]);
    }

    fn after_arrival(rules: super::super::MoveRules, cursor: bool) -> Inventory {
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item(30, 37)]));
        for (from, to) in [(30, 8), (8, 30), (30, 24)] {
            planned_move(&mut inventory, rules, from, to);
        }
        let mut arrival = item(30, 50);
        arrival.details.id = 2;
        inventory.apply(if cursor {
            InventoryUpdate::Cursor(vec![arrival])
        } else {
            InventoryUpdate::Set(vec![arrival])
        });
        inventory
    }

    #[test]
    fn arrival_after_a_carried_round_trip_retains_the_old_empty_cursor_origin() {
        use super::super::MoveRules;
        for rules in [MoveRules::Takp, MoveRules::Quarm] {
            for cursor in [false, true] {
                let mut inventory = Inventory::default();
                inventory.apply(InventoryUpdate::Snapshot(vec![item(22, 37)]));
                for (from, to) in [(22, 30), (30, 28), (28, 30), (30, 22)] {
                    planned_move(&mut inventory, rules, from, to);
                }
                let mut arrival = item(30, 50);
                arrival.details.id = 2;
                inventory.apply(if cursor {
                    InventoryUpdate::Cursor(vec![arrival])
                } else {
                    InventoryUpdate::Set(vec![arrival])
                });
                assert_eq!(
                    inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                    Ok(87)
                );
                assert!(!inventory.stale());
                assert!(inventory.carried_weight_is_conservative());
                assert_eq!(inventory.unresolved_predictions(), 4);
                assert!(inventory
                    .prediction_origins()
                    .any(|(slot, origin)| { slot == InventorySlot(22) && origin.is_some() }));
            }
        }
    }

    #[test]
    fn arrival_weight_bounds_every_recorded_prefix_without_confirming_placement() {
        use super::super::MoveRules;
        for rules in [MoveRules::Takp, MoveRules::Quarm] {
            for cursor in [false, true] {
                let inventory = after_arrival(rules, cursor);
                let bound = inventory
                    .carried_weight_tenths(crate::GameDialect::EqMac, 0)
                    .unwrap();
                assert!(inventory.carried_weight_is_conservative());
                assert_eq!(inventory.unresolved_predictions(), 3);
                assert!(inventory
                    .prediction_origins()
                    .any(|(slot, origin)| { slot == InventorySlot(24) && origin.is_none() }));
                for prefix in 0..=3 {
                    let mut actual = Inventory::default();
                    actual.apply(InventoryUpdate::Snapshot(vec![item(30, 37)]));
                    for &(from, to) in [(30, 8), (8, 30), (30, 24)].iter().take(prefix) {
                        planned_move(&mut actual, rules, from, to);
                    }
                    actual.apply(InventoryUpdate::Snapshot(
                        actual.items().values().cloned().collect(),
                    ));
                    let mut arrival = item(30, 50);
                    arrival.details.id = 2;
                    actual.apply(if cursor {
                        InventoryUpdate::Cursor(vec![arrival])
                    } else {
                        InventoryUpdate::Set(vec![arrival])
                    });
                    assert!(
                        bound
                            >= actual
                                .carried_weight_tenths(crate::GameDialect::EqMac, 0)
                                .unwrap()
                    );
                }
            }
        }
    }

    #[test]
    fn affecting_intermediate_bank_swap_cannot_be_certified_by_equal_endpoints_or_arrival() {
        use super::super::MoveRules;
        for rules in [MoveRules::Takp, MoveRules::Quarm] {
            let mut inventory = Inventory::default();
            let mut heavy = item(2000, 100);
            heavy.details.id = 3;
            inventory.apply(InventoryUpdate::Snapshot(vec![item(30, 37), heavy]));
            planned_move_with_bank(&mut inventory, rules, 30, 2000, true);
            planned_move_with_bank(&mut inventory, rules, 30, 2000, true);
            planned_move(&mut inventory, rules, 30, 24);
            assert_eq!(
                inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                Err(WeightUnavailable::Prediction)
            );
            let mut arrival = item(30, 50);
            arrival.details.id = 2;
            inventory.apply(InventoryUpdate::Cursor(vec![arrival]));
            assert!(!inventory.carried_weight_is_conservative());
            assert_eq!(
                inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                Err(WeightUnavailable::Prediction)
            );
            let snapshot = inventory.items().values().cloned().collect();
            inventory.apply(InventoryUpdate::Snapshot(snapshot));
            assert_eq!(
                inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                Ok(87)
            );
        }
    }

    #[test]
    fn plain_moves_retain_the_arrival_bound_but_bank_and_swap_changes_do_not() {
        use super::super::MoveRules;
        for rules in [MoveRules::Takp, MoveRules::Quarm] {
            let mut inventory = after_arrival(rules, true);
            for (from, to) in [(30, 25), (24, 30), (30, 26)] {
                planned_move(&mut inventory, rules, from, to);
                assert_eq!(
                    inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                    Ok(87)
                );
                assert!(inventory.predicted());
            }
            let mut bank = inventory.clone();
            planned_move_with_bank(&mut bank, rules, 25, 2000, true);
            assert_eq!(
                bank.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                Err(WeightUnavailable::Prediction)
            );
            let mut other = item(27, 6);
            other.details.id = 3;
            inventory.apply(InventoryUpdate::Set(vec![other]));
            planned_move(&mut inventory, rules, 25, 30);
            planned_move(&mut inventory, rules, 30, 27);
            assert_eq!(
                inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                Err(WeightUnavailable::Prediction)
            );
            let snapshot = inventory.items().values().cloned().collect();
            inventory.apply(InventoryUpdate::Snapshot(snapshot));
            assert!(!inventory.carried_weight_is_conservative());
            assert_eq!(
                inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                Ok(93)
            );
        }
    }

    #[test]
    fn ambiguous_stack_or_repeated_item_arrivals_do_not_rebase_the_weight() {
        use super::super::MoveRules;
        for rules in [MoveRules::Takp, MoveRules::Quarm] {
            for stack in [false, true] {
                let mut original = item(30, 37);
                if stack {
                    original.stack_count = Some(2);
                    original.rules.stack_size = 20;
                }
                let mut inventory = Inventory::default();
                inventory.apply(InventoryUpdate::Snapshot(vec![original]));
                planned_move(&mut inventory, rules, 30, 24);
                let mut arrival = item(30, 50);
                arrival.details.id = if stack { 2 } else { 1 };
                inventory.apply(InventoryUpdate::Cursor(vec![arrival]));
                assert!(!inventory.carried_weight_is_conservative());
                assert_eq!(
                    inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                    Err(WeightUnavailable::Prediction)
                );
            }
        }
    }

    #[test]
    fn queued_arrival_promotion_rebases_mass_without_settling_prior_moves() {
        use super::super::MoveRules;
        for rules in [MoveRules::Takp, MoveRules::Quarm] {
            let mut inventory = Inventory::default();
            inventory.apply(InventoryUpdate::Snapshot(vec![item(30, 37)]));
            let mut arrival = item(30, 50);
            arrival.details.id = 2;
            inventory.apply(InventoryUpdate::Cursor(vec![arrival]));
            assert_eq!(inventory.queued().count(), 1);
            assert_eq!(
                inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                Ok(37)
            );
            planned_move(&mut inventory, rules, 30, 24);
            assert_eq!(inventory.queued().count(), 0);
            assert_eq!(
                inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                Ok(87)
            );
            assert!(inventory.carried_weight_is_conservative());
            assert_eq!(inventory.unresolved_predictions(), 1);
        }
    }

    #[test]
    fn arrival_mass_recovery_never_retires_affecting_or_unread_equipment_history() {
        use super::super::MoveRules;
        for rules in [MoveRules::Takp, MoveRules::Quarm] {
            for kind in 0..3 {
                let mut original = item(30, 37);
                match kind {
                    0 => original.details.bonuses.as_mut().unwrap().strength = 7,
                    1 => original.details.bonuses.as_mut().unwrap().strength = -7,
                    _ => original.details.bonuses = None,
                }
                let mut inventory = Inventory::default();
                inventory.apply(InventoryUpdate::Snapshot(vec![original]));
                for (from, to) in [(30, 8), (8, 30), (30, 24)] {
                    planned_move(&mut inventory, rules, from, to);
                }
                let mut arrival = item(30, 50);
                arrival.details.id = 2;
                inventory.apply(InventoryUpdate::Cursor(vec![arrival]));
                assert_eq!(
                    inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                    Ok(87)
                );
                assert!(inventory.equipment_stats_uncertain());
                assert!(inventory.predicted());
            }
        }
    }

    #[test]
    fn stat_neutral_equipment_history_preserves_weight_without_confirming_moves() {
        use super::super::MoveRules;
        for rules in [MoveRules::EqEmu, MoveRules::Takp, MoveRules::Quarm] {
            let original = item(24, 37);
            let mut inventory = Inventory::default();
            inventory.apply(InventoryUpdate::Snapshot(vec![original.clone()]));
            for (from, to) in [(24, 30), (30, 13), (13, 30), (30, 24)] {
                planned_move(&mut inventory, rules, from, to);
                assert_eq!(
                    inventory.carried_weight_tenths(crate::GameDialect::EqMac, 0),
                    Ok(37)
                );
                assert!(!inventory.equipment_stats_uncertain());
                assert!(inventory.predicted());
            }
            assert_eq!(inventory.items()[&InventorySlot(24)], original);
            assert_eq!(inventory.unresolved_predictions(), 4);
            assert!(inventory
                .prediction_origins()
                .any(|(slot, origin)| slot == InventorySlot(13) && origin.is_none()));
        }
    }

    #[test]
    fn affecting_and_missing_equipment_history_survives_local_round_trips() {
        use super::super::MoveRules;
        for kind in 0..5 {
            let mut original = item(24, 37);
            match kind {
                0 => original.details.bonuses.as_mut().unwrap().strength = 7,
                1 => original.details.bonuses.as_mut().unwrap().strength = -7,
                2 => original.details.bonuses = None,
                3 => original.details.equipment = None,
                _ => {
                    original.details.equipment.as_mut().unwrap().worn =
                        Some(crate::items::WornEffect {
                            spell_id: 777,
                            effect_type: 2,
                            level: 1,
                            level2: 1,
                        });
                }
            }
            let mut inventory = Inventory::default();
            inventory.apply(InventoryUpdate::Snapshot(vec![original.clone()]));
            planned_move(&mut inventory, MoveRules::Quarm, 24, 30);
            planned_move(&mut inventory, MoveRules::Quarm, 30, 13);
            assert!(inventory.equipment_stats_uncertain());
            planned_move(&mut inventory, MoveRules::Quarm, 13, 30);
            planned_move(&mut inventory, MoveRules::Quarm, 30, 24);
            assert_eq!(inventory.items()[&InventorySlot(24)], original);
            assert!(inventory.equipment_stats_uncertain());
            assert!(inventory.predicted());
            // Matching carried evidence does not acknowledge the equipment slot.
            inventory.apply(InventoryUpdate::Set(vec![original.clone()]));
            assert!(inventory.equipment_stats_uncertain());
            // A partial record has no request revision: even matching equipment
            // coverage cannot establish that every pending prefix has completed.
            inventory.apply(InventoryUpdate::Remove(InventorySlot(13)));
            assert!(inventory.equipment_stats_uncertain());
            assert!(inventory.predicted());
            inventory.apply(InventoryUpdate::Snapshot(vec![original]));
            assert!(!inventory.equipment_stats_uncertain());
            assert!(!inventory.predicted());
        }
    }

    #[test]
    fn original_affecting_equipment_out_and_back_does_not_erase_intermediate_uncertainty() {
        use super::super::MoveRules;
        for strength in [-7, 7] {
            let mut original = item(13, 37);
            original.details.bonuses.as_mut().unwrap().strength = strength;
            let mut inventory = Inventory::default();
            inventory.apply(InventoryUpdate::Snapshot(vec![original.clone()]));
            planned_move(&mut inventory, MoveRules::Quarm, 13, 30);
            assert!(inventory.equipment_stats_uncertain());
            planned_move(&mut inventory, MoveRules::Quarm, 30, 13);
            assert_eq!(inventory.items()[&InventorySlot(13)], original);
            assert!(inventory.equipment_stats_uncertain());
            assert!(inventory.predicted());
            // The same slot value could be an older report. Retain the history
            // until a complete authoritative baseline, without inferring ACKs.
            inventory.apply(InventoryUpdate::Set(vec![original.clone()]));
            assert!(inventory.equipment_stats_uncertain());
            assert!(inventory.predicted());
            inventory.apply(InventoryUpdate::Snapshot(vec![original]));
            assert!(!inventory.equipment_stats_uncertain());
        }
    }

    #[test]
    fn older_partial_slot_coverage_cannot_clear_a_newer_affecting_origin_or_clone() {
        use super::super::MoveRules;
        let mut original = item(13, 37);
        original.details.bonuses.as_mut().unwrap().strength = 7;
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![original.clone()]));
        planned_move(&mut inventory, MoveRules::Quarm, 13, 30);
        planned_move(&mut inventory, MoveRules::Quarm, 30, 13);
        inventory.apply(InventoryUpdate::Set(vec![original.clone()]));
        assert!(!inventory
            .prediction_origins()
            .any(|(slot, _)| slot == InventorySlot(13)));
        assert!(inventory.equipment_stats_uncertain());

        let mut tail = inventory.clone();
        planned_move(&mut tail, MoveRules::Quarm, 13, 30);
        planned_move(&mut tail, MoveRules::Quarm, 30, 13);
        assert!(tail
            .prediction_origins()
            .any(|(slot, _)| slot == InventorySlot(13)));
        // This indistinguishable old value covers the newer origin under the
        // legacy slot journal, but cannot certify its contribution history.
        tail.apply(InventoryUpdate::Set(vec![original.clone()]));
        tail.apply(InventoryUpdate::Remove(InventorySlot::CURSOR));
        assert!(!tail.predicted());
        assert!(tail.equipment_stats_uncertain());
        assert!(inventory.equipment_stats_uncertain());
        tail.apply(InventoryUpdate::Snapshot(vec![original]));
        assert!(!tail.equipment_stats_uncertain());
        assert!(inventory.equipment_stats_uncertain());
    }

    #[test]
    fn correction_and_reset_need_a_full_baseline_before_affecting_history_retires() {
        use super::super::MoveRules;
        let mut original = item(24, 37);
        original.details.bonuses.as_mut().unwrap().strength = -7;
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![original.clone()]));
        planned_move(&mut inventory, MoveRules::Quarm, 24, 30);
        planned_move(&mut inventory, MoveRules::Quarm, 30, 13);
        inventory.apply(InventoryUpdate::Remove(InventorySlot(13)));
        inventory.apply(InventoryUpdate::Invalidated);
        inventory.apply(InventoryUpdate::Set(vec![original.clone()]));
        inventory.apply(InventoryUpdate::Remove(InventorySlot::CURSOR));
        assert!(inventory.stale());
        assert!(inventory.equipment_stats_uncertain());
        let pending = inventory.clone();
        inventory.apply(InventoryUpdate::Snapshot(vec![original]));
        assert!(!inventory.stale());
        assert!(!inventory.equipment_stats_uncertain());
        assert!(pending.stale());
        assert!(pending.equipment_stats_uncertain());

        let mut unadmitted = Inventory::default();
        unadmitted.apply(InventoryUpdate::Set(vec![item(13, 37)]));
        assert!(!unadmitted.received());
        // No recorded complete baseline means false cannot establish readiness.
        assert_eq!(
            unadmitted.carried_weight_tenths(crate::GameDialect::EqMac, 0),
            Err(WeightUnavailable::Inventory)
        );
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
