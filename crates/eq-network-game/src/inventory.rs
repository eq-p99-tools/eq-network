//! Titanium inventory state and validated moves, independent of transport and UI.
mod actions;
mod activation;
mod banking;
mod titanium;
pub use actions::{InventoryActor, InventoryMove, MoveQuantity};
pub use activation::{ClickEffect, ClickKind, ItemActivation, ItemUse};
pub use banking::banker_in_range;
pub use titanium::decode;
pub(crate) use titanium::parse as parse_items;

use crate::items::ItemDetails;
use serde::Serialize;
use std::collections::BTreeMap;

/// A Titanium inventory address; unknown slots retain their original number.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct InventorySlot(pub i32);

impl InventorySlot {
    /// Classic personal bank roots and their contents; shared bank is unsupported.
    #[must_use]
    pub const fn is_personal_bank(self) -> bool {
        matches!(self.0, 2000..=2007 | 2031..=2110)
    }

    /// Parent container and zero-based index for a known bag-content address.
    #[must_use]
    pub fn parent(self) -> Option<(Self, u8)> {
        for (start, end, parent) in [
            (251, 330, 22),
            (331, 340, 30),
            (2031, 2190, 2000),
            (2531, 2550, 2500),
        ] {
            if (start..=end).contains(&self.0) {
                let offset = self.0 - start;
                return Some((Self(parent + offset / 10), u8::try_from(offset % 10).ok()?));
            }
        }
        None
    }

    /// Resolves a bag index without using the child's serialized placeholder slot.
    #[must_use]
    pub fn child(self, index: u8) -> Option<Self> {
        if index >= 10 {
            return None;
        }
        let base = match self.0 {
            22..=29 => 251 + (self.0 - 22) * 10,
            30 => 331,
            2000..=2015 => 2031 + (self.0 - 2000) * 10,
            2500..=2501 => 2531 + (self.0 - 2500) * 10,
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
            _ => format!("Slot {}", self.0),
        }
    }
}

/// Placement constraints from the Titanium static item definition.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ItemPlacement {
    /// Maximum stack quantity from the server definition; zero means unknown.
    pub stack_size: u32,
    /// Item size category.
    pub size: u8,
    /// Largest size category accepted by this container.
    pub bag_size: u8,
    /// Container specialization (for example a quiver).
    pub bag_type: u8,
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
    /// Server definition constraints used when placing an item.
    pub rules: ItemPlacement,
    /// Exact inventory location.
    pub slot: InventorySlot,
    /// Item definition delivered with the instance, suitable for local inspection.
    pub details: ItemDetails,
    /// Icon number in the user's installed UI atlas.
    pub icon: u32,
    /// Stack quantity, only for items marked stackable by the server.
    pub stack_count: Option<u32>,
    /// Remaining instance charges; negative sentinel values are preserved.
    pub charges: i32,
    /// Number of usable container slots, zero for non-containers.
    pub bag_slots: u8,
}

/// Authoritative updates and explicitly identified local predictions.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum InventoryUpdate {
    /// Local post-send projection, not a server acknowledgment.
    Prediction(Vec<InventoryItem>),
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
    /// Server explicitly removes a whole slot, including its container contents.
    Remove(InventorySlot),
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
}

/// Current inventory projection; an absent snapshot is distinct from an empty one.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[allow(clippy::struct_excessive_bools)] // Admission, invalidation, prediction and correction are distinct state.
pub struct Inventory {
    items: BTreeMap<InventorySlot, InventoryItem>,
    received: bool,
    stale: bool,
    revision: u64,
    predicted: bool,
    unconfirmed: BTreeMap<InventorySlot, Option<InventoryItem>>,
    correcting: bool,
}

impl Inventory {
    /// Monotonic local revision used to reject stale UI intents.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    /// Whether any contents include post-send predictions.
    #[must_use]
    pub const fn predicted(&self) -> bool {
        self.predicted
    }

    /// Slots changed by pending predictions and their contents before the first change.
    /// An empty origin is retained too: predicted removal or insertion is not confirmation.
    /// Origins remain until a covering server update or a complete snapshot arrives.
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

    /// Applies a fully decoded update; a new snapshot replaces previous admission data.
    pub fn apply(&mut self, update: InventoryUpdate) {
        self.revision = self.revision.wrapping_add(1);
        match update {
            InventoryUpdate::Prediction(items) => {
                let next: BTreeMap<_, _> =
                    items.into_iter().map(|item| (item.slot, item)).collect();
                for slot in self.items.keys().chain(next.keys()) {
                    if self.items.get(slot) != next.get(slot) {
                        self.unconfirmed
                            .entry(*slot)
                            .or_insert_with(|| self.items.get(slot).cloned());
                    }
                }
                self.items = next;
                self.predicted = !self.unconfirmed.is_empty();
            }
            InventoryUpdate::Consume { slot, charge } => match self.items.get_mut(&slot) {
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
            },
            InventoryUpdate::Snapshot(items) => {
                self.items = items.into_iter().map(|item| (item.slot, item)).collect();
                self.received = true;
                self.predicted = false;
                self.stale = false;
                self.unconfirmed.clear();
                self.correcting = false;
            }
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
                    self.reconcile(root.slot, &items);
                    self.remove(root.slot);
                }
                for item in items {
                    self.items.insert(item.slot, item);
                }
            }
            InventoryUpdate::Remove(slot) => {
                self.reconcile(slot, &[]);
                self.remove(slot);
            }
            InventoryUpdate::Deduct { slot, quantity } => match self.items.get_mut(&slot) {
                Some(item) if item.stack_count.is_some_and(|count| count > quantity) => {
                    item.stack_count = item.stack_count.map(|count| count - quantity);
                }
                Some(_) => {
                    self.reconcile(slot, &[]);
                    self.remove(slot);
                }
                // The server took something this projection does not have.
                None => self.stale = true,
            },
            InventoryUpdate::Invalidated => self.stale = true,
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
    /// Contradictions freeze new moves until every outstanding slot is authoritative.
    fn reconcile(&mut self, root: InventorySlot, authoritative: &[InventoryItem]) {
        let covered: Vec<_> = self
            .unconfirmed
            .keys()
            .copied()
            .filter(|slot| *slot == root || slot.parent().is_some_and(|(parent, _)| parent == root))
            .collect();
        for slot in covered {
            let actual = authoritative.iter().find(|item| item.slot == slot);
            if self.items.get(&slot) != actual {
                self.correcting = true;
            }
            self.unconfirmed.remove(&slot);
        }
        self.predicted = !self.unconfirmed.is_empty();
        if !self.predicted {
            self.correcting = false;
        }
    }
}

#[cfg(test)]
mod tests;
