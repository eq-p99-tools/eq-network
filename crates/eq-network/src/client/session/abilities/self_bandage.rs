use super::{bind_wound, strings, BindWoundUpdate, Refusal, World};
use eq_network_game::{
    inventory::{Inventory, InventorySlot, InventoryUpdate},
    message::Message,
    world::{PostureState, WorldEvent},
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const RESULT_WINDOW: Duration = Duration::from_secs(12);

#[derive(Default)]
pub(super) struct SelfBandage(Option<Attempt>);

struct Attempt {
    actor: u16,
    stock: BTreeMap<InventorySlot, u32>,
    updated: bool,
    ended: bool,
    expires: Instant,
    notified: bool,
}

fn stock(inventory: &Inventory) -> BTreeMap<InventorySlot, u32> {
    inventory
        .items()
        .iter()
        .filter(|(slot, item)| {
            (slot.is_equipment() || slot.is_carried())
                && inventory.custody_known(**slot)
                && item.rules.item_type == bind_wound::BANDAGE
                && item.stack_count != Some(0)
        })
        .map(|(slot, item)| (*slot, item.stack_count.unwrap_or(1)))
        .collect()
}

fn current(inventory: &Inventory) -> bool {
    inventory.received()
        && !inventory.stale()
        && !inventory.predicted()
        && !inventory.ground_drop_pending()
}

impl SelfBandage {
    pub(super) fn check(
        &self,
        world: &World,
        actor: u16,
    ) -> Result<BTreeMap<InventorySlot, u32>, Refusal> {
        if !world.ready() || !world.is_player(actor) || world.lifecycle.blocks_motion() {
            return Err("The bandager is unavailable".into());
        }
        if self.0.is_some() {
            return Err("Wait for the previous bandage result and current stock".into());
        }
        if *world.target != Some(actor) {
            return Err("Target yourself to bind your wounds".into());
        }
        if world.body.stop_pending() == Some(true)
            || !matches!(
                world.posture.current(),
                Some(PostureState::Standing | PostureState::Sitting)
            )
        {
            return Err("Stay still while standing or sitting to bind your wounds".into());
        }
        if !current(&world.inventory) {
            return Err("Bandage stock is not current".into());
        }
        let stock = stock(&world.inventory);
        if stock.is_empty() {
            return Err(Refusal::official(
                "You have no bandage",
                strings::NO_BANDAGES,
                Vec::new(),
            ));
        }
        Ok(stock)
    }

    pub(super) fn submitted(
        &mut self,
        actor: u16,
        stock: BTreeMap<InventorySlot, u32>,
        now: Instant,
    ) {
        self.0 = Some(Attempt {
            actor,
            stock,
            updated: false,
            ended: false,
            expires: now + RESULT_WINDOW,
            notified: false,
        });
    }

    pub(super) fn holds_inventory(&self) -> bool {
        self.0.as_ref().is_some_and(|attempt| !attempt.updated)
    }

    pub(super) fn timed_out(&mut self, now: Instant) -> bool {
        let Some(attempt) = self
            .0
            .as_mut()
            .filter(|attempt| now >= attempt.expires && !attempt.notified)
        else {
            return false;
        };
        attempt.notified = true;
        true
    }

    pub(super) fn observe(&mut self, message: &Message, world: &World) -> bool {
        let Some(attempt) = self
            .0
            .as_mut()
            .filter(|attempt| world.is_player(attempt.actor))
        else {
            return false;
        };
        let was_current = attempt.updated;
        match message {
            Message::Event(WorldEvent::BindWound(
                BindWoundUpdate::Ended(_) | BindWoundUpdate::Released,
            )) => attempt.ended = true,
            Message::Event(WorldEvent::Inventory(update))
                if world.inventory.received()
                    && !world.inventory.stale()
                    && !world.inventory.ground_drop_pending() =>
            {
                attempt.updated |= match update {
                    InventoryUpdate::Snapshot(_) => true,
                    InventoryUpdate::Set(items) => items.first().is_some_and(|root| {
                        stock_decreased(&world.inventory, &attempt.stock, root.slot)
                    }),
                    InventoryUpdate::Consume { slot, .. } | InventoryUpdate::Remove(slot) => {
                        stock_decreased(&world.inventory, &attempt.stock, *slot)
                    }
                    _ => false,
                };
            }
            _ => (),
        }
        let received_stock = !was_current && attempt.updated;
        if attempt.ended && attempt.updated {
            self.0 = None;
        }
        received_stock
    }
}

fn stock_decreased(
    inventory: &Inventory,
    stock: &BTreeMap<InventorySlot, u32>,
    root: InventorySlot,
) -> bool {
    stock.iter().any(|(&slot, &before)| {
        (slot == root || slot.parent().is_some_and(|(parent, _)| parent == root))
            && inventory.custody_known(slot)
            && inventory.items().get(&slot).map_or(0, |item| {
                if item.rules.item_type == bind_wound::BANDAGE {
                    item.stack_count.unwrap_or(1)
                } else {
                    0
                }
            }) < before
    })
}
