//! Eating and drinking: on the session's own when the player turns hungry or
//! thirsty, as the official client does, and by hand. Servers report no item
//! eaten, so the bite comes out of the inventory here.
//!
//! `EQEmu` answers every bite with one stamina report
//! (`Client::Handle_OP_Consume`), besides the report every 46 seconds. A
//! report sent before the server took every bite in flight counts too
//! little; eating on it would eat twice, so the session waits for the
//! report after the answers.
use super::{change, Out, World};
use anyhow::Result;
use eq_network_game::{
    food::{self, AutoEat, Meal, Nourishment, Shortage},
    inventory::{Inventory, InventorySlot, InventoryUpdate},
    request::Request,
    world::WorldEvent,
};

/// How fed and watered the player is, as the server last said, and what the
/// session may eat and drink for them.
pub(super) struct Meals {
    last: Option<Nourishment>,
    auto_eat: AutoEat,
    /// Bites sent whose answers have not come.
    bites: u32,
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
            bites: 0,
        }
    }

    /// What the session may eat and drink on its own from now on.
    pub(super) const fn choose(&mut self, auto_eat: AutoEat) {
        self.auto_eat = auto_eat;
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
        if self.bites > 0 {
            self.bites -= 1;
            return Ok(());
        }
        let (mut food, mut water) = (None, None);
        for meal in [Meal::Food, Meal::Drink] {
            if nourishment.of(meal) > food::HUNGRY {
                continue;
            }
            if let Some(slot) = first(&world.inventory, meal, self.auto_eat) {
                self.eat(slot, meal, false, world, out)?;
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
        &mut self,
        slot: InventorySlot,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<Option<(&'static str, Option<u32>)>> {
        let Some(meal) = world
            .inventory
            .items()
            .get(&slot)
            .and_then(|item| Meal::of_item_type(item.rules.item_type))
        else {
            return Ok(Some(("You cannot eat or drink that", None)));
        };
        // Servers turn down a bite past full; the official client says so
        // first, in its own words (eqstr 12074 and 12077).
        if self.last.is_some_and(|last| last.of(meal) >= food::FULL) {
            return Ok(Some(match meal {
                Meal::Food => ("You are too full to eat more", Some(12074)),
                Meal::Drink => ("You are too full to drink more", Some(12077)),
            }));
        }
        self.eat(slot, meal, true, world, out)?;
        Ok(None)
    }

    /// Sends the bite and takes it from the inventory, as the server does
    /// without saying so.
    fn eat(
        &mut self,
        slot: InventorySlot,
        meal: Meal,
        by_hand: bool,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        out.request(&Request::Consume {
            slot,
            meal,
            by_hand,
        })?;
        self.bites += 1;
        change(InventoryUpdate::Deduct { slot, quantity: 1 }, world, out)
    }
}
