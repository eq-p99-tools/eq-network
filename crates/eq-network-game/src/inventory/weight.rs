//! Carried item weight from complete authoritative inventory definitions.
use super::{Inventory, InventoryItem, InventorySlot, TradeCursor};
use crate::GameDialect;
use std::collections::BTreeMap;

/// Maximum whole weight used by the native movement calculation.
pub const MAX_CARRIED_WEIGHT: u16 = 32_766;

/// A weight that cannot be established from this inventory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeightUnavailable {
    /// No complete snapshot, a stale/correcting one, or an unbounded drop state.
    Inventory,
    /// Pending moves change the carried weight.
    Prediction,
    /// Orphaned contents or an invalid container reduction.
    Container,
    /// A counted item carries augments, whose weight the Titanium program
    /// adds and this sum does not yet (6.570 G2).
    Augments,
}

impl Inventory {
    /// Unread inventory outlives partial coverage of the submitted instance.
    /// Keep that fact separate so core falls cannot use a lighter estimate.
    pub(super) fn finish_drop_weight(&mut self, previous: bool) {
        if !previous && !self.ground_drop_projected() {
            return;
        }
        match self.carried_weight_tenths(GameDialect::Titanium, 0) {
            Ok(weight) => {
                if self.ground_drop_projected() {
                    self.drop_fall_bound = Some(weight);
                }
            }
            // Ordinary finite move predictions retain their settlement policy.
            Err(WeightUnavailable::Prediction) => (),
            Err(
                WeightUnavailable::Inventory
                | WeightUnavailable::Container
                | WeightUnavailable::Augments,
            ) => {
                self.drop_mass_unknown = true;
            }
        }
    }

    /// Conservative current Titanium load after local drop projection. A
    /// missing/stale or unbounded current view never falls back to a lighter sum.
    #[must_use]
    pub fn ground_drop_weight_tenths(&self, coins: u64) -> Option<u64> {
        self.carried_weight_tenths(GameDialect::Titanium, coins)
            .ok()
            .or_else(|| self.drop_fall_bound.map(|weight| weight + coins / 4))
    }

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
    /// client reads heavier, the slower side. It adds augments' weight,
    /// which this sum does not yet, so an augmented item counted here makes
    /// the weight unknown ([`WeightUnavailable::Augments`], 6.570 G2). The `EQMac` program's sum
    /// (0x4bbb78) is the same but for the effect and augments: a stack once,
    /// each bag content at the bag's reduction in floating point (the exact
    /// value this tenths sum reaches by reducing the bag's aggregate), coins
    /// at 1/40 less the best purse, held to 32,766. Bank/trade/queued cursor
    /// items do not count. Predictions that preserve the total remain usable.
    /// A new cursor arrival after a plain carried relocation retains placement
    /// uncertainty while using a conservative mass bound: an alternative where
    /// that arrival is still queued weighs less, never more. This does not
    /// establish the relocated item's endpoint or acknowledge a move.
    /// Known root-stack splits/merges can instead bound all conserved units by
    /// their finite touched-slot footprint, including independent refused or
    /// partial moves. Repeating the same edges adds no weight penalty.
    /// After a Titanium drop projection, retain each locally removed instance
    /// in full and the heaviest displayed/queued cursor group. A limbo arrival
    /// may still be queued on the server. This bounds the load without claiming
    /// which item it holds, and can deliberately read heavier.
    ///
    /// # Errors
    /// Rejects missing/stale state, malformed cursor groups or bags, and
    /// unbounded affecting history.
    pub fn carried_weight_tenths(
        &self,
        generation: crate::GameDialect,
        coins: u64,
    ) -> Result<u64, WeightUnavailable> {
        if !self.received() || self.stale() || self.drop_mass_unknown {
            return Err(WeightUnavailable::Inventory);
        }
        if !self.station_carried.is_empty() {
            return Err(WeightUnavailable::Prediction);
        }
        let possibly_offered_weight = match self.trade_cursor {
            TradeCursor::Clear => 0,
            TradeCursor::Offered { weight_tenths, .. } => u64::from(weight_tenths),
            TradeCursor::Uncertain => return Err(WeightUnavailable::Prediction),
        };
        let current: BTreeMap<_, _> = self.items().iter().map(|(&s, i)| (s, i)).collect();
        let visible_weight = total(&current, generation, coins)?;
        let mut weight = visible_weight;
        if self.ground_drop_projected() {
            weight += self.projected_drop_extra(generation)?;
        }
        if self.weight_history_uncertain {
            return Err(WeightUnavailable::Prediction);
        }
        let stack_bound = self.stack_weight.as_ref().filter(|bound| bound.active());
        if let Some(bound) = stack_bound {
            return bound
                .weight(&self.items, weight)
                .map(|weight| weight + possibly_offered_weight)
                .ok_or(WeightUnavailable::Prediction);
        }
        if self.predicted() {
            let mut before = current;
            for (slot, origin) in self.prediction_origins() {
                if (slot == InventorySlot::CURSOR && self.destroyed.is_some())
                    || self.weight_rebased.contains(&slot)
                {
                    continue;
                }
                if let Some(item) = origin {
                    before.insert(slot, item);
                } else {
                    before.remove(&slot);
                }
            }
            if total(&before, generation, coins)? != visible_weight {
                return Err(WeightUnavailable::Prediction);
            }
        }
        // Unconfirmed bandaging can only reduce the recorded stock. The
        // unchanged item records keep the heavier load in this bound.
        // Keep the destroyed instance's whole mass as an upper bound until
        // received cursor/snapshot evidence arrives. No lighter load is inferred.
        Ok(weight
            + possibly_offered_weight
            + self
                .destroyed
                .as_ref()
                .filter(|item| {
                    !item.rules.ground_drop_serial.is_some_and(|serial| {
                        self.drop_mass.contains_key(&(item.details.id, serial))
                    })
                })
                .map_or(0, |item| u64::from(item.details.weight_tenths)))
    }

    /// Unplaced whole instances remain full-weight even after later bag moves.
    #[expect(
        clippy::match_same_arms,
        reason = "each RoF2 arm stays its own until a read of its program replaces it (6.489)"
    )]
    fn projected_drop_extra(&self, generation: GameDialect) -> Result<u64, WeightUnavailable> {
        match generation {
            GameDialect::Titanium => (),
            // Titanium's, inferred for RoF2 (6.489).
            GameDialect::Rof2 => (),
            GameDialect::EqMac => return Err(WeightUnavailable::Inventory),
        }
        // Retain every distinct submitted instance in full, even if a
        // later projected move could put it into a reduction bag. This
        // intentionally overestimates an unreported refusal, never less.
        let retained: u64 = self
            .drop_mass
            .values()
            .map(|item| u64::from(item.details.weight_tenths))
            .sum::<u64>();
        let group_weight = |group: &[InventoryItem]| {
            let items: BTreeMap<_, _> = group.iter().map(|item| (item.slot, item)).collect();
            if items.len() != group.len() {
                return Err(WeightUnavailable::Inventory);
            }
            total(&items, generation, 0)
        };
        let shown: Vec<_> = self
            .items
            .values()
            .filter(|item| item.slot == InventorySlot::CURSOR || item.slot.is_in_cursor_bag())
            .cloned()
            .collect();
        let shown_weight = group_weight(&shown)?;
        let heaviest = self.queued.iter().try_fold(shown_weight, |weight, group| {
            group_weight(group).map(|next| weight.max(next))
        })?;
        Ok(retained + heaviest - shown_weight)
    }
}

#[expect(
    clippy::match_same_arms,
    reason = "each RoF2 arm stays its own until a read of its program replaces it (6.489)"
)]
fn total(
    items: &BTreeMap<InventorySlot, &InventoryItem>,
    generation: crate::GameDialect,
    coins: u64,
) -> Result<u64, WeightUnavailable> {
    let mut weight = 0;
    let mut contents: BTreeMap<InventorySlot, u64> = BTreeMap::new();
    for (&slot, item) in items {
        let counted = slot.is_equipment()
            || slot.is_pack()
            || slot == InventorySlot::CURSOR
            || slot.is_carried()
            || slot.is_in_cursor_bag();
        if counted && item.augmented() {
            return Err(WeightUnavailable::Augments);
        }
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
            match generation {
                crate::GameDialect::Titanium => {
                    weight += raw - raw * u64::from(bag.rules.bag_weight_reduction) / 100;
                }
                // Titanium's, inferred for RoF2 (6.489).
                crate::GameDialect::Rof2 => {
                    weight += raw - raw * u64::from(bag.rules.bag_weight_reduction) / 100;
                }
                crate::GameDialect::EqMac => *contents.entry(parent).or_default() += raw,
            }
        }
    }
    for (parent, raw) in contents {
        let reduction = u64::from(items[&parent].rules.bag_weight_reduction);
        weight += raw - raw * reduction / 100;
    }
    let coin_weight = coins / 4;
    let reduction = match generation {
        crate::GameDialect::EqMac => items
            .iter()
            .filter(|(slot, item)| slot.is_pack() && (17201..=17230).contains(&item.details.id))
            .map(|(_, item)| u64::from(item.rules.bag_weight_reduction))
            .max()
            .unwrap_or(0),
        crate::GameDialect::Titanium => 0,
        // Titanium's, inferred for RoF2 (6.489).
        crate::GameDialect::Rof2 => 0,
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
            augments: Default::default(),
            details: ItemDetails {
                base_held_model: None,
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
    fn a_station_clear_settles_carried_custody_after_the_refusal_window() {
        use crate::GameDialect::Titanium;

        let ingredient = item(30, 50);
        let mut station = ingredient.clone();
        station.slot = InventorySlot(4000);
        let mut charm = item(24, 100);
        charm.details.id = 2;
        charm.details.bonuses.as_mut().unwrap().strength = 7;
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![ingredient, charm.clone()]));
        inventory.apply(InventoryUpdate::Prediction(vec![
            station.clone(),
            charm.clone(),
        ]));
        charm.slot = InventorySlot(0);
        inventory.apply(InventoryUpdate::Prediction(vec![station, charm]));
        assert!(inventory.equipment_stats_uncertain());

        inventory.apply(InventoryUpdate::Settled);
        assert!(!inventory.equipment_stats_uncertain());
        assert!(inventory.station_moves_unconfirmed());
        assert_eq!(
            inventory.carried_weight_tenths(Titanium, 0),
            Err(WeightUnavailable::Prediction)
        );
        for clear in [
            InventoryUpdate::WorldEmptied,
            InventoryUpdate::WorldDetached,
        ] {
            let mut inventory = inventory.clone();
            inventory.apply(clear);
            assert!(!inventory.station_moves_unconfirmed());
            assert!(inventory.handoff_returns_settleable());
            assert_eq!(
                inventory.carried_weight_tenths(Titanium, 0),
                Err(WeightUnavailable::Prediction)
            );
            inventory.apply(InventoryUpdate::Settled);
            assert!(!inventory.custody_known(InventorySlot::CURSOR));
            assert!(!inventory.handoff_returns_unsettled());
            assert_eq!(inventory.carried_weight_tenths(Titanium, 0), Ok(100));
        }
    }

    fn project(inventory: &mut Inventory) {
        let projection = super::super::InventoryDropProjection {
            revision: inventory.revision(),
            item: Box::new(inventory.items()[&InventorySlot::CURSOR].clone()),
        };
        inventory.apply(InventoryUpdate::GroundDropProjected(projection));
    }

    #[test]
    fn projected_drops_keep_distinct_whole_mass_coins_once_and_cover_only_same_instance() {
        use crate::GameDialect::Titanium;
        let mut original = item(30, 50);
        original.rules.ground_drop_serial = std::num::NonZeroU32::new(17);
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![
            item(24, 100),
            original.clone(),
        ]));
        project(&mut inventory);
        assert_eq!(inventory.carried_weight_tenths(Titanium, 40), Ok(160));
        let mut stack = item(30, 1_500);
        stack.stack_count = Some(5);
        stack.rules.stack_size = 20;
        stack.rules.ground_drop_serial = std::num::NonZeroU32::new(18);
        inventory.apply(InventoryUpdate::Cursor(vec![stack.clone()]));
        assert_eq!(inventory.carried_weight_tenths(Titanium, 40), Ok(1_660));
        project(&mut inventory);
        inventory.apply(InventoryUpdate::Settled);
        assert_eq!(inventory.carried_weight_tenths(Titanium, 40), Ok(1_660));
        // An unrelated instance reusing the serial does not retire old mass.
        let mut other = original.clone();
        other.details.id += 1;
        inventory.apply(InventoryUpdate::Set(vec![other]));
        assert_eq!(inventory.carried_weight_tenths(Titanium, 40), Ok(1_710));
        inventory.apply(InventoryUpdate::Set(vec![original]));
        // The previous cursor candidate can still be carried. Only the exact
        // received identity retires a retained entry, not another item's serial.
        assert_eq!(inventory.carried_weight_tenths(Titanium, 40), Ok(1_710));
        assert!(!inventory
            .drop_mass
            .contains_key(&(1, std::num::NonZeroU32::new(17).unwrap())));
        inventory.apply(InventoryUpdate::Set(vec![stack.clone()]));
        assert_eq!(inventory.carried_weight_tenths(Titanium, 40), Ok(1_710));
        assert!(!inventory
            .drop_mass
            .contains_key(&(1, std::num::NonZeroU32::new(18).unwrap())));
        assert!(inventory.ground_drop_projected());
        inventory.apply(InventoryUpdate::Snapshot(vec![item(24, 100), stack]));
        assert!(!inventory.ground_drop_projected());
        assert_eq!(inventory.carried_weight_tenths(Titanium, 40), Ok(1_610));
    }

    #[test]
    fn projected_drop_mass_survives_later_reduction_bag_moves_without_duplicate_growth() {
        let mut original = item(30, 500);
        original.rules.ground_drop_serial = std::num::NonZeroU32::new(17);
        let mut bag = item(22, 10);
        bag.bag_slots = 10;
        bag.rules.bag_weight_reduction = 100;
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![
            original.clone(),
            bag.clone(),
            item(23, 70),
        ]));
        let projection = super::super::InventoryDropProjection {
            revision: inventory.revision(),
            item: Box::new(original),
        };
        inventory.apply(InventoryUpdate::GroundDropProjected(projection.clone()));
        for _ in 0..4 {
            inventory.apply(InventoryUpdate::GroundDropProjected(projection.clone()));
            planned_move(&mut inventory, super::super::MoveRules::EqEmu, 23, 251);
            inventory.apply(InventoryUpdate::Settled);
            assert_eq!(inventory.ground_drop_weight_tenths(40), Some(520));
            planned_move(&mut inventory, super::super::MoveRules::EqEmu, 251, 23);
            inventory.apply(InventoryUpdate::Settled);
            assert_eq!(inventory.ground_drop_weight_tenths(40), Some(590));
            assert_eq!(inventory.drop_mass.len(), 1);
        }
        inventory.apply(InventoryUpdate::Invalidated);
        assert_eq!(inventory.ground_drop_weight_tenths(40), None);
    }

    #[test]
    fn a_later_destroy_of_a_possible_queued_arrival_keeps_mass_beyond_silence() {
        use crate::GameDialect::Titanium;
        for known_identity in [false, true] {
            let mut first = item(30, 50);
            first.rules.ground_drop_serial = std::num::NonZeroU32::new(17);
            let mut arrival = item(30, 1_500);
            arrival.rules.ground_drop_serial =
                known_identity.then_some(std::num::NonZeroU32::new(18).unwrap());
            let mut inventory = Inventory::default();
            inventory.apply(InventoryUpdate::Snapshot(vec![first]));
            project(&mut inventory);
            inventory.apply(InventoryUpdate::Cursor(vec![arrival]));
            inventory.apply(InventoryUpdate::DestroyCursor);
            inventory.apply(InventoryUpdate::Settled);
            if known_identity {
                assert_eq!(inventory.carried_weight_tenths(Titanium, 40), Ok(1_560));
            } else {
                assert_eq!(
                    inventory.carried_weight_tenths(Titanium, 40),
                    Err(WeightUnavailable::Inventory)
                );
                assert_eq!(inventory.ground_drop_weight_tenths(40), None);
            }
            inventory.apply(InventoryUpdate::Snapshot(Vec::new()));
            assert_eq!(inventory.carried_weight_tenths(Titanium, 40), Ok(10));
        }
    }

    #[test]
    fn projected_drop_unbounded_received_items_stay_unknown_after_partial_instance_coverage() {
        let mut original = item(30, 50);
        original.rules.ground_drop_serial = std::num::NonZeroU32::new(17);
        let mut augmented = item(24, 100);
        augmented.augments[0] = Some(42);
        for update in [
            InventoryUpdate::Set(vec![augmented]),
            InventoryUpdate::Set(vec![item(251, 100)]),
        ] {
            let mut inventory = Inventory::default();
            inventory.apply(InventoryUpdate::Snapshot(vec![original.clone()]));
            project(&mut inventory);
            inventory.apply(update);
            assert_eq!(inventory.ground_drop_weight_tenths(0), None);
            inventory.apply(InventoryUpdate::Set(vec![original.clone()]));
            assert!(inventory.ground_drop_projected());
            assert_eq!(inventory.ground_drop_weight_tenths(0), None);
            inventory.apply(InventoryUpdate::Snapshot(vec![original.clone()]));
            assert!(!inventory.ground_drop_projected());
            assert_eq!(inventory.ground_drop_weight_tenths(0), Some(50));
        }
    }

    #[test]
    fn projected_cursor_candidate_mass_survives_a_different_resync_token() {
        let mut original = item(30, 50);
        original.rules.ground_drop_serial = std::num::NonZeroU32::new(17);
        let mut candidate = item(30, 1_500);
        candidate.details.id = 2;
        candidate.rules.ground_drop_serial = std::num::NonZeroU32::new(18);
        let mut token = item(30, 1);
        token.details.id = 3;
        token.rules.ground_drop_serial = std::num::NonZeroU32::new(19);
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![original]));
        project(&mut inventory);
        inventory.apply(InventoryUpdate::Cursor(vec![candidate.clone()]));
        assert_eq!(inventory.items()[&InventorySlot::CURSOR], candidate);
        assert_eq!(inventory.ground_drop_weight_tenths(0), Some(1_550));
        inventory.apply(InventoryUpdate::Set(vec![token]));
        inventory.apply(InventoryUpdate::Remove(InventorySlot::CURSOR));
        assert!(inventory.stale());
        let bound = inventory
            .ground_drop_weight_tenths(0)
            .expect("plain known instances retain a finite fall bound");
        assert!(
            bound >= 1_500,
            "the unconfirmed candidate can still be carried; bound={bound}"
        );
    }

    #[test]
    fn projected_cursor_candidate_mass_survives_banking_its_projected_copy() {
        let mut original = item(30, 50);
        original.rules.ground_drop_serial = std::num::NonZeroU32::new(17);
        let mut candidate = item(30, 1_500);
        candidate.details.id = 2;
        candidate.rules.ground_drop_serial = std::num::NonZeroU32::new(18);
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![original]));
        project(&mut inventory);
        inventory.apply(InventoryUpdate::Cursor(vec![candidate.clone()]));
        assert_eq!(inventory.items()[&InventorySlot::CURSOR], candidate);
        planned_move_with_bank(
            &mut inventory,
            super::super::MoveRules::EqEmu,
            30,
            2000,
            true,
        );
        inventory.apply(InventoryUpdate::Settled);
        assert_eq!(
            inventory.items()[&InventorySlot(2000)].details.id,
            candidate.details.id
        );
        let bound = inventory
            .ground_drop_weight_tenths(0)
            .expect("plain known instances retain a finite weight bound");
        assert!(bound >= 1_500, "banking the projection does not prove the candidate left the server cursor; bound={bound}");
    }

    #[test]
    fn projected_cursor_candidate_mass_deduplicates_moves_without_blocking_its_later_drop() {
        let mut original = item(30, 50);
        original.rules.ground_drop_serial = std::num::NonZeroU32::new(17);
        let mut candidate = item(30, 1_500);
        candidate.details.id = 2;
        candidate.rules.ground_drop_serial = std::num::NonZeroU32::new(18);
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![original]));
        project(&mut inventory);
        inventory.apply(InventoryUpdate::Cursor(vec![candidate.clone()]));
        for _ in 0..3 {
            planned_move(&mut inventory, super::super::MoveRules::EqEmu, 30, 22);
            inventory.apply(InventoryUpdate::Settled);
            assert_eq!(inventory.drop_mass.len(), 2);
            assert!(inventory.ground_drop_weight_tenths(0).unwrap() >= 1_550);
            planned_move(&mut inventory, super::super::MoveRules::EqEmu, 22, 30);
            inventory.apply(InventoryUpdate::Settled);
            assert_eq!(inventory.drop_mass.len(), 2);
        }
        let request = super::super::InventoryDrop {
            session_id: 1,
            own_id: 1,
            revision: inventory.revision(),
            item: candidate,
            created: std::time::Instant::now(),
        };
        assert_eq!(inventory.plan_ground_drop(&request).unwrap().get(), 18);
        let projection = super::super::InventoryDropProjection::from(&request);
        inventory.apply(InventoryUpdate::GroundDropProjected(projection.clone()));
        assert!(inventory.projected_cursor_empty());
        assert_eq!(inventory.drop_mass.len(), 2);
        assert_eq!(inventory.ground_drop_weight_tenths(0), Some(1_550));
        let after = inventory.clone();
        inventory.apply(InventoryUpdate::GroundDropProjected(projection));
        assert_eq!(inventory, after);
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
