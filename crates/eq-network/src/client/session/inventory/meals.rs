//! Eating and drinking: on the session's own when the player turns hungry or
//! thirsty, as the official client does, and by hand. Servers report no item
//! eaten, so the bite comes out of the inventory here.
use super::{change, Out, World};
use anyhow::Result;
use eq_network_game::{
    food::{self, AutoEat, Meal, Nourishment, Shortage},
    inventory::{Inventory, InventorySlot, InventoryUpdate},
    world::WorldEvent,
};

/// How fed and watered the player is, as the server last said, and what the
/// session may eat and drink for them.
pub(super) struct Meals {
    last: Option<Nourishment>,
    auto_eat: AutoEat,
}

/// The first food or drink the player carries that the choice takes, as the
/// official client looks for it: each general slot in turn, then what the
/// bag in it holds.
fn first(inventory: &Inventory, meal: Meal, auto_eat: AutoEat) -> Option<InventorySlot> {
    let items = inventory.items();
    let is_meal = |slot: &InventorySlot| {
        items.get(slot).is_some_and(|item| {
            Meal::of_item_type(item.rules.item_type) == Some(meal) && auto_eat.takes(&item.details)
        })
    };
    (22..=29).map(InventorySlot).find_map(|slot| {
        if is_meal(&slot) {
            return Some(slot);
        }
        let bag = items.get(&slot)?.bag_slots;
        (0..bag)
            .filter_map(|index| slot.child(index))
            .find(|child| is_meal(child))
    })
}

impl Meals {
    pub(super) const fn new(auto_eat: AutoEat) -> Self {
        Self {
            last: None,
            auto_eat,
        }
    }

    /// Notes how fed and watered the profile says the player is.
    pub(super) fn admit(&mut self, nourishment: Nourishment) {
        self.last = Some(nourishment);
    }

    /// Follows the server's word, eating and drinking for a hungry or thirsty
    /// player from what they carry, and telling the host when there is
    /// nothing the session may eat or drink.
    pub(super) fn nourished(
        &mut self,
        nourishment: Nourishment,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.last = Some(nourishment);
        let (mut food, mut water) = (None, None);
        for meal in [Meal::Food, Meal::Drink] {
            if nourishment.of(meal) > food::HUNGRY {
                continue;
            }
            if let Some(slot) = first(&world.inventory, meal, self.auto_eat) {
                eat(slot, meal, false, world, out)?;
                continue;
            }
            let shortage = if first(&world.inventory, meal, AutoEat::Anything).is_some() {
                Shortage::OnlyModified
            } else {
                Shortage::Nothing
            };
            match meal {
                Meal::Food => food = Some(shortage),
                Meal::Drink => water = Some(shortage),
            }
        }
        if food.is_some() || water.is_some() {
            out.log
                .send(super::ClientEvent::World(WorldEvent::NothingToEat {
                    food,
                    water,
                }))?;
        }
        Ok(())
    }

    /// Eats or drinks the item in a slot by hand, or says why not.
    pub(super) fn by_hand(
        &self,
        slot: InventorySlot,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<Option<&'static str>> {
        let Some(meal) = world
            .inventory
            .items()
            .get(&slot)
            .and_then(|item| Meal::of_item_type(item.rules.item_type))
        else {
            return Ok(Some("You cannot eat or drink that"));
        };
        // Servers turn down a bite past full; the official client says so first.
        if self.last.is_some_and(|last| last.of(meal) >= food::FULL) {
            return Ok(Some(match meal {
                Meal::Food => "You could not possibly eat any more, you would explode!",
                Meal::Drink => "You could not possibly drink any more, you would explode!",
            }));
        }
        eat(slot, meal, true, world, out)?;
        Ok(None)
    }
}

/// Sends the bite and takes it from the inventory, as the server does
/// without saying so.
fn eat(
    slot: InventorySlot,
    meal: Meal,
    by_hand: bool,
    world: &mut World,
    out: &mut Out<'_, '_>,
) -> Result<()> {
    out.send(&food::consume(slot, meal, by_hand))?;
    change(InventoryUpdate::Deduct { slot, quantity: 1 }, world, out)
}
