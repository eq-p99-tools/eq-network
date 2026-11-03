//! Rapid moves through the zone's real admission, policy, gate and wire.
use super::super::feature::testing;
use super::*;
use crate::client::ServerProtocol;
use eq_network_game::{
    food::AutoEat,
    inventory::{InventoryItem, InventoryMove, InventorySlot, InventoryUpdate, MoveQuantity},
    world::WorldEvent,
};
use std::num::NonZeroU32;

fn setup(protocol: ServerProtocol, items: Vec<InventoryItem>) -> (Features, World) {
    let server = servers::server_type(protocol);
    let mut features = Features::new(server, "Synthetic", AutoEat::Anything);
    let mut world = World::new(5);
    world.player.admit(testing::player(7));
    world.own_spawn = Some(7);
    world.admitted = Some(Instant::now());
    features
        .admit(
            &Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(items))),
            &mut world,
        )
        .unwrap();
    testing::run_on(server.wire(), |out| features.admitted(&mut world, out))
        .result
        .unwrap();
    (features, world)
}

fn command(world: &World, from: i32, to: i32, quantity: MoveQuantity) -> ClientCommand {
    ClientCommand::MoveInventory(InventoryMove {
        session_id: world.session_id,
        revision: world.inventory.revision(),
        from: InventorySlot(from),
        to: InventorySlot(to),
        quantity,
        created: Instant::now(),
    })
}

fn submit(
    protocol: ServerProtocol,
    features: &mut Features,
    world: &mut World,
    command: &ClientCommand,
) -> testing::Outcome<Result<bool>> {
    testing::run_on(servers::server_type(protocol).wire(), |out| {
        if let Some(reason) = features.refusal(command, world, Instant::now()) {
            actions::refuse(command, reason, out.log)?;
            Ok(true)
        } else {
            features.handle(command, world, out)
        }
    })
}

fn observe(
    protocol: ServerProtocol,
    features: &mut Features,
    world: &mut World,
    update: InventoryUpdate,
) {
    testing::run_on(servers::server_type(protocol).wire(), |out| {
        features.observe(&Message::Event(WorldEvent::Inventory(update)), world, out)
    })
    .result
    .unwrap();
}

fn stack(slot: i32, count: u32) -> InventoryItem {
    let mut item = testing::item(slot);
    item.stack_count = Some(count);
    item.rules.stack_size = 20;
    item
}

#[test]
fn rapid_moves_send_in_order_on_takp_quarm_and_titanium() {
    for (protocol, opcode, cursor) in [
        (ServerProtocol::Takp, 0x2c41, 0),
        (ServerProtocol::Quarm, 0x2c41, 0),
        (ServerProtocol::EqEmu, 0x420f, 30),
    ] {
        let (mut features, mut world) = setup(protocol, vec![stack(22, 12)]);
        let mut sent = Vec::new();
        let start = Instant::now();
        for (index, (from, to)) in [(22, 30), (30, 22), (22, 30), (30, 23)]
            .into_iter()
            .enumerate()
        {
            if index == 1 {
                // Regression: returning a TAKP stack after 1.3 seconds must
                // not wait for our former two-second hold. No wall-clock sleep.
                testing::run_on(servers::server_type(protocol).wire(), |out| {
                    features.tick(start + Duration::from_millis(1300), &mut world, out)
                })
                .result
                .unwrap();
            }
            let request = command(&world, from, to, MoveQuantity::Whole);
            let outcome = submit(protocol, &mut features, &mut world, &request);
            outcome.result.unwrap();
            assert_eq!(outcome.sent.len(), 1, "{protocol:?}: {:?}", outcome.events);
            sent.extend(outcome.sent);
            // Replaying the same UI intent cannot duplicate the packet.
            assert_eq!(
                submit(protocol, &mut features, &mut world, &request).sent,
                []
            );
        }
        let words = |from: u32, to: u32| {
            [from, to, 0]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            sent.iter()
                .map(|packet| (packet.opcode, packet.body.clone()))
                .collect::<Vec<_>>(),
            [
                (opcode, words(22, cursor)),
                (opcode, words(cursor, 22)),
                (opcode, words(22, cursor)),
                (opcode, words(cursor, 23))
            ]
        );
        assert_eq!(world.inventory.unresolved_predictions(), 4);
        assert_eq!(
            world.inventory.items()[&InventorySlot(23)].stack_count,
            Some(12)
        );
        // Slot evidence, not invented per-move ACKs: the empty cursor alone
        // leaves every move's other changed slot uncertain.
        observe(
            protocol,
            &mut features,
            &mut world,
            InventoryUpdate::Remove(InventorySlot::CURSOR),
        );
        assert_eq!(world.inventory.unresolved_predictions(), 4);
        observe(
            protocol,
            &mut features,
            &mut world,
            InventoryUpdate::Remove(InventorySlot(22)),
        );
        assert_eq!(world.inventory.unresolved_predictions(), 1);
        let final_item = world.inventory.items()[&InventorySlot(23)].clone();
        observe(
            protocol,
            &mut features,
            &mut world,
            InventoryUpdate::Set(vec![final_item]),
        );
        assert_eq!(world.inventory.unresolved_predictions(), 0);
        assert!(!world.inventory.predicted());
        assert!(!world.inventory.stale());
    }
}

#[test]
fn eqmac_silence_and_out_of_order_corrections_never_confirm_moves() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let original = vec![stack(22, 12)];
        let (mut features, mut world) = setup(protocol, original.clone());
        for (from, to) in [(22, 30), (30, 23)] {
            let request = command(&world, from, to, MoveQuantity::Whole);
            assert_eq!(
                submit(protocol, &mut features, &mut world, &request)
                    .sent
                    .len(),
                1
            );
        }
        testing::run_on(servers::server_type(protocol).wire(), |out| {
            features.tick(Instant::now() + Duration::from_secs(60), &mut world, out)
        })
        .result
        .unwrap();
        assert_eq!(world.inventory.unresolved_predictions(), 2);
        // A late replacement describes an intermediate cursor, not the
        // current projection. Never assign it to the oldest pending move.
        observe(
            protocol,
            &mut features,
            &mut world,
            InventoryUpdate::Set(vec![stack(30, 12)]),
        );
        assert!(world.inventory.stale());
        for update in [
            InventoryUpdate::Remove(InventorySlot(23)),
            InventoryUpdate::Set(original.clone()),
            InventoryUpdate::Remove(InventorySlot::CURSOR),
        ] {
            observe(protocol, &mut features, &mut world, update.clone());
            observe(protocol, &mut features, &mut world, update);
        }
        assert!(world.inventory.stale());
        let request = command(&world, 22, 30, MoveQuantity::Whole);
        assert_eq!(
            submit(protocol, &mut features, &mut world, &request).sent,
            []
        );
        observe(
            protocol,
            &mut features,
            &mut world,
            InventoryUpdate::Snapshot(original),
        );
        assert!(!world.inventory.stale());
        assert_eq!(world.inventory.unresolved_predictions(), 0);
    }
}

#[test]
fn matching_stack_swap_sends_nothing_but_an_explicit_merge_still_works() {
    for protocol in [
        ServerProtocol::Takp,
        ServerProtocol::Quarm,
        ServerProtocol::EqEmu,
    ] {
        let (mut features, mut world) = setup(protocol, vec![stack(30, 3), stack(22, 4)]);
        let swap = command(&world, 30, 22, MoveQuantity::Whole);
        assert_eq!(submit(protocol, &mut features, &mut world, &swap).sent, []);
        let merge = command(
            &world,
            30,
            22,
            MoveQuantity::Count(NonZeroU32::new(3).unwrap()),
        );
        assert_eq!(
            submit(protocol, &mut features, &mut world, &merge)
                .sent
                .len(),
            1
        );
        assert_eq!(
            world.inventory.items()[&InventorySlot(22)].stack_count,
            Some(7)
        );
    }
}
