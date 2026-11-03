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
    inventory::{Inventory, InventorySlot, InventoryUpdate},
    message::Message,
    objects::{ContainerView, ObjectUpdate, Objects},
    request::Request,
    world::{Position, WorldEvent},
};
use std::{collections::BTreeMap, time::Instant};

/// Where a picked-up item arrives.
const CURSOR: InventorySlot = InventorySlot(30);

/// Keeps the zone's object table and picks items up.
#[derive(Debug)]
pub(super) struct GroundObjects {
    results: PickupResults,
}

/// Preserve the established Titanium path while `EQMac`'s separate result and
/// cursor delivery are integrated. This is a generation interface, not a
/// server-name check in the controller.
#[derive(Debug)]
enum PickupResults {
    Existing,
    SeparateCursorDelivery,
}

impl Default for GroundObjects {
    fn default() -> Self {
        Self::for_generation(eq_network_game::GameDialect::Titanium)
    }
}

impl GroundObjects {
    /// The generation's pickup result ownership. Server policy chooses this
    /// when offering the feature; no `EQMac` capability is enabled by decoding.
    pub(super) fn for_generation(generation: eq_network_game::GameDialect) -> Self {
        Self {
            results: match generation {
                eq_network_game::GameDialect::EqMac => PickupResults::SeparateCursorDelivery,
                _ => PickupResults::Existing,
            },
        }
    }
}

impl PickupResults {
    /// Only the separately evidenced result path introduces a pending pickup.
    fn sent(&self, drop_id: u32, player_id: u16) -> Option<Pickup> {
        match self {
            Self::Existing => None,
            Self::SeparateCursorDelivery => Some(Pickup {
                drop_id,
                player_id: u32::from(player_id),
                removed: false,
                delivered: false,
            }),
        }
    }
}

/// The zone's objects in this admission. Every feature reads it; only this
/// module changes it.
#[derive(Debug, Default)]
pub(super) struct ZoneObjects {
    table: Objects,
    /// An ID may be placed again after removal or a model/position change.
    /// A request made before that placement must not act on its replacement.
    placed_at: BTreeMap<u32, Instant>,
    /// The click reply cannot identify a refused object, so only one pickup
    /// may be unresolved. This is not a predicted item or a move acknowledgement.
    pickup: Option<Pickup>,
}

/// Visibility and inventory arrive independently, in either order.
#[derive(Debug)]
struct Pickup {
    drop_id: u32,
    player_id: u32,
    removed: bool,
    delivered: bool,
}

impl std::ops::Deref for ZoneObjects {
    type Target = Objects;

    fn deref(&self) -> &Objects {
        &self.table
    }
}

/// Lets another feature's tests start with objects in the zone.
#[cfg(test)]
impl From<Objects> for ZoneObjects {
    fn from(objects: Objects) -> Self {
        let mut state = Self::default();
        state.apply(&objects.admission());
        state
    }
}

impl ZoneObjects {
    /// Records placements alongside the table through this module's one owner.
    fn apply(&mut self, update: &ObjectUpdate) {
        let now = Instant::now();
        match update {
            ObjectUpdate::Snapshot(objects) => {
                self.placed_at = objects.iter().map(|object| (object.drop_id, now)).collect();
            }
            ObjectUpdate::Spawn(object) => {
                self.placed_at.insert(object.drop_id, now);
            }
            ObjectUpdate::Remove { drop_id, .. } => {
                self.placed_at.remove(drop_id);
            }
            ObjectUpdate::Kept { .. }
            | ObjectUpdate::Container(_)
            | ObjectUpdate::Closed { .. } => (),
        }
        self.table.apply(update);
    }

    /// A host request names the current placement, not an earlier use of its ID.
    fn check_current(&self, drop_id: u32, created: Instant) -> Result<()> {
        ensure!(self.pickup.is_none(), "wait for the ground pickup result");
        let placed = self
            .placed_at
            .get(&drop_id)
            .ok_or_else(|| anyhow!("that is no longer there"))?;
        ensure!(created >= *placed, "stale ground object request");
        Ok(())
    }

    /// Only a matching server result ends the unresolved click. The separate
    /// inventory owner applies an item; this owner only records that it arrived.
    fn pickup_news(&mut self, event: &WorldEvent) {
        let Some(pickup) = &mut self.pickup else {
            return;
        };
        match event {
            WorldEvent::Objects(ObjectUpdate::Kept { player_id })
                if *player_id == pickup.player_id =>
            {
                self.pickup = None;
                return;
            }
            WorldEvent::Objects(ObjectUpdate::Remove { drop_id, taken_by })
                if *drop_id == pickup.drop_id =>
            {
                if *taken_by != Some(pickup.player_id) {
                    // Visibility alone cannot answer our click: a later
                    // unidentified refusal could otherwise settle a newer one.
                    return;
                }
                pickup.removed = true;
            }
            WorldEvent::Inventory(InventoryUpdate::Cursor(items))
                if items.iter().any(|item| item.slot == CURSOR) =>
            {
                pickup.delivered = true;
            }
            _ => (),
        }
        if pickup.removed && pickup.delivered {
            self.pickup = None;
        }
    }
}

impl Feature for GroundObjects {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![Capability::GroundItems]
    }

    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        if let Message::Event(WorldEvent::Objects(update)) = message {
            world.objects.apply(update);
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

    /// The incoming cursor item is still unknown until its authoritative
    /// delivery. A timeout never authorizes a retry or invents an empty cursor.
    fn holds(&self, world: &World, _now: Instant) -> Vec<(super::actions::Resource, &'static str)> {
        world
            .objects
            .pickup
            .as_ref()
            .map(|_| {
                (
                    super::actions::Resource::Inventory,
                    "Wait for the ground pickup result",
                )
            })
            .into_iter()
            .collect()
    }

    /// Picks up an item from the ground.
    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::PickUp {
            drop_id, created, ..
        } = command
        else {
            return Ok(());
        };
        let checked = world
            .objects
            .check_current(*drop_id, *created)
            .and_then(|()| {
                pickup(
                    &world.objects,
                    *drop_id,
                    world.player_at(),
                    &world.inventory,
                    out.wire.reaches().objects,
                )
            });
        let error = match checked {
            Ok(request) => {
                let (player_id, _) = world
                    .player_at()
                    .ok_or_else(|| anyhow!("player is unavailable"))?;
                out.request(&request)?;
                world.objects.pickup = self.results.sent(*drop_id, player_id);
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
        let Message::Event(event) = message else {
            return Ok(());
        };
        world.objects.pickup_news(event);
        let WorldEvent::Objects(update) = event else {
            return Ok(());
        };
        world.objects.apply(update);
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
/// cursor, so the current projection must be empty. Unrelated move uncertainty
/// does not hold another action; the inventory owner retains that uncertainty.
fn pickup(
    objects: &Objects,
    drop_id: u32,
    player: Option<(u16, Position)>,
    inventory: &Inventory,
    reach: eq_network_game::reach::Reach,
) -> Result<Request> {
    ensure!(
        inventory.received() && !inventory.stale(),
        "the inventory is not loaded"
    );
    ensure!(
        !inventory.items().contains_key(&CURSOR),
        "put down the item on your cursor first"
    );
    let (spawn_id, position) = player.ok_or_else(|| anyhow!("player is unavailable"))?;
    objects.check_pickup(drop_id, spawn_id, position, reach)?;
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
        GroundObjects::default()
            .admit(&Message::Event(spawn), &mut world)
            .unwrap();
        world.objects.table
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

    const REACH: eq_network_game::reach::Reach = eq_network_game::reach::TITANIUM.objects;

    #[test]
    fn pickups_need_a_loaded_inventory_and_an_empty_cursor() {
        let objects = table();
        let player = Some((9, Position::default()));
        assert_eq!(
            pickup(&objects, 71, player, &loaded(false), REACH).unwrap(),
            Request::PickUp(71)
        );
        let error = |player, inventory: &Inventory| {
            pickup(&objects, 71, player, inventory, REACH)
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
    fn a_predicted_empty_cursor_does_not_hold_pickups_back() {
        let objects = table();
        let mut inventory = loaded(false);
        inventory.apply(InventoryUpdate::Prediction(vec![item(InventorySlot(23))]));
        assert_eq!(
            pickup(
                &objects,
                71,
                Some((9, Position::default())),
                &inventory,
                REACH
            )
            .unwrap(),
            Request::PickUp(71)
        );
    }
}

#[cfg(test)]
#[path = "objects/eqmac_tests.rs"]
mod eqmac_tests;
