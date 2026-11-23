//! TAKP's and Quarm's combines and world containers through each server's
//! own wire, policy and inventory owner (6.598).
use super::*;
use crate::client::{session::feature::testing, ServerProtocol};
use eq_network_game::{
    food::AutoEat,
    inventory::{InventoryMove, InventorySlot, InventoryUpdate, MoveQuantity},
    tradeskills::WORLD_CONTAINER,
    world::WorldEvent,
};

const MAC: [ServerProtocol; 2] = [ServerProtocol::Takp, ServerProtocol::Quarm];
const FORGE: u32 = 72;

/// An admitted player (7) with a sewing kit in pack slot 23 holding two
/// components, next to a forge.
fn setup(protocol: ServerProtocol) -> (Features, World) {
    let mut features = Features::new(servers::server_type(protocol), "Tester", AutoEat::default());
    let mut world = World::new(5);
    world.player.admit(testing::player(7));
    world.own_spawn = Some(7);
    world.admitted = Some(Instant::now());
    let mut kit = testing::item(23);
    kit.bag_slots = 10;
    kit.rules.bag_type = 16;
    let mut inventory = eq_network_game::inventory::Inventory::default();
    inventory.apply(InventoryUpdate::Snapshot(vec![
        kit,
        testing::item(261),
        testing::item(262),
    ]));
    world.inventory = inventory.into();
    testing::run_on(servers::server_type(protocol).wire(), |out| {
        features.admitted(&mut world, out)
    })
    .result
    .unwrap();
    hear(protocol, &mut features, &mut world, 0x2c40, &forge());
    (features, world)
}

/// A forge placement (`Object_Struct`), type 17 at +212, at the origin.
fn forge() -> Vec<u8> {
    let mut body = vec![0; 222];
    body[12..16].copy_from_slice(&FORGE.to_le_bytes());
    body[156..161].copy_from_slice(b"FORGE");
    body[208..212].copy_from_slice(&u32::MAX.to_le_bytes());
    body[212..216].copy_from_slice(&17_u32.to_le_bytes());
    body
}

/// Both servers' answer to the forge click (`ClickObjectAction_Struct`).
fn answer(open: u8) -> [u8; 20] {
    let mut body = [0; 20];
    body[..4].copy_from_slice(&7_u32.to_le_bytes());
    body[4..8].copy_from_slice(&FORGE.to_le_bytes());
    body[8] = open;
    body[10] = 17;
    body[12] = 10;
    body[16..18].copy_from_slice(&1100_u16.to_le_bytes());
    body
}

/// A made-up item record at `place`, as `OP_ObjectItemPacket` and
/// `OP_SummonedItem` carry it.
fn record(id: i16, place: i16) -> Vec<u8> {
    let mut record = vec![0; 360];
    record[..14].copy_from_slice(b"Synthetic item");
    record[175] = 1;
    record[176] = 1;
    record[180..182].copy_from_slice(&id.to_le_bytes());
    record[184..186].copy_from_slice(&place.to_le_bytes());
    record
}

/// The server's delete of an `EQMac` slot (`OP_MoveItem`, to and count all
/// ones).
fn delete(slot: i32) -> Vec<u8> {
    [
        slot.to_le_bytes(),
        u32::MAX.to_le_bytes(),
        u32::MAX.to_le_bytes(),
    ]
    .concat()
}

fn hear(
    protocol: ServerProtocol,
    features: &mut Features,
    world: &mut World,
    opcode: u16,
    body: &[u8],
) -> testing::Outcome<Result<()>> {
    let server = servers::server_type(protocol);
    let outcome = testing::run_on(server.wire(), |out| -> Result<()> {
        for mut message in server.wire().messages(opcode, body, server.item_rules()) {
            assert!(
                !matches!(message, Message::Unreadable { .. }),
                "{opcode:#06x}"
            );
            features.explain(&mut message, world);
            features.observe(&message, world, out)?;
        }
        Ok(())
    });
    outcome.result.as_ref().unwrap();
    outcome
}

fn command(
    protocol: ServerProtocol,
    features: &mut Features,
    world: &mut World,
    command: &ClientCommand,
) -> testing::Outcome<Result<bool>> {
    testing::run_on(servers::server_type(protocol).wire(), |out| {
        features.handle(command, world, out)
    })
}

fn open_forge() -> ClientCommand {
    ClientCommand::OpenContainer {
        session_id: 5,
        drop_id: FORGE,
        created: Instant::now(),
    }
}

fn close_forge() -> ClientCommand {
    ClientCommand::CloseContainer {
        session_id: 5,
        drop_id: FORGE,
        created: Instant::now(),
    }
}

fn combine(container: InventorySlot) -> ClientCommand {
    ClientCommand::Combine {
        session_id: 5,
        container,
        created: Instant::now(),
    }
}

fn move_item(world: &World, from: i32, to: i32) -> ClientCommand {
    ClientCommand::MoveInventory(InventoryMove {
        session_id: world.session_id,
        revision: world.inventory.revision(),
        sequence: None,
        from: InventorySlot(from),
        to: InventorySlot(to),
        quantity: MoveQuantity::Whole,
        created: Instant::now(),
    })
}

/// Opens the forge for the player and hears what it holds at place 0.
fn opened(protocol: ServerProtocol) -> (Features, World) {
    let (mut features, mut world) = setup(protocol);
    let outcome = command(protocol, &mut features, &mut world, &open_forge());
    // The click is the 8-byte `OP_ClickObject` a pickup sends.
    assert_eq!(outcome.sent.len(), 1);
    assert_eq!(outcome.sent[0].opcode, 0x2b40);
    assert_eq!(
        outcome.sent[0].body,
        [FORGE, 7].map(u32::to_le_bytes).concat()
    );
    hear(protocol, &mut features, &mut world, 0xd740, &answer(1));
    assert!(world.container.is_open());
    hear(protocol, &mut features, &mut world, 0xfb40, &record(99, 0));
    assert!(world.inventory.items().contains_key(&InventorySlot(4000)));
    (features, world)
}

#[test]
fn a_mac_forge_opens_holds_its_items_and_a_close_leaves_them_inside() {
    for protocol in MAC {
        let (mut features, mut world) = opened(protocol);
        // The close repeats the server's answer with `open` cleared.
        let outcome = command(protocol, &mut features, &mut world, &close_forge());
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, 0xd740);
        assert_eq!(outcome.sent[0].body, answer(0));
        assert!(!world.container.is_open());
        // A second open waits for the server's answer to the close.
        let outcome = command(protocol, &mut features, &mut world, &open_forge());
        assert_eq!(outcome.sent, []);
        // The empty answer ends the use; the items stay in the forge, so
        // they leave the player's view.
        hear(protocol, &mut features, &mut world, 0xd740, &[]);
        assert!(world.container.idle());
        assert!(!world.inventory.items().keys().any(|slot| slot.is_world()));
        let outcome = command(protocol, &mut features, &mut world, &open_forge());
        assert_eq!(outcome.sent.len(), 1);
    }
}

#[test]
fn a_mac_forge_in_use_by_another_holds_nothing_and_takes_no_moves() {
    for protocol in MAC {
        let (mut features, mut world) = setup(protocol);
        command(protocol, &mut features, &mut world, &open_forge());
        hear(protocol, &mut features, &mut world, 0xd740, &answer(0));
        assert!(world.container.idle());
        hear(protocol, &mut features, &mut world, 0x7841, &record(55, 0));
        assert!(world.inventory.items().contains_key(&InventorySlot::CURSOR));
        let moving = move_item(&world, 30, 4000);
        let outcome = command(protocol, &mut features, &mut world, &moving);
        assert!(outcome.sent.is_empty(), "{protocol:?}");
        // Nothing waits: the forge can be clicked again.
        let outcome = command(protocol, &mut features, &mut world, &open_forge());
        assert_eq!(outcome.sent.len(), 1);
    }
}

#[test]
fn a_mac_forge_takes_from_the_cursor_and_gives_back_onto_it_only() {
    for protocol in MAC {
        let (mut features, mut world) = opened(protocol);
        // Out onto an empty pack slot is refused before sending.
        let moving = move_item(&world, 4000, 24);
        let outcome = command(protocol, &mut features, &mut world, &moving);
        assert!(outcome.sent.is_empty(), "{protocol:?}");
        // Out onto the cursor goes, in Mac numbers: place 4000 to the
        // cursor's 0.
        let moving = move_item(&world, 4000, 30);
        let outcome = command(protocol, &mut features, &mut world, &moving);
        assert_eq!(outcome.sent.len(), 1, "{protocol:?}");
        assert_eq!(
            outcome.sent[0].body,
            [4000_u32, 0, 0].map(u32::to_le_bytes).concat()
        );
        // A component from the kit cannot go straight in.
        let moving = move_item(&world, 261, 4001);
        let outcome = command(protocol, &mut features, &mut world, &moving);
        assert!(outcome.sent.is_empty(), "{protocol:?}");
    }
}

#[test]
fn a_lore_refused_mac_pickup_stays_in_the_forge() {
    for protocol in MAC {
        let (mut features, mut world) = opened(protocol);
        let before = world.inventory.items()[&InventorySlot(4000)].clone();
        let moving = move_item(&world, 4000, 30);
        command(protocol, &mut features, &mut world, &moving);
        assert!(world.inventory.items().contains_key(&InventorySlot::CURSOR));
        // The delete of the destination, then the duplicate-lore line.
        hear(protocol, &mut features, &mut world, 0x2c41, &delete(0));
        assert_eq!(
            world.inventory.items().get(&InventorySlot(4000)),
            Some(&before)
        );
        assert!(!world.inventory.items().contains_key(&InventorySlot::CURSOR));
    }
}

#[test]
fn a_mac_forge_combine_clears_before_its_reset_and_holds_nothing_after() {
    for protocol in MAC {
        let (mut features, mut world) = opened(protocol);
        let outcome = command(
            protocol,
            &mut features,
            &mut world,
            &combine(WORLD_CONTAINER),
        );
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, 0x0541);
        let mut expected = [0_u8; 32];
        expected[4..6].copy_from_slice(&1000_i16.to_le_bytes());
        assert_eq!(outcome.sent[0].body, expected);
        assert_ne!(features.holds(&world, Instant::now()), []);
        // A second combine waits for this one.
        let outcome = command(
            protocol,
            &mut features,
            &mut world,
            &combine(WORLD_CONTAINER),
        );
        assert_eq!(outcome.sent, []);
        // `OP_ClearObject`, the product, then the reset.
        hear(
            protocol,
            &mut features,
            &mut world,
            0x0542,
            &[1, 0, 0, 0, 0, 0, 0, 0],
        );
        assert!(!world.inventory.items().contains_key(&InventorySlot(4000)));
        hear(protocol, &mut features, &mut world, 0x7841, &record(55, 0));
        hear(protocol, &mut features, &mut world, 0x0541, &[]);
        assert_eq!(features.holds(&world, Instant::now()), []);
        assert!(world.container.is_open());
        assert_eq!(
            world.inventory.items()[&InventorySlot::CURSOR].details.id,
            55
        );
    }
}

#[test]
fn a_mac_kit_combine_follows_the_deletes_even_when_the_kit_is_replaced() {
    for protocol in MAC {
        let (mut features, mut world) = setup(protocol);
        let outcome = command(
            protocol,
            &mut features,
            &mut world,
            &combine(InventorySlot(23)),
        );
        assert_eq!(outcome.sent.len(), 1);
        let mut expected = [0_u8; 32];
        expected[4] = 23;
        assert_eq!(outcome.sent[0].body, expected);
        // Each component's delete, in Mac numbers (one lower), as received.
        hear(protocol, &mut features, &mut world, 0x2c41, &delete(260));
        assert!(!world.inventory.items().contains_key(&InventorySlot(261)));
        assert!(world.inventory.items().contains_key(&InventorySlot(262)));
        hear(protocol, &mut features, &mut world, 0x2c41, &delete(261));
        // The product, then a recipe that replaces its container deletes
        // the kit itself, and the reset follows.
        hear(protocol, &mut features, &mut world, 0x7841, &record(55, 0));
        hear(protocol, &mut features, &mut world, 0x2c41, &delete(23));
        hear(protocol, &mut features, &mut world, 0x0541, &[]);
        assert_eq!(features.holds(&world, Instant::now()), []);
        assert_eq!(
            world.inventory.items().keys().copied().collect::<Vec<_>>(),
            [InventorySlot::CURSOR]
        );
    }
}

#[test]
fn a_mac_combine_refused_without_a_reset_waits_a_bounded_time() {
    for protocol in MAC {
        let (mut features, mut world) = setup(protocol);
        let start = Instant::now();
        command(
            protocol,
            &mut features,
            &mut world,
            &combine(InventorySlot(23)),
        );
        // A class or race refusal is a line and no reset: the line's words
        // release nothing, only the time does.
        let tick = |features: &mut Features, world: &mut World, at| {
            testing::run_on(servers::server_type(protocol).wire(), |out| {
                features.tick(at, world, out)
            })
            .result
            .unwrap();
        };
        tick(&mut features, &mut world, start + Duration::from_secs(9));
        assert_ne!(features.holds(&world, Instant::now()), []);
        tick(&mut features, &mut world, start + Duration::from_secs(11));
        assert_eq!(features.holds(&world, Instant::now()), []);
        assert!(world.inventory.items().contains_key(&InventorySlot(261)));
    }
}

#[test]
fn a_mac_forge_closes_on_death_and_ends_on_despawn_or_a_new_zone() {
    for protocol in MAC {
        // Death: the session closes it, as neither server does.
        let (mut features, mut world) = opened(protocol);
        let death = Message::Event(WorldEvent::Death(eq_network_game::zoning::Death {
            spawn_id: 7,
            killer_id: 0,
            corpse_id: 0,
            bind_zone_id: 0,
            corpse_name: None,
        }));
        let outcome = testing::run_on(servers::server_type(protocol).wire(), |out| {
            features.observe(&death, &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].body, answer(0));
        assert!(!world.container.is_open());
        hear(protocol, &mut features, &mut world, 0xd740, &[]);
        assert!(world.container.idle());
        // Despawn: the forge leaving the zone ends the hold.
        let (mut features, mut world) = opened(protocol);
        hear(
            protocol,
            &mut features,
            &mut world,
            0x2b40,
            &[FORGE, 0].map(u32::to_le_bytes).concat(),
        );
        assert!(!world.container.is_open());
        assert!(!world.inventory.items().keys().any(|slot| slot.is_world()));
        // A new zone is a new world, with nothing open.
        let (_, next) = setup(protocol);
        assert!(next.container.idle());
    }
}
