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

fn admission(
    protocol: ServerProtocol,
    session_id: u64,
    drop_ids: &[u32],
) -> (GroundObjects, Belongings, World) {
    let mut world = World::new(session_id);
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
    for &drop_id in drop_ids {
        receive(
            protocol,
            eqmac::SPAWN_OPCODE,
            &placement(drop_id, b"IT63_ACTORDEF", 1.0),
            &mut objects,
            &mut belongings,
            &mut world,
        );
    }
    (objects, belongings, world)
}

#[test]
fn the_submission_clock_bounds_expiry_without_retry_or_inventory_authority() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let (mut objects, _belongings, mut world) = admission(protocol, 5, &[71]);
        let submitted = Instant::now() + Duration::from_secs(3600);
        world.objects.pickup = objects.results.sent(71, 7, submitted);
        assert_eq!(
            world.objects.pickup.as_ref().unwrap().expires,
            submitted + PICKUP_WINDOW
        );
        let inventory = world.inventory.clone();
        let deadline = submitted + PICKUP_WINDOW;
        let before = deadline.checked_sub(Duration::from_nanos(1)).unwrap();
        let wire = servers::server_type(protocol).wire();
        let outcome = testing::run_on(wire, |out| objects.tick(before, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(objects.holds(&world, before).len(), 1);
        assert_eq!(outcome.sent, []);
        assert!(outcome.events.is_empty(), "{:?}", outcome.events);
        let outcome = testing::run_on(wire, |out| objects.tick(deadline, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(objects.holds(&world, deadline), []);
        assert_eq!(outcome.sent, []);
        assert!(matches!(
            outcome.events.as_slice(),
            [ClientEvent::Diagnostic(reason)]
                if reason == "Ground pickup wait ended; its result is still unknown"
        ));
        assert!(world.objects.ambiguous_pickup_results);
        assert_eq!(*world.inventory, inventory);
        assert!(world.objects.entries().contains_key(&71));
        let outcome = testing::run_on(wire, |out| {
            objects.tick(deadline + PICKUP_WINDOW, &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert!(outcome.events.is_empty(), "{:?}", outcome.events);
        assert!(PickupResults::Existing.sent(71, 7, submitted).is_none());
    }
}

#[test]
fn each_later_pickup_wait_is_bounded_without_restoring_lost_attribution() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let (mut objects, mut belongings, mut world) = admission(protocol, 5, &[71, 72]);
        let wire = servers::server_type(protocol).wire();
        for drop_id in [71, 72, 71] {
            let outcome = testing::run_on(wire, |out| {
                objects.handle(&command(drop_id), &mut world, out)
            });
            outcome.result.unwrap();
            assert_eq!(outcome.sent, [eqmac::pickup(drop_id, 7)]);
            let expires = world.objects.pickup.as_ref().unwrap().expires;
            if world.objects.ambiguous_pickup_results {
                // An old untagged refusal cannot identify this later pickup,
                // including when a previously requested ID is used again.
                receive(
                    protocol,
                    eqmac::CLICK_OPCODE,
                    &[u32::MAX.to_le_bytes(), 7_u32.to_le_bytes()].concat(),
                    &mut objects,
                    &mut belongings,
                    &mut world,
                );
                assert_eq!(objects.holds(&world, expires).len(), 1);
            }
            let before = expires.checked_sub(Duration::from_nanos(1)).unwrap();
            let outcome = testing::run_on(wire, |out| objects.tick(before, &mut world, out));
            outcome.result.unwrap();
            assert_eq!(objects.holds(&world, before).len(), 1);
            assert_eq!(outcome.sent, []);
            let outcome = testing::run_on(wire, |out| objects.tick(expires, &mut world, out));
            outcome.result.unwrap();
            assert_eq!(objects.holds(&world, expires), []);
            assert_eq!(outcome.sent, []);
            assert!(world.objects.ambiguous_pickup_results);
            assert!(world.inventory.items().is_empty());
            assert!(world.objects.entries().contains_key(&drop_id));
        }
    }
}

#[test]
fn a_refusal_keeps_the_object_until_a_new_admission_supplies_its_own_table() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        for (session_id, drop_ids) in [(5, &[71][..]), (6, &[][..]), (7, &[71][..])] {
            let (mut objects, mut belongings, mut world) =
                admission(protocol, session_id, drop_ids);
            let wire = servers::server_type(protocol).wire();
            if session_id == 5 {
                let outcome =
                    testing::run_on(wire, |out| objects.handle(&command(71), &mut world, out));
                outcome.result.unwrap();
                assert_eq!(outcome.sent, [eqmac::pickup(71, 7)]);
                receive(
                    protocol,
                    eqmac::CLICK_OPCODE,
                    &[u32::MAX.to_le_bytes(), 7_u32.to_le_bytes()].concat(),
                    &mut objects,
                    &mut belongings,
                    &mut world,
                );
                let later = Instant::now() + Duration::from_secs(60);
                let outcome = testing::run_on(wire, |out| objects.tick(later, &mut world, out));
                outcome.result.unwrap();
                assert_eq!(outcome.sent, []);
                assert!(outcome.events.is_empty(), "{:?}", outcome.events);
                assert_eq!(objects.holds(&world, later), []);
                assert!(world.inventory.items().is_empty());
            }
            let replay = testing::run_on(wire, |out| objects.admitted(&mut world, out));
            replay.result.unwrap();
            assert_eq!(replay.sent, []);
            assert!(matches!(
                replay.events.as_slice(),
                [ClientEvent::World(WorldEvent::Objects(ObjectUpdate::Snapshot(objects)))]
                    if objects.iter().map(|object| object.drop_id).eq(drop_ids.iter().copied())
            ));
        }
    }
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
fn cursor_delivery_can_precede_disappearance_without_granting_an_item_twice() {
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
    receive(
        protocol,
        eqmac::CLICK_OPCODE,
        &[71_u32.to_le_bytes(), 7_u32.to_le_bytes()].concat(),
        &mut objects,
        &mut belongings,
        &mut world,
    );
    assert_eq!(objects.holds(&world, Instant::now()), []);
}

#[derive(Clone, Copy)]
enum Cancellation {
    Foreign,
    Unattributed,
    Snapshot,
    Expiry,
}

/// Cancels through the real wire owners or production timer, with no retry.
fn cancel_pending(
    protocol: ServerProtocol,
    cancellation: Cancellation,
    objects: &mut GroundObjects,
    belongings: &mut Belongings,
    world: &mut World,
) {
    match cancellation {
        Cancellation::Foreign | Cancellation::Unattributed => receive(
            protocol,
            eqmac::CLICK_OPCODE,
            &[
                71_u32.to_le_bytes(),
                (if matches!(cancellation, Cancellation::Foreign) {
                    9_u32
                } else {
                    0
                })
                .to_le_bytes(),
            ]
            .concat(),
            objects,
            belongings,
            world,
        ),
        Cancellation::Snapshot => receive(protocol, 0xf641, &[0, 0], objects, belongings, world),
        Cancellation::Expiry => {
            let wire = servers::server_type(protocol).wire();
            let expires = world.objects.pickup.as_ref().unwrap().expires;
            let before = expires.checked_sub(Duration::from_nanos(1)).unwrap();
            let outcome = testing::run_on(wire, |out| objects.tick(before, world, out));
            outcome.result.unwrap();
            assert_eq!(objects.holds(world, before).len(), 1);
            let outcome = testing::run_on(wire, |out| objects.tick(expires, world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent, [], "expiry never retries");
            let outcome = testing::run_on(wire, |out| {
                objects.tick(expires + PICKUP_WINDOW, world, out)
            });
            outcome.result.unwrap();
            assert_eq!(outcome.sent, []);
            assert!(outcome.events.is_empty(), "expiry is reported only once");
        }
    }
}

#[test]
fn cancellation_releases_the_wait_but_old_results_cannot_settle_a_new_pickup() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        for cancellation in [
            Cancellation::Foreign,
            Cancellation::Unattributed,
            Cancellation::Snapshot,
            Cancellation::Expiry,
        ] {
            for next_id in [71, 72] {
                let wire = servers::server_type(protocol).wire();
                let mut world = World::new(5);
                world.player.admit(testing::player(7));
                let mut objects =
                    GroundObjects::for_generation(eq_network_game::GameDialect::EqMac);
                let mut belongings = Belongings::new(eq_network_game::food::AutoEat::default());
                receive(
                    protocol,
                    0xf641,
                    &[0, 0],
                    &mut objects,
                    &mut belongings,
                    &mut world,
                );
                for id in [71, 72] {
                    receive(
                        protocol,
                        eqmac::SPAWN_OPCODE,
                        &placement(id, b"IT63_ACTORDEF", 1.0),
                        &mut objects,
                        &mut belongings,
                        &mut world,
                    );
                }
                let outcome =
                    testing::run_on(wire, |out| objects.handle(&command(71), &mut world, out));
                outcome.result.unwrap();
                assert_eq!(outcome.sent, [eqmac::pickup(71, 7)]);
                cancel_pending(
                    protocol,
                    cancellation,
                    &mut objects,
                    &mut belongings,
                    &mut world,
                );
                assert_eq!(objects.holds(&world, Instant::now()), []);
                assert!(
                    world.inventory.items().is_empty(),
                    "cancellation grants no item"
                );
                receive(
                    protocol,
                    eqmac::SPAWN_OPCODE,
                    &placement(next_id, b"IT63_ACTORDEF", 1.0),
                    &mut objects,
                    &mut belongings,
                    &mut world,
                );
                let outcome = testing::run_on(wire, |out| {
                    objects.handle(&command(next_id), &mut world, out)
                });
                outcome.result.unwrap();
                assert_eq!(outcome.sent, [eqmac::pickup(next_id, 7)]);

                // Even a reused ID cannot make old named/untagged result facts
                // uniquely attributable to this new pickup.
                for (drop_id, player_id) in [(u32::MAX, 7_u32), (71, 7), (71, 9), (71, 0)] {
                    receive(
                        protocol,
                        eqmac::CLICK_OPCODE,
                        &[drop_id.to_le_bytes(), player_id.to_le_bytes()].concat(),
                        &mut objects,
                        &mut belongings,
                        &mut world,
                    );
                    assert_eq!(objects.holds(&world, Instant::now()).len(), 1);
                }
                receive(
                    protocol,
                    0x7841,
                    &cursor_item(),
                    &mut objects,
                    &mut belongings,
                    &mut world,
                );
                assert_eq!(world.inventory.items()[&CURSOR].details.id, 1234);
                assert_eq!(
                    objects.holds(&world, Instant::now()).len(),
                    1,
                    "an old cursor packet cannot complete the newer pickup"
                );
                let expires = world.objects.pickup.as_ref().unwrap().expires;
                let outcome = testing::run_on(wire, |out| objects.tick(expires, &mut world, out));
                outcome.result.unwrap();
                assert_eq!(outcome.sent, []);
                assert_eq!(objects.holds(&world, expires), []);
                assert!(
                    world.objects.pickup.is_none(),
                    "stale requests are discarded"
                );
                assert!(world.objects.ambiguous_pickup_results);
                assert_eq!(objects.holds(&World::new(6), Instant::now()), []);
            }
        }
    }
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
