//! An explicit Titanium cursor drop, distinct from an inventory move.
use super::{Inventory, InventoryItem, InventorySlot, InventoryUpdate};
use crate::command::EncodedCommand;
use anyhow::{ensure, Context, Result};
use serde::{ser::SerializeSeq, Serialize, Serializer};
use std::{collections::BTreeMap, num::NonZeroU32, time::Instant};

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

/// Explicit local projection tied to the submitted inventory and instance.
/// No object broadcast or elapsed refusal window acknowledges this removal.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InventoryDropProjection {
    /// Inventory revision at submission.
    pub revision: u64,
    /// Exact item submitted, retaining its verified instance and whole mass.
    pub item: Box<InventoryItem>,
}

impl From<&InventoryDrop> for InventoryDropProjection {
    fn from(request: &InventoryDrop) -> Self {
        Self {
            revision: request.revision,
            item: Box::new(request.item.clone()),
        }
    }
}

/// Diagnostic records are a sequence; compound identity keys are not JSON keys.
pub(super) fn serialize_dropped<S: Serializer>(
    dropped: &BTreeMap<(u32, NonZeroU32), InventoryItem>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    let mut records = serializer.serialize_seq(Some(dropped.len()))?;
    for item in dropped.values() {
        records.serialize_element(item)?;
    }
    records.end()
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

/// Slot-only mutations cannot identify the instance after a projected drop.
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
    /// Locally removes only the submitted instance. A newer received cursor
    /// record wins, including a same-instance refusal, without a retry.
    pub(super) fn project_ground_drop(&mut self, projection: InventoryDropProjection) {
        if self.cursor_revision > projection.revision {
            return;
        }
        let item = *projection.item;
        let Some(serial) = item.rules.ground_drop_serial else {
            self.stale = true;
            return;
        };
        if item.slot != InventorySlot::CURSOR || item.bag_slots != 0 || item.augmented() {
            self.stale = true;
            return;
        }
        let matches = self
            .items
            .get(&InventorySlot::CURSOR)
            .is_some_and(|current| {
                current.rules.ground_drop_serial == Some(serial)
                    && InventoryItem::same_slot_contents(Some(current), Some(&item))
            });
        self.retain_drop_mass(item);
        if matches {
            self.remove(InventorySlot::CURSOR);
            self.projected_custody
                .entry(InventorySlot::CURSOR)
                .or_default()
                .cursor = true;
            self.drop_cursor = CursorBasis::Empty;
        }
    }

    /// An uncertain transport result retains removal and withdraws readiness.
    pub(super) fn project_uncertain_drop(&mut self, projection: InventoryDropProjection) {
        self.project_ground_drop(projection);
        self.stale = true;
    }

    /// Include a whole locally removed instance once, at its heaviest known
    /// mass. Unknown identities cannot establish a reusable finite bound.
    pub(super) fn retain_drop_mass(&mut self, item: InventoryItem) {
        let Some(serial) = item.rules.ground_drop_serial else {
            self.drop_mass_unknown = true;
            return;
        };
        let key = (item.details.id, serial);
        if item.bag_slots != 0
            || item.augmented()
            || self.dropped.len() >= 1024 && !self.dropped.contains_key(&key)
        {
            self.drop_mass_unknown = true;
            return;
        }
        self.dropped
            .entry(key)
            .and_modify(|old| {
                if old.details.weight_tenths < item.details.weight_tenths {
                    *old = item.clone();
                }
            })
            .or_insert(item);
    }

    /// A limbo arrival can still be queued on the server after a drop. A later
    /// destruction must retain its mass beyond ordinary local settlement.
    pub(super) fn destroy_with_drop_mass(&mut self) {
        if self.ground_drop_projected() {
            if let Some(item) = self.items.get(&InventorySlot::CURSOR).cloned() {
                self.retain_drop_mass(item);
            }
        }
        self.predict_destruction();
    }

    /// A received definition of the same instance replaces its shadow mass.
    /// A queued limbo arrival is not yet a counted carried slot.
    pub(super) fn cover_dropped_instances(&mut self, update: &InventoryUpdate) {
        if self.ground_drop_projected() {
            if let InventoryUpdate::Cursor(items) | InventoryUpdate::Set(items) = update {
                if items
                    .first()
                    .is_some_and(|item| item.slot == InventorySlot::CURSOR)
                {
                    let slots: std::collections::BTreeSet<_> =
                        items.iter().map(|item| item.slot).collect();
                    if slots.len() != items.len()
                        || items
                            .iter()
                            .skip(1)
                            .any(|item| !item.slot.is_in_cursor_bag())
                    {
                        self.drop_mass_unknown = true;
                        return;
                    }
                }
            }
        }
        let items = match update {
            InventoryUpdate::Set(items) => items,
            InventoryUpdate::Cursor(items) if !self.items.contains_key(&InventorySlot::CURSOR) => {
                items
            }
            _ => return,
        };
        for item in items {
            if let Some(serial) = item.rules.ground_drop_serial {
                self.dropped.remove(&(item.details.id, serial));
            }
        }
    }

    /// Whether this update establishes an explicit received or local cursor
    /// identity. Local movement does not become a received acknowledgment.
    pub(super) fn observe_drop_cursor(&mut self, update: &InventoryUpdate) -> bool {
        let cursor = InventorySlot::CURSOR;
        match update {
            InventoryUpdate::Snapshot(_) => true,
            InventoryUpdate::Prediction(items) => {
                let before = self.items().get(&cursor);
                let after = items.iter().find(|item| item.slot == cursor);
                let serial = |item: Option<&InventoryItem>| {
                    item.and_then(|item| item.rules.ground_drop_serial)
                };
                if self.cursor_revision != 0 || serial(before).is_some() || serial(after).is_some()
                {
                    return true;
                }
                // Preserve the previous conservative basis for inventories
                // without a verified drop instance, including EQMac inputs.
                if serial(before) != serial(after)
                    || !InventoryItem::same_slot_contents(before, after)
                {
                    self.drop_cursor = CursorBasis::Unknown;
                }
                false
            }
            InventoryUpdate::Set(items) => items.first().is_some_and(|item| item.slot == cursor),
            InventoryUpdate::Remove(slot) => *slot == cursor,
            InventoryUpdate::Cursor(_) => {
                self.drop_cursor == CursorBasis::Empty && !self.items().contains_key(&cursor)
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

    /// Records the identified local or received cursor after queue promotion.
    pub(super) fn record_drop_cursor(&mut self) {
        self.drop_cursor = match self.items().get(&InventorySlot::CURSOR) {
            None => CursorBasis::Empty,
            Some(item) => item
                .rules
                .ground_drop_serial
                .map_or(CursorBasis::Unknown, CursorBasis::Item),
        };
    }

    /// Validates one eligible ordinary item at the current identified cursor.
    ///
    /// # Errors
    /// Refuses stale, predicted, queued or ambiguous inventory, and items
    /// lacking positively identified, droppable Titanium instance data.
    pub fn plan_ground_drop(&self, request: &InventoryDrop) -> Result<NonZeroU32> {
        let serial = self.check_ground_drop_input(request)?;
        ensure!(
            self.drop_cursor == CursorBasis::Item(serial),
            "Wait for the server to identify the current cursor item"
        );
        Ok(serial)
    }

    /// Checks the current item predicates before an excluded-type world click.
    /// This does not authorize a submission; [`Self::plan_ground_drop`] also
    /// requires its historical cursor basis. The native raw predicate remains
    /// unmapped; positive decoded instance eligibility stays conservative.
    ///
    /// # Errors
    /// Refuses stale/predicted state, changed identity/revision, unknown instance
    /// eligibility, quantities, cursor contents, queue or carried weight.
    pub fn check_ground_drop_input(&self, request: &InventoryDrop) -> Result<NonZeroU32> {
        ensure!(
            self.received() && !self.stale() && !self.predicted(),
            "Wait for current received inventory before dropping an item"
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
            self.carried_weight_tenths(crate::GameDialect::Titanium, 0)
                .is_ok(),
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
                .check_ground_drop_input(&request(&inventory, item.clone()))
                .is_err());
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
    fn checked_input_does_not_authorize_submission_after_unresolved_cursor_history() {
        let first = decoded("17", "255", "0");
        let next = decoded("18", "255", "0");
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![first]));
        inventory.apply(InventoryUpdate::DestroyCursor);
        inventory.apply(InventoryUpdate::Settled);
        inventory.apply(InventoryUpdate::Cursor(vec![next.clone()]));
        let request = request(&inventory, next);
        assert_eq!(
            inventory.check_ground_drop_input(&request).unwrap().get(),
            18
        );
        assert_eq!(
            inventory
                .plan_ground_drop(&request)
                .unwrap_err()
                .to_string(),
            "Wait for the server to identify the current cursor item"
        );
    }

    #[test]
    fn local_drop_projection_releases_the_cursor_without_a_received_receipt() {
        let first = decoded("17", "255", "0");
        let second = decoded("18", "255", "0");
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![first.clone()]));
        let projection = InventoryDropProjection::from(&request(&inventory, first));
        inventory.apply(InventoryUpdate::GroundDropProjected(projection));
        assert!(inventory.ground_drop_projected());
        assert!(inventory.projected_cursor_empty());
        assert!(!inventory.custody_known(InventorySlot::CURSOR));
        assert!(!inventory.predicted());
        assert_eq!(inventory.items().get(&InventorySlot::CURSOR), None);
        inventory.apply(InventoryUpdate::Settled);
        assert!(inventory.ground_drop_projected());
        inventory.apply(InventoryUpdate::Cursor(vec![second.clone()]));
        let next = request(&inventory, second.clone());
        assert_eq!(inventory.plan_ground_drop(&next).unwrap().get(), 18);
        inventory.apply(InventoryUpdate::GroundDropProjected((&next).into()));
        assert_eq!(inventory.dropped.len(), 2);
        assert!(inventory.projected_cursor_empty());
        let diagnostic = serde_json::to_value(&inventory).unwrap();
        assert_eq!(diagnostic["dropped"].as_array().unwrap().len(), 2);
        // Replica replay must retain this as local state, never a fabricated
        // received empty snapshot or a received drop acknowledgment.
        let updates = inventory.admission_updates();
        assert!(matches!(&updates[..], [InventoryUpdate::LocalReplica(_)]));
        let mut replica = Inventory::default();
        for update in updates {
            replica.apply(update);
        }
        assert_eq!(replica, inventory);
        inventory.apply(InventoryUpdate::Set(vec![second]));
        assert_eq!(inventory.dropped.len(), 1);
        inventory.apply(InventoryUpdate::Snapshot(Vec::new()));
        assert!(!inventory.ground_drop_projected());
    }

    #[test]
    fn delayed_projection_respects_covering_cursor_records_but_not_unrelated_slots() {
        for covering in [false, true] {
            let item = decoded("17", "255", "0");
            let mut inventory = Inventory::default();
            inventory.apply(InventoryUpdate::Snapshot(vec![item.clone()]));
            let projection = InventoryDropProjection::from(&request(&inventory, item.clone()));
            let mut update = item;
            if !covering {
                update.slot = InventorySlot(24);
            }
            inventory.apply(InventoryUpdate::Set(vec![update]));
            inventory.apply(InventoryUpdate::GroundDropProjected(projection));
            assert_eq!(
                inventory.items().contains_key(&InventorySlot::CURSOR),
                covering
            );
            assert_eq!(inventory.ground_drop_projected(), !covering);
        }
    }

    #[test]
    fn uncertain_drop_stays_removed_and_stale_across_silence_without_retry() {
        let item = decoded("17", "255", "0");
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item.clone()]));
        let request = request(&inventory, item);
        inventory.apply(InventoryUpdate::GroundDropUncertain((&request).into()));
        inventory.apply(InventoryUpdate::Settled);
        assert!(inventory.stale() && inventory.ground_drop_projected());
        assert!(!inventory.items().contains_key(&InventorySlot::CURSOR));
        assert!(inventory.plan_ground_drop(&request).is_err());
    }

    #[test]
    fn slot_only_cursor_mutation_after_projection_withdraws_readiness() {
        let item = decoded("17", "255", "0");
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item.clone()]));
        let request = request(&inventory, item);
        inventory.apply(InventoryUpdate::GroundDropProjected((&request).into()));
        inventory.apply(InventoryUpdate::Remove(InventorySlot::CURSOR));
        assert!(inventory.stale() && inventory.ground_drop_projected());
        assert!(inventory.ground_drop_weight_tenths(0).is_some());
        inventory.apply(InventoryUpdate::Invalidated);
        assert!(inventory.ground_drop_weight_tenths(0).is_none());
    }

    #[test]
    fn invalidation_and_partial_instance_coverage_cannot_restore_a_lighter_fall_estimate() {
        let item = decoded("17", "255", "0");
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item.clone()]));
        let request = request(&inventory, item.clone());
        inventory.apply(InventoryUpdate::GroundDropProjected((&request).into()));
        inventory.apply(InventoryUpdate::Invalidated);
        inventory.apply(InventoryUpdate::Set(vec![item.clone()]));
        assert!(inventory.stale());
        // The partial record covers this instance, not the unread inventory.
        // Keep the drop-specific unknown marker so falls cannot use the
        // generic estimated-weight fallback after the last shadow retires.
        assert!(inventory.ground_drop_projected());
        assert_eq!(inventory.ground_drop_weight_tenths(0), None);
        inventory.apply(InventoryUpdate::Snapshot(vec![item]));
        assert!(!inventory.ground_drop_projected());
        assert!(inventory.ground_drop_weight_tenths(0).is_some());
    }

    #[test]
    fn titanium_inventory_reads_a_serialized_child_on_an_ordinary_item_as_its_augment() {
        // A common item's children are its augment sockets (6.570).
        let parent = serialized("17", "255", "0");
        let root = parent.strip_suffix("||||||||||\0").unwrap();
        let child = serialized("18", "0", "0");
        let child = child.trim_end_matches('\0').replace('"', "\\\"");
        let snapshot = format!("{root}|\"{child}\"|||||||||\0");
        let Some(InventoryUpdate::Snapshot(items)) =
            crate::inventory::decode(0x5394, snapshot.as_bytes()).unwrap()
        else {
            panic!("snapshot");
        };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].augments, [Some(42), None, None, None, None]);
    }

    #[test]
    fn a_locally_moved_received_instance_can_drop_after_ordinary_move_settlement() {
        let mut item = decoded("17", "255", "0");
        item.slot = InventorySlot(22);
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item.clone()]));
        item.slot = InventorySlot::CURSOR;
        inventory.apply(InventoryUpdate::Prediction(vec![item.clone()]));
        assert!(inventory
            .plan_ground_drop(&request(&inventory, item.clone()))
            .is_err());
        inventory.apply(InventoryUpdate::Settled);
        assert!(!inventory.predicted());
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
