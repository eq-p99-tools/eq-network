//! Validated carried, equipment, personal-bank and trade moves. No destroy sentinel is encoded.
use super::{Inventory, InventoryItem, InventorySlot, InventoryUpdate};
use crate::command::EncodedCommand;
use anyhow::{ensure, Context, Result};
use std::{num::NonZeroU32, time::Instant};

/// Requested quantity; whole items use Titanium's zero-count move/swap form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoveQuantity {
    /// Move the entire item, stack, or container.
    Whole,
    /// Transfer this many units into an empty slot or matching stack.
    Count(NonZeroU32),
}
/// One UI intent bound to the admission and inventory revision it was selected from.
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryMove {
    /// Zone-admission identifier.
    pub session_id: u64,
    /// Inventory revision observed when selecting the source.
    pub revision: u64,
    /// Occupied carried, equipment, or accessible personal-bank slot.
    pub from: InventorySlot,
    /// Supported destination; cursor moves may swap occupied slots when access permits.
    pub to: InventorySlot,
    /// Whole item or explicit stack quantity.
    pub quantity: MoveQuantity,
    /// Local enqueue time; old requests cannot replay after a stall.
    pub created: Instant,
}
/// `OP_MoveItem`: an item moved between slots.
pub const MOVE_OPCODE: u16 = 0x420f;

/// Known character eligibility used for equipment checks; the server remains authoritative.
#[derive(Clone, Copy, Debug)]
pub struct InventoryActor {
    /// Current banker proximity, recomputed by the admitted worker before submission.
    pub bank_access: bool,
    /// Numeric player class, if decoded.
    pub class: Option<u32>,
    /// Profile deity identifier, if decoded.
    pub deity: Option<u32>,
    /// Server-provided dual-wield skill (protocol skill 22), if known.
    pub dual_wield: Option<u32>,
    /// Base race from the character profile.
    pub race: u32,
    /// Current level.
    pub level: u8,
    /// How many trade slots the open give or trade window has; zero when none
    /// is open.
    pub trade_slots: u8,
    /// Whether the open window's partner may take NO DROP items: an NPC may,
    /// another player never.
    pub trade_no_drop: bool,
}

impl Inventory {
    /// Selects the next carried destination, filling compatible stacks before empty slots.
    /// Re-evaluate after every submitted move; never cache a sequence across revisions.
    ///
    /// # Errors
    /// Rejects unavailable inventory, an empty cursor, or no compatible space.
    pub fn auto_store_destination(&self, actor: InventoryActor) -> Result<InventorySlot> {
        ensure!(
            self.received && !self.stale(),
            "Wait for a current inventory snapshot"
        );
        let source = self
            .items
            .get(&InventorySlot(30))
            .context("The cursor is empty")?;
        let mut slots: Vec<_> = (22..=29).map(InventorySlot).collect();
        for root in 22..=29 {
            let root = InventorySlot(root);
            if let Some(bag) = self.items.get(&root) {
                slots.extend((0..bag.bag_slots).filter_map(|index| root.child(index)));
            }
        }
        for &slot in &slots {
            let Some(destination) = self.items.get(&slot) else {
                continue;
            };
            if source.details.id != destination.details.id {
                continue;
            }
            let Some(available) = source.stack_count else {
                continue;
            };
            let Some(existing) = destination.stack_count else {
                continue;
            };
            let amount = available.min(destination.rules.stack_size.saturating_sub(existing));
            let Some(amount) = NonZeroU32::new(amount) else {
                continue;
            };
            let request = InventoryMove {
                session_id: 0,
                revision: self.revision,
                from: InventorySlot(30),
                to: slot,
                quantity: MoveQuantity::Count(amount),
                created: Instant::now(),
            };
            if self.plan_move(&request, actor).is_ok() {
                return Ok(slot);
            }
        }
        for slot in slots {
            if self.items.contains_key(&slot) {
                continue;
            }
            let request = InventoryMove {
                session_id: 0,
                revision: self.revision,
                from: InventorySlot(30),
                to: slot,
                quantity: MoveQuantity::Whole,
                created: Instant::now(),
            };
            if self.plan_move(&request, actor).is_ok() {
                return Ok(slot);
            }
        }
        anyhow::bail!("No compatible inventory space; item remains on the cursor")
    }

    /// Validates and sends once, committing a local prediction only after transport success.
    /// The zone session has already refused a request for an earlier admission
    /// or made too long ago, in its one freshness check.
    ///
    /// # Errors
    /// Rejects old revisions, stale inventory, invalid placement or quantity,
    /// unsupported swaps and actions. Send failures leave state unchanged.
    pub fn submit_move(
        &mut self,
        request: &InventoryMove,
        actor: InventoryActor,
        send: impl FnOnce(&EncodedCommand) -> Result<()>,
    ) -> Result<InventoryUpdate> {
        let update = self.plan_move(request, actor)?;
        let mut body = [0; 12];
        body[..4].copy_from_slice(&u32::try_from(request.from.0)?.to_le_bytes());
        body[4..8].copy_from_slice(&u32::try_from(request.to.0)?.to_le_bytes());
        let count = match request.quantity {
            MoveQuantity::Whole => 0,
            MoveQuantity::Count(n) => n.get(),
        };
        body[8..].copy_from_slice(&count.to_le_bytes());
        send(&EncodedCommand {
            opcode: MOVE_OPCODE,
            body: body.to_vec(),
        })?;
        self.apply(update.clone());
        Ok(update)
    }

    /// Computes placement without modifying inventory or sending a packet.
    ///
    /// # Errors
    /// Rejects unsupported or inconsistent moves using the latest known inventory.
    pub fn plan_move(
        &self,
        request: &InventoryMove,
        actor: InventoryActor,
    ) -> Result<InventoryUpdate> {
        ensure!(
            self.received && !self.stale(),
            "Wait for a current inventory snapshot"
        );
        ensure!(
            request.revision == self.revision,
            "Inventory changed; select the item again"
        );
        ensure!(request.from != request.to, "Choose a different destination");
        ensure!(
            movable(request.from, actor) && movable(request.to, actor),
            "Slot is unsupported or personal banking requires a nearby banker"
        );
        ensure!(
            !request.from.is_trade(),
            "Cancel the trade to take an item back"
        );
        if request.to.is_trade() {
            check_trade(request, self.items.contains_key(&request.to))?;
            // `EQEmu` disconnects a client that offers another player a NO
            // DROP item, or a bag holding one.
            ensure!(
                actor.trade_no_drop
                    || !self.items.iter().any(|(slot, item)| {
                        (*slot == request.from
                            || slot.parent().is_some_and(|(bag, _)| bag == request.from))
                            && item.details.flags.iter().any(|flag| flag == "NO DROP")
                    }),
                "NO DROP items cannot be traded"
            );
        }
        let source = self
            .items
            .get(&request.from)
            .context("Source slot is empty")?;
        if let Some(destination) = self.items.get(&request.to) {
            if let MoveQuantity::Count(amount) = request.quantity {
                return self.plan_merge(request, source, destination, amount.get(), actor);
            }
            return self.plan_cursor_swap(request, source, destination, actor);
        }
        if let Some((parent, index)) = request.from.parent() {
            ensure!(
                self.items
                    .get(&parent)
                    .is_some_and(|bag| index < bag.bag_slots),
                "Source container is not known"
            );
        }
        ensure!(
            !self.items.keys().any(|slot| slot
                .parent()
                .is_some_and(|(parent, _)| parent == request.to)),
            "Destination has inconsistent container contents"
        );
        self.check_slot(request.to, source, actor)?;
        let mut result = self.items.clone();
        let count = match request.quantity {
            MoveQuantity::Whole => None,
            MoveQuantity::Count(n) => {
                let available = source.stack_count.context("This item is not a stack")?;
                ensure!(n.get() <= available, "Not enough items in the stack");
                ensure!(source.bag_slots == 0, "Containers cannot be split");
                Some((n.get(), available))
            }
        };
        let mut moved = source.clone();
        moved.slot = request.to;
        match count {
            Some((amount, available)) if amount < available => {
                moved.stack_count = Some(amount);
                result
                    .get_mut(&request.from)
                    .context("Missing source")?
                    .stack_count = Some(available - amount);
            }
            _ => {
                result.remove(&request.from);
            }
        }
        result.insert(request.to, moved);
        for (slot, child) in &self.items {
            if let Some((parent, index)) = slot.parent() {
                if parent == request.from {
                    let target = request
                        .to
                        .child(index)
                        .context("Container contents cannot fit here")?;
                    let mut child = child.clone();
                    child.slot = target;
                    result.remove(slot);
                    result.insert(target, child);
                }
            }
        }
        Ok(InventoryUpdate::Prediction(result.into_values().collect()))
    }

    fn plan_merge(
        &self,
        request: &InventoryMove,
        source: &InventoryItem,
        destination: &InventoryItem,
        amount: u32,
        actor: InventoryActor,
    ) -> Result<InventoryUpdate> {
        ensure!(
            request.from == InventorySlot(30),
            "Pick the stack up before merging"
        );
        ensure!(
            source.details.id == destination.details.id,
            "Stacks contain different items"
        );
        ensure!(
            source.bag_slots == 0 && destination.bag_slots == 0,
            "Containers cannot stack"
        );
        let available = source.stack_count.context("Source is not stackable")?;
        let existing = destination
            .stack_count
            .context("Destination is not stackable")?;
        let capacity = destination.rules.stack_size;
        ensure!(
            capacity > 0 && capacity == source.rules.stack_size,
            "Stack capacity is unknown or inconsistent"
        );
        ensure!(
            available <= capacity && existing <= capacity,
            "Invalid stack quantity"
        );
        ensure!(
            amount <= available && amount <= capacity - existing,
            "Not enough items or stack space"
        );
        self.check_slot(request.to, source, actor)?;
        let mut result = self.items.clone();
        if amount == available {
            result.remove(&request.from);
        } else {
            result
                .get_mut(&request.from)
                .context("Missing source")?
                .stack_count = Some(available - amount);
        }
        result
            .get_mut(&request.to)
            .context("Missing destination")?
            .stack_count = Some(existing + amount);
        Ok(InventoryUpdate::Prediction(result.into_values().collect()))
    }

    fn plan_cursor_swap(
        &self,
        request: &InventoryMove,
        source: &InventoryItem,
        destination: &InventoryItem,
        actor: InventoryActor,
    ) -> Result<InventoryUpdate> {
        ensure!(
            request.from == InventorySlot(30),
            "Pick the item up before swapping"
        );
        ensure!(
            request.quantity == MoveQuantity::Whole,
            "Partial-stack swaps are not supported"
        );
        ensure!(
            source.details.id != destination.details.id || source.stack_count.is_none(),
            "Matching stacks require a merge operation"
        );
        self.check_slot(request.to, source, actor)?;
        self.check_slot(request.from, destination, actor)?;
        let mut result = self.items.clone();
        // Remove both subtrees before remapping: inserting while removing could
        // overwrite a bag child whose slot belongs to the other subtree.
        for slot in self.items.keys() {
            if *slot == request.from
                || *slot == request.to
                || slot
                    .parent()
                    .is_some_and(|(parent, _)| parent == request.from || parent == request.to)
            {
                result.remove(slot);
            }
        }
        for item in self.items.values() {
            let target = if item.slot == request.from {
                Some(request.to)
            } else if item.slot == request.to {
                Some(request.from)
            } else if let Some((parent, index)) = item.slot.parent() {
                if parent == request.from {
                    Some(
                        request
                            .to
                            .child(index)
                            .context("Destination cannot hold bag contents")?,
                    )
                } else if parent == request.to {
                    Some(
                        request
                            .from
                            .child(index)
                            .context("Cursor cannot hold bag contents")?,
                    )
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(target) = target {
                let mut moved = item.clone();
                moved.slot = target;
                ensure!(
                    result.insert(target, moved).is_none(),
                    "Conflicting container contents"
                );
            }
        }
        Ok(InventoryUpdate::Prediction(result.into_values().collect()))
    }

    fn check_slot(
        &self,
        slot: InventorySlot,
        item: &InventoryItem,
        actor: InventoryActor,
    ) -> Result<()> {
        if let Some((parent, index)) = slot.parent() {
            let bag = self
                .items
                .get(&parent)
                .context("Container has not been received")?;
            ensure!(index < bag.bag_slots, "Outside this bag's capacity");
            ensure!(item.bag_slots == 0, "Bags cannot be placed inside bags");
            ensure!(
                item.rules.size <= bag.rules.bag_size,
                "Item is too large for this bag"
            );
            ensure!(
                bag.rules.bag_type != 2 || item.rules.item_type == 27,
                "Only arrows fit in a quiver"
            );
            ensure!(
                bag.rules.bag_type != 8,
                "This specialized container is not supported yet"
            );
        }
        if slot.is_equipment() {
            let bit = u32::try_from(slot.0)?;
            ensure!(
                item.details.slots & (1 << bit) != 0,
                "Item cannot be equipped in this slot"
            );
            let class = actor
                .class
                .filter(|value| (1..=16).contains(value))
                .context("Character class is not known")?;
            ensure!(
                item.details.classes & (1 << (class - 1)) != 0,
                "Your class cannot equip this item"
            );
            let race_bit = match actor.race {
                1..=12 => actor.race - 1,
                128 => 12,
                _ => anyhow::bail!("Race eligibility is not supported yet"),
            };
            ensure!(
                item.details.races & (1 << race_bit) != 0,
                "Your race cannot equip this item"
            );
            if item.rules.deity_mask != 0 {
                let mask = actor
                    .deity
                    .and_then(deity_bit)
                    .context("Character deity is not known")?;
                ensure!(
                    item.rules.deity_mask & mask != 0,
                    "Your deity cannot equip this item"
                );
            }
            ensure!(
                !item
                    .details
                    .stats
                    .iter()
                    .any(|stat| stat.label == "Required level"
                        && stat.value > i32::from(actor.level)),
                "Your level is too low for this item"
            );
            if slot.0 == 13 && matches!(item.rules.item_type, 1 | 4 | 35) {
                ensure!(
                    !self.items.contains_key(&InventorySlot(14)),
                    "Empty your secondary slot first"
                );
            }
            if slot.0 == 14 {
                ensure!(
                    !matches!(item.rules.item_type, 1 | 4 | 35),
                    "Two-handed weapons cannot be equipped in the secondary slot"
                );
                if matches!(item.rules.item_type, 0 | 2 | 3 | 45) {
                    ensure!(
                        matches!(actor.class, Some(1 | 4 | 7 | 8 | 9 | 15))
                            && actor.dual_wield.is_some_and(|skill| skill > 0),
                        "A trained dual-wield skill is required for an off-hand weapon"
                    );
                }
                ensure!(
                    !self
                        .items
                        .get(&InventorySlot(13))
                        .is_some_and(|primary| matches!(primary.rules.item_type, 1 | 4 | 35)),
                    "Unequip the two-handed weapon first"
                );
            }
        }
        Ok(())
    }
}
fn movable(slot: InventorySlot, actor: InventoryActor) -> bool {
    slot.is_equipment()
        || slot.is_carried()
        || slot == InventorySlot::CURSOR
        || (actor.bank_access && slot.is_personal_bank())
        || (slot.is_trade() && slot.0 - 3000 < i32::from(actor.trade_slots))
}

/// What servers accept into a trade slot: an item from the cursor, whole into
/// an empty slot or merged onto the same stack. `EQEmu` disconnects a client
/// that sends anything else (`Trade::AddEntity`, `Client::SwapItem`).
fn check_trade(request: &InventoryMove, occupied: bool) -> Result<()> {
    ensure!(
        request.from == InventorySlot::CURSOR,
        "Pick the item up to hand it over"
    );
    match request.quantity {
        MoveQuantity::Whole => ensure!(!occupied, "That trade slot is taken"),
        MoveQuantity::Count(_) => ensure!(occupied, "Hand over the whole stack on the cursor"),
    }
    Ok(())
}

/// EQ item masks reserve bit zero for either agnostic profile identifier.
fn deity_bit(deity: u32) -> Option<u32> {
    match deity {
        140 | 396 => Some(1),
        201..=216 => Some(1 << (deity - 200)),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
