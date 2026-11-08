//! An explicit Titanium cursor drop, distinct from an inventory move.
use super::{Inventory, InventoryItem, InventorySlot, InventoryUpdate};
use crate::command::EncodedCommand;
use anyhow::{ensure, Context, Result};
use std::{num::NonZeroU32, time::Instant};

/// A drop bound to the actor, admission, revision and observed cursor instance.
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryDrop {
    /// Current zone admission.
    pub session_id: u64,
    /// Actor's own spawn ID in this admission.
    pub own_id: u16,
    /// Inventory revision seen by the caller.
    pub revision: u64,
    /// The whole observed item or stack, including its instance serial.
    pub item: InventoryItem,
    /// When this explicit request was made.
    pub created: Instant,
}

/// Titanium `eqgame.exe` 0x497bc0 and 0x4bd379 send `OP_GroundSpawn` with
/// an empty body. The server supplies the item, object ID and position.
#[must_use]
pub fn titanium_ground_drop() -> EncodedCommand {
    EncodedCommand {
        opcode: crate::objects::SPAWN_OPCODE,
        body: Vec::new(),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CursorBasis {
    #[default]
    Unknown,
    Empty,
    Item(NonZeroU32),
}

/// Slot-only mutations cannot identify a cursor instance during a drop hold.
pub(super) fn unidentified_cursor_mutation(update: &InventoryUpdate) -> bool {
    match update {
        InventoryUpdate::Remove(slot)
        | InventoryUpdate::Used(slot)
        | InventoryUpdate::Consume { slot, .. }
        | InventoryUpdate::Deduct { slot, .. } => {
            *slot == InventorySlot::CURSOR || slot.is_in_cursor_bag()
        }
        _ => false,
    }
}

impl Inventory {
    /// Records submission and withdraws the basis for another cursor action.
    pub(super) fn hold_ground_drop(&mut self, serial: NonZeroU32) {
        self.drop_weight = super::weight::DropWeight::start(self);
        self.ground_drop = Some(serial);
        self.drop_cursor = CursorBasis::Unknown;
    }

    /// Whether this update directly identifies the cursor. A local move can
    /// invalidate the basis, and settling ordinary moves by silence cannot
    /// recover it for a subsequent drop.
    pub(super) fn observe_drop_cursor(&mut self, update: &InventoryUpdate) -> bool {
        let cursor = InventorySlot::CURSOR;
        match update {
            InventoryUpdate::Snapshot(_) => true,
            InventoryUpdate::Set(items) => items.first().is_some_and(|item| item.slot == cursor),
            InventoryUpdate::Remove(slot) => *slot == cursor,
            InventoryUpdate::Cursor(_) => {
                self.drop_cursor == CursorBasis::Empty && !self.items().contains_key(&cursor)
            }
            InventoryUpdate::Prediction(items) => {
                let before = self.items().get(&cursor);
                let after = items.iter().find(|item| item.slot == cursor);
                let serial = |item: Option<&InventoryItem>| {
                    item.and_then(|item| item.rules.ground_drop_serial)
                };
                if serial(before) != serial(after)
                    || !InventoryItem::same_slot_contents(before, after)
                {
                    self.drop_cursor = CursorBasis::Unknown;
                }
                false
            }
            InventoryUpdate::DestroyCursor | InventoryUpdate::Invalidated => {
                self.drop_cursor = CursorBasis::Unknown;
                false
            }
            InventoryUpdate::Consume { slot, .. }
            | InventoryUpdate::Used(slot)
            | InventoryUpdate::Deduct { slot, .. }
                if *slot == cursor =>
            {
                self.drop_cursor = CursorBasis::Unknown;
                false
            }
            _ => false,
        }
    }

    /// Records the result of a received cursor update after queue promotion.
    pub(super) fn record_drop_cursor(&mut self) {
        self.drop_cursor = match self.items().get(&InventorySlot::CURSOR) {
            None => CursorBasis::Empty,
            Some(item) => item
                .rules
                .ground_drop_serial
                .map_or(CursorBasis::Unknown, CursorBasis::Item),
        };
    }

    /// Validates one eligible ordinary item at the current received cursor.
    ///
    /// # Errors
    /// Refuses stale, predicted, queued or ambiguous inventory, and items
    /// lacking positively identified, droppable Titanium instance data.
    pub fn plan_ground_drop(&self, request: &InventoryDrop) -> Result<NonZeroU32> {
        ensure!(
            self.received() && !self.stale() && !self.predicted(),
            "Wait for current received inventory before dropping an item"
        );
        ensure!(
            !self.ground_drop_pending(),
            "Wait for a fresh inventory snapshot after the drop"
        );
        ensure!(
            self.revision() == request.revision,
            "Inventory changed; select the cursor again"
        );
        let item = self
            .items()
            .get(&InventorySlot::CURSOR)
            .context("The cursor is empty")?;
        ensure!(
            request.item.slot == InventorySlot::CURSOR
                && request.item.rules.ground_drop_serial == item.rules.ground_drop_serial
                && InventoryItem::same_slot_contents(Some(item), Some(&request.item)),
            "The cursor item changed; select it again"
        );
        ensure!(
            item.bag_slots == 0
                && self.queued().next().is_none()
                && !self.items().keys().any(|slot| slot.is_in_cursor_bag()),
            "Ground dropping needs one ordinary cursor item without contents"
        );
        ensure!(
            item.stack_count
                .is_none_or(|count| count > 0 && count <= item.rules.stack_size),
            "The cursor stack quantity is unavailable"
        );
        let serial = item
            .rules
            .ground_drop_serial
            .context("The cursor item is not a verified droppable instance")?;
        ensure!(
            self.drop_cursor == CursorBasis::Item(serial),
            "Wait for the server to identify the current cursor item"
        );
        ensure!(
            super::weight::DropWeight::start(self).is_some(),
            "A cursor drop needs a current carried item weight"
        );
        Ok(serial)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::parse_items;

    fn serialized(serial: &str, no_drop: &str, attuned: &str) -> String {
        let mut definition = vec!["0"; 159];
        definition[1] = "Synthetic drop fixture";
        definition[4] = "42";
        definition[7] = no_drop;
        let header = [
            "0", "0", "30", "0", "1", "0", serial, "0", "0", attuned, "0",
        ];
        format!(
            "{}|\"{}\"||||||||||\0",
            header.join("|"),
            definition.join("|")
        )
    }

    fn decoded(serial: &str, no_drop: &str, attuned: &str) -> InventoryItem {
        parse_items(serialized(serial, no_drop, attuned).as_bytes())
            .unwrap()
            .remove(0)
    }

    fn request(inventory: &Inventory, item: InventoryItem) -> InventoryDrop {
        InventoryDrop {
            session_id: 5,
            own_id: 7,
            revision: inventory.revision(),
            item,
            created: Instant::now(),
        }
    }

    #[test]
    fn drop_has_an_empty_body_and_requires_known_eligible_cursor_custody() {
        let packet = titanium_ground_drop();
        assert_eq!(packet.opcode, 0x0f47);
        assert_eq!(packet.body.as_slice(), b"");
        let mut inventory = Inventory::default();
        for item in [
            decoded("0", "255", "0"),
            decoded("17", "0", "0"),
            decoded("17", "255", "1"),
            decoded("17", "255", "2"),
        ] {
            inventory.apply(InventoryUpdate::Snapshot(vec![item.clone()]));
            assert!(inventory
                .plan_ground_drop(&request(&inventory, item))
                .is_err());
        }
        let item = decoded("17", "255", "0");
        inventory.apply(InventoryUpdate::Snapshot(vec![item.clone()]));
        let drop = request(&inventory, item);
        assert_eq!(inventory.plan_ground_drop(&drop).unwrap().get(), 17);
        let replacement = decoded("18", "255", "0");
        assert!(InventoryItem::same_slot_contents(
            Some(&drop.item),
            Some(&replacement)
        ));
        assert!(inventory
            .plan_ground_drop(&request(&inventory, replacement.clone()))
            .is_err());
        inventory.apply(InventoryUpdate::Set(vec![replacement]));
        assert!(inventory.plan_ground_drop(&drop).is_err());
    }

    #[test]
    fn cursor_drop_requires_a_snapshot_after_unidentified_cursor_changes() {
        let item = decoded("17", "255", "0");
        let replacement = decoded("18", "255", "0");
        let cursor = InventorySlot::CURSOR;
        let mut pending = Inventory::default();
        pending.apply(InventoryUpdate::Snapshot(vec![item.clone()]));
        let serial = pending
            .plan_ground_drop(&request(&pending, item.clone()))
            .unwrap();
        pending.apply(InventoryUpdate::GroundDropPending(serial));
        for (updates, displayed, queued) in [
            (vec![InventoryUpdate::Settled], &item, 0),
            (
                vec![InventoryUpdate::Cursor(vec![replacement.clone()])],
                &item,
                1,
            ),
            (
                vec![InventoryUpdate::Set(vec![replacement.clone()])],
                &replacement,
                0,
            ),
            (
                vec![
                    InventoryUpdate::Set(vec![replacement.clone()]),
                    InventoryUpdate::Set(vec![item.clone()]),
                ],
                &item,
                0,
            ),
            (
                vec![InventoryUpdate::Deduct {
                    slot: cursor,
                    quantity: 1,
                }],
                &item,
                0,
            ),
            (vec![InventoryUpdate::Invalidated], &item, 0),
        ] {
            let mut inventory = pending.clone();
            for update in updates {
                inventory.apply(update);
            }
            assert!(inventory.ground_drop_pending());
            assert!(inventory
                .plan_ground_drop(&request(&inventory, displayed.clone()))
                .is_err());
            inventory.apply(InventoryUpdate::Remove(cursor));
            assert!(inventory.stale() && inventory.ground_drop_pending());
            assert_eq!(inventory.items().get(&cursor), Some(displayed));
            assert_eq!(inventory.queued().count(), queued);
            inventory.apply(InventoryUpdate::Snapshot(vec![replacement.clone()]));
            assert!(!inventory.stale() && !inventory.ground_drop_pending());
            assert_eq!(inventory.queued().count(), 0);
            assert_eq!(
                inventory
                    .plan_ground_drop(&request(&inventory, replacement.clone()))
                    .unwrap()
                    .get(),
                18
            );
        }
    }

    #[test]
    fn titanium_inventory_rejects_a_serialized_child_on_an_ordinary_item() {
        let parent = serialized("17", "255", "0");
        let root = parent.strip_suffix("||||||||||\0").unwrap();
        let child = serialized("18", "0", "0");
        let child = child.trim_end_matches('\0').replace('"', "\\\"");
        let snapshot = format!("{root}|\"{child}\"|||||||||\0");
        let error = crate::inventory::decode(0x5394, snapshot.as_bytes()).unwrap_err();
        assert_eq!(error.to_string(), "item outside container capacity");
    }

    #[test]
    fn silence_does_not_authorize_a_drop_from_a_projected_cursor() {
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(Vec::new()));
        let item = decoded("17", "255", "0");
        inventory.apply(InventoryUpdate::Prediction(vec![item.clone()]));
        inventory.apply(InventoryUpdate::Settled);
        assert!(!inventory.predicted());
        assert!(inventory
            .plan_ground_drop(&request(&inventory, item.clone()))
            .is_err());
        inventory.apply(InventoryUpdate::Set(vec![item.clone()]));
        assert!(inventory
            .plan_ground_drop(&request(&inventory, item))
            .is_ok());
    }

    #[test]
    fn a_drop_requires_an_established_non_cursor_weight() {
        let item = decoded("17", "255", "0");
        let mut orphan = item.clone();
        orphan.slot = InventorySlot(251);
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item.clone(), orphan]));
        assert!(inventory
            .plan_ground_drop(&request(&inventory, item))
            .unwrap_err()
            .to_string()
            .contains("carried item weight"));
    }
}
