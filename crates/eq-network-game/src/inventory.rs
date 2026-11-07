//! Titanium inventory state and validated moves, independent of transport and UI.
mod actions;
mod activation;
mod banking;
mod destruction;
mod eqmac;
mod ground_drop;
mod titanium;
mod weight;
pub use actions::{
    titanium_move, InventoryActor, InventoryMove, MoveQuantity, MoveRules, MoveSequence,
    MOVE_OPCODE,
};
pub use activation::{
    titanium_item_cast, ClickEffect, ClickKind, ItemActivation, ItemUse, CAST_OPCODE,
};
pub use banking::banker_in_range;
pub use destruction::{titanium_destroy, InventoryDestroy};
pub(crate) use eqmac::details as eqmac_details;
pub(crate) use eqmac::merchant_stock as eqmac_merchant_stock;
pub(crate) use eqmac::record_at as eqmac_record_at;
pub(crate) use eqmac::trade_view as eqmac_trade_view;
pub use eqmac::{
    decode as decode_eqmac, decode_quarm, decode_with as decode_eqmac_with,
    move_item as eqmac_move, takp_item_hit_points, EqMacItemRules, EqMacStackFlag,
    ItemHitPointCount, ItemRules, UnsettledItem, Wearer,
};
pub use ground_drop::{titanium_ground_drop, InventoryDrop};
pub use titanium::decode;
pub(crate) use titanium::{parse as parse_items, parse_at};
pub use weight::WeightUnavailable;

use crate::items::ItemDetails;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::num::NonZeroU32;

/// The cursor; items pushed onto it queue behind the one shown there.
const CURSOR: InventorySlot = InventorySlot::CURSOR;

/// Titanium inventory bags have ten addressable child slots
/// (`EQEmu` `common/patches/titanium_limits.h`, `invbag::SLOT_COUNT`).
const BAG_SLOT_COUNT: u8 = 10;

/// A Titanium inventory address; unknown slots retain their original number.
/// The named slots and ranges below are the one vocabulary for them.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct InventorySlot(pub i32);

impl InventorySlot {
    /// The cursor, where a picked-up item rides.
    pub const CURSOR: Self = Self(30);

    /// The address of an `EQMac` slot, which numbers the cursor 0 where
    /// Titanium has the charm, has no charm, and numbers each bag's contents
    /// one lower (TAKP `common/patches/mac_limits.h`); None for a number
    /// `EQMac` does not use.
    #[must_use]
    pub const fn from_eqmac(slot: i32) -> Option<Self> {
        match slot {
            0 => Some(Self::CURSOR),
            1..=29 | 2000..=2007 | 3000..=3007 | 4000..=4009 => Some(Self(slot)),
            250..=339 | 2030..=2109 | 3030..=3109 => Some(Self(slot + 1)),
            _ => None,
        }
    }

    /// The `EQMac` number of this slot, the inverse of
    /// [`InventorySlot::from_eqmac`]; None where `EQMac` has no such slot, as
    /// for the charm.
    #[must_use]
    pub const fn to_eqmac(self) -> Option<i32> {
        match self.0 {
            30 => Some(0),
            1..=29 | 2000..=2007 | 3000..=3007 | 4000..=4009 => Some(self.0),
            251..=340 | 2031..=2110 | 3031..=3110 => Some(self.0 - 1),
            _ => None,
        }
    }

    /// Where the player's own items lie, which a server's item update may
    /// name: worn, carried, on the cursor, in the bank or the shared bank,
    /// and in a bag in any of them. Not the trade slots, nor the world
    /// container's.
    #[must_use]
    pub const fn is_held(self) -> bool {
        matches!(
            self.0,
            0..=30 | 251..=340 | 2000..=2015 | 2031..=2190 | 2500..=2501 | 2531..=2550
        )
    }

    /// Worn equipment, from the charm to the ammo slot.
    #[must_use]
    pub const fn is_equipment(self) -> bool {
        matches!(self.0, 0..=21)
    }

    /// One of the eight carried pack slots, which may hold a bag.
    #[must_use]
    pub const fn is_pack(self) -> bool {
        matches!(self.0, 22..=29)
    }

    /// Carried in the inventory: a pack slot or what a carried bag holds.
    /// Neither the cursor nor a bag on it is carried.
    #[must_use]
    pub const fn is_carried(self) -> bool {
        matches!(self.0, 22..=29 | 251..=330)
    }

    /// What a bag on the cursor holds.
    #[must_use]
    pub const fn is_in_cursor_bag(self) -> bool {
        matches!(self.0, 331..=340)
    }

    /// One of the ten slots of the world container open for the player, such
    /// as a forge: 4000 to 4009.
    #[must_use]
    pub const fn is_world(self) -> bool {
        matches!(self.0, 4000..=4009)
    }

    /// Classic personal bank roots and their contents; shared bank is unsupported.
    #[must_use]
    pub const fn is_personal_bank(self) -> bool {
        matches!(self.0, 2000..=2007 | 2031..=2110)
    }

    /// One of the player's eight trade slots, which the give window shows the
    /// first four of. An item enters one only from the cursor, while a give or
    /// trade window is open.
    #[must_use]
    pub const fn is_trade(self) -> bool {
        matches!(self.0, 3000..=3007)
    }

    /// A trade slot or what a bag in one holds.
    #[must_use]
    pub const fn is_in_trade(self) -> bool {
        matches!(self.0, 3000..=3007 | 3031..=3110)
    }

    /// Parent container and zero-based index for a known bag-content address.
    /// The carried bags' contents are 251 to 330, a cursor bag's 331 to 340,
    /// the bank bags' 2031 to 2190, the shared bank bags' 2531 to 2550 and the
    /// trade slots' bags' 3031 to 3110.
    #[must_use]
    pub fn parent(self) -> Option<(Self, u8)> {
        for (start, end, parent) in [
            (251, 330, 22),
            (331, 340, 30),
            (2031, 2190, 2000),
            (2531, 2550, 2500),
            (3031, 3110, 3000),
        ] {
            if (start..=end).contains(&self.0) {
                let offset = self.0 - start;
                let count = i32::from(BAG_SLOT_COUNT);
                return Some((
                    Self(parent + offset / count),
                    u8::try_from(offset % count).ok()?,
                ));
            }
        }
        None
    }

    /// Resolves a bag index without using the child's serialized placeholder slot.
    #[must_use]
    pub fn child(self, index: u8) -> Option<Self> {
        if index >= BAG_SLOT_COUNT {
            return None;
        }
        let count = i32::from(BAG_SLOT_COUNT);
        let base = match self.0 {
            22..=29 => 251 + (self.0 - 22) * count,
            30 => 331,
            2000..=2015 => 2031 + (self.0 - 2000) * count,
            2500..=2501 => 2531 + (self.0 - 2500) * count,
            3000..=3007 => 3031 + (self.0 - 3000) * count,
            _ => return None,
        };
        Some(Self(base + i32::from(index)))
    }

    /// Human-readable slot name; unsupported addresses remain visible.
    #[must_use]
    pub fn label(self) -> String {
        const EQUIPMENT: [&str; 22] = [
            "Charm",
            "Left ear",
            "Head",
            "Face",
            "Right ear",
            "Neck",
            "Shoulders",
            "Arms",
            "Back",
            "Left wrist",
            "Right wrist",
            "Ranged",
            "Hands",
            "Primary",
            "Secondary",
            "Left finger",
            "Right finger",
            "Chest",
            "Legs",
            "Feet",
            "Waist",
            "Ammo",
        ];
        if let Some((parent, index)) = self.parent() {
            return format!("{} / {}", parent.label(), index + 1);
        }
        match self.0 {
            0..=21 => usize::try_from(self.0)
                .ok()
                .and_then(|index| EQUIPMENT.get(index))
                .unwrap_or(&"Equipment")
                .to_string(),
            22..=29 => format!("Pack {}", self.0 - 21),
            30 => "Cursor".into(),
            2000..=2015 => format!("Bank {}", self.0 - 1999),
            2500..=2501 => format!("Shared bank {}", self.0 - 2499),
            3000..=3007 => format!("Trade {}", self.0 - 2999),
            _ => format!("Slot {}", self.0),
        }
    }
}

/// Placement constraints from the server's item definition.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ItemPlacement {
    /// Titanium instance serial when its received definition is eligible for
    /// the ordinary drop action. Unknown or restricted items have no value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ground_drop_serial: Option<NonZeroU32>,
    /// Common-item `EQMac` restrictions, retaining unknown values for inspection.
    pub eqmac: Option<EqMacItemRules>,
    /// Maximum stack quantity from the server definition; zero means unknown.
    pub stack_size: u32,
    /// Item size category.
    pub size: u8,
    /// Largest size category accepted by this container.
    pub bag_size: u8,
    /// Container specialization (for example a quiver).
    pub bag_type: u8,
    /// Percentage of contents' weight removed by the container (0..=100).
    pub bag_weight_reduction: u8,
    /// Item category, including weapon handedness.
    pub item_type: u8,
    /// Deity restrictions; zero is unrestricted.
    pub deity_mask: u32,
}

/// One occupied slot, retaining instance quantities separately from static stats.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InventoryItem {
    /// Server-supplied click effect and instance reuse information; never inferred from the name.
    pub activation: ItemActivation,
    /// Spell taught by a scroll, when supplied by the server item definition.
    pub scroll_spell: Option<u32>,
    /// What the item reads as, when it is a book or a note.
    pub book: Option<crate::books::Book>,
    /// Server definition constraints used when placing an item.
    pub rules: ItemPlacement,
    /// Exact inventory location.
    pub slot: InventorySlot,
    /// Item definition delivered with the instance, suitable for local
    /// inspection, with the item's picture.
    pub details: ItemDetails,
    /// Stack quantity, only for items marked stackable by the server.
    pub stack_count: Option<u32>,
    /// Remaining instance charges; negative sentinel values are preserved.
    pub charges: i32,
    /// Number of usable container slots, zero for non-containers.
    pub bag_slots: u8,
}

impl InventoryItem {
    /// Compare slot contents for placement decisions. Model IDs and the
    /// instance serial used to authorize a ground drop are checked separately.
    #[must_use]
    pub fn same_slot_contents(left: Option<&Self>, right: Option<&Self>) -> bool {
        match (left, right) {
            (Some(left), Some(right))
                if left.details.base_held_model != right.details.base_held_model
                    || left.rules.ground_drop_serial != right.rules.ground_drop_serial =>
            {
                let mut comparable = left.clone();
                comparable.details.base_held_model = right.details.base_held_model;
                comparable.rules.ground_drop_serial = right.rules.ground_drop_serial;
                &comparable == right
            }
            _ => left == right,
        }
    }
}

/// Authoritative updates and explicitly identified local predictions.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum InventoryUpdate {
    /// Local post-send projection, not a server acknowledgment.
    Prediction(Vec<InventoryItem>),
    /// Local whole-cursor removal after a successful destroy submission. It
    /// remains uncertain across silence settlement; only received state settles it.
    DestroyCursor,
    /// A drop was submitted for this cursor instance. Its custody is unresolved.
    GroundDropPending(NonZeroU32),
    /// One server-reported stack unit or charge consumed.
    Consume {
        /// Slot affected by the server.
        slot: InventorySlot,
        /// True for a charge, false for a stack unit or whole non-stack item.
        charge: bool,
    },
    /// Full zone-admission inventory, including populated bag slots.
    Snapshot(Vec<InventoryItem>),
    /// A replacement root or bag slot plus any serialized children.
    Set(Vec<InventoryItem>),
    /// An item (and any bag contents) pushed onto the cursor, as Titanium's limbo
    /// packet does: it shows there when the cursor is empty and otherwise waits
    /// behind the item there until that one leaves. Servers never resend the
    /// queue to Titanium clients, which move the next item up themselves.
    Cursor(Vec<InventoryItem>),
    /// Server explicitly removes a whole slot, including its container contents.
    Remove(InventorySlot),
    /// A unit or charge the server used up from the item in a slot, which
    /// stays there: a stack loses a unit and anything else a charge. `EQMac`
    /// servers say no more (`OP_DeleteCharge`), where Titanium's say which
    /// with [`InventoryUpdate::Consume`].
    Used(InventorySlot),
    /// Units the server took without sending an item update, as for a sale: a
    /// stack loses `quantity` units and any other item leaves its slot.
    Deduct {
        /// Slot the units came from.
        slot: InventorySlot,
        /// Units taken from a stack.
        quantity: u32,
    },
    /// A mutation needs a refreshed definition before quantities can be trusted.
    Invalidated,
    /// Pending predictions the server had time to refuse and did not; they now
    /// count as its contents. Servers such as `EQEmu` never acknowledge a
    /// successful move and answer a refused one at once, by resending its slots.
    Settled,
    /// The give or trade window closed: what the trade slots held left them,
    /// handed over, or on its way back as the server's item updates. Servers
    /// empty the slots without saying so.
    TradeEmptied,
    /// The world container open for the player emptied: a combine used what
    /// it held, or it closed and the server put what was left back in the
    /// inventory, as its item updates say (`OP_ClearObject`).
    WorldEmptied,
}

/// Current inventory projection; an absent snapshot is distinct from an empty one.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[allow(clippy::struct_excessive_bools)] // Admission, invalidation, prediction and correction are distinct state.
pub struct Inventory {
    items: BTreeMap<InventorySlot, InventoryItem>,
    /// Items waiting behind the cursor's, each with its bag contents.
    queued: VecDeque<Vec<InventoryItem>>,
    received: bool,
    stale: bool,
    revision: u64,
    predicted: bool,
    unconfirmed: BTreeMap<InventorySlot, Option<InventoryItem>>,
    /// Unresolved changed slots for each prediction, in submission order.
    /// This is evidence bookkeeping, never a queue that delays sending.
    #[serde(skip)]
    predictions: VecDeque<BTreeSet<InventorySlot>>,
    /// Slots whose history included affecting or unreadable equipment since
    /// the last full snapshot or existing policy-driven settlement. Unversioned
    /// partial updates cannot certify which pending prefix the server applied.
    #[serde(skip)]
    equipment_uncertainty: BTreeSet<InventorySlot>,
    /// Origins of plain whole-item relocations, separate from slot confirmation.
    #[serde(skip)]
    relocations: BTreeMap<InventorySlot, InventorySlot>,
    /// Property-only baseline for plain carried relocations followed by a new
    /// cursor arrival. Placement origins remain unresolved. Cursor queue
    /// alternatives can only make this weight bound heavier than the server.
    #[serde(skip)]
    weight_rebased: BTreeSet<InventorySlot>,
    /// An affecting transition invalidated the narrow arrival weight proof.
    #[serde(skip)]
    weight_history_uncertain: bool,
    /// Conserved units in a finite root-slot footprint, not confirmed placement.
    #[serde(skip)]
    stack_weight: Option<stack_weight::StackWeightBound>,
    correcting: bool,
    /// Original cursor mass retained after explicit destructive submission.
    /// Never retired by the ordinary movement silence heuristic.
    destroyed: Option<InventoryItem>,
    ground_drop: Option<NonZeroU32>,
    #[serde(skip)]
    drop_cursor: ground_drop::CursorBasis,
}

impl Inventory {
    /// Whether a submitted drop still awaits authoritative cursor state.
    #[must_use]
    pub const fn ground_drop_pending(&self) -> bool {
        self.ground_drop.is_some()
    }

    /// Monotonic local revision used to reject stale UI intents.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    /// Whether any contents include post-send predictions.
    #[must_use]
    pub const fn predicted(&self) -> bool {
        self.predicted || self.destroyed.is_some()
    }

    /// Number of local predictions with slots still awaiting server evidence.
    /// A covering update resolves only those slots, across all affected entries;
    /// it is not a positive acknowledgment of any individual move.
    #[must_use]
    pub fn unresolved_predictions(&self) -> usize {
        self.predictions.len()
    }

    /// Whether any unresolved stat-bearing equipment history may change item
    /// bonuses or a worn effect. Only explicitly decoded zero bonuses with known
    /// equipment rules and no worn effect are neutral. This includes intermediate
    /// moves: returning to the original slots does not clear an affecting history.
    /// Unversioned partial slot updates cannot retire it; a full snapshot or
    /// existing policy-driven settlement starts a new contribution baseline.
    /// False does not confirm inventory placement or establish carried weight.
    /// Callers must still require a received, non-stale inventory.
    #[must_use]
    pub fn equipment_stats_uncertain(&self) -> bool {
        !self.equipment_uncertainty.is_empty()
    }

    /// Slots changed by pending predictions and their contents before the first change.
    /// An empty origin is retained too: predicted removal or insertion is not confirmation.
    /// Origins remain until a covering server update, a settlement or a complete
    /// snapshot arrives. Authoritative consumption also removes a consumed item's
    /// traced origin without confirming independent moves.
    pub fn prediction_origins(
        &self,
    ) -> impl Iterator<Item = (InventorySlot, Option<&InventoryItem>)> {
        self.unconfirmed
            .iter()
            .map(|(slot, item)| (*slot, item.as_ref()))
    }

    /// Items ordered by their wire slot, including bag contents.
    #[must_use]
    pub const fn items(&self) -> &BTreeMap<InventorySlot, InventoryItem> {
        &self.items
    }

    /// Whether the server has supplied a complete snapshot.
    #[must_use]
    pub const fn received(&self) -> bool {
        self.received
    }

    /// Whether an unsupported or malformed update makes the displayed snapshot stale.
    #[must_use]
    pub const fn stale(&self) -> bool {
        self.stale || self.correcting
    }

    /// Whether a contradictory server update left other predicted slots unresolved.
    #[must_use]
    pub const fn awaiting_correction(&self) -> bool {
        self.correcting
    }

    /// Items waiting behind the cursor's, next first, each with its bag contents.
    pub fn queued(&self) -> impl Iterator<Item = &[InventoryItem]> {
        self.queued.iter().map(Vec::as_slice)
    }

    /// Applies a fully decoded update; a new snapshot replaces previous admission data.
    pub fn apply(&mut self, update: InventoryUpdate) {
        self.revision = self.revision.wrapping_add(1);
        let received_cursor = self.observe_drop_cursor(&update);
        self.change(update);
        // As the Titanium client does, the next queued item moves up as soon
        // as the cursor empties.
        while !self.items.contains_key(&CURSOR) {
            let Some(next) = self.queued.pop_front() else {
                break;
            };
            self.rebase_arriving_items(&next);
            self.items
                .extend(next.into_iter().map(|item| (item.slot, item)));
        }
        if received_cursor && !self.stale() {
            self.record_drop_cursor();
        }
    }

    fn change(&mut self, update: InventoryUpdate) {
        self.observe_stack_weight(&update);
        let arrival = self.rebase_cursor_arrival(&update);
        self.observe_rebased_weight(&update, arrival);
        self.retain_relocations(&update);
        match update {
            InventoryUpdate::Prediction(items) => self.predict(items),
            InventoryUpdate::DestroyCursor => self.predict_destruction(),
            InventoryUpdate::GroundDropPending(serial) => self.ground_drop = Some(serial),
            InventoryUpdate::Consume { slot, charge } => {
                let consumed = self
                    .items
                    .get(&slot)
                    .filter(|item| !charge && item.stack_count.is_none() && item.bag_slots == 0)
                    .cloned();
                match self.items.get_mut(&slot) {
                    Some(item) if charge => {
                        if item.charges > 0 {
                            item.charges -= 1;
                        }
                    }
                    Some(item) if item.stack_count.is_some_and(|count| count > 1) => {
                        item.stack_count = item.stack_count.map(|count| count - 1);
                    }
                    Some(_) => self.remove(slot),
                    None if charge => self.stale = true,
                    None => (),
                }
                self.reconcile_consumed(slot, consumed.as_ref());
            }
            InventoryUpdate::Snapshot(items) => self.replace_snapshot(items),
            InventoryUpdate::Set(items) => {
                let additional = items
                    .iter()
                    .filter(|item| !self.items.contains_key(&item.slot))
                    .count();
                if self.items.len() + additional > 1024 {
                    self.stale = true;
                    return;
                }
                if let Some(root) = items.first() {
                    if root.slot == CURSOR {
                        self.destroyed = None;
                    }
                    if arrival {
                        self.cover_arrival(root.slot);
                    } else {
                        self.reconcile(root.slot, &items);
                    }
                    self.remove(root.slot);
                }
                for item in items {
                    self.items.insert(item.slot, item);
                }
            }
            InventoryUpdate::Cursor(items) => {
                let queued: usize = self.queued.iter().map(Vec::len).sum();
                if self.items.len() + queued + items.len() > 1024 {
                    self.stale = true;
                } else if self.items.contains_key(&CURSOR) {
                    self.queued.push_back(items);
                } else {
                    // Nothing was predicted into an empty cursor, so there is
                    // nothing here to confirm or contradict.
                    self.items
                        .extend(items.into_iter().map(|item| (item.slot, item)));
                }
            }
            InventoryUpdate::Remove(slot) => {
                if let Some(serial) = self.ground_drop.filter(|_| slot == CURSOR) {
                    if self
                        .items
                        .get(&slot)
                        .and_then(|item| item.rules.ground_drop_serial)
                        != Some(serial)
                    {
                        // An unversioned removal might refer to the dropped
                        // item, not the newer item now occupying the cursor.
                        self.stale = true;
                        return;
                    }
                    self.ground_drop = None;
                }
                if slot == CURSOR && self.destroyed.is_some() {
                    // A new cursor arrival/replacement cannot be identified as
                    // the item an unversioned late removal names.
                    if self.items.contains_key(&CURSOR) {
                        self.stale = true;
                        return;
                    }
                    self.destroyed = None;
                }
                self.reconcile(slot, &[]);
                self.remove(slot);
            }
            InventoryUpdate::Used(slot) => self.use_up(slot),
            InventoryUpdate::Deduct { slot, quantity } => self.deduct(slot, quantity),
            InventoryUpdate::Invalidated => self.stale = true,
            InventoryUpdate::TradeEmptied => {
                self.items.retain(|slot, _| !slot.is_in_trade());
                // A prediction into a trade slot is resolved: the item is gone.
                self.unconfirmed.retain(|slot, _| !slot.is_in_trade());
                self.resolved();
            }
            InventoryUpdate::WorldEmptied => {
                self.items.retain(|slot, _| !slot.is_world());
                self.unconfirmed.retain(|slot, _| !slot.is_world());
                self.resolved();
            }
            InventoryUpdate::Settled => {
                self.unconfirmed
                    .retain(|slot, _| self.destroyed.is_some() && *slot == CURSOR);
                self.equipment_uncertainty.clear();
                self.weight_rebased.clear();
                self.weight_history_uncertain = false;
                self.stack_weight = None;
                self.resolved();
            }
        }
    }

    /// Replaces every inventory baseline with a complete received snapshot.
    fn replace_snapshot(&mut self, items: Vec<InventoryItem>) {
        self.destroyed = None;
        self.ground_drop = None;
        self.items = items.into_iter().map(|item| (item.slot, item)).collect();
        self.queued.clear();
        self.received = true;
        self.predicted = false;
        self.stale = false;
        self.unconfirmed.clear();
        self.predictions.clear();
        self.equipment_uncertainty.clear();
        self.weight_rebased.clear();
        self.weight_history_uncertain = false;
        self.stack_weight = None;
        self.correcting = false;
    }

    /// Units the server took from `slot` without an item update.
    fn deduct(&mut self, slot: InventorySlot, quantity: u32) {
        if !self.items.contains_key(&slot) {
            // The server took something this projection does not have.
            self.stale = true;
            return;
        }
        // The server took what it held there, and this projection has an
        // item there too: a prediction that put it there was right.
        for covered in self.covered(slot) {
            self.unconfirmed.remove(&covered);
        }
        self.resolved();
        match self.items.get_mut(&slot) {
            Some(item) if item.stack_count.is_some_and(|count| count > quantity) => {
                item.stack_count = item.stack_count.map(|count| count - quantity);
            }
            _ => self.remove(slot),
        }
    }

    /// A unit or charge the server used up from the item in `slot`, which
    /// stays there.
    fn use_up(&mut self, slot: InventorySlot) {
        match self.items.get_mut(&slot) {
            Some(item) => match item.stack_count {
                Some(count) if count > 1 => item.stack_count = Some(count - 1),
                // The server kept a stack this projection counts as one.
                Some(_) => self.stale = true,
                None if item.charges > 0 => item.charges -= 1,
                None => (),
            },
            // The server used up what this projection does not have.
            None => self.stale = true,
        }
    }

    /// Replays accumulated pre-admission state without mistaking partial data for a snapshot.
    #[must_use]
    pub fn admission_updates(&self) -> Vec<InventoryUpdate> {
        let mut updates = if self.received {
            vec![InventoryUpdate::Snapshot(
                self.items.values().cloned().collect(),
            )]
        } else {
            self.items
                .values()
                .cloned()
                .map(|item| InventoryUpdate::Set(vec![item]))
                .collect()
        };
        updates.extend(self.queued.iter().cloned().map(InventoryUpdate::Cursor));
        updates.extend(self.ground_drop.map(InventoryUpdate::GroundDropPending));
        if self.stale() {
            updates.push(InventoryUpdate::Invalidated);
        }
        updates
    }

    fn remove(&mut self, slot: InventorySlot) {
        self.items
            .retain(|key, _| *key != slot && key.parent().is_none_or(|(parent, _)| parent != slot));
    }

    /// A slot update confirms only that slot and its serialized container contents.
    /// Contradictions freeze new moves until every outstanding slot is authoritative
    /// or settled.
    fn reconcile(&mut self, root: InventorySlot, authoritative: &[InventoryItem]) {
        for slot in self.covered(root) {
            let actual = authoritative.iter().find(|item| item.slot == slot);
            if !InventoryItem::same_slot_contents(self.items.get(&slot), actual) {
                self.correcting = true;
            }
            self.unconfirmed.remove(&slot);
        }
        self.resolved();
    }

    /// Unconfirmed slots a server update for `root` covers: the slot and its
    /// container contents.
    fn covered(&self, root: InventorySlot) -> Vec<InventorySlot> {
        self.unconfirmed
            .keys()
            .copied()
            .filter(|slot| *slot == root || slot.parent().is_some_and(|(parent, _)| parent == root))
            .collect()
    }

    /// Ends a correction once no prediction is left unconfirmed.
    fn resolved(&mut self) {
        // Evidence is slot-specific and may arrive out of order. Retain the
        // original prediction order without guessing which move was accepted.
        for prediction in &mut self.predictions {
            prediction.retain(|slot| self.unconfirmed.contains_key(slot));
        }
        self.predictions.retain(|prediction| !prediction.is_empty());
        self.predicted = !self.unconfirmed.is_empty() || self.destroyed.is_some();
        if !self.predicted {
            self.correcting = false;
            self.weight_rebased.clear();
            self.stack_weight = None;
        }
    }
}

mod arrivals;
mod prediction;
mod stack_weight;

#[cfg(test)]
pub(crate) mod tests;
