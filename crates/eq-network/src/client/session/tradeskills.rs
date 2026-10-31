//! Tradeskill combines, and the world containers they happen in. The player
//! opens a world container such as a forge within reach, and closes it,
//! when the server puts what it still holds back in the inventory. The
//! session sends a combine for a tradeskill container carried in a pack
//! slot, or for the world container open for the player, and holds the
//! inventory until the server answers: the components leave and what was
//! made arrives only after that answer, so a move or a second combine in
//! between would act on what the server is about to change. As the official
//! client does, it refuses a combine while the cursor holds an item or
//! coins, where what was made would land, and names the official client's
//! own words for that.
use super::{
    actions::{self, Resource},
    feature::{Feature, Out, World},
    objects::close_request,
    ClientCommand, ClientEvent,
};
use anyhow::{anyhow, Result};
use eq_network_game::{
    inventory::InventorySlot,
    message::Message,
    objects::{ContainerView, ObjectUpdate},
    request::Request,
    tradeskills::{self, CombineUpdate, HANDS_FULL, WORLD_CONTAINER},
    world::WorldEvent,
};
use std::time::Instant;

/// The world container the player asked to open, or has open. Every
/// feature reads it; only this module changes it.
#[derive(Debug, Default)]
pub(super) struct OpenContainer(Option<Container>);

/// Where the player's world container stands.
#[derive(Clone, Debug)]
enum Container {
    /// Asked to open; the server has not answered.
    Asked(u32),
    /// Open for the player.
    Open(ContainerView),
}

impl OpenContainer {
    /// Whether a world container is open for the player.
    pub(super) const fn is_open(&self) -> bool {
        matches!(self.0, Some(Container::Open(_)))
    }

    /// Whether the player asked to open this container and waits for it.
    pub(super) const fn expects(&self, drop_id: u32) -> bool {
        matches!(self.0, Some(Container::Asked(asked)) if asked == drop_id)
    }
}

/// Lets another feature's tests start with a container open.
#[cfg(test)]
impl From<ContainerView> for OpenContainer {
    fn from(view: ContainerView) -> Self {
        Self(Some(Container::Open(view)))
    }
}

/// Combines in the player's tradeskill containers, and opens and closes
/// world containers.
#[derive(Default)]
pub(super) struct Tradeskills {
    /// The container whose combine waits for the server's answer.
    combining: Option<InventorySlot>,
    /// When the open container's reach was last checked.
    reach_checked: Option<Instant>,
}

impl Feature for Tradeskills {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Tradeskills]
    }

    fn holds(&self, _world: &World, _now: Instant) -> Vec<(Resource, &'static str)> {
        self.combining
            .map(|_| (Resource::Inventory, "Wait for the combine to finish"))
            .into_iter()
            .collect()
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::Combine { .. }
                | ClientCommand::OpenContainer { .. }
                | ClientCommand::CloseContainer { .. }
        )
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let (session_id, container) = match *command {
            ClientCommand::Combine {
                session_id,
                container,
                ..
            } => (session_id, container),
            ClientCommand::OpenContainer { drop_id, .. } => return open(drop_id, world, out),
            ClientCommand::CloseContainer { .. } => return close(world, out),
            _ => return Ok(()),
        };
        if container == WORLD_CONTAINER {
            if !world.container.is_open() {
                return actions::refuse(command, "No world container is open.", out.log);
            }
        } else if !world
            .inventory
            .items()
            .get(&container)
            .is_some_and(tradeskills::can_combine_in)
        {
            return actions::refuse(command, "That is not a tradeskill container.", out.log);
        }
        if world.inventory.items().contains_key(&InventorySlot::CURSOR)
            || !world.coins.cursor.is_empty()
        {
            let reason = "Your cursor must be empty to combine.";
            out.log
                .send(ClientEvent::World(WorldEvent::CombineRefused {
                    session_id,
                    reason: reason.into(),
                    string_id: Some(HANDS_FULL),
                }))?;
            return out.log.diagnostic(reason.into());
        }
        out.request(&Request::Combine(container))?;
        self.combining = Some(container);
        out.log.send(ClientEvent::World(WorldEvent::Combine(
            CombineUpdate::Started(container),
        )))
    }

    /// Hears the server answer a combine, and open the container the player
    /// asked for or say that someone else is using it.
    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let reach = out.wire.reaches().containers;
        keep_in_reach((&mut self.reach_checked, reach), now, world, out)
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match message {
            Message::Event(WorldEvent::Combine(CombineUpdate::Answered)) => self.combining = None,
            Message::Event(WorldEvent::Objects(ObjectUpdate::Container(view)))
                if world.is_player(view.player_id) && world.container.expects(view.drop_id) =>
            {
                world.container.0 = view.open.then(|| Container::Open(view.clone()));
            }
            _ => (),
        }
        Ok(())
    }
}

/// Opens a world container within reach, unless one is open or asked for
/// already; the host hears how the request went as it does for a pickup,
/// and the server's answer as a container update.
fn open(drop_id: u32, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
    let checked = if world.container.0.is_some() {
        Err(anyhow!("close the container you are using first"))
    } else {
        world
            .player_at()
            .ok_or_else(|| anyhow!("player is unavailable"))
            .and_then(|(spawn_id, position)| {
                world
                    .objects
                    .check_open(drop_id, spawn_id, position, out.wire.reaches().objects)
            })
    };
    let error = match checked {
        Ok(()) => {
            out.request(&Request::OpenContainer(drop_id))?;
            world.container.0 = Some(Container::Asked(drop_id));
            None
        }
        Err(error) => Some(error.to_string()),
    };
    out.log.send(ClientEvent::World(WorldEvent::ObjectAction {
        session_id: world.session_id,
        drop_id,
        error,
    }))
}

/// Closes the world container open for the player once they are out of the
/// generation's reach of it, checked as often as its program checks
/// ([`eq_network_game::reach::CONTAINER_CHECK`]), and tells the host.
fn keep_in_reach(
    (checked, reach): (&mut Option<Instant>, Option<eq_network_game::reach::Reach>),
    now: Instant,
    world: &mut World,
    out: &mut Out<'_, '_>,
) -> Result<()> {
    let Some(reach) = reach else {
        return Ok(());
    };
    if checked.is_some_and(|at| {
        now.saturating_duration_since(at) <= eq_network_game::reach::CONTAINER_CHECK
    }) {
        return Ok(());
    }
    *checked = Some(now);
    let Some(Container::Open(view)) = &world.container.0 else {
        return Ok(());
    };
    let drop_id = view.drop_id;
    let (Some(object), Some((_, position))) =
        (world.objects.entries().get(&drop_id), world.player_at())
    else {
        return Ok(());
    };
    if reach.admits(position, object.position) {
        return Ok(());
    }
    close(world, out)?;
    out.log.send(ClientEvent::World(WorldEvent::Objects(
        ObjectUpdate::Closed { drop_id },
    )))
}

/// Closes the world container open for the player; the server puts what it
/// still holds back in the inventory. One still waiting for the server's
/// answer is forgotten, and that answer closed when it comes.
fn close(world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
    match world.container.0.take() {
        Some(Container::Open(view)) => out.request(&close_request(&view)),
        Some(Container::Asked(_)) | None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::{
        inventory::{Inventory, InventoryUpdate},
        tradeskills::titanium_combine,
    };

    fn combine(container: i32) -> ClientCommand {
        ClientCommand::Combine {
            session_id: 5,
            container: InventorySlot(container),
            created: Instant::now(),
        }
    }

    /// A sewing kit in pack slot 23 and a backpack in 24.
    fn world() -> World {
        let mut world = World::new(5);
        let mut kit = testing::item(23);
        kit.bag_slots = 10;
        kit.rules.bag_type = 16;
        let mut backpack = testing::item(24);
        backpack.bag_slots = 8;
        backpack.rules.bag_type = 5;
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![kit, backpack]));
        world.inventory = inventory.into();
        world
    }

    fn refused<R>(outcome: &testing::Outcome<R>) -> bool {
        outcome
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::CombineRefused { .. })))
    }

    /// The two features a world container involves, in the session's order.
    #[derive(Default)]
    struct Features {
        objects: super::super::objects::GroundObjects,
        tradeskills: Tradeskills,
    }

    /// A forge (5) beside the admitted player (7).
    fn at_forge() -> (Features, World) {
        let mut world = World::new(5);
        let mut table = eq_network_game::objects::Objects::default();
        table.apply(&ObjectUpdate::Spawn(
            eq_network_game::objects::GroundObject {
                drop_id: 5,
                model: "FORGE".into(),
                position: eq_network_game::world::Position::default(),
                object_type: 17,
            },
        ));
        world.objects = table.into();
        world.player.admit(testing::player(7));
        world.own_spawn = Some(7);
        (Features::default(), world)
    }

    fn view(open: bool) -> ContainerView {
        ContainerView {
            player_id: 7,
            drop_id: 5,
            open,
            object_type: 17,
            icon: 1,
            name: "Forge".into(),
        }
    }

    /// How many packets a command sent.
    fn command(command: &ClientCommand, features: &mut Features, world: &mut World) -> usize {
        let outcome = testing::run(|out| features.tradeskills.handle(command, world, out));
        outcome.result.unwrap();
        outcome.sent.len()
    }

    /// How many packets hearing an update sent, as the session offers it to
    /// the ground objects feature and then to this one.
    fn hear(update: ObjectUpdate, features: &mut Features, world: &mut World) -> usize {
        let message = Message::Event(WorldEvent::Objects(update));
        let outcome = testing::run(|out| {
            features.objects.observe(&message, world, out)?;
            features.tradeskills.observe(&message, world, out)
        });
        outcome.result.unwrap();
        outcome.sent.len()
    }

    fn open_forge() -> ClientCommand {
        ClientCommand::OpenContainer {
            session_id: 5,
            drop_id: 5,
            created: Instant::now(),
        }
    }

    #[test]
    fn a_mac_era_container_closes_once_the_player_is_out_of_reach() {
        let (mut features, mut world) = at_forge();
        command(&open_forge(), &mut features, &mut world);
        hear(
            ObjectUpdate::Container(view(true)),
            &mut features,
            &mut world,
        );
        let mut player = testing::player(7);
        player.position.x = 20.0;
        world.player.admit(player);
        let start = Instant::now();
        // TAKP's reach (its containers aren't encoded yet, so Titanium's
        // packets carry the close here).
        let reach = eq_network_game::reach::EQMAC.containers;
        let mut checked = None;
        let mut tick = |world: &mut World, reach, now| {
            let outcome = testing::run(|out| keep_in_reach((&mut checked, reach), now, world, out));
            assert!(outcome.result.is_ok());
            outcome
        };
        // At 20 the container stays open.
        let outcome = tick(&mut world, reach, start);
        assert!(outcome.sent.is_empty() && world.container.is_open());
        let mut player = testing::player(7);
        player.position.x = 20.5;
        world.player.admit(player);
        // Titanium's reach is unread: it never closes one.
        let soon = start + std::time::Duration::from_millis(500);
        let outcome = tick(&mut world, None, soon);
        assert!(outcome.sent.is_empty() && world.container.is_open());
        // Checked again only more than 500 ms on, and then closed.
        let outcome = tick(&mut world, reach, soon);
        assert!(outcome.sent.is_empty() && world.container.is_open());
        let later = start + std::time::Duration::from_millis(501);
        let outcome = tick(&mut world, reach, later);
        assert_eq!(outcome.sent.len(), 1);
        assert!(!world.container.is_open());
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::Objects(
                ObjectUpdate::Closed { drop_id: 5 }
            ))]
        ));
    }

    #[test]
    fn a_forge_opens_when_asked_and_closes_when_the_player_closes_it() {
        let (mut features, mut world) = at_forge();
        assert_eq!(command(&open_forge(), &mut features, &mut world), 1);
        // A second container waits until this one closes.
        assert_eq!(command(&open_forge(), &mut features, &mut world), 0);
        assert!(!world.container.is_open());
        let opened = ObjectUpdate::Container(view(true));
        assert_eq!(hear(opened, &mut features, &mut world), 0);
        assert!(world.container.is_open());
        let close = ClientCommand::CloseContainer { session_id: 5 };
        assert_eq!(command(&close, &mut features, &mut world), 1);
        assert!(!world.container.is_open());
        // Closing nothing sends nothing.
        assert_eq!(command(&close, &mut features, &mut world), 0);
    }

    #[test]
    fn a_container_in_use_stays_shut_and_one_opened_unasked_is_closed() {
        let (mut features, mut world) = at_forge();
        command(&open_forge(), &mut features, &mut world);
        // Someone else is using it.
        let in_use = ObjectUpdate::Container(view(false));
        assert_eq!(hear(in_use, &mut features, &mut world), 0);
        assert!(!world.container.is_open());
        assert_eq!(command(&open_forge(), &mut features, &mut world), 1);
        let close = ClientCommand::CloseContainer { session_id: 5 };
        command(&close, &mut features, &mut world);
        // An answer nobody waits for is closed again.
        let opened = ObjectUpdate::Container(view(true));
        assert_eq!(hear(opened, &mut features, &mut world), 1);
        assert!(!world.container.is_open());
    }

    #[test]
    fn a_tradeskill_container_combines_and_holds_the_inventory_until_answered() {
        let mut tradeskills = Tradeskills::default();
        let mut world = world();
        let outcome = testing::run(|out| tradeskills.handle(&combine(23), &mut world, out));
        assert_eq!(outcome.sent, [titanium_combine(InventorySlot(23)).unwrap()]);
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Combine(CombineUpdate::Started(InventorySlot(
                23
            ))))
        )));
        let now = Instant::now();
        assert_eq!(
            tradeskills.holds(&world, now),
            [(Resource::Inventory, "Wait for the combine to finish")]
        );
        testing::run(|out| {
            tradeskills.observe(
                &Message::Event(WorldEvent::Combine(CombineUpdate::Answered)),
                &mut world,
                out,
            )
        })
        .result
        .unwrap();
        assert_eq!(tradeskills.holds(&world, now), []);
    }

    #[test]
    fn a_full_cursor_refuses_the_combine() {
        let mut tradeskills = Tradeskills::default();
        let mut on_cursor = world();
        let mut inventory = Inventory::default();
        let mut kit = testing::item(23);
        kit.bag_slots = 10;
        kit.rules.bag_type = 16;
        inventory.apply(InventoryUpdate::Snapshot(vec![
            kit,
            testing::item(InventorySlot::CURSOR.0),
        ]));
        on_cursor.inventory = inventory.into();
        let mut coins = world();
        coins.coins = eq_network_game::money::Wallet {
            cursor: eq_network_game::world::Coins {
                copper: 3,
                ..Default::default()
            },
            ..Default::default()
        }
        .into();
        for mut world in [on_cursor, coins] {
            let outcome = testing::run(|out| tradeskills.handle(&combine(23), &mut world, out));
            assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
            assert!(outcome.events.iter().any(|event| matches!(
                event,
                ClientEvent::World(WorldEvent::CombineRefused {
                    string_id: Some(HANDS_FULL),
                    ..
                })
            )));
        }
    }

    #[test]
    fn the_world_container_combines_only_while_open() {
        let mut tradeskills = Tradeskills::default();
        let mut world = world();
        let outcome =
            testing::run(|out| tradeskills.handle(&combine(WORLD_CONTAINER.0), &mut world, out));
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert!(refused(&outcome));
        world.container = ContainerView {
            player_id: 7,
            drop_id: 5,
            open: true,
            object_type: 17,
            icon: 1,
            name: "Forge".into(),
        }
        .into();
        let outcome =
            testing::run(|out| tradeskills.handle(&combine(WORLD_CONTAINER.0), &mut world, out));
        assert_eq!(outcome.sent, [titanium_combine(WORLD_CONTAINER).unwrap()]);
    }

    #[test]
    fn bags_and_empty_slots_are_refused() {
        let mut tradeskills = Tradeskills::default();
        let mut world = world();
        for container in [24, 25] {
            let outcome =
                testing::run(|out| tradeskills.handle(&combine(container), &mut world, out));
            assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
            assert!(refused(&outcome));
        }
        assert_eq!(tradeskills.holds(&world, Instant::now()), []);
    }
}
