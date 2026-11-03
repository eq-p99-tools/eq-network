//! Synthetic `EQMac` packets through the wire and the normal feature owners.
use super::*;
use crate::client::{
    session::{feature::testing, inventory::Belongings, servers},
    ServerProtocol,
};
use eq_network_game::{inventory::InventoryUpdate, objects::eqmac};

fn read(protocol: ServerProtocol, opcode: u16, body: &[u8]) -> Vec<Message> {
    let server = servers::server_type(protocol);
    server.wire().messages(opcode, body, server.item_rules())
}

fn placement(id: u32, model: &[u8], x: f32) -> Vec<u8> {
    let mut body = vec![0; 222];
    body[12..16].copy_from_slice(&id.to_le_bytes());
    body[148..152].copy_from_slice(&x.to_le_bytes());
    body[156..156 + model.len()].copy_from_slice(model);
    body
}

fn command(drop_id: u32) -> ClientCommand {
    ClientCommand::PickUp {
        session_id: 5,
        drop_id,
        created: Instant::now(),
    }
}

fn cursor_item() -> [u8; 360] {
    let mut item = [0; 360];
    item[..14].copy_from_slice(b"Synthetic item");
    item[180..182].copy_from_slice(&1234_i16.to_le_bytes());
    item
}

/// Deliver exactly what the selected generation's wire reads to the two owners.
fn receive(
    protocol: ServerProtocol,
    opcode: u16,
    body: &[u8],
    objects: &mut GroundObjects,
    belongings: &mut Belongings,
    world: &mut World,
) {
    let wire = servers::server_type(protocol).wire();
    let messages = read(protocol, opcode, body);
    assert_eq!(messages.len(), 1);
    let outcome = testing::run_on(wire, |out| -> Result<()> {
        for message in &messages {
            assert!(!matches!(message, Message::Unreadable { .. }));
            belongings.observe(message, world, out)?;
            objects.observe(message, world, out)?;
        }
        Ok(())
    });
    outcome.result.unwrap();
    assert_eq!(outcome.sent, []);
}

#[test]
fn eqmac_pickups_need_current_objects_in_reach_and_authoritative_item_packets() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let wire = servers::server_type(protocol).wire();
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        let mut objects = GroundObjects::for_generation(eq_network_game::GameDialect::EqMac);
        let mut belongings = Belongings::new(eq_network_game::food::AutoEat::default());
        receive(
            protocol,
            0xf641,
            &[0, 0],
            &mut objects,
            &mut belongings,
            &mut world,
        );
        receive(
            protocol,
            eqmac::SPAWN_OPCODE,
            &placement(71, b"IT63_ACTORDEF", 19.5),
            &mut objects,
            &mut belongings,
            &mut world,
        );
        let request = command(71);
        let outcome = testing::run_on(wire, |out| objects.handle(&request, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [eqmac::pickup(71, 7)]);
        assert!(world.objects.entries().contains_key(&71));
        assert!(world.inventory.items().is_empty(), "sending grants no item");
        assert_eq!(objects.holds(&world, Instant::now()).len(), 1);
        let duplicate = testing::run_on(wire, |out| objects.handle(&command(71), &mut world, out));
        duplicate.result.unwrap();
        assert_eq!(duplicate.sent, []);

        // Another actor's denial cannot answer this player's unresolved click.
        receive(
            protocol,
            eqmac::CLICK_OPCODE,
            &[u32::MAX.to_le_bytes(), 9_u32.to_le_bytes()].concat(),
            &mut objects,
            &mut belongings,
            &mut world,
        );
        assert_eq!(objects.holds(&world, Instant::now()).len(), 1);

        let denied = [u32::MAX.to_le_bytes(), 7_u32.to_le_bytes()].concat();
        receive(
            protocol,
            eqmac::CLICK_OPCODE,
            &denied,
            &mut objects,
            &mut belongings,
            &mut world,
        );
        assert!(world.objects.entries().contains_key(&71));
        assert!(world.inventory.items().is_empty());
        assert_eq!(objects.holds(&world, Instant::now()), []);
        let request = command(71);
        let outcome = testing::run_on(wire, |out| objects.handle(&request, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [eqmac::pickup(71, 7)]);

        let removed = [71_u32.to_le_bytes(), 7_u32.to_le_bytes()].concat();
        receive(
            protocol,
            eqmac::CLICK_OPCODE,
            &removed,
            &mut objects,
            &mut belongings,
            &mut world,
        );
        assert!(!world.objects.entries().contains_key(&71));
        assert!(world.inventory.items().is_empty(), "removal grants no item");
        assert_eq!(objects.holds(&world, Instant::now()).len(), 1);

        // EQMac slot 0 is the cursor; the existing item decoder normalizes it.
        receive(
            protocol,
            0x7841,
            &cursor_item(),
            &mut objects,
            &mut belongings,
            &mut world,
        );
        assert_eq!(world.inventory.items()[&CURSOR].details.id, 1234);
        assert!(!world.inventory.predicted());
        assert!(!world.inventory.stale());
        assert!(world.objects.entries().is_empty());
        assert_eq!(objects.holds(&world, Instant::now()), []);
    }
}

#[test]
fn cursor_delivery_can_precede_disappearance_but_silence_never_retries() {
    let protocol = ServerProtocol::Takp;
    let wire = servers::server_type(protocol).wire();
    let mut world = World::new(5);
    world.player.admit(testing::player(7));
    let mut objects = GroundObjects::for_generation(eq_network_game::GameDialect::EqMac);
    let mut belongings = Belongings::new(eq_network_game::food::AutoEat::default());
    receive(
        protocol,
        0xf641,
        &[0, 0],
        &mut objects,
        &mut belongings,
        &mut world,
    );
    receive(
        protocol,
        eqmac::SPAWN_OPCODE,
        &placement(71, b"IT63_ACTORDEF", 1.0),
        &mut objects,
        &mut belongings,
        &mut world,
    );
    let outcome = testing::run_on(wire, |out| objects.handle(&command(71), &mut world, out));
    outcome.result.unwrap();
    receive(
        protocol,
        0x7841,
        &cursor_item(),
        &mut objects,
        &mut belongings,
        &mut world,
    );
    assert_eq!(world.inventory.items()[&CURSOR].details.id, 1234);
    assert_eq!(objects.holds(&world, Instant::now()).len(), 1);
    let later = Instant::now() + std::time::Duration::from_secs(60);
    let outcome = testing::run_on(wire, |out| objects.tick(later, &mut world, out));
    outcome.result.unwrap();
    assert_eq!(outcome.sent, []);
    assert_eq!(objects.holds(&world, later).len(), 1);
    receive(
        protocol,
        eqmac::CLICK_OPCODE,
        &[71_u32.to_le_bytes(), 7_u32.to_le_bytes()].concat(),
        &mut objects,
        &mut belongings,
        &mut world,
    );
    assert_eq!(objects.holds(&world, later), []);
}

#[test]
fn another_actors_removal_cannot_acknowledge_our_click_or_a_newer_request() {
    let protocol = ServerProtocol::Takp;
    let wire = servers::server_type(protocol).wire();
    let mut world = World::new(5);
    world.player.admit(testing::player(7));
    let mut inventory = Inventory::default();
    inventory.apply(InventoryUpdate::Snapshot(Vec::new()));
    world.inventory = inventory.into();
    let mut objects = GroundObjects::for_generation(eq_network_game::GameDialect::EqMac);
    for id in [71, 72] {
        for message in read(
            protocol,
            eqmac::SPAWN_OPCODE,
            &placement(id, b"IT63_ACTORDEF", 1.0),
        ) {
            objects.admit(&message, &mut world).unwrap();
        }
    }
    let outcome = testing::run_on(wire, |out| objects.handle(&command(71), &mut world, out));
    outcome.result.unwrap();
    assert_eq!(outcome.sent.len(), 1);
    for (id, player) in [(72_u32, 7_u32), (71, 9)] {
        for message in read(
            protocol,
            eqmac::CLICK_OPCODE,
            &[id.to_le_bytes(), player.to_le_bytes()].concat(),
        ) {
            testing::run_on(wire, |out| objects.observe(&message, &mut world, out))
                .result
                .unwrap();
        }
        assert_eq!(objects.holds(&world, Instant::now()).len(), 1);
    }
    // Even a replacement cannot make another request safe before our answer.
    for message in read(
        protocol,
        eqmac::SPAWN_OPCODE,
        &placement(72, b"IT63_ACTORDEF", 1.0),
    ) {
        objects.admit(&message, &mut world).unwrap();
    }
    let duplicate = testing::run_on(wire, |out| objects.handle(&command(72), &mut world, out));
    duplicate.result.unwrap();
    assert_eq!(duplicate.sent, []);
    assert!(world.inventory.items().is_empty());
    assert_eq!(objects.holds(&World::new(6), Instant::now()), []);
}

#[test]
fn replaced_removed_distant_and_fixture_objects_never_send_a_pickup() {
    let protocol = ServerProtocol::Takp;
    let wire = servers::server_type(protocol).wire();
    let mut objects = GroundObjects::for_generation(eq_network_game::GameDialect::EqMac);
    let mut world = World::new(5);
    world.player.admit(testing::player(7));
    let mut inventory = Inventory::default();
    inventory.apply(InventoryUpdate::Snapshot(Vec::new()));
    world.inventory = inventory.into();
    let before = command(71);
    let place = |id, model: &[u8], x, objects: &mut GroundObjects, world: &mut World| {
        let messages = read(protocol, eqmac::SPAWN_OPCODE, &placement(id, model, x));
        objects.admit(&messages[0], world).unwrap();
    };
    place(71, b"IT63_ACTORDEF", 0.0, &mut objects, &mut world);
    let check = |request: &ClientCommand, objects: &mut GroundObjects, world: &mut World| {
        let outcome = testing::run_on(wire, |out| objects.handle(request, world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert!(matches!(
            &outcome.events[..],
            [ClientEvent::World(WorldEvent::ObjectAction {
                error: Some(_),
                ..
            })]
        ));
    };
    check(&before, &mut objects, &mut world);
    let old = command(71);
    let message = read(
        protocol,
        eqmac::CLICK_OPCODE,
        &[71_u32.to_le_bytes(), 0_u32.to_le_bytes()].concat(),
    );
    objects.admit(&message[0], &mut world).unwrap();
    check(&old, &mut objects, &mut world);
    place(71, b"IT63_ACTORDEF", 0.0, &mut objects, &mut world);
    check(&old, &mut objects, &mut world);
    // EQMac requires strictly less than 20 in three dimensions.
    place(72, b"IT63_ACTORDEF", 20.0, &mut objects, &mut world);
    check(&command(72), &mut objects, &mut world);
    place(73, b"FORGE_ACTORDEF", 0.0, &mut objects, &mut world);
    check(&command(73), &mut objects, &mut world);
    check(&command(74), &mut objects, &mut world);
}

#[test]
fn object_admission_replays_server_state_and_malformed_updates_leave_it_intact() {
    let protocol = ServerProtocol::Takp;
    let wire = servers::server_type(protocol).wire();
    let mut objects = GroundObjects::for_generation(eq_network_game::GameDialect::EqMac);
    let mut world = World::new(5);
    for message in read(
        protocol,
        eqmac::SPAWN_OPCODE,
        &placement(71, b"IT63_ACTORDEF", 1.0),
    ) {
        objects.admit(&message, &mut world).unwrap();
    }
    let snapshot = world.objects.admission();
    let outcome = testing::run_on(wire, |out| objects.admitted(&mut world, out));
    outcome.result.unwrap();
    assert!(
        matches!(&outcome.events[..], [ClientEvent::World(WorldEvent::Objects(update))] if *update == snapshot)
    );
    let invalid = read(protocol, eqmac::SPAWN_OPCODE, &[0; 224]);
    assert!(matches!(
        invalid[..],
        [Message::Unreadable {
            part: eq_network_game::message::Part::World,
            ..
        }]
    ));
    objects.admit(&invalid[0], &mut world).unwrap();
    assert_eq!(world.objects.admission(), snapshot);
    assert!(world.inventory.items().is_empty());
    // The feature never owns or fabricates an inventory update.
    objects
        .admit(
            &Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(Vec::new()))),
            &mut world,
        )
        .unwrap();
    assert!(!world.inventory.received());
}
