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
/// Local ordering identity, independent of server acknowledgments.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct MoveSequence {
    /// Strictly increasing request identity within one admission.
    pub id: u64,
    /// The preceding queued move whose local submission this move depends on.
    /// None starts a sequence from an observed worker snapshot.
    pub after: Option<u64>,
}
/// One UI intent bound to the admission and inventory revision it was selected from.
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryMove {
    /// Zone-admission identifier.
    pub session_id: u64,
    /// Inventory revision observed when selecting the source.
    pub revision: u64,
    /// Ordered local projection lineage; None for callers using worker snapshots.
    pub sequence: Option<MoveSequence>,
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

/// Titanium's `OP_MoveItem` for a move already planned; a whole item moves
/// with a count of zero.
///
/// # Errors
/// Rejects slots the packet cannot carry.
pub fn titanium_move(
    from: InventorySlot,
    to: InventorySlot,
    quantity: MoveQuantity,
) -> Result<EncodedCommand> {
    let mut body = [0; 12];
    body[..4].copy_from_slice(&u32::try_from(from.0)?.to_le_bytes());
    body[4..8].copy_from_slice(&u32::try_from(to.0)?.to_le_bytes());
    let count = match quantity {
        MoveQuantity::Whole => 0,
        MoveQuantity::Count(n) => n.get(),
    };
    body[8..].copy_from_slice(&count.to_le_bytes());
    Ok(EncodedCommand {
        opcode: MOVE_OPCODE,
        body: body.to_vec(),
    })
}

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
    /// Whether a world container is open for the player, whose ten slots
    /// items may then go into and come out of.
    pub world_container: bool,
}

/// Whose rules a move must also meet: the server type's own, beyond the
/// placement rules every server shares.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MoveRules {
    /// `EQEmu`'s, which P99 shares (`Client::SwapItem`).
    #[default]
    EqEmu,
    /// TAKP's, which keep bags, books and arrows out of more worn slots and
    /// an instrument from either hand while the other is full
    /// (`ItemInstance::IsSlotAllowed`, `common/item_instance.cpp`) and have
    /// no charm slot. TAKP answers a move it refuses at once by resending
    /// its two slots, then disconnects a player below status 10
    /// (`Client::SwapItemResync`, `zone/inventory.cpp`), and answers no move
    /// it accepts. It also hands the player an item it queued behind the
    /// cursor only once the cursor empties, saying nothing of it before
    /// (`ZoneDatabase::SaveCursor`, `zone/zonedb.cpp`). Moves remain ungated.
    Takp,
    /// Quarm's source-checked `EQMac` placement rules. Moves go out immediately
    /// and remain unconfirmed: silence is not an ACK.
    /// Direct trade/world-container moves remain outside this policy.
    Quarm,
}

impl MoveRules {
    /// Whether this policy offers explicit destruction of a cursor item.
    #[must_use]
    pub const fn supports_cursor_destruction(self) -> bool {
        matches!(self, Self::EqEmu)
    }

    /// Legacy Titanium refusal window. `EQMac` has no generic positive move
    /// acknowledgment: elapsed time never confirms its inventory predictions.
    #[must_use]
    pub const fn settles_by_silence(self) -> bool {
        match self {
            Self::EqEmu => true,
            Self::Takp | Self::Quarm => false,
        }
    }

    /// Whether the reported rank establishes training under this policy.
    fn trained_skill(self, value: Option<u32>) -> bool {
        value
            .and_then(|value| match self {
                Self::EqEmu => Some(value),
                Self::Takp | Self::Quarm => crate::abilities::eqmac::skill_rank(value),
            })
            .is_some_and(|rank| rank > 0)
    }
}

/// The worn slots, in the program's order.
const WORN: std::ops::RangeInclusive<i32> = 0..=21;
/// The general slots, in the program's order.
const GENERAL: std::ops::RangeInclusive<i32> = 22..=29;

impl Inventory {
    /// Selects the next carried destination, filling compatible stacks before empty slots.
    /// Re-evaluate after every submitted move; never cache a sequence across revisions.
    ///
    /// # Errors
    /// Rejects unavailable inventory, an empty cursor, or no compatible space.
    pub fn auto_store_destination(&self, actor: InventoryActor) -> Result<InventorySlot> {
        let source = self.cursor_source()?;
        let mut slots: Vec<_> = GENERAL.map(InventorySlot).collect();
        slots.extend(self.bag_slots());
        slots
            .iter()
            .find(|&&slot| self.stack_onto(source, slot, actor).is_some())
            .or_else(|| slots.iter().find(|&&slot| self.place_into(slot, actor)))
            .copied()
            .context("No compatible inventory space; item remains on the cursor")
    }

    /// Where the Titanium program's `/autoinventory` puts the cursor item,
    /// and how much of it (read R46 section 1). First the equipment
    /// (0x47477f): onto the same item in a worn slot, else the first empty
    /// worn slot it fits and the player can use. Then the general slots and
    /// bags (0x4bcf0b): onto a general stack; unless it is a bag, onto a
    /// stack inside a bag; the first empty general slot; unless it is a bag,
    /// the first empty bag slot. One move is planned: what doesn't fit on a
    /// stack stays on the cursor (inferred).
    ///
    /// # Errors
    /// Rejects unavailable inventory, an empty cursor, or no place the item
    /// fits.
    pub fn auto_inventory_destination(
        &self,
        actor: InventoryActor,
    ) -> Result<(InventorySlot, MoveQuantity)> {
        let source = self.cursor_source()?;
        let worn: Vec<_> = WORN.map(InventorySlot).collect();
        let general: Vec<_> = GENERAL.map(InventorySlot).collect();
        let bags: Vec<_> = if source.bag_slots == 0 {
            self.bag_slots().collect()
        } else {
            Vec::new()
        };
        let stack = |slots: &[InventorySlot]| {
            slots.iter().find_map(|&slot| {
                self.stack_onto(source, slot, actor)
                    .map(|amount| (slot, MoveQuantity::Count(amount)))
            })
        };
        let empty = |slots: &[InventorySlot]| {
            slots
                .iter()
                .find(|&&slot| self.place_into(slot, actor))
                .map(|&slot| (slot, MoveQuantity::Whole))
        };
        stack(&worn)
            .or_else(|| empty(&worn))
            .or_else(|| stack(&general))
            .or_else(|| stack(&bags))
            .or_else(|| empty(&general))
            .or_else(|| empty(&bags))
            .context("No compatible inventory space; item remains on the cursor")
    }

    /// The item on the cursor, from a current inventory.
    fn cursor_source(&self) -> Result<&InventoryItem> {
        ensure!(
            self.received && !self.stale(),
            "Wait for a current inventory snapshot"
        );
        self.items
            .get(&InventorySlot::CURSOR)
            .context("The cursor is empty")
    }

    /// The slots of the bags in the general slots, bag by bag in slot order.
    fn bag_slots(&self) -> impl Iterator<Item = InventorySlot> + '_ {
        GENERAL.map(InventorySlot).flat_map(move |root| {
            let count = self.items.get(&root).map_or(0, |bag| bag.bag_slots);
            (0..count).filter_map(move |index| root.child(index))
        })
    }

    /// How many of the cursor's stack a stack of the same item in `slot`
    /// takes, if the move is allowed.
    fn stack_onto(
        &self,
        source: &InventoryItem,
        slot: InventorySlot,
        actor: InventoryActor,
    ) -> Option<NonZeroU32> {
        let destination = self.items.get(&slot)?;
        if source.details.id != destination.details.id {
            return None;
        }
        let available = source.stack_count?;
        let existing = destination.stack_count?;
        let amount =
            NonZeroU32::new(available.min(destination.rules.stack_size.saturating_sub(existing)))?;
        self.plan_move(&self.cursor_move(slot, MoveQuantity::Count(amount)), actor)
            .is_ok()
            .then_some(amount)
    }

    /// Whether the whole cursor item may go into `slot`, which is empty.
    fn place_into(&self, slot: InventorySlot, actor: InventoryActor) -> bool {
        !self.items.contains_key(&slot)
            && self
                .plan_move(&self.cursor_move(slot, MoveQuantity::Whole), actor)
                .is_ok()
    }

    /// A move from the cursor to `slot`, for planning only.
    fn cursor_move(&self, to: InventorySlot, quantity: MoveQuantity) -> InventoryMove {
        InventoryMove {
            sequence: None,
            session_id: 0,
            revision: self.revision,
            from: InventorySlot::CURSOR,
            to,
            quantity,
            created: Instant::now(),
        }
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
        send(&titanium_move(request.from, request.to, request.quantity)?)?;
        self.apply(update.clone());
        Ok(update)
    }

    /// Computes placement under `EQEmu`'s rules without modifying inventory or
    /// sending a packet.
    ///
    /// # Errors
    /// Rejects unsupported or inconsistent moves using the latest known inventory.
    pub fn plan_move(
        &self,
        request: &InventoryMove,
        actor: InventoryActor,
    ) -> Result<InventoryUpdate> {
        self.plan_move_for(MoveRules::EqEmu, request, actor)
    }

    /// Computes placement under a server type's rules without modifying
    /// inventory or sending a packet.
    ///
    /// # Errors
    /// Rejects unsupported or inconsistent moves using the latest known
    /// inventory, and moves the server type's rules refuse.
    pub fn plan_move_for(
        &self,
        rules: MoveRules,
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
            !self.bandage_stock_uncertain(request.from)
                && !self.bandage_stock_uncertain(request.to),
            "Wait for current stock in the affected bandage slot"
        );
        ensure!(
            movable(request.from, actor) && movable(request.to, actor),
            "Slot is unsupported or personal banking requires a nearby banker"
        );
        ensure!(
            !request.from.is_trade(),
            "Cancel the trade to take an item back"
        );
        if matches!(rules, MoveRules::Takp | MoveRules::Quarm) {
            Self::check_eqmac_move(request)?;
        }
        if request.to.is_trade() {
            self.check_trade(request, actor)?;
        }
        if request.from.is_world() || request.to.is_world() {
            check_world(request, self.items.contains_key(&request.to))?;
        }
        let source = self
            .items
            .get(&request.from)
            .context("Source slot is empty")?;
        if rules == MoveRules::Quarm {
            self.check_quarm_source(request, source)?;
        }
        self.check_source_container(request.from)?;
        if let Some(destination) = self.items.get(&request.to) {
            if let MoveQuantity::Count(amount) = request.quantity {
                return self.plan_merge(rules, request, source, destination, amount.get(), actor);
            }
            return self.plan_cursor_swap(rules, request, source, destination, actor);
        }
        ensure!(
            !self.items.keys().any(|slot| slot
                .parent()
                .is_some_and(|(parent, _)| parent == request.to)),
            "Destination has inconsistent container contents"
        );
        self.check_slot(rules, request.to, source, actor)?;
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
        rules: MoveRules,
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
        self.check_slot(rules, request.to, source, actor)?;
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
        rules: MoveRules,
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
        self.check_slot(rules, request.to, source, actor)?;
        self.check_slot(rules, request.from, destination, actor)?;
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
        rules: MoveRules,
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
        if slot.is_world() {
            ensure!(
                item.bag_slots == 0,
                "Containers cannot be placed inside containers"
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
                130 => 13,
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
            if matches!(rules, MoveRules::Takp | MoveRules::Quarm) {
                self.check_takp_worn(slot, item)?;
            }
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
                            && rules.trained_skill(actor.dual_wield),
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
impl Inventory {
    /// What servers accept into a trade slot. `EQEmu` may disconnect a
    /// client that sends an invalid move or offers a NO DROP item.
    fn check_trade(&self, request: &InventoryMove, actor: InventoryActor) -> Result<()> {
        ensure!(
            request.from == InventorySlot::CURSOR,
            "Pick the item up to hand it over"
        );
        let occupied = self.items.contains_key(&request.to);
        match request.quantity {
            MoveQuantity::Whole => ensure!(!occupied, "That trade slot is taken"),
            MoveQuantity::Count(_) => ensure!(occupied, "Hand over the whole stack on the cursor"),
        }
        ensure!(
            actor.trade_no_drop
                || !self.items.iter().any(|(slot, item)| {
                    (*slot == request.from
                        || slot.parent().is_some_and(|(bag, _)| bag == request.from))
                        && item.details.flags.iter().any(|flag| flag == "NO DROP")
                }),
            "NO DROP items cannot be traded"
        );
        Ok(())
    }

    /// An observed child can only move while its containing bag is known.
    fn check_source_container(&self, slot: InventorySlot) -> Result<()> {
        if let Some((parent, index)) = slot.parent() {
            ensure!(
                self.items
                    .get(&parent)
                    .is_some_and(|bag| index < bag.bag_slots),
                "Source container is not known"
            );
        }
        Ok(())
    }

    /// Both Mac policies require encodable slots. The program's move routine
    /// has no pending-move gate (`EQMac` 0x422b1c).
    ///
    /// Into a world container both servers' `Client::SwapItem` put the whole
    /// cursor item, whatever count the move names, so a part of a stack is
    /// refused here rather than predicted wrongly (6.598). With
    /// [`check_world`] this keeps every world move to one Quarm accepts:
    /// in only from the cursor, out only onto the empty cursor.
    fn check_eqmac_move(request: &InventoryMove) -> Result<()> {
        ensure!(
            request.from.to_eqmac().is_some() && request.to.to_eqmac().is_some(),
            "This server has no such slot"
        );
        ensure!(
            !request.to.is_world() || request.quantity == MoveQuantity::Whole,
            "Put the whole item in the container"
        );
        Ok(())
    }

    /// Rejects malformed observed quantities and inaccessible containers before
    /// Quarm can enter its disconnecting resynchronization path.
    fn check_quarm_source(&self, request: &InventoryMove, source: &InventoryItem) -> Result<()> {
        for item in [Some(source), self.items.get(&request.to)]
            .into_iter()
            .flatten()
        {
            if let Some(mac) = item.rules.eqmac {
                ensure!(mac.gm_flag == 0, "Restricted or unknown GM item flag");
                ensure!(
                    !matches!(mac.stack_flag, super::EqMacStackFlag::Unknown(_)),
                    "Unknown item stack flag"
                );
            }
        }
        ensure!(
            [request.from, request.to]
                .iter()
                .all(|slot| { (slot.is_held() || slot.is_world()) && !slot.is_in_cursor_bag() }),
            "This inventory action is not supported on Quarm"
        );
        if let Some(count) = source.stack_count {
            ensure!(
                count > 0 && source.rules.stack_size > 0 && count <= source.rules.stack_size,
                "Invalid or unknown stack quantity"
            );
        }
        Ok(())
    }

    /// What TAKP refuses in a worn slot beyond the rules every server
    /// shares (`ItemInstance::IsSlotAllowed`): bags and books anywhere,
    /// arrows in the range slot, anything in the primary hand while the
    /// secondary holds an instrument, and an instrument in the secondary
    /// hand while the primary is full. A book without a text to read is not
    /// told apart from other items (inferred: none has worn slots).
    fn check_takp_worn(&self, slot: InventorySlot, item: &InventoryItem) -> Result<()> {
        const INSTRUMENTS: std::ops::RangeInclusive<u8> = 23..=26;
        ensure!(
            item.bag_slots == 0 && item.book.is_none(),
            "Bags and books cannot be worn"
        );
        if slot.0 == 11 {
            ensure!(
                !matches!(item.rules.item_type, 27 | 28),
                "Arrows go in the ammo slot"
            );
        }
        if slot.0 == 13 {
            ensure!(
                !self
                    .items
                    .get(&InventorySlot(14))
                    .is_some_and(|secondary| INSTRUMENTS.contains(&secondary.rules.item_type)),
                "Put away the instrument in your secondary hand first"
            );
        }
        if slot.0 == 14 && INSTRUMENTS.contains(&item.rules.item_type) {
            ensure!(
                !self.items.contains_key(&InventorySlot(13)),
                "Empty your primary hand before holding an instrument"
            );
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
        || (actor.world_container && slot.is_world())
}

/// What `EQEmu` handles in a world container's slots (`Client::SwapItem`):
/// an item from the cursor goes in, onto the same stack or in place of what
/// was there, and an item comes out whole onto an empty cursor.
fn check_world(request: &InventoryMove, occupied: bool) -> Result<()> {
    if request.from.is_world() {
        ensure!(
            request.to == InventorySlot::CURSOR
                && request.quantity == MoveQuantity::Whole
                && !occupied,
            "Pick the item up onto an empty cursor"
        );
    } else {
        ensure!(
            request.from == InventorySlot::CURSOR,
            "Pick the item up to put it in the container"
        );
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
