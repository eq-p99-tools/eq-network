//! Items on the ground: the zone's object table, pickups, and world containers,
//! which are not supported yet, so one that opens for this player is closed again.
use super::{ClientCommand, ClientEvent, Events, Session};
use anyhow::{anyhow, ensure, Result};
use eq_network_game::{
    inventory::{Inventory, InventorySlot},
    objects::{ObjectUpdate, Objects, CLICK_OPCODE, CONTAINER_OPCODE},
    world::{Position, WorldEvent},
};
use std::time::{Duration, Instant};

/// Where a picked-up item arrives.
const CURSOR: InventorySlot = InventorySlot(30);

/// The zone's objects in this admission.
#[derive(Debug, Default)]
pub(super) struct GroundObjects(Objects);

impl GroundObjects {
    /// Records an update received before the zone is ready.
    pub(super) fn apply(&mut self, update: &ObjectUpdate) {
        self.0.apply(update);
    }

    /// The table to report when the zone becomes ready.
    pub(super) fn admission(&self) -> ObjectUpdate {
        self.0.admission()
    }

    /// Records a server update, and closes a container that opened for this
    /// player, so the server does not keep it in use.
    pub(super) fn observe(
        &mut self,
        update: &ObjectUpdate,
        own_spawn: Option<u16>,
        session: &mut Session,
        log: &mut Events<'_>,
    ) -> Result<()> {
        self.0.apply(update);
        if let ObjectUpdate::Container(view) = update {
            if view.open && own_spawn.is_some_and(|id| u32::from(id) == view.player_id) {
                session.send(CONTAINER_OPCODE, &view.close_packet())?;
                log.diagnostic(format!(
                    "Closed world container {}: containers are not supported yet",
                    view.drop_id
                ))?;
            }
        }
        Ok(())
    }

    /// Handles a pickup request; false for any other command.
    pub(super) fn handle(
        &self,
        command: &ClientCommand,
        session_id: u64,
        player: Option<(u16, Position)>,
        inventory: &Inventory,
        session: &mut Session,
        log: &mut Events<'_>,
    ) -> Result<bool> {
        let ClientCommand::PickUp {
            session_id: requested,
            drop_id,
            created,
        } = command
        else {
            return Ok(false);
        };
        let pickup = Pickup {
            requested: *requested,
            drop_id: *drop_id,
            created: *created,
        };
        let error = match self.pickup(&pickup, session_id, player, inventory, Instant::now()) {
            Ok(body) => {
                session.send(CLICK_OPCODE, &body)?;
                None
            }
            Err(error) => Some(error.to_string()),
        };
        log.send(ClientEvent::World(WorldEvent::ObjectAction {
            session_id,
            drop_id: *drop_id,
            error,
        }))?;
        Ok(true)
    }

    /// Checks a pickup against the admission, the cursor and the reach. The
    /// item arrives on the cursor, so it must be empty, and an unsettled move
    /// could still change it.
    fn pickup(
        &self,
        request: &Pickup,
        session_id: u64,
        player: Option<(u16, Position)>,
        inventory: &Inventory,
        now: Instant,
    ) -> Result<[u8; 8]> {
        ensure!(
            request.requested == session_id
                && request.created <= now
                && now.duration_since(request.created) < Duration::from_secs(1),
            "stale pickup request"
        );
        ensure!(
            inventory.received() && !inventory.stale(),
            "the inventory is not loaded"
        );
        ensure!(
            !inventory.predicted(),
            "wait for the last item move to settle"
        );
        ensure!(
            !inventory.items().contains_key(&CURSOR),
            "put down the item on your cursor first"
        );
        let (spawn_id, position) = player.ok_or_else(|| anyhow!("player is unavailable"))?;
        self.0.pickup_packet(request.drop_id, spawn_id, position)
    }
}

struct Pickup {
    requested: u64,
    drop_id: u32,
    created: Instant,
}

#[cfg(test)]
mod tests {
    use super::*;
    use eq_network_game::{
        inventory::{InventoryItem, InventoryUpdate},
        objects::GroundObject,
    };

    fn table() -> GroundObjects {
        let mut objects = GroundObjects::default();
        objects.apply(&ObjectUpdate::Spawn(GroundObject {
            drop_id: 71,
            model: "IT63_ACTORDEF".into(),
            position: Position::default(),
            object_type: 0,
        }));
        objects
    }

    fn item(slot: InventorySlot) -> InventoryItem {
        InventoryItem {
            activation: eq_network_game::inventory::ItemActivation::default(),
            scroll_spell: None,
            rules: eq_network_game::inventory::ItemPlacement::default(),
            slot,
            icon: 0,
            stack_count: None,
            charges: 0,
            bag_slots: 0,
            details: eq_network_game::items::ItemDetails {
                equipment: None,
                bonuses: None,
                id: 42,
                name: "Synthetic item".into(),
                lore: String::new(),
                weight_tenths: 0,
                slots: 0,
                classes: 0,
                races: 0,
                flags: vec![],
                stats: vec![],
            },
        }
    }

    fn loaded(cursor: bool) -> Inventory {
        let mut inventory = Inventory::default();
        let items = if cursor {
            vec![item(CURSOR)]
        } else {
            Vec::new()
        };
        inventory.apply(InventoryUpdate::Snapshot(items));
        inventory
    }

    #[test]
    fn pickups_need_a_fresh_request_a_loaded_inventory_and_an_empty_cursor() {
        let objects = table();
        let now = Instant::now();
        let player = Some((9, Position::default()));
        let request = Pickup {
            requested: 5,
            drop_id: 71,
            created: now,
        };
        assert_eq!(
            objects
                .pickup(&request, 5, player, &loaded(false), now)
                .unwrap(),
            [71, 0, 0, 0, 9, 0, 0, 0]
        );
        let error = |request: &Pickup, player, inventory: &Inventory, at| {
            objects
                .pickup(request, 5, player, inventory, at)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(
            error(&request, player, &loaded(true), now),
            "put down the item on your cursor first"
        );
        assert_eq!(
            error(&request, player, &Inventory::default(), now),
            "the inventory is not loaded"
        );
        assert_eq!(
            error(&request, None, &loaded(false), now),
            "player is unavailable"
        );
        assert_eq!(
            error(
                &request,
                player,
                &loaded(false),
                now + Duration::from_secs(1)
            ),
            "stale pickup request"
        );
        let other_session = Pickup {
            requested: 4,
            ..request
        };
        assert_eq!(
            error(&other_session, player, &loaded(false), now),
            "stale pickup request"
        );
    }

    #[test]
    fn an_unsettled_move_holds_pickups_back() {
        let objects = table();
        let now = Instant::now();
        let mut inventory = loaded(false);
        inventory.apply(InventoryUpdate::Prediction(vec![item(InventorySlot(23))]));
        let request = Pickup {
            requested: 5,
            drop_id: 71,
            created: now,
        };
        assert_eq!(
            objects
                .pickup(&request, 5, Some((9, Position::default())), &inventory, now)
                .unwrap_err()
                .to_string(),
            "wait for the last item move to settle"
        );
    }
}
