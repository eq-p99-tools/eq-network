//! Items on the ground: the zone's object table, pickups, and world containers,
//! which are not supported yet, so one that opens for this player is closed again.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{anyhow, ensure, Result};
use eq_network_game::{
    command::EncodedCommand,
    inventory::{Inventory, InventorySlot},
    objects::{ObjectUpdate, Objects},
    world::{Position, WorldEvent},
};
use std::time::{Duration, Instant};

/// Where a picked-up item arrives.
const CURSOR: InventorySlot = InventorySlot(30);

/// The zone's objects in this admission.
#[derive(Debug, Default)]
pub(super) struct GroundObjects(Objects);

impl Feature for GroundObjects {
    fn admit(&mut self, event: &WorldEvent) {
        if let WorldEvent::Objects(update) = event {
            self.0.apply(update);
        }
    }

    fn admission(&self) -> Option<WorldEvent> {
        Some(WorldEvent::Objects(self.0.admission()))
    }

    /// Handles a pickup request; false for any other command.
    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
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
        let player = world.player_at();
        let checked = self.pickup(
            &pickup,
            world.session_id,
            player,
            &world.inventory,
            Instant::now(),
        );
        let error = match checked {
            Ok(packet) => {
                out.send(&packet)?;
                None
            }
            Err(error) => Some(error.to_string()),
        };
        out.log.send(ClientEvent::World(WorldEvent::ObjectAction {
            session_id: world.session_id,
            drop_id: *drop_id,
            error,
        }))?;
        Ok(true)
    }

    /// Records a server update, and closes a container that opened for this
    /// player, so the server does not keep it in use.
    fn observe(
        &mut self,
        event: &WorldEvent,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let WorldEvent::Objects(update) = event else {
            return Ok(());
        };
        self.0.apply(update);
        if let ObjectUpdate::Container(view) = update {
            if view.open && world.is_player(view.player_id) {
                out.send(&view.close_packet())?;
                out.log.diagnostic(format!(
                    "Closed world container {}: containers are not supported yet",
                    view.drop_id
                ))?;
            }
        }
        Ok(())
    }
}

impl GroundObjects {
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
    ) -> Result<EncodedCommand> {
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
        objects.admit(&WorldEvent::Objects(ObjectUpdate::Spawn(GroundObject {
            drop_id: 71,
            model: "IT63_ACTORDEF".into(),
            position: Position::default(),
            object_type: 0,
        })));
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
                .unwrap()
                .body,
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
