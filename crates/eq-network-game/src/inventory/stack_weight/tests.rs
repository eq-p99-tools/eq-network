use super::*;
use crate::{
    inventory::{
        InventoryActor, InventoryMove, ItemActivation, ItemPlacement, MoveQuantity, MoveRules,
        WeightUnavailable,
    },
    items::{EquipmentRules, ItemBonuses, ItemDetails},
    GameDialect,
};
use std::{num::NonZeroU32, time::Instant};

fn item(slot: i32, id: u32, count: Option<u32>, weight: u32) -> InventoryItem {
    InventoryItem {
        activation: ItemActivation::default(),
        scroll_spell: None,
        book: None,
        slot: InventorySlot(slot),
        stack_count: count,
        charges: 0,
        bag_slots: 0,
        rules: ItemPlacement {
            stack_size: 20,
            item_type: 8,
            size: 1,
            ..ItemPlacement::default()
        },
        details: ItemDetails {
            equipment: Some(EquipmentRules::default()),
            bonuses: Some(ItemBonuses::default()),
            id,
            name: "Synthetic conserved stack".into(),
            lore: String::new(),
            weight_tenths: weight,
            slots: u32::MAX,
            classes: u32::MAX,
            races: u32::MAX,
            flags: vec![],
            stats: vec![],
            price: None,
            icon: None,
        },
    }
}

fn state(items: Vec<InventoryItem>) -> Inventory {
    let mut inventory = Inventory::default();
    inventory.apply(InventoryUpdate::Snapshot(items));
    inventory
}

fn actor() -> InventoryActor {
    InventoryActor {
        bank_access: true,
        deity: None,
        dual_wield: None,
        class: Some(1),
        race: 1,
        level: 60,
        trade_slots: 0,
        trade_no_drop: false,
        world_container: false,
    }
}

fn quantity(amount: u32) -> MoveQuantity {
    MoveQuantity::Count(NonZeroU32::new(amount).unwrap())
}

fn plan(
    inventory: &Inventory,
    rules: MoveRules,
    from: i32,
    to: i32,
    amount: MoveQuantity,
) -> anyhow::Result<InventoryUpdate> {
    inventory.plan_move_for(
        rules,
        &InventoryMove {
            session_id: 1,
            revision: inventory.revision(),
            sequence: None,
            from: InventorySlot(from),
            to: InventorySlot(to),
            quantity: amount,
            created: Instant::now(),
        },
        actor(),
    )
}

fn move_item(
    inventory: &mut Inventory,
    rules: MoveRules,
    from: i32,
    to: i32,
    amount: MoveQuantity,
) {
    let update = plan(inventory, rules, from, to, amount).unwrap();
    inventory.apply(update);
}

fn weight(inventory: &Inventory, dialect: GameDialect) -> u64 {
    inventory.carried_weight_tenths(dialect, 0).unwrap()
}

#[test]
fn one_unit_full_stacks_and_zero_or_heavy_weights_keep_their_safe_bounds() {
    for count in [1, 20] {
        for mass in [0, 7, 2000] {
            let mut inventory = state(vec![item(22, 1, Some(count), mass)]);
            for (from, to, amount) in [
                (22, 30, quantity(1)),
                (30, 29, MoveQuantity::Whole),
                (29, 30, MoveQuantity::Whole),
                (30, 22, quantity(1)),
            ] {
                move_item(&mut inventory, MoveRules::Takp, from, to, amount);
            }
            let bound = if count == 1 { mass } else { 3 * mass };
            assert_eq!(weight(&inventory, GameDialect::EqMac), u64::from(bound));
            assert!(inventory.predicted());
        }
    }
}

#[test]
fn an_active_bound_does_not_cover_bank_bag_purse_or_inter_family_swap_changes() {
    for kind in 0..4 {
        let mut other = item(24, 2, None, 11);
        if kind == 1 {
            other.bag_slots = 1;
            other.rules.bag_size = 4;
            other.rules.bag_weight_reduction = 50;
        } else if kind == 2 {
            other.details.id = 17201;
        }
        let mut inventory = state(vec![item(22, 1, Some(4), 7), other]);
        move_item(&mut inventory, MoveRules::Takp, 22, 30, quantity(1));
        assert!(inventory
            .carried_weight_tenths(GameDialect::EqMac, 0)
            .is_ok());
        let to = match kind {
            0 => 2000,
            1 => InventorySlot(24).child(0).unwrap().0,
            _ => 24,
        };
        move_item(&mut inventory, MoveRules::Takp, 30, to, MoveQuantity::Whole);
        assert_eq!(
            inventory.carried_weight_tenths(GameDialect::EqMac, 0),
            Err(WeightUnavailable::Prediction),
            "case {kind}"
        );
    }
}

#[test]
fn split_and_merge_keep_a_finite_mass_bound_and_pending_placement() {
    for (rules, dialect) in [
        (MoveRules::Takp, GameDialect::EqMac),
        (MoveRules::Quarm, GameDialect::EqMac),
        (MoveRules::EqEmu, GameDialect::Titanium),
    ] {
        let mut inventory = state(vec![item(22, 1, Some(19), 1), item(24, 2, None, 43)]);
        assert_eq!(weight(&inventory, dialect), 44);
        for cycle in 0..6 {
            for (from, to, amount) in [
                (22, 30, quantity(1)),
                (30, 29, MoveQuantity::Whole),
                (29, 30, MoveQuantity::Whole),
                (30, 22, quantity(1)),
            ] {
                move_item(&mut inventory, rules, from, to, amount);
                let expected = if cycle == 0 && to == 30 && from == 22 {
                    45
                } else {
                    46
                };
                assert_eq!(weight(&inventory, dialect), expected);
                assert!(inventory.predicted());
                assert!(inventory.carried_weight_is_conservative());
                assert!(!inventory.stale());
                assert!(!inventory.equipment_stats_uncertain());
            }
        }
        assert_eq!(inventory.items()[&InventorySlot(22)].stack_count, Some(19));
        assert!(!inventory.items().contains_key(&InventorySlot::CURSOR));
        assert!(inventory.unresolved_predictions() > 0);
    }
}

/// Each branch uses the actual planner on its own state. Zero means refusal;
/// counted actions also branch over every smaller accepted transfer. A whole
/// action may swap an unexpectedly occupied root after an earlier refusal.
fn outcomes(
    states: &[Inventory],
    rules: MoveRules,
    from: i32,
    to: i32,
    amount: MoveQuantity,
) -> Vec<Inventory> {
    let mut result = states.to_vec();
    let quantities: Vec<_> = match amount {
        MoveQuantity::Whole => vec![MoveQuantity::Whole],
        MoveQuantity::Count(count) => (1..=count.get()).map(quantity).collect(),
    };
    for inventory in states {
        for &amount in &quantities {
            if let Ok(update) = plan(inventory, rules, from, to, amount) {
                let mut next = inventory.clone();
                next.apply(update);
                let next = state(next.items().values().cloned().collect());
                if !result.iter().any(|old| old.items() == next.items()) {
                    result.push(next);
                }
            }
        }
    }
    result
}

#[test]
fn independent_refusals_partial_transfers_and_initial_occupants_fit_the_bound() {
    for rules in [MoveRules::Takp, MoveRules::Quarm, MoveRules::EqEmu] {
        for occupied in [false, true] {
            let mut projected = state(vec![item(22, 1, Some(4), 7)]);
            let mut actual = vec![projected.clone()];
            if occupied {
                projected = state(vec![item(22, 1, Some(4), 7), item(29, 2, None, 11)]);
                actual = vec![projected.clone()];
                // The local projection relocates this occupant; any server
                // refusal may leave it at29 for a later counted/whole command.
                for (from, to) in [(29, 30), (30, 24)] {
                    move_item(&mut projected, rules, from, to, MoveQuantity::Whole);
                    actual = outcomes(&actual, rules, from, to, MoveQuantity::Whole);
                }
            }
            let mut most_states = actual.len();
            for _ in 0..4 {
                for (from, to, amount) in [
                    (22, 30, quantity(2)),
                    (30, 29, MoveQuantity::Whole),
                    (29, 30, MoveQuantity::Whole),
                    (30, 22, quantity(2)),
                ] {
                    move_item(&mut projected, rules, from, to, amount);
                    actual = outcomes(&actual, rules, from, to, amount);
                    most_states = most_states.max(actual.len());
                    for branch in &actual {
                        for dialect in [GameDialect::EqMac, GameDialect::Titanium] {
                            assert!(weight(&projected, dialect) >= weight(branch, dialect));
                        }
                    }
                }
            }
            assert!(
                most_states > 10,
                "did not exercise independent branches: {most_states}"
            );
            assert!(projected.predicted());
            assert_eq!(
                weight(&projected, GameDialect::EqMac),
                if occupied { 39 } else { 21 }
            );
        }
    }
}

#[test]
fn unrelated_history_and_ambiguous_definitions_cannot_start_or_extend_the_proof() {
    for kind in 0..7 {
        let mut inventory = state(vec![item(22, 1, Some(4), 7), item(24, 2, None, 11)]);
        if kind == 0 {
            move_item(
                &mut inventory,
                MoveRules::Takp,
                24,
                2000,
                MoveQuantity::Whole,
            );
            move_item(
                &mut inventory,
                MoveRules::Takp,
                2000,
                24,
                MoveQuantity::Whole,
            );
        } else {
            let mut change = inventory.items().values().cloned().collect::<Vec<_>>();
            match kind {
                1 => change[0].details.bonuses = None,
                2 => change[0].details.id = 17201,
                3 => change[0].bag_slots = 1,
                4 => change.push(item(29, 1, Some(2), 9)),
                5 => change[0].rules.stack_size = 0,
                6 => change[0].stack_count = Some(0),
                _ => unreachable!(),
            }
            inventory.apply(InventoryUpdate::Snapshot(change));
        }
        // Feed a structurally plausible projection even where the real planner
        // already rejects the malformed source; the property guard still fails.
        let mut changed = inventory.items().clone();
        let mut cursor = changed[&InventorySlot(22)].clone();
        cursor.slot = InventorySlot::CURSOR;
        cursor.stack_count = Some(1);
        changed.get_mut(&InventorySlot(22)).unwrap().stack_count = Some(3);
        changed.insert(InventorySlot::CURSOR, cursor);
        inventory.apply(InventoryUpdate::Prediction(changed.into_values().collect()));
        if kind == 4 {
            // This root has the same ID with a different definition. Extending
            // into it must fail even though the projected destination was empty.
            assert!(plan(&inventory, MoveRules::Takp, 30, 29, quantity(1)).is_ok());
            move_item(&mut inventory, MoveRules::Takp, 30, 29, quantity(1));
        }
        assert_eq!(
            inventory.carried_weight_tenths(GameDialect::EqMac, 0),
            Err(WeightUnavailable::Prediction),
            "case {kind}"
        );
    }
}

#[test]
fn every_external_inventory_update_invalidates_the_property_until_full_reset() {
    for update in [
        InventoryUpdate::Cursor(vec![item(30, 3, None, 5)]),
        InventoryUpdate::Set(vec![item(22, 1, Some(3), 7)]),
        InventoryUpdate::Remove(InventorySlot(24)),
        InventoryUpdate::Consume {
            slot: InventorySlot(22),
            charge: false,
        },
        InventoryUpdate::Deduct {
            slot: InventorySlot(22),
            quantity: 1,
        },
        InventoryUpdate::Used(InventorySlot(22)),
        InventoryUpdate::Invalidated,
    ] {
        let mut inventory = state(vec![item(22, 1, Some(4), 7), item(24, 2, None, 11)]);
        move_item(&mut inventory, MoveRules::Takp, 22, 30, quantity(1));
        assert_eq!(weight(&inventory, GameDialect::EqMac), 25);
        inventory.apply(update);
        assert!(inventory
            .carried_weight_tenths(GameDialect::EqMac, 0)
            .is_err());
        let fresh = inventory.items().values().cloned().collect();
        inventory.apply(InventoryUpdate::Snapshot(fresh));
        assert!(!inventory.predicted());
        assert!(!inventory.carried_weight_is_conservative());
        assert!(inventory
            .carried_weight_tenths(GameDialect::EqMac, 0)
            .is_ok());
    }
}
