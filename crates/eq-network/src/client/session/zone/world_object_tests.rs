//! Real server policy/zone dispatch with synthetic world-object packets.
use super::*;
use crate::client::{session::feature::testing, ServerProtocol};
use eq_network_game::{
    objects::ObjectUpdate,
    world::{Capability, WorldEvent},
};

fn read(protocol: ServerProtocol, opcode: u16, body: &[u8]) -> Vec<Message> {
    let server = servers::server_type(protocol);
    server.wire().messages(opcode, body, server.item_rules())
}

fn staged(protocol: ServerProtocol) -> (Features, World) {
    let server = servers::server_type(protocol);
    let mut features = Features::new(server, "Tester", eq_network_game::food::AutoEat::default());
    let mut world = World::new(5);
    let mut object = vec![0; 222];
    object[12..16].copy_from_slice(&71_u32.to_le_bytes());
    let model = b"IT63_ACTORDEF";
    object[156..156 + model.len()].copy_from_slice(model);
    // Synthetic zlib of one44-byte record: model DOOR, size100, ID3,
    // initial endpoint1 and inversion1. No captured bytes are used.
    let doors = vec![
        1, 0, 120, 156, 115, 241, 247, 15, 98, 192, 11, 82, 128, 152, 153, 129, 145, 145, 129, 1,
        0, 55, 58, 1, 158,
    ];
    for (opcode, body) in [
        (0xf641, vec![0, 0]),
        (0x2c40, object),
        (0xf741, doors),
        (0x8e40, vec![3, 3]),
    ] {
        let messages = read(protocol, opcode, &body);
        assert_eq!(messages.len(), 1);
        for message in messages {
            assert!(!matches!(message, Message::Unreadable { .. }));
            features.admit(&message, &mut world).unwrap();
        }
    }
    (features, world)
}

#[test]
fn eqmac_admission_replays_doors_but_offers_only_evidenced_ground_interaction() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let wire = servers::server_type(protocol).wire();
        let (mut features, mut world) = staged(protocol);
        assert!(features.capabilities().contains(&Capability::GroundItems));
        assert!(!features.capabilities().contains(&Capability::Doors));
        let outcome = testing::run_on(wire, |out| features.admitted(&mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert!(outcome.events.iter().any(|event| matches!(event,
            ClientEvent::World(WorldEvent::Doors(eq_network_game::doors::DoorUpdate::Spawn(doors)))
                if doors.len() == 1 && doors[0].id == 3 && doors[0].action == Some(3) && doors[0].active_endpoint() == Some(false))));
        assert!(outcome.events.iter().any(|event| matches!(event,
            ClientEvent::World(WorldEvent::Objects(ObjectUpdate::Snapshot(objects)))
                if objects.len() == 1 && objects[0].drop_id == 71)));
        world.player.admit(testing::player(7));
        let click = ClientCommand::ClickDoor {
            session_id: 5,
            door_id: 3,
            created: Instant::now(),
        };
        let outcome = testing::run_on(wire, |out| features.handle(&click, &mut world, out));
        assert!(!outcome.result.unwrap());
        assert_eq!(outcome.sent, []);
        assert!(wire
            .encode(
                &eq_network_game::request::Request::ClickDoor(3),
                eq_network_game::request::Sender {
                    name: "Tester",
                    spawn_id: Some(7)
                }
            )
            .is_err());
    }
}

#[test]
fn a_new_eqmac_zone_replays_empty_state_and_never_reuses_an_old_object() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let server = servers::server_type(protocol);
        let mut features =
            Features::new(server, "Tester", eq_network_game::food::AutoEat::default());
        let mut world = World::new(6);
        world.player.admit(testing::player(7));
        let outcome = testing::run_on(server.wire(), |out| features.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(outcome.events.iter().any(|event| matches!(event, ClientEvent::World(WorldEvent::Doors(eq_network_game::doors::DoorUpdate::Spawn(doors))) if doors.is_empty())));
        assert!(outcome.events.iter().any(|event| matches!(event, ClientEvent::World(WorldEvent::Objects(ObjectUpdate::Snapshot(objects))) if objects.is_empty())));
        let old_object = ClientCommand::PickUp {
            session_id: 6,
            drop_id: 71,
            created: Instant::now(),
        };
        let outcome = testing::run_on(server.wire(), |out| {
            features.handle(&old_object, &mut world, out)
        });
        assert!(outcome.result.unwrap());
        assert_eq!(outcome.sent, []);
        assert_eq!(features.holds(&world, Instant::now()), []);
    }
}

#[test]
fn eqmac_policy_routes_pickups_and_foreign_removal_cancels_the_wait() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let wire = servers::server_type(protocol).wire();
        let (mut features, mut world) = staged(protocol);
        world.player.admit(testing::player(7));
        world.admitted = Some(Instant::now());
        let pickup = || ClientCommand::PickUp {
            session_id: 5,
            drop_id: 71,
            created: Instant::now(),
        };
        let outcome = testing::run_on(wire, |out| features.handle(&pickup(), &mut world, out));
        assert!(outcome.result.unwrap());
        assert_eq!(
            outcome.sent,
            [eq_network_game::objects::eqmac::pickup(71, 7)]
        );
        assert!(world.inventory.items().is_empty());
        let outcome = testing::run_on(wire, |out| features.handle(&pickup(), &mut world, out));
        assert!(outcome.result.unwrap());
        assert_eq!(outcome.sent, []);
        let gone = [71_u32.to_le_bytes(), 9_u32.to_le_bytes()].concat();
        let outcome = testing::run_on(wire, |out| {
            hear(
                read(protocol, 0x2b40, &gone),
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        assert!(outcome.result.unwrap().is_none());
        assert_eq!(outcome.sent, []);
        assert!(world.objects.entries().is_empty());
        assert!(world.inventory.items().is_empty());
        assert_eq!(features.holds(&world, Instant::now()), []);
        // A snapshot is authoritative inventory state, not pickup success.
        let outcome = testing::run_on(wire, |out| {
            hear(
                read(protocol, 0xf641, &[0, 0]),
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        assert!(outcome.result.unwrap().is_none());
        assert_eq!(features.holds(&world, Instant::now()), []);
        let denied = [u32::MAX.to_le_bytes(), 7_u32.to_le_bytes()].concat();
        let outcome = testing::run_on(wire, |out| {
            hear(
                read(protocol, 0x2b40, &denied),
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        assert!(outcome.result.unwrap().is_none());
        assert_eq!(features.holds(&world, Instant::now()), []);
    }
}

#[test]
fn projected_empty_cursor_allows_pickup_without_acknowledging_prior_moves() {
    use eq_network_game::inventory::{InventoryMove, InventorySlot, InventoryUpdate, MoveQuantity};

    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let wire = servers::server_type(protocol).wire();
        let (mut features, mut world) = staged(protocol);
        let mut player = testing::player(7);
        player.class = Some(1);
        player.race = 1;
        player.level = 10;
        world.player.admit(player);
        world.own_spawn = Some(7);
        features
            .admit(
                &Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(vec![
                    testing::item(30),
                ]))),
                &mut world,
            )
            .unwrap();
        world.admitted = Some(Instant::now());
        let movement = ClientCommand::MoveInventory(InventoryMove {
            session_id: 5,
            revision: world.inventory.revision(),
            from: InventorySlot::CURSOR,
            to: InventorySlot(22),
            quantity: MoveQuantity::Whole,
            created: Instant::now(),
        });
        assert_eq!(features.refusal(&movement, &world, Instant::now()), None);
        let outcome = testing::run_on(wire, |out| features.handle(&movement, &mut world, out));
        assert!(outcome.result.unwrap());
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(world.inventory.unresolved_predictions(), 1);
        assert!(!world.inventory.items().contains_key(&InventorySlot::CURSOR));
        let pickup = ClientCommand::PickUp {
            session_id: 5,
            drop_id: 71,
            created: Instant::now(),
        };
        assert_eq!(features.refusal(&pickup, &world, Instant::now()), None);
        let outcome = testing::run_on(wire, |out| features.handle(&pickup, &mut world, out));
        assert!(outcome.result.unwrap());
        assert_eq!(
            outcome.sent,
            [eq_network_game::objects::eqmac::pickup(71, 7)]
        );
        assert_eq!(world.inventory.unresolved_predictions(), 1);

        let mut item = [0; 360];
        item[..14].copy_from_slice(b"Synthetic item");
        item[180..182].copy_from_slice(&1234_i16.to_le_bytes());
        let outcome = testing::run_on(wire, |out| {
            hear(
                read(protocol, 0x7841, &item),
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        assert!(outcome.result.unwrap().is_none());
        assert_eq!(
            world.inventory.items()[&InventorySlot::CURSOR].details.id,
            1234
        );
        assert_eq!(world.inventory.unresolved_predictions(), 1);
        assert!(!world.inventory.stale());
        let removed = [71_u32.to_le_bytes(), 7_u32.to_le_bytes()].concat();
        let outcome = testing::run_on(wire, |out| {
            hear(
                read(protocol, 0x2b40, &removed),
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        assert!(outcome.result.unwrap().is_none());
        assert_eq!(world.inventory.unresolved_predictions(), 1);
        assert_eq!(features.holds(&world, Instant::now()), []);
    }
}

#[test]
fn pending_eqmac_pickup_allows_inventory_moves_and_cancellation_preserves_their_ledger() {
    use eq_network_game::inventory::{InventoryMove, InventorySlot, InventoryUpdate, MoveQuantity};
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let wire = servers::server_type(protocol).wire();
        let (mut features, mut world) = staged(protocol);
        let mut player = testing::player(7);
        player.class = Some(1);
        player.race = 1;
        player.level = 10;
        world.player.admit(player);
        world.own_spawn = Some(7);
        features
            .admit(
                &Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(vec![
                    testing::item(22),
                ]))),
                &mut world,
            )
            .unwrap();
        world.admitted = Some(Instant::now());
        let pickup = ClientCommand::PickUp {
            session_id: 5,
            drop_id: 71,
            created: Instant::now(),
        };
        let outcome = testing::run_on(wire, |out| features.handle(&pickup, &mut world, out));
        assert!(outcome.result.unwrap());
        assert_eq!(
            outcome.sent,
            [eq_network_game::objects::eqmac::pickup(71, 7)]
        );
        assert!(features.refusal(&pickup, &world, Instant::now()).is_some());
        assert_eq!(
            features
                .holds(&world, Instant::now())
                .iter()
                .map(|(resource, _)| *resource)
                .collect::<Vec<_>>(),
            [actions::Resource::GroundPickup]
        );

        let movement = ClientCommand::MoveInventory(InventoryMove {
            session_id: 5,
            revision: world.inventory.revision(),
            from: InventorySlot(22),
            to: InventorySlot(23),
            quantity: MoveQuantity::Whole,
            created: Instant::now(),
        });
        assert_eq!(features.refusal(&movement, &world, Instant::now()), None);
        let outcome = testing::run_on(wire, |out| features.handle(&movement, &mut world, out));
        assert!(outcome.result.unwrap());
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(world.inventory.unresolved_predictions(), 1);
        assert!(!world.inventory.items().contains_key(&InventorySlot(22)));
        assert!(world.inventory.items().contains_key(&InventorySlot(23)));
        let gone = [71_u32.to_le_bytes(), 9_u32.to_le_bytes()].concat();
        let outcome = testing::run_on(wire, |out| {
            hear(
                read(protocol, 0x2b40, &gone),
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        assert!(outcome.result.unwrap().is_none());
        assert_eq!(outcome.sent, []);
        assert_eq!(features.holds(&world, Instant::now()), []);
        assert_eq!(
            world.inventory.unresolved_predictions(),
            1,
            "pickup cancellation is not an inventory move acknowledgement"
        );
    }
}

#[test]
fn eqmac_snapshot_cancels_the_wait_without_making_a_late_refusal_answer_a_new_pickup() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let wire = servers::server_type(protocol).wire();
        let (mut features, mut world) = staged(protocol);
        world.player.admit(testing::player(7));
        world.own_spawn = Some(7);
        world.admitted = Some(Instant::now());
        testing::run_on(wire, |out| features.admitted(&mut world, out))
            .result
            .unwrap();
        let pickup = || ClientCommand::PickUp {
            session_id: 5,
            drop_id: 71,
            created: Instant::now(),
        };
        let outcome = testing::run_on(wire, |out| features.handle(&pickup(), &mut world, out));
        assert!(outcome.result.unwrap());
        let outcome = testing::run_on(wire, |out| {
            hear(
                read(protocol, 0xf641, &[0, 0]),
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        assert!(outcome.result.unwrap().is_none());
        assert_eq!(outcome.sent, []);
        assert_eq!(features.holds(&world, Instant::now()), []);
        assert!(world.inventory.items().is_empty());
        let outcome = testing::run_on(wire, |out| features.handle(&pickup(), &mut world, out));
        assert!(outcome.result.unwrap());
        assert_eq!(
            outcome.sent,
            [eq_network_game::objects::eqmac::pickup(71, 7)]
        );
        let refused = [u32::MAX.to_le_bytes(), 7_u32.to_le_bytes()].concat();
        let outcome = testing::run_on(wire, |out| {
            hear(
                read(protocol, 0x2b40, &refused),
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        assert!(outcome.result.unwrap().is_none());
        assert!(features
            .refusal(&pickup(), &world, Instant::now())
            .is_some());
        // The local bound eventually cancels, without another outbound request.
        let outcome = testing::run_on(wire, |out| {
            features.tick(Instant::now() + Duration::from_secs(3), &mut world, out)
        });
        outcome.result.unwrap();
        assert!(
            outcome
                .sent
                .iter()
                .all(|packet| packet.opcode != eq_network_game::objects::eqmac::CLICK_OPCODE),
            "other features may heartbeat, but the pickup is never retried"
        );
        assert_eq!(features.holds(&world, Instant::now()), []);
    }
}
