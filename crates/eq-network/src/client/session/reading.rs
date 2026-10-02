//! Reading books and notes. The session asks for the text a carried item
//! names, and refuses an item that is not readable; servers answer with the
//! text only when they have it, so an unknown text gets no answer.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::request::Request;

/// Asks for the texts of the player's books and notes.
pub(super) struct Reading;

impl Feature for Reading {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Reading]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::ReadItem { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::ReadItem { slot, .. } = *command else {
            return Ok(());
        };
        let book = world
            .inventory
            .items()
            .get(&slot)
            .and_then(|item| item.book.clone());
        match book {
            Some(book) => out.request(&Request::ReadBook(book)),
            None => actions::refuse(command, "That is not something you can read.", out.log),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;
    use eq_network_game::{
        books::{titanium_request, Book},
        inventory::{Inventory, InventorySlot, InventoryUpdate},
        world::WorldEvent,
    };

    fn read(slot: i32) -> ClientCommand {
        ClientCommand::ReadItem {
            session_id: 5,
            slot: InventorySlot(slot),
        }
    }

    #[test]
    fn a_carried_note_asks_for_its_text_and_anything_else_is_refused() {
        let mut world = World::new(5);
        let mut note = testing::item(23);
        note.book = Some(Book {
            file: "CHT_001".into(),
            kind: 0,
        });
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![
            note.clone(),
            testing::item(24),
        ]));
        world.inventory = inventory.into();
        let outcome = testing::run(|out| Reading.handle(&read(23), &mut world, out));
        assert_eq!(
            outcome.sent,
            [titanium_request(note.book.as_ref().unwrap()).unwrap()]
        );
        for slot in [24, 25] {
            let outcome = testing::run(|out| Reading.handle(&read(slot), &mut world, out));
            assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
            assert!(outcome
                .events
                .iter()
                .any(|event| matches!(event, ClientEvent::World(WorldEvent::ReadRefused { .. }))));
        }
    }
}
