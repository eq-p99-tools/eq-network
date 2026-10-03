//! Items on the ground: the zone's object table, which every feature reads,
//! and pickups. A world container such as a forge is the tradeskills
//! feature's to open and close; one that opens for this player without being
//! asked for is closed again, so the server does not keep it in use.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{anyhow, ensure, Result};
use eq_network_game::{
    inventory::{Inventory, InventorySlot},
    message::Message,
    objects::{ContainerView, ObjectUpdate, Objects},
    request::Request,
    world::{Position, WorldEvent},
};

/// Where a picked-up item arrives.
const CURSOR: InventorySlot = InventorySlot(30);

/// Keeps the zone's object table and picks items up.
#[derive(Debug, Default)]
pub(super) struct GroundObjects;

/// The zone's objects in this admission. Every feature reads it; only this
/// module changes it.
#[derive(Debug, Default)]
pub(super) struct ZoneObjects(Objects);

impl std::ops::Deref for ZoneObjects {
    type Target = Objects;

    fn deref(&self) -> &Objects {
        &self.0
    }
}

/// Lets another feature's tests start with objects in the zone.
#[cfg(test)]
impl From<Objects> for ZoneObjects {
    fn from(objects: Objects) -> Self {
        Self(objects)
    }
}

impl Feature for GroundObjects {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![Capability::GroundItems]
    }

    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        if let Message::Event(WorldEvent::Objects(update)) = message {
            world.objects.0.apply(update);
        }
        Ok(())
    }

    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        out.log.send(ClientEvent::World(WorldEvent::Objects(
            world.objects.admission(),
        )))
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::PickUp { .. })
    }

    /// Picks up an item from the ground.
    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::PickUp { drop_id, .. } = command else {
            return Ok(());
        };
        let checked = pickup(
            &world.objects,
            *drop_id,
            world.player_at(),
            &world.inventory,
        );
        let error = match checked {
            Ok(request) => {
                out.request(&request)?;
                None
            }
            Err(error) => Some(error.to_string()),
        };
        out.log.send(ClientEvent::World(WorldEvent::ObjectAction {
            session_id: world.session_id,
            drop_id: *drop_id,
            error,
        }))
    }

    /// Records a server update, and closes a container that opened for this
    /// player unasked, so the server does not keep it in use.
    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let Message::Event(WorldEvent::Objects(update)) = message else {
            return Ok(());
        };
        world.objects.0.apply(update);
        let ObjectUpdate::Container(view) = update else {
            return Ok(());
        };
        if view.open && world.is_player(view.player_id) && !world.container.expects(view.drop_id) {
            out.request(&close_request(view))?;
            out.log.diagnostic(format!(
                "Closed world container {}, which opened unasked",
                view.drop_id
            ))?;
        }
        Ok(())
    }
}

/// `OP_ClickObjectAction` closing a container: its own record, repeated.
pub(super) fn close_request(view: &ContainerView) -> Request {
    Request::CloseContainer {
        drop_id: view.drop_id,
        object_type: view.object_type,
        icon: view.icon,
        name: view.name.clone(),
    }
}

/// Checks a pickup against the cursor and the reach. The item arrives on the
/// cursor, so it must be empty, and an unsettled move could still change it.
fn pickup(
    objects: &Objects,
    drop_id: u32,
    player: Option<(u16, Position)>,
    inventory: &Inventory,
) -> Result<Request> {
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
    objects.check_pickup(drop_id, spawn_id, position)?;
    Ok(Request::PickUp(drop_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eq_network_game::{
        inventory::{InventoryItem, InventoryUpdate},
        objects::GroundObject,
    };

    fn table() -> Objects {
        let mut world = World::new(5);
        let spawn = WorldEvent::Objects(ObjectUpdate::Spawn(GroundObject {
            drop_id: 71,
            model: "IT63_ACTORDEF".into(),
            position: Position::default(),
            object_type: 0,
        }));
        GroundObjects
            .admit(&Message::Event(spawn), &mut world)
            .unwrap();
        world.objects.0
    }

    fn item(slot: InventorySlot) -> InventoryItem {
        InventoryItem {
            activation: eq_network_game::inventory::ItemActivation::default(),
            scroll_spell: None,
            book: None,
            rules: eq_network_game::inventory::ItemPlacement::default(),
            slot,
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
                price: None,
                icon: None,
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
    fn pickups_need_a_loaded_inventory_and_an_empty_cursor() {
        let objects = table();
        let player = Some((9, Position::default()));
        assert_eq!(
            pickup(&objects, 71, player, &loaded(false)).unwrap(),
            Request::PickUp(71)
        );
        let error = |player, inventory: &Inventory| {
            pickup(&objects, 71, player, inventory)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(
            error(player, &loaded(true)),
            "put down the item on your cursor first"
        );
        assert_eq!(
            error(player, &Inventory::default()),
            "the inventory is not loaded"
        );
        assert_eq!(error(None, &loaded(false)), "player is unavailable");
    }

    #[test]
    fn an_unsettled_move_holds_pickups_back() {
        let objects = table();
        let mut inventory = loaded(false);
        inventory.apply(InventoryUpdate::Prediction(vec![item(InventorySlot(23))]));
        assert_eq!(
            pickup(&objects, 71, Some((9, Position::default())), &inventory)
                .unwrap_err()
                .to_string(),
            "wait for the last item move to settle"
        );
    }
}
