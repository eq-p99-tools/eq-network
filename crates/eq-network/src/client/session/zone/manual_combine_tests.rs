//! Carried combines through the actual P99 policy, wire and inventory owner.
use super::*;
use crate::client::{session::feature::testing, ServerProtocol};
use eq_network_game::{
    food::AutoEat,
    inventory::{Inventory, InventorySlot, InventoryUpdate},
    objects::{ContainerView, GroundObject},
    tradeskills::{CombineUpdate, WORLD_CONTAINER},
    world::{Capability, WorldEvent},
};

fn setup() -> (Features, World) {
    setup_in(5)
}

fn setup_in(session_id: u64) -> (Features, World) {
    let mut features = Features::new(
        servers::server_type(ServerProtocol::Project1999),
        "Tester",
        AutoEat::default(),
    );
    let mut world = World::new(session_id);
    world.player.admit(testing::player(7));
    world.own_spawn = Some(7);
    world.admitted = Some(Instant::now());
    let mut kit = testing::item(23);
    kit.bag_slots = 10;
    kit.rules.bag_type = 16;
    let mut inventory = Inventory::default();
    inventory.apply(InventoryUpdate::Snapshot(vec![kit, testing::item(261)]));
    world.inventory = inventory.into();
    testing::run(|out| features.admitted(&mut world, out))
        .result
        .unwrap();
    (features, world)
}

fn combine(slot: InventorySlot) -> ClientCommand {
    ClientCommand::Combine {
        session_id: 5,
        container: slot,
        created: Instant::now(),
    }
}

fn hear_packets(features: &mut Features, world: &mut World, opcode: u16, body: &[u8]) {
    let server = servers::server_type(ServerProtocol::Project1999);
    let outcome = testing::run_on(server.wire(), |out| -> Result<()> {
        for mut message in server.wire().messages(opcode, body, server.item_rules()) {
            features.explain(&mut message, world);
            features.observe(&message, world, out)?;
        }
        Ok(())
    });
    outcome.result.unwrap();
    assert!(outcome
        .sent
        .iter()
        .all(|packet| packet.opcode != eq_network_game::tradeskills::COMBINE_OPCODE));
}

#[test]
fn p99_handles_carried_combines_and_refuses_missing_station_state() {
    let (mut features, mut world) = setup();
    assert!(features.capabilities().contains(&Capability::Tradeskills));
    assert!(features
        .capabilities()
        .contains(&Capability::WorldContainers));
    for command in [
        ClientCommand::OpenContainer {
            session_id: 5,
            drop_id: 71,
            created: Instant::now(),
        },
        ClientCommand::CloseContainer {
            session_id: 5,
            drop_id: 42,
            created: Instant::now(),
        },
        combine(WORLD_CONTAINER),
    ] {
        assert_eq!(command.capability(), Some(Capability::WorldContainers));
        let outcome = testing::run(|out| features.handle(&command, &mut world, out));
        assert!(outcome.result.unwrap());
        assert_eq!(outcome.sent, []);
        match command {
            ClientCommand::OpenContainer { .. } => assert!(matches!(
                outcome.events.first(),
                Some(ClientEvent::World(WorldEvent::ObjectAction {
                    session_id: 5,
                    drop_id: 71,
                    error: Some(_),
                }))
            )),
            ClientCommand::Combine { .. } => assert!(matches!(
                outcome.events.first(),
                Some(ClientEvent::World(WorldEvent::CombineRefused {
                    session_id: 5,
                    ..
                }))
            )),
            _ => assert_eq!(outcome.events.len(), 1),
        }
    }
    let before = world.inventory.items().clone();
    let outcome = testing::run(|out| features.handle(&combine(InventorySlot(23)), &mut world, out));
    assert!(outcome.result.unwrap());
    assert_eq!(
        outcome.sent,
        [eq_network_game::tradeskills::titanium_combine(InventorySlot(23)).unwrap()]
    );
    assert!(outcome.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::Combine(CombineUpdate::Started(InventorySlot(
            23
        ))))
    )));
    assert_eq!(world.inventory.items(), &before);
    assert_eq!(features.holds(&world, Instant::now()).len(), 1);
    assert!(features
        .refusal(&combine(InventorySlot(23)), &world, Instant::now())
        .is_some());
}

#[test]
fn p99_and_eqemu_providers_send_the_exact_native_four_byte_carried_slot() {
    for protocol in [ServerProtocol::Project1999, ServerProtocol::EqEmu] {
        let server = servers::server_type(protocol);
        let mut features = Features::new(server, "Tester", AutoEat::default());
        let (_, mut world) = setup();
        let outcome = testing::run_on(server.wire(), |out| {
            features.handle(&combine(InventorySlot(23)), &mut world, out)
        });
        assert!(outcome.result.unwrap(), "{protocol:?}");
        assert_eq!(outcome.sent.len(), 1, "{protocol:?}");
        assert_eq!(outcome.sent[0].opcode, 0x0b40, "{protocol:?}");
        assert_eq!(outcome.sent[0].body, [23, 0, 0, 0], "{protocol:?}");
    }
}

#[test]
fn p99_forge_candidates_need_a_matching_server_forge_reply_before_combining() {
    use eq_network_game::objects;
    let (mut features, mut world) = setup();
    let object = |model: &[u8]| {
        let mut body = [0; 92];
        body[12..16].copy_from_slice(&71_u32.to_le_bytes());
        body[44..44 + model.len()].copy_from_slice(model);
        body
    };
    hear_packets(
        &mut features,
        &mut world,
        objects::SPAWN_OPCODE,
        &object(b"OTHER_ACTORDEF"),
    );
    let open = || ClientCommand::OpenContainer {
        session_id: 5,
        drop_id: 71,
        created: Instant::now(),
    };
    let unknown = testing::run(|out| features.handle(&open(), &mut world, out));
    assert!(unknown.result.unwrap());
    assert_eq!(unknown.sent, []);
    hear_packets(
        &mut features,
        &mut world,
        objects::SPAWN_OPCODE,
        &object(b"FORGE_ACTORDEF"),
    );
    let sent = testing::run(|out| features.handle(&open(), &mut world, out));
    assert!(sent.result.unwrap());
    assert_eq!(sent.sent, [objects::titanium_pickup(71, 7)]);
    assert!(!world.container.is_open());

    let mut answer = [0; 92];
    answer[..4].copy_from_slice(&7_u32.to_le_bytes());
    answer[4..8].copy_from_slice(&71_u32.to_le_bytes());
    answer[8..12].copy_from_slice(&1_u32.to_le_bytes());
    answer[12..16].copy_from_slice(&15_u32.to_le_bytes()); // An oven is outside this offer.
    let server = servers::server_type(ServerProtocol::Project1999);
    let rejection = testing::run_on(server.wire(), |out| {
        hear(
            server
                .wire()
                .messages(objects::CONTAINER_OPCODE, &answer, server.item_rules()),
            &mut features,
            &mut world,
            out,
            true,
        )
    });
    rejection.result.unwrap();
    assert_eq!(rejection.sent.len(), 1);
    assert!(!rejection.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::Objects(ObjectUpdate::Container(_)))
    )));
    let content = Message::Event(WorldEvent::Inventory(InventoryUpdate::Set(vec![
        testing::item(4000),
    ])));
    let late = testing::run(|out| hear(vec![content], &mut features, &mut world, out, true));
    late.result.unwrap();
    assert!(late.events.is_empty());
    assert!(world.inventory.items().keys().all(|slot| !slot.is_world()));
    assert!(!world.container.is_open());
    let waiting = testing::run(|out| features.handle(&open(), &mut world, out));
    assert!(waiting.result.unwrap());
    assert_eq!(waiting.sent, []);
    hear_packets(&mut features, &mut world, 0x21ed, &[1, 0, 0, 0, 0, 0, 0, 0]);
    let retried = testing::run(|out| features.handle(&open(), &mut world, out));
    assert!(retried.result.unwrap());
    assert_eq!(retried.sent, [objects::titanium_pickup(71, 7)]);
    answer[12..16].copy_from_slice(&u32::from(eq_network_game::tradeskills::FORGE).to_le_bytes());
    hear_packets(
        &mut features,
        &mut world,
        objects::CONTAINER_OPCODE,
        &answer,
    );
    assert!(world.container.is_open());
    assert!(world.inventory.items().keys().all(|slot| !slot.is_world()));
    let empty = testing::run(|out| features.handle(&combine(WORLD_CONTAINER), &mut world, out));
    assert!(empty.result.unwrap());
    assert_eq!(empty.sent, []);
}

#[test]
fn eqemu_ordered_reopen_fences_close_after_a_reset_with_or_without_combine_clear() {
    let server = servers::server_type(ServerProtocol::EqEmu);
    for combine_clears in [false, true] {
        let mut features = Features::new(server, "Tester", AutoEat::default());
        let (_, mut world) = setup();
        let (object, reply) = eqemu_forge();
        let reply = || Message::Event(WorldEvent::Objects(reply.clone()));
        let open = || ClientCommand::OpenContainer {
            session_id: 5,
            drop_id: 71,
            created: Instant::now(),
        };
        let clear = || Message::Event(WorldEvent::Inventory(InventoryUpdate::WorldEmptied));
        let content = || {
            Message::Event(WorldEvent::Inventory(InventoryUpdate::Set(vec![
                testing::item(4000),
            ])))
        };
        let outcome = testing::run_on(server.wire(), |out| -> Result<()> {
            hear(vec![object], &mut features, &mut world, out, true)?;
            features.handle(&open(), &mut world, out)?;
            hear(
                vec![reply(), content()],
                &mut features,
                &mut world,
                out,
                true,
            )?;
            assert!(world.container.is_open());
            features.handle(&combine(WORLD_CONTAINER), &mut world, out)?;
            hear(
                vec![Message::Event(WorldEvent::Combine(CombineUpdate::Answered))],
                &mut features,
                &mut world,
                out,
                true,
            )?;
            assert_ne!(features.holds(&world, Instant::now()), []);
            features.handle(
                &ClientCommand::CloseContainer {
                    session_id: 5,
                    drop_id: 71,
                    created: Instant::now(),
                },
                &mut world,
                out,
            )?;
            let mut first = Vec::new();
            if !combine_clears {
                // This was a reset-only failure. The sole clear belongs to
                // close, following the independent return.
                first.push(Message::Event(WorldEvent::Inventory(
                    InventoryUpdate::Cursor(vec![testing::item(30)]),
                )));
            }
            first.push(clear());
            hear(first, &mut features, &mut world, out, true)?;
            assert_eq!(features.holds(&world, Instant::now()), []);
            assert!(!world.container.is_open());
            assert!(world.inventory.items().keys().all(|slot| !slot.is_world()));
            assert_eq!(
                world.inventory.items().contains_key(&InventorySlot::CURSOR),
                !combine_clears,
            );
            features.handle(&open(), &mut world, out)?;
            if combine_clears {
                // This is the close's clear arriving before the matching
                // newer reply on the same ordered stream.
                hear(vec![clear()], &mut features, &mut world, out, true)?;
            }
            assert!(!world.container.is_open());
            hear(
                vec![reply(), content()],
                &mut features,
                &mut world,
                out,
                true,
            )?;
            assert!(world.container.is_open());
            assert!(world.inventory.items().contains_key(&InventorySlot(4000)));
            Ok(())
        });
        outcome.result.unwrap();
        assert_eq!(
            outcome
                .sent
                .iter()
                .map(|packet| packet.opcode)
                .collect::<Vec<_>>(),
            [0x3bc2, 0x0b40, 0x6937, 0x3bc2]
        );
        assert_eq!(
            outcome
                .events
                .iter()
                .filter(|event| matches!(
                    event,
                    ClientEvent::World(WorldEvent::Objects(ObjectUpdate::Container(_)))
                ))
                .count(),
            2
        );
    }
}

fn eqemu_forge() -> (Message, ObjectUpdate) {
    let object = ObjectUpdate::Spawn(GroundObject {
        drop_id: 71,
        model: "FORGE".into(),
        position: eq_network_game::world::Position::default(),
        object_type: 17,
    });
    let reply = ObjectUpdate::Container(ContainerView {
        player_id: 7,
        drop_id: 71,
        open: true,
        object_type: 17,
        icon: 1,
        name: "Forge".into(),
    });
    (Message::Event(WorldEvent::Objects(object)), reply)
}

#[test]
fn a_received_reset_releases_the_gate_without_consuming_or_creating_items() {
    let (mut features, mut world) = setup();
    testing::run(|out| features.handle(&combine(InventorySlot(23)), &mut world, out))
        .result
        .unwrap();
    let before = world.inventory.items().clone();
    let outcome = testing::run(|out| {
        features.tick(Instant::now() + Duration::from_secs(60), &mut world, out)
    });
    outcome.result.unwrap();
    assert!(outcome
        .sent
        .iter()
        .all(|packet| packet.opcode != eq_network_game::tradeskills::COMBINE_OPCODE));
    assert_ne!(features.holds(&world, Instant::now()), []);
    hear_packets(&mut features, &mut world, 0x0b40, &[1]);
    assert_ne!(features.holds(&world, Instant::now()), []);
    hear_packets(&mut features, &mut world, 0x0b40, &[]);
    assert_eq!(features.holds(&world, Instant::now()), []);
    assert_eq!(world.inventory.items(), &before);
    // Failed recipes can reset the same gate with no item updates. Nothing
    // here treats that reset, local submission or silence as a successful craft.
    assert!(!world.inventory.items().contains_key(&InventorySlot::CURSOR));
    // Only an actual server inventory mutation consumes a component.
    let consumed = [
        261_u32.to_le_bytes(),
        u32::MAX.to_le_bytes(),
        u32::MAX.to_le_bytes(),
    ]
    .concat();
    hear_packets(&mut features, &mut world, 0x4d81, &consumed);
    assert!(!world.inventory.items().contains_key(&InventorySlot(261)));
    assert!(world.inventory.items().contains_key(&InventorySlot(23)));
    let outcome = testing::run(|out| {
        features.observe(
            &Message::Event(WorldEvent::Inventory(InventoryUpdate::Cursor(vec![
                testing::item(30),
            ]))),
            &mut world,
            out,
        )
    });
    outcome.result.unwrap();
    assert_eq!(outcome.sent, []);
    assert!(world.inventory.items().contains_key(&InventorySlot::CURSOR));
}

#[test]
fn the_current_container_and_components_are_rechecked_before_sending() {
    for case in 0..4 {
        let (mut features, mut world) = setup();
        let mut items = world
            .inventory
            .items()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        match case {
            0 => items.retain(|item| item.slot != InventorySlot(261)),
            1 => items.push(testing::item(30)),
            2 => {
                items
                    .iter_mut()
                    .find(|item| item.slot == InventorySlot(261))
                    .unwrap()
                    .stack_count = Some(2);
            }
            _ => {
                items
                    .iter_mut()
                    .find(|item| item.slot == InventorySlot(23))
                    .unwrap()
                    .rules
                    .bag_type = 5;
            }
        }
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(items));
        world.inventory = inventory.into();
        let before = world.inventory.items().clone();
        let outcome =
            testing::run(|out| features.handle(&combine(InventorySlot(23)), &mut world, out));
        assert!(outcome.result.unwrap());
        assert_eq!(outcome.sent, []);
        assert!(matches!(
            outcome.events.as_slice(),
            [ClientEvent::World(WorldEvent::CombineRefused { .. })]
        ));
        assert_eq!(world.inventory.items(), &before);
        assert_eq!(features.holds(&world, Instant::now()), []);
    }
    let (mut features, mut world) = setup();
    let old = combine(InventorySlot(23));
    let changed = Message::Event(WorldEvent::Inventory(InventoryUpdate::Set(vec![
        testing::item(261),
    ])));
    testing::run(|out| features.observe(&changed, &mut world, out))
        .result
        .unwrap();
    let outcome = testing::run(|out| features.handle(&old, &mut world, out));
    assert!(outcome.result.unwrap());
    assert_eq!(outcome.sent, []);
    assert!(
        matches!(outcome.events.as_slice(), [ClientEvent::World(WorldEvent::CombineRefused { reason, .. })]
        if reason == "The inventory changed since that combine was requested")
    );
}

#[test]
fn a_combine_reset_never_confirms_an_unrelated_inventory_projection() {
    let (mut features, mut world) = setup();
    let mut items = world
        .inventory
        .items()
        .values()
        .cloned()
        .collect::<Vec<_>>();
    items.push(testing::item(22));
    let mut inventory = Inventory::default();
    inventory.apply(InventoryUpdate::Snapshot(items.clone()));
    items
        .iter_mut()
        .find(|item| item.slot == InventorySlot(22))
        .unwrap()
        .slot = InventorySlot(24);
    inventory.apply(InventoryUpdate::Prediction(items));
    assert!(inventory.predicted());
    let unresolved = inventory.unresolved_predictions();
    assert!(unresolved > 0);
    world.inventory = inventory.into();
    let outcome = testing::run(|out| features.handle(&combine(InventorySlot(23)), &mut world, out));
    assert!(outcome.result.unwrap());
    assert_eq!(outcome.sent.len(), 1);
    hear_packets(&mut features, &mut world, 0x0b40, &[]);
    assert_eq!(features.holds(&world, Instant::now()), []);
    assert!(world.inventory.predicted());
    assert_eq!(world.inventory.unresolved_predictions(), unresolved);
}

#[test]
fn a_new_admission_drops_a_pending_combine_and_other_profiles_keep_their_offers() {
    let (mut features, mut world) = setup();
    testing::run(|out| features.handle(&combine(InventorySlot(23)), &mut world, out))
        .result
        .unwrap();
    assert_ne!(features.holds(&world, Instant::now()), []);
    let (fresh, world) = setup_in(6);
    assert_eq!(fresh.holds(&world, Instant::now()), []);
    assert_eq!(
        fresh.refusal(&combine(InventorySlot(23)), &world, Instant::now()),
        Some("That was meant for an earlier zone visit")
    );
    for (protocol, carried, station) in [
        (ServerProtocol::Project1999, true, true),
        (ServerProtocol::EqEmu, true, true),
        (ServerProtocol::Quarm, false, false),
        (ServerProtocol::Takp, false, false),
    ] {
        let server = servers::server_type(protocol);
        let mut features = Features::new(server, "Tester", AutoEat::default());
        let caps = features.capabilities();
        assert_eq!(
            caps.contains(&Capability::Tradeskills),
            carried,
            "{protocol:?}"
        );
        assert_eq!(
            caps.contains(&Capability::WorldContainers),
            station,
            "{protocol:?}"
        );
        if !carried {
            let (_, mut world) = setup();
            for command in [
                combine(InventorySlot(23)),
                combine(WORLD_CONTAINER),
                ClientCommand::OpenContainer {
                    session_id: 5,
                    drop_id: 71,
                    created: Instant::now(),
                },
                ClientCommand::CloseContainer {
                    session_id: 5,
                    drop_id: 42,
                    created: Instant::now(),
                },
            ] {
                let outcome = testing::run_on(server.wire(), |out| {
                    features.handle(&command, &mut world, out)
                });
                assert!(!outcome.result.unwrap(), "{protocol:?}");
                assert!(outcome.sent.is_empty(), "{protocol:?}");
            }
        }
    }
}
