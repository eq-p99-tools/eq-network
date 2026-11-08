//! Synthetic reads through actual P99 policy, admission, gate and decoder.
use super::super::{feature::testing, inventory::Carried};
use super::*;
use crate::client::ServerProtocol;
use eq_network_game::{
    books::{Book, ReadSelection, READ_OPCODE},
    food::AutoEat,
    inventory::{InventorySlot, InventoryUpdate},
    world::{Capability, WorldEvent},
};

fn setup(flag: u8) -> (Features, World) {
    let server = servers::server_type(ServerProtocol::Project1999);
    let mut features = Features::new(server, "Synthetic", AutoEat::default());
    let mut world = World::new(5);
    world.player.admit(testing::player(7));
    world.own_spawn = Some(7);
    world.admitted = Some(Instant::now());
    let mut item = testing::item(23);
    item.book = Some(Book {
        kind: flag,
        file: "SyntheticText".into(),
    });
    features
        .admit(
            &Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(vec![item]))),
            &mut world,
        )
        .unwrap();
    testing::run_on(server.wire(), |out| features.admitted(&mut world, out))
        .result
        .unwrap();
    assert!(testing::run_on(server.wire(), |out| {
        settle(&mut features, &mut world, out, true)
    })
    .result
    .unwrap()
    .is_none());
    (features, world)
}

fn observe(features: &mut Features, world: &mut World, update: InventoryUpdate) {
    testing::run_on(
        servers::server_type(ServerProtocol::Project1999).wire(),
        |out| features.observe(&Message::Event(WorldEvent::Inventory(update)), world, out),
    )
    .result
    .unwrap();
}

fn request(world: &World, slot: i32) -> ClientCommand {
    ClientCommand::ReadItem {
        session_id: world.session_id,
        revision: world.inventory.revision(),
        selection: Box::new(ReadSelection::capture(&world.inventory, InventorySlot(slot)).unwrap()),
        created: Instant::now(),
    }
}

fn submit(
    features: &mut Features,
    world: &mut World,
    command: &ClientCommand,
) -> testing::Outcome<Result<bool>> {
    testing::run_on(
        servers::server_type(ServerProtocol::Project1999).wire(),
        |out| {
            if let Some(reason) = features.refusal(command, world, Instant::now()) {
                actions::refuse(command, reason, out.log)?;
                Ok(true)
            } else {
                features.handle(command, world, out)
            }
        },
    )
}

#[test]
fn p99_note_and_book_dispatch_send_only_requests_until_server_text_arrives() {
    for kind in [0, 1] {
        let (mut features, mut world) = setup(kind);
        assert!(features.capabilities().contains(&Capability::Reading));
        let before = world.inventory.clone();
        let command = request(&world, 23);
        let output = submit(&mut features, &mut world, &command);
        assert!(output.result.unwrap());
        assert_eq!(output.sent.len(), 1);
        assert_eq!(output.sent[0].opcode, READ_OPCODE);
        assert_eq!(
            output.sent[0].body,
            [vec![255, kind], b"SyntheticText\0".to_vec()].concat()
        );
        assert!(
            output.events.is_empty(),
            "submission supplied text: {:?}",
            output.events
        );
        assert_eq!(world.inventory.items(), before.items());
        let wire = servers::server_type(ServerProtocol::Project1999).wire();
        let body = [vec![255, kind], b"Synthetic`server text\0".to_vec()].concat();
        let output = testing::run_on(wire, |out| {
            hear(
                read_messages(
                    wire,
                    READ_OPCODE,
                    &body,
                    servers::server_type(ServerProtocol::Project1999).item_rules(),
                ),
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        assert!(output.result.unwrap().is_none());
        assert!(
            output.sent.is_empty(),
            "server text caused sends: {:?}",
            output.sent
        );
        assert!(
            matches!(output.events.as_slice(), [ClientEvent::World(WorldEvent::BookText(text))]
            if text.kind == kind && text.text == "Synthetic`server text"),
            "unexpected reading events: {:?}",
            output.events
        );
        assert_eq!(world.inventory.items(), before.items());
    }
}

#[test]
fn p99_reading_refuses_changed_or_invalid_inventory_instead_of_reading_a_replacement() {
    for case in 0..6 {
        let (mut features, mut world) = setup(if case == 3 { 2 } else { 0 });
        let command = request(&world, 23);
        match case {
            0 => observe(
                &mut features,
                &mut world,
                InventoryUpdate::Snapshot(vec![testing::item(23)]),
            ),
            1 => observe(&mut features, &mut world, InventoryUpdate::Invalidated),
            2 => world.inventory = Carried::default(),
            4 => {
                let mut item = world.inventory.items()[&InventorySlot(23)].clone();
                item.book.as_mut().unwrap().file = "a\0b".into();
                observe(
                    &mut features,
                    &mut world,
                    InventoryUpdate::Snapshot(vec![item]),
                );
            }
            5 => observe(
                &mut features,
                &mut world,
                InventoryUpdate::Snapshot(Vec::new()),
            ),
            _ => (),
        }
        // Changed-slot cases use the old selection. Invalid metadata uses
        // the new selection so its codec refusal is exercised too.
        let command = if case == 4 {
            request(&world, 23)
        } else {
            command
        };
        let before = world.inventory.clone();
        let output = submit(&mut features, &mut world, &command);
        assert!(output.result.unwrap());
        assert!(output.sent.is_empty(), "case {case}: {:?}", output.sent);
        assert!(matches!(output.events.first(), Some(ClientEvent::World(
            WorldEvent::ReadRefused { session_id: 5, reason })) if !reason.is_empty()));
        assert_eq!(world.inventory.items(), before.items());
    }
}

#[test]
fn p99_reading_uses_normal_admission_and_age_guards() {
    for case in 0..3 {
        let (mut features, mut world) = setup(0);
        let mut command = request(&world, 23);
        if let ClientCommand::ReadItem {
            session_id,
            created,
            ..
        } = &mut command
        {
            match case {
                0 => *session_id = 4,
                1 => *created -= Duration::from_secs(30),
                _ => world.admitted = None,
            }
        }
        let output = submit(&mut features, &mut world, &command);
        assert!(output.result.unwrap());
        assert!(output.sent.is_empty(), "case {case}: {:?}", output.sent);
        assert!(matches!(
            output.events.first(),
            Some(ClientEvent::World(WorldEvent::ReadRefused { .. }))
        ));
    }
}

#[test]
fn p99_unknown_and_truncated_texts_never_become_note_events() {
    let (mut features, mut world) = setup(0);
    let server = servers::server_type(ServerProtocol::Project1999);
    for body in [
        &[][..],
        &[255, 0][..],
        &[255, 0, b'x'][..],
        &[255, 2, 0][..],
        &[255, 255, 0][..],
    ] {
        let output = testing::run_on(server.wire(), |out| {
            hear(
                read_messages(server.wire(), READ_OPCODE, body, server.item_rules()),
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        assert!(output.result.unwrap().is_none());
        assert!(
            output.sent.is_empty(),
            "malformed text caused sends: {:?}",
            output.sent
        );
        assert!(!output
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::BookText(_)))));
        assert!(output.events.iter().any(|event| matches!(event,
            ClientEvent::Diagnostic(reason) if reason.contains("payload_len="))));
    }
}

#[test]
fn p99_reading_requires_carried_slots_and_known_bag_contents() {
    for slot in [2000, 2500, 3000, 4000, 251] {
        let (mut features, mut world) = setup(0);
        let mut note = world.inventory.items()[&InventorySlot(23)].clone();
        note.slot = InventorySlot(slot);
        observe(
            &mut features,
            &mut world,
            InventoryUpdate::Snapshot(vec![note]),
        );
        if slot == 251 {
            assert!(ReadSelection::capture(&world.inventory, InventorySlot(slot)).is_err());
            continue;
        }
        let command = request(&world, slot);
        let output = submit(&mut features, &mut world, &command);
        assert!(output.result.unwrap());
        assert!(output.sent.is_empty(), "slot {slot}: {:?}", output.sent);
        assert!(matches!(
            output.events.first(),
            Some(ClientEvent::World(WorldEvent::ReadRefused { .. }))
        ));
    }
    let (mut features, mut world) = setup(1);
    let mut note = world.inventory.items()[&InventorySlot(23)].clone();
    note.slot = InventorySlot(251);
    let mut bag = testing::item(22);
    bag.bag_slots = 1;
    observe(
        &mut features,
        &mut world,
        InventoryUpdate::Snapshot(vec![bag, note]),
    );
    let command = request(&world, 251);
    observe(
        &mut features,
        &mut world,
        InventoryUpdate::Set(vec![testing::item(24)]),
    );
    let mut enriched = world.inventory.items()[&InventorySlot(251)].clone();
    enriched.details.base_held_model = std::num::NonZeroU16::new(1);
    enriched.rules.titanium_link_hash = Some(42);
    observe(
        &mut features,
        &mut world,
        InventoryUpdate::Set(vec![enriched]),
    );
    let output = submit(&mut features, &mut world, &command);
    assert!(output.result.unwrap());
    assert_eq!(output.sent.len(), 1);
    assert_eq!(output.sent[0].opcode, READ_OPCODE);
    assert_eq!(output.sent[0].body, b"\xff\x01SyntheticText\0");
}

#[test]
fn queued_read_refuses_replaced_parent_or_intervening_pending_move() {
    for pending_move in [false, true] {
        let (mut features, mut world) = setup(0);
        let mut note = world.inventory.items()[&InventorySlot(23)].clone();
        note.slot = InventorySlot(251);
        let mut bag = testing::item(22);
        bag.bag_slots = 1;
        observe(
            &mut features,
            &mut world,
            InventoryUpdate::Snapshot(vec![bag.clone(), note.clone()]),
        );
        let command = request(&world, 251);
        let update = if pending_move {
            InventoryUpdate::Prediction(vec![testing::item(24)])
        } else {
            bag.details.id += 1;
            InventoryUpdate::Set(vec![bag, note])
        };
        observe(&mut features, &mut world, update);
        let output = submit(&mut features, &mut world, &command);
        assert!(output.result.unwrap());
        assert_eq!(output.sent, []);
        assert!(matches!(
            output.events.first(),
            Some(ClientEvent::World(WorldEvent::ReadRefused { .. }))
        ));
    }
}
