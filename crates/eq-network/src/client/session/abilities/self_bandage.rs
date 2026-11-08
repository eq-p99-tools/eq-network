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
    expires: Instant,
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
        if !current(&world.inventory) || world.inventory.bandage_placement_unconfirmed() {
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
            expires: now + RESULT_WINDOW,
        });
    }

    pub(super) fn holds_inventory(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(|attempt| !attempt.stock.is_empty())
    }

    pub(super) fn expired(&self, now: Instant) -> bool {
        self.0
            .as_ref()
            .is_some_and(|attempt| now >= attempt.expires)
    }

    /// Release the operation. The inventory replicas retain only the slots
    /// that could still have supplied stock.
    pub(super) fn retire(&mut self) -> Option<InventoryUpdate> {
        let attempt = self.0.take()?;
        (!attempt.stock.is_empty())
            .then(|| InventoryUpdate::BandageStockUncertain(attempt.stock.into_keys().collect()))
    }

    pub(super) fn observe(&mut self, message: &Message, world: &World) -> Option<InventoryUpdate> {
        let attempt = self
            .0
            .as_mut()
            .filter(|attempt| world.is_player(attempt.actor))?;
        if matches!(
            message,
            Message::Event(WorldEvent::BindWound(
                BindWoundUpdate::Ended(_) | BindWoundUpdate::Released
            ))
        ) {
            return self.retire();
        }
        let inventory = &world.inventory;
        if attempt.stock.is_empty()
            || !inventory.received()
            || inventory.stale()
            || inventory.ground_drop_pending()
        {
            return None;
        }
        let Message::Event(WorldEvent::Inventory(update)) = message else {
            return None;
        };
        if matches!(update, InventoryUpdate::Snapshot(_)) {
            // Applying the snapshot already cleared the inventory flag.
            attempt.stock.clear();
            return None;
        }
        let root = match update {
            InventoryUpdate::Set(items) => items.first()?.slot,
            InventoryUpdate::Consume {
                slot,
                charge: false,
            }
            | InventoryUpdate::Remove(slot) => *slot,
            _ => return None,
        };
        attempt.stock.retain(|&slot, before| {
            (slot != root && slot.parent().is_none_or(|(parent, _)| parent != root))
                || !inventory.custody_known(slot)
                || inventory
                    .items()
                    .get(&slot)
                    .filter(|item| item.rules.item_type == bind_wound::BANDAGE)
                    .map_or(0, |item| item.stack_count.unwrap_or(1))
                    >= *before
        });
        attempt
            .stock
            .is_empty()
            .then_some(InventoryUpdate::BandageStockPending(false))
    }
}
