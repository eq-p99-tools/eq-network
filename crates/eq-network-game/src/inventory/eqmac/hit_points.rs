//! The HP TAKP counts for the player's items (`itembonuses.HP`), which its
//! own HP update leaves out of the current HP (`zone/mob.cpp`
//! `Mob::SendHPUpdate`: `GetHP() - itembonuses.HP`), so a host adds it back
//! and a bleed-out check counts it. One count for both.
//!
//! Rules from TAKP (`EQMacEmu/Server` 047d0f8) `zone/bonuses.cpp`: the worn
//! items, from the first ear slot up to but not including ammo
//! (`Client::CalcItemBonuses`), then the first food and the first drink
//! carried (`Client::CalcEdibleBonuses`), each through
//! `Client::AddItemBonuses`.
//!
//! What `EQMac`'s item records cannot tell, so that the count may differ
//! (inferred, not checked live):
//! - a record carries one effect (`MacItem`, `common/patches/mac.cpp`), so a
//!   worn effect on an item that also clicks or casts a scroll is not sent,
//!   and an item with both a proc and a worn effect shows its proc as the
//!   worn one;
//! - an effect's level is its Level, or its Level2 when that is zero, where
//!   TAKP casts a worn effect without a Level at the player's level.
use super::super::{Inventory, InventoryItem, InventorySlot};

/// `ItemTypeFood` and `ItemTypeDrink` (`common/item_data.h`).
const FOOD: u8 = 14;
const DRINK: u8 = 15;

/// The player as TAKP's item bonuses see them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Wearer {
    /// Current level.
    pub level: u8,
    /// Base race, as the profile gives it.
    pub race: u32,
    /// Class.
    pub class: u32,
}

/// What TAKP counts for the player's items, and what the count could not
/// settle; a host may take a count with unsettled items as approximate, and
/// a check that must not guess, as none.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ItemHitPointCount {
    /// HP the items add.
    pub hit_points: i64,
    /// What may make the count differ from TAKP's.
    pub unsettled: Vec<UnsettledItem>,
}

/// Something [`takp_item_hit_points`] could not settle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsettledItem {
    /// The inventory has not arrived, cannot be trusted, or has moves in
    /// flight; nothing is counted.
    Inventory,
    /// A worn effect whose spell the caller had no data for; its HP is not
    /// counted.
    WornEffect {
        /// Where the item is.
        slot: InventorySlot,
        /// The effect's spell.
        spell_id: u32,
    },
    /// An item whose required level is above the player's: TAKP counts it
    /// only for an account of status 80 or more, which a client cannot tell.
    /// The profile's GM flag does not settle it, as TAKP keeps that for
    /// status 40 or more (`zone/client_packet.cpp`, `minStatusToBeGM` in
    /// `common/features.h`). It is not counted.
    RequiredLevel {
        /// Where the item is.
        slot: InventorySlot,
    },
}

/// The HP TAKP counts for the player's items: what the worn items (ear to
/// waist) add, and the first food and the first drink found in the pack
/// slots in order, a bag's contents in its place. Each counts when it is a
/// common item the player's race and class can equip, or food or drink, and
/// the player is at least the level it requires. It adds its HP in full
/// from its recommended level and scaled below it
/// (`Client::CalcRecommendedLevelBonus`), none for drink, and what its worn
/// effect adds, which `worn_effect` says
/// from the spell and the level it is cast at, or None without the spell's
/// data.
#[must_use]
pub fn takp_item_hit_points(
    inventory: &Inventory,
    wearer: Wearer,
    worn_effect: impl Fn(u32, u8) -> Option<i64>,
) -> ItemHitPointCount {
    let mut count = ItemHitPointCount::default();
    if !inventory.received() || inventory.stale() || inventory.predicted() {
        count.unsettled.push(UnsettledItem::Inventory);
        return count;
    }
    let items = inventory.items();
    for item in items
        .range(InventorySlot(1)..=InventorySlot(20))
        .map(|(_, item)| item)
    {
        count.add(item, wearer, &worn_effect);
    }
    let (mut food, mut drink) = (false, false);
    for pack in (22..=29).map(InventorySlot) {
        if food && drink {
            break;
        }
        let Some(item) = items.get(&pack) else {
            continue;
        };
        let carried: Vec<&InventoryItem> = if item.bag_slots > 0 {
            (0..10)
                .filter_map(|index| items.get(&pack.child(index)?))
                .collect()
        } else {
            vec![item]
        };
        for item in carried {
            match item.rules.item_type {
                FOOD if !food => food = true,
                DRINK if !drink => drink = true,
                _ => continue,
            }
            count.add(item, wearer, &worn_effect);
        }
    }
    count
}

impl ItemHitPointCount {
    /// What one item adds, as `Client::AddItemBonuses` counts it.
    fn add(
        &mut self,
        item: &InventoryItem,
        wearer: Wearer,
        worn_effect: &impl Fn(u32, u8) -> Option<i64>,
    ) {
        let edible = matches!(item.rules.item_type, FOOD | DRINK);
        if !edible && !equipable(item, wearer) {
            return;
        }
        let hit_points = match item.details.bonuses {
            Some(bonuses) if item.rules.item_type != DRINK => bonuses.hit_points,
            _ => 0,
        };
        let rules = item.details.equipment.unwrap_or_default();
        // Bags and books add nothing either way.
        if hit_points == 0 && rules.worn.is_none() {
            return;
        }
        if u32::from(wearer.level) < rules.required_level {
            self.unsettled
                .push(UnsettledItem::RequiredLevel { slot: item.slot });
            return;
        }
        self.hit_points += recommended(hit_points, wearer.level, rules.recommended_level);
        if let Some(worn) = rules.worn {
            let level = u8::try_from(worn.level)
                .ok()
                .filter(|level| *level > 0)
                .unwrap_or(wearer.level);
            match worn_effect(worn.spell_id, level) {
                Some(hit_points) => self.hit_points += hit_points,
                None => self.unsettled.push(UnsettledItem::WornEffect {
                    slot: item.slot,
                    spell_id: worn.spell_id,
                }),
            }
        }
    }
}

/// TAKP's `ItemData::IsEquipable` (`common/item_data.cpp`): an item for some
/// slot, the player's race (`GetPlayerRaceBit`, `common/races.cpp`) and
/// class (`GetPlayerClassBit`, warrior to beastlord).
fn equipable(item: &InventoryItem, wearer: Wearer) -> bool {
    let race = match wearer.race {
        race @ 1..=12 => 1 << (race - 1),
        128 => 1 << 12,
        130 => 1 << 13,
        _ => 0,
    };
    let class = match wearer.class {
        class @ 1..=15 => 1 << (class - 1),
        _ => 0,
    };
    item.details.slots != 0 && item.details.races & race != 0 && item.details.classes & class != 0
}

/// `Client::CalcRecommendedLevelBonus`: below the recommended level, the
/// value times the level over it, that ratio cut to ten-thousandths first,
/// then rounded half away from zero; in full from it, or without one.
fn recommended(value: i32, level: u8, recommended: u32) -> i64 {
    let (level, recommended) = (i64::from(level), i64::from(recommended));
    if recommended == 0 || level >= recommended {
        return i64::from(value);
    }
    let scaled = level * 10_000 / recommended * i64::from(value);
    if scaled < 0 {
        (scaled - 5000) / 10_000
    } else {
        (scaled + 5000) / 10_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        inventory::{InventoryUpdate, ItemActivation, ItemPlacement},
        items::{EquipmentRules, ItemBonuses, ItemDetails, WornEffect},
    };

    const WARRIOR_HUMAN: Wearer = Wearer {
        level: 20,
        race: 1,
        class: 1,
    };

    /// A made-up common item in `slot` that any race and class can wear
    /// anywhere, adding `hit_points`.
    fn item(slot: i32, hit_points: i32) -> InventoryItem {
        InventoryItem {
            activation: ItemActivation::default(),
            scroll_spell: None,
            book: None,
            rules: ItemPlacement {
                stack_size: 1,
                item_type: 10,
                ..ItemPlacement::default()
            },
            slot: InventorySlot(slot),
            details: ItemDetails {
                equipment: Some(EquipmentRules::default()),
                bonuses: Some(ItemBonuses {
                    hit_points,
                    ..ItemBonuses::default()
                }),
                id: 1000,
                name: "Synthetic item".into(),
                lore: String::new(),
                weight_tenths: 0,
                slots: 0x003f_fffe,
                classes: 0x7fff,
                races: 0x3fff,
                flags: Vec::new(),
                stats: Vec::new(),
                price: Some(0),
                icon: Some(0),
            },
            stack_count: None,
            charges: 0,
            bag_slots: 0,
        }
    }

    fn held(items: Vec<InventoryItem>) -> Inventory {
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(items));
        inventory
    }

    fn count(items: Vec<InventoryItem>, wearer: Wearer) -> ItemHitPointCount {
        takp_item_hit_points(&held(items), wearer, |_, _| None)
    }

    #[test]
    fn worn_items_count_from_ear_to_waist() {
        let items = vec![item(1, 10), item(20, 20), item(21, 400), item(22, 800)];
        assert_eq!(count(items, WARRIOR_HUMAN).hit_points, 30);
        // Nothing to count without an inventory to trust.
        let unknown = takp_item_hit_points(&Inventory::default(), WARRIOR_HUMAN, |_, _| None);
        assert_eq!(unknown.unsettled, [UnsettledItem::Inventory]);
        let mut stale = held(vec![item(1, 10)]);
        stale.apply(InventoryUpdate::Invalidated);
        assert_eq!(
            takp_item_hit_points(&stale, WARRIOR_HUMAN, |_, _| None),
            ItemHitPointCount {
                hit_points: 0,
                unsettled: vec![UnsettledItem::Inventory],
            }
        );
    }

    #[test]
    fn only_what_the_player_can_equip_counts_unless_it_is_eaten() {
        let mut gnomish = item(2, 10);
        gnomish.details.races = 1 << 11;
        let mut clerical = item(3, 20);
        clerical.details.classes = 1 << 1;
        let mut nowhere = item(4, 40);
        nowhere.details.slots = 0;
        let mut ration = item(5, 80);
        ration.details.classes = 0;
        ration.rules.item_type = FOOD;
        let items = vec![gnomish, clerical, nowhere, ration];
        assert_eq!(count(items.clone(), WARRIOR_HUMAN).hit_points, 80);
        let gnome_cleric = Wearer {
            race: 12,
            class: 2,
            ..WARRIOR_HUMAN
        };
        assert_eq!(count(items.clone(), gnome_cleric).hit_points, 110);
        // Iksar and Vah Shir have bits of their own; other races none.
        let mut iksar = item(1, 5);
        iksar.details.races = 1 << 12;
        let mut vah_shir = item(2, 7);
        vah_shir.details.races = 1 << 13;
        for (race, expected) in [(128, 5), (130, 7), (330, 0)] {
            let wearer = Wearer {
                race,
                ..WARRIOR_HUMAN
            };
            assert_eq!(
                count(vec![iksar.clone(), vah_shir.clone()], wearer).hit_points,
                expected,
                "race {race}"
            );
        }
    }

    #[test]
    fn levels_scale_below_the_recommended_and_unsettle_below_the_required() {
        let mut recommended_30 = item(1, 101);
        recommended_30.details.equipment = Some(EquipmentRules {
            recommended_level: 30,
            ..EquipmentRules::default()
        });
        // 20 / 30 is 6666 ten-thousandths, times 101 is 67.3266: 67.
        assert_eq!(
            count(vec![recommended_30.clone()], WARRIOR_HUMAN).hit_points,
            67
        );
        // Halves round away from zero, penalties too: 5000 x 3 and x -3.
        let half = Wearer {
            level: 15,
            ..WARRIOR_HUMAN
        };
        for (hit_points, expected) in [(3, 2), (-3, -2)] {
            recommended_30.details.bonuses = Some(ItemBonuses {
                hit_points,
                ..ItemBonuses::default()
            });
            assert_eq!(
                count(vec![recommended_30.clone()], half).hit_points,
                expected
            );
        }
        let mut required_30 = item(2, 50);
        required_30.details.equipment = Some(EquipmentRules {
            required_level: 30,
            ..EquipmentRules::default()
        });
        assert_eq!(
            count(vec![required_30.clone(), item(3, 4)], WARRIOR_HUMAN),
            ItemHitPointCount {
                hit_points: 4,
                unsettled: vec![UnsettledItem::RequiredLevel {
                    slot: InventorySlot(2)
                }],
            }
        );
        // At the required level it counts; one that adds nothing never matters.
        let thirty = Wearer {
            level: 30,
            ..WARRIOR_HUMAN
        };
        assert_eq!(count(vec![required_30.clone()], thirty).hit_points, 50);
        required_30.details.bonuses = Some(ItemBonuses::default());
        assert_eq!(
            count(vec![required_30], WARRIOR_HUMAN),
            ItemHitPointCount::default()
        );
    }

    #[test]
    fn worn_effects_count_what_the_spell_data_says() {
        let mut belt = item(20, 10);
        belt.details.equipment = Some(EquipmentRules {
            worn: Some(WornEffect {
                spell_id: 123,
                effect_type: 2,
                level: 0,
                level2: 0,
            }),
            ..EquipmentRules::default()
        });
        let mut cloak = belt.clone();
        cloak.slot = InventorySlot(8);
        if let Some(rules) = cloak.details.equipment.as_mut() {
            rules.worn = Some(WornEffect {
                spell_id: 456,
                effect_type: 2,
                level: 40,
                level2: 40,
            });
        }
        let inventory = held(vec![belt, cloak]);
        // A worn effect without a level is cast at the player's.
        let known = takp_item_hit_points(&inventory, WARRIOR_HUMAN, |spell, level| {
            Some(i64::from(spell) + i64::from(level) * 1000)
        });
        assert_eq!(known.hit_points, 10 + 123 + 20_000 + 10 + 456 + 40_000);
        assert_eq!(known.unsettled, Vec::<UnsettledItem>::new());
        let unknown = takp_item_hit_points(&inventory, WARRIOR_HUMAN, |spell, _| {
            (spell == 123).then_some(5)
        });
        assert_eq!(
            unknown,
            ItemHitPointCount {
                hit_points: 25,
                unsettled: vec![UnsettledItem::WornEffect {
                    slot: InventorySlot(8),
                    spell_id: 456
                }],
            }
        );
    }

    #[test]
    fn the_first_food_and_drink_carried_count_and_drink_adds_no_hp() {
        let edible = |slot: i32, item_type: u8, hit_points: i32| {
            let mut food = item(slot, hit_points);
            food.rules.item_type = item_type;
            food
        };
        let mut bag = item(23, 0);
        bag.bag_slots = 4;
        bag.details.bonuses = Some(ItemBonuses::default());
        let in_bag = |index: u8, item_type: u8, hit_points: i32| {
            edible(
                InventorySlot(23).child(index).unwrap().0,
                item_type,
                hit_points,
            )
        };
        // Pack 22 holds plain gear; the bag in pack 23 holds a drink, then
        // two foods; pack 24 holds more food. The bag's first food counts.
        let mut drink_with_effect = in_bag(0, DRINK, 300);
        drink_with_effect.details.equipment = Some(EquipmentRules {
            worn: Some(WornEffect {
                spell_id: 9,
                effect_type: 2,
                level: 1,
                level2: 1,
            }),
            ..EquipmentRules::default()
        });
        let items = vec![
            item(22, 1000),
            bag,
            drink_with_effect,
            in_bag(1, FOOD, 2),
            in_bag(2, FOOD, 4),
            edible(24, FOOD, 8),
            edible(25, DRINK, 16),
        ];
        let carried = takp_item_hit_points(&held(items), WARRIOR_HUMAN, |_, _| Some(32));
        // The drink adds its effect's 32, not its 300; the first food its 2.
        assert_eq!(carried.hit_points, 32 + 2);
        assert_eq!(carried.unsettled, Vec::<UnsettledItem>::new());
    }

    #[test]
    fn scaling_matches_takps_integer_steps() {
        assert_eq!(recommended(100, 10, 0), 100);
        assert_eq!(recommended(100, 30, 30), 100);
        assert_eq!(recommended(100, 1, 3), 33);
        assert_eq!(recommended(7, 1, 2), 4);
        assert_eq!(recommended(-7, 1, 2), -4);
        assert_eq!(recommended(32_767, 64, 65), 32_262);
    }
}
