//! Tradeskill combines. The session sends a combine for a tradeskill
//! container carried in a pack slot, and holds the inventory until the
//! server answers: the components leave and what was made arrives only after
//! that answer, so a move or a second combine in between would act on what
//! the server is about to change. As the official client does, it refuses a
//! combine while the cursor holds an item or coins, where what was made
//! would land, and names the official client's own words for that.
use super::{
    actions::{self, Resource},
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    inventory::InventorySlot,
    message::Message,
    request::Request,
    tradeskills::{self, CombineUpdate, HANDS_FULL},
    world::WorldEvent,
};
use std::time::Instant;

/// Combines in the player's own tradeskill containers.
#[derive(Default)]
pub(super) struct Tradeskills {
    /// The container whose combine waits for the server's answer.
    combining: Option<InventorySlot>,
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
        matches!(command, ClientCommand::Combine { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::Combine {
            session_id,
            container,
            ..
        } = *command
        else {
            return Ok(());
        };
        let combines = world
            .inventory
            .items()
            .get(&container)
            .is_some_and(tradeskills::can_combine_in);
        if !combines {
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

    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::Event(WorldEvent::Combine(CombineUpdate::Answered)) = message {
            self.combining = None;
        }
        Ok(())
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
