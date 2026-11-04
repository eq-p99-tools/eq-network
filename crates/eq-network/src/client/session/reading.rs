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
            Some(book) => match out.encode(&Request::ReadBook(book)) {
                Ok(packet) => out.send(&packet),
                Err(error) => actions::refuse(command, &error.to_string(), out.log),
            },
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
            assert_eq!(outcome.sent, [], "unreadable item sent a request");
            assert!(outcome
                .events
                .iter()
                .any(|event| matches!(event, ClientEvent::World(WorldEvent::ReadRefused { .. }))));
        }
    }

    #[test]
    fn eqmac_reading_uses_its_wire_and_refuses_invalid_names_without_losing_inventory() {
        use crate::client::{session::servers, ServerProtocol};
        use eq_network_game::{books::eqmac_request, message::Message};
        for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
            let server = servers::server_type(protocol);
            for kind in [0, 1] {
                let mut note = testing::item(23);
                note.book = Some(Book {
                    file: "SyntheticText".into(),
                    kind,
                });
                let mut world = World::new(5);
                let mut inventory = Inventory::default();
                inventory.apply(InventoryUpdate::Snapshot(vec![
                    note.clone(),
                    testing::item(24),
                ]));
                world.inventory = inventory.clone().into();
                let output = testing::run_on(server.wire(), |out| {
                    Reading.handle(&read(23), &mut world, out)
                });
                output.result.unwrap();
                assert_eq!(
                    output.sent,
                    [eqmac_request(note.book.as_ref().unwrap()).unwrap()]
                );
                assert!(
                    matches!(output.events.as_slice(), []),
                    "unexpected local read result: {:?}",
                    output.events
                );
                assert_eq!(world.inventory.items(), inventory.items());
                let text = [vec![kind], b"Synthetic server text\0".to_vec()].concat();
                let replies = server.wire().messages(
                    eq_network_game::books::EQMAC_READ_OPCODE,
                    &text,
                    server.item_rules(),
                );
                assert!(
                    matches!(replies.as_slice(), [Message::Event(WorldEvent::BookText(text))] if text.kind == kind && text.text == "Synthetic server text")
                );
                for slot in [24, 25] {
                    let output = testing::run_on(server.wire(), |out| {
                        Reading.handle(&read(slot), &mut world, out)
                    });
                    output.result.unwrap();
                    assert_eq!(output.sent, []);
                    assert!(matches!(
                        output.events.as_slice(),
                        [
                            ClientEvent::World(WorldEvent::ReadRefused { session_id: 5, .. }),
                            ClientEvent::Diagnostic(_)
                        ]
                    ));
                }
                note.book.as_mut().unwrap().file = "x".repeat(20);
                inventory.apply(InventoryUpdate::Set(vec![note]));
                world.inventory = inventory.into();
                let output = testing::run_on(server.wire(), |out| {
                    Reading.handle(&read(23), &mut world, out)
                });
                output.result.unwrap();
                assert_eq!(output.sent, []);
                assert!(
                    matches!(output.events.as_slice(), [ClientEvent::World(WorldEvent::ReadRefused { session_id: 5, reason }), ClientEvent::Diagnostic(_)] if reason == "not a book a server can find")
                );
            }
        }
    }
}
