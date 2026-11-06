//! Explicit whole-cursor destruction; never a negative ordinary move slot.
use super::{Inventory, InventoryItem, InventorySlot, InventoryUpdate, MoveRules, BAG_SLOT_COUNT};
use crate::command::EncodedCommand;
use anyhow::{ensure, Context, Result};
use std::time::Instant;

/// Confirmed intent bound to the exact received cursor and inventory revision.
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryDestroy {
    /// Current zone admission.
    pub session_id: u64,
    /// Inventory revision shown by the confirmation.
    pub revision: u64,
    /// Whole instance, including stack quantity, charges and container capacity.
    pub item: InventoryItem,
    /// Fresh confirmation time, not the time the dialog opened.
    pub created: Instant,
}

/// Titanium's explicit destroy sender: `MoveItem`, cursor, invalid destination,
/// zero count. Zero destroys the whole stack, not one unit. `DeleteItem` is a
/// different operation (unit consumption).
#[must_use]
pub fn titanium_destroy() -> EncodedCommand {
    let mut body = Vec::with_capacity(12);
    for word in [30_u32, u32::MAX, 0] {
        body.extend_from_slice(&word.to_le_bytes());
    }
    EncodedCommand {
        opcode: super::MOVE_OPCODE,
        body,
    }
}

impl Inventory {
    /// Projects only after successful submission; preserve mass separately.
    pub(super) fn predict_destruction(&mut self) {
        let Some(item) = self.items.get(&InventorySlot::CURSOR).cloned() else {
            self.stale = true;
            return;
        };
        let next = self
            .items
            .values()
            .filter(|item| item.slot != InventorySlot::CURSOR && !item.slot.is_in_cursor_bag())
            .cloned()
            .collect();
        let uncertain = self.weight_history_uncertain;
        self.predict(next);
        self.weight_history_uncertain = uncertain;
        self.destroyed = Some(item);
    }

    /// Checks the deliberately bounded first slice: one current cursor item,
    /// whole stack, no contents, queue or unresolved earlier inventory action.
    ///
    /// # Errors
    /// Refuses unavailable state, replacement/quantity changes, unsupported
    /// providers and populated containers. No packet or local change is made.
    pub fn plan_destroy(
        &self,
        rules: MoveRules,
        request: &InventoryDestroy,
    ) -> Result<InventoryUpdate> {
        ensure!(
            rules.supports_cursor_destruction(),
            "Item destruction is unavailable for this server"
        );
        ensure!(
            self.received() && !self.stale(),
            "Wait for a current inventory snapshot"
        );
        ensure!(
            !self.predicted(),
            "Wait for received inventory state before destroying an item"
        );
        ensure!(
            self.revision() == request.revision,
            "Inventory changed; select Destroy again"
        );
        let item = self
            .items()
            .get(&InventorySlot::CURSOR)
            .context("The cursor is empty")?;
        ensure!(
            request.item.slot == InventorySlot::CURSOR && item == &request.item,
            "The cursor item changed; select Destroy again"
        );
        ensure!(
            self.queued().next().is_none(),
            "Place queued cursor items before destroying an item"
        );
        ensure!(
            item.stack_count
                .is_none_or(|count| count > 0 && count <= item.rules.stack_size),
            "The cursor stack quantity is unavailable"
        );
        ensure!(
            item.bag_slots <= BAG_SLOT_COUNT
                && !self.items().keys().any(|slot| slot.is_in_cursor_bag()),
            "Empty the cursor container before destroying it"
        );
        Ok(InventoryUpdate::DestroyCursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        inventory::{ItemActivation, ItemPlacement},
        items::ItemDetails,
        GameDialect,
    };

    fn item() -> InventoryItem {
        InventoryItem {
            activation: ItemActivation::default(),
            scroll_spell: None,
            book: None,
            slot: InventorySlot::CURSOR,
            stack_count: Some(7),
            charges: 7,
            bag_slots: 0,
            rules: ItemPlacement {
                stack_size: 20,
                ..Default::default()
            },
            details: ItemDetails {
                base_held_model: None,
                equipment: None,
                bonuses: None,
                id: 42,
                name: "Disposable fixture".into(),
                lore: String::new(),
                weight_tenths: 15,
                slots: 0,
                classes: 0,
                races: 0,
                flags: Vec::new(),
                stats: Vec::new(),
                price: None,
                icon: None,
            },
        }
    }

    fn fixture(item: InventoryItem) -> (Inventory, InventoryDestroy) {
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item.clone()]));
        let request = InventoryDestroy {
            session_id: 7,
            revision: inventory.revision(),
            item,
            created: Instant::now(),
        };
        (inventory, request)
    }

    #[test]
    fn destruction_encodes_whole_cursor_without_exposing_a_negative_move_slot() {
        let packet = titanium_destroy();
        assert_eq!(packet.opcode, 0x420f);
        assert_eq!(packet.body, [30, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 0]);
        assert!(super::super::titanium_move(
            InventorySlot::CURSOR,
            InventorySlot(-1),
            super::super::MoveQuantity::Whole
        )
        .is_err());
        let (inventory, request) = fixture(item());
        assert_eq!(
            inventory.plan_destroy(MoveRules::EqEmu, &request).unwrap(),
            InventoryUpdate::DestroyCursor
        );
        assert_eq!(
            inventory.items()[&InventorySlot::CURSOR].stack_count,
            Some(7)
        );
    }

    #[test]
    fn changed_quantity_replacement_revision_and_unsupported_rules_refuse() {
        let (mut inventory, request) = fixture(item());
        for rules in [MoveRules::Takp, MoveRules::Quarm] {
            assert!(inventory.plan_destroy(rules, &request).is_err());
        }
        let mut changed = request.clone();
        changed.item.stack_count = Some(6);
        assert!(inventory.plan_destroy(MoveRules::EqEmu, &changed).is_err());
        inventory.apply(InventoryUpdate::Set(vec![request.item.clone()]));
        assert!(inventory.plan_destroy(MoveRules::EqEmu, &request).is_err());
        changed.revision = inventory.revision();
        changed.item = request.item.clone();
        inventory.apply(InventoryUpdate::Invalidated);
        assert!(inventory.plan_destroy(MoveRules::EqEmu, &changed).is_err());
    }

    #[test]
    fn containers_need_empty_known_contents_and_cursor_queue_is_not_destroyed() {
        let mut bag = item();
        bag.stack_count = None;
        bag.bag_slots = 4;
        let (mut inventory, mut request) = fixture(bag);
        assert!(inventory.plan_destroy(MoveRules::EqEmu, &request).is_ok());
        let mut content = item();
        content.slot = InventorySlot(331);
        inventory.apply(InventoryUpdate::Set(vec![content]));
        request.revision = inventory.revision();
        assert!(inventory.plan_destroy(MoveRules::EqEmu, &request).is_err());
        let (mut inventory, mut request) = fixture(item());
        inventory.apply(InventoryUpdate::Cursor(vec![item()]));
        request.revision = inventory.revision();
        assert!(inventory.plan_destroy(MoveRules::EqEmu, &request).is_err());
        assert_eq!(inventory.queued().count(), 1);
    }

    #[test]
    fn silence_does_not_confirm_destroy_or_lower_weight_and_received_removal_does() {
        let (mut inventory, request) = fixture(item());
        inventory.apply(inventory.plan_destroy(MoveRules::EqEmu, &request).unwrap());
        assert!(!inventory.items().contains_key(&InventorySlot::CURSOR));
        assert!(inventory.predicted());
        assert!(inventory.carried_weight_is_conservative());
        assert_eq!(
            inventory.carried_weight_tenths(GameDialect::Titanium, 0),
            Ok(15)
        );
        assert!(inventory.plan_destroy(MoveRules::EqEmu, &request).is_err());
        inventory.apply(InventoryUpdate::Settled);
        assert!(inventory.predicted());
        assert_eq!(inventory.prediction_origins().count(), 1);
        assert_eq!(
            inventory.carried_weight_tenths(GameDialect::Titanium, 0),
            Ok(15)
        );
        inventory.apply(InventoryUpdate::Remove(InventorySlot::CURSOR));
        assert!(!inventory.predicted());
        assert_eq!(
            inventory.carried_weight_tenths(GameDialect::Titanium, 0),
            Ok(0)
        );
    }

    #[test]
    fn correction_restores_item_and_late_unversioned_removal_cannot_delete_new_arrival() {
        let (mut inventory, request) = fixture(item());
        inventory.apply(inventory.plan_destroy(MoveRules::EqEmu, &request).unwrap());
        inventory.apply(InventoryUpdate::Set(vec![item()]));
        assert!(!inventory.predicted());
        assert_eq!(
            inventory.items()[&InventorySlot::CURSOR].stack_count,
            Some(7)
        );
        let (mut inventory, request) = fixture(item());
        inventory.apply(inventory.plan_destroy(MoveRules::EqEmu, &request).unwrap());
        let mut arrival = item();
        arrival.details.id = 99;
        inventory.apply(InventoryUpdate::Cursor(vec![arrival.clone()]));
        inventory.apply(InventoryUpdate::Remove(InventorySlot::CURSOR));
        assert!(inventory.stale());
        assert_eq!(inventory.items()[&InventorySlot::CURSOR], arrival);
        inventory.apply(InventoryUpdate::Snapshot(vec![arrival]));
        assert!(!inventory.stale());
        assert!(!inventory.predicted());
    }
}
