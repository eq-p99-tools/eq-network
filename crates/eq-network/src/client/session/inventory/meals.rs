//! Eating and drinking: on the session's own when the player turns hungry or
//! thirsty, as the official client does, and by hand. Servers report no item
//! eaten, so the bite comes out of the inventory here.
use super::{change, Out, World};
use anyhow::Result;
use eq_network_game::{
    food::{self, Meal, Nourishment},
    inventory::{Inventory, InventorySlot, InventoryUpdate},
    world::WorldEvent,
};

/// How fed and watered the player is, as the server last said.
#[derive(Default)]
pub(super) struct Meals {
    last: Option<Nourishment>,
}

/// The first food or drink the player carries, as the official client looks
/// for it: each general slot in turn, then what the bag in it holds.
fn first(inventory: &Inventory, meal: Meal) -> Option<InventorySlot> {
    let items = inventory.items();
    let is_meal = |slot: &InventorySlot| {
        items
            .get(slot)
            .is_some_and(|item| Meal::of_item_type(item.rules.item_type) == Some(meal))
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
    /// Notes how fed and watered the profile says the player is.
    pub(super) fn admit(&mut self, nourishment: Nourishment) {
        self.last = Some(nourishment);
    }

    /// Follows the server's word, eating and drinking for a hungry or thirsty
    /// player from what they carry, and telling the host when there is
    /// nothing to eat or drink.
    pub(super) fn nourished(
        &mut self,
        nourishment: Nourishment,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.last = Some(nourishment);
        let mut lacking = (false, false);
        for meal in [Meal::Food, Meal::Drink] {
            if nourishment.of(meal) > food::HUNGRY {
                continue;
            }
            match first(&world.inventory, meal) {
                Some(slot) => eat(slot, meal, false, world, out)?,
                None if meal == Meal::Food => lacking.0 = true,
                None => lacking.1 = true,
            }
        }
        if lacking.0 || lacking.1 {
            out.log
                .send(super::ClientEvent::World(WorldEvent::NothingToEat {
                    food: lacking.0,
                    water: lacking.1,
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
