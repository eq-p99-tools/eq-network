//! Wholly synthetic records through decoding, explanation, worker and host projection.
use super::*;
use eq_network_game::{
    inventory::{Inventory, InventoryMove, MoveQuantity, WeightUnavailable},
    movement::{SpeedInputState, SpeedInputs},
    GameDialect,
};

fn record(slot: u16, id: u16, spell: u16, weight: u8) -> Vec<u8> {
    let mut bytes = vec![0; 360];
    bytes[..16].copy_from_slice(b"Synthetic scroll");
    bytes[174] = weight;
    bytes[175] = 1;
    bytes[176] = 1;
    bytes[180..182].copy_from_slice(&id.to_le_bytes());
    bytes[184..186].copy_from_slice(&slot.to_le_bytes());
    bytes[253] = 20;
    bytes[276] = 3;
    bytes[279] = 7;
    bytes[280..282].copy_from_slice(&spell.to_le_bytes());
    bytes
}

fn mirror(host: &mut Inventory, events: &[ClientEvent]) {
    for event in events {
        if let ClientEvent::World(WorldEvent::Inventory(update)) = event {
            host.apply(update.clone());
        }
    }
}

fn reply(
    features: &mut Features,
    world: &mut World,
    host: &mut Inventory,
    opcode: u16,
    body: &[u8],
) -> Vec<ClientEvent> {
    let server = servers::server_type(ServerProtocol::Quarm);
    let wire = server.wire();
    let output = testing::run_on(wire, |out| {
        hear(
            wire.messages(opcode, body, server.item_rules()),
            features,
            world,
            out,
            true,
        )
    });
    assert!(output.result.unwrap().is_none());
    mirror(host, &output.events);
    assert_eq!(host, &*world.inventory);
    output.events
}

fn scrolls() -> (Features, World, Inventory) {
    let (mut features, mut world) = caster();
    let items = [(22, 90, 73), (23, 91, 74)]
        .into_iter()
        .map(|(slot, id, spell)| {
            let Some(InventoryUpdate::Set(mut items)) =
                eq_network_game::inventory::decode_quarm(0x6441, &record(slot, id, spell, 30))
                    .unwrap()
            else {
                panic!("synthetic item did not decode")
            };
            items.remove(0)
        })
        .collect();
    let snapshot = InventoryUpdate::Snapshot(items);
    features
        .admit(
            &Message::Event(WorldEvent::Inventory(snapshot.clone())),
            &mut world,
        )
        .unwrap();
    let host = (*world.inventory).clone();
    (features, world, host)
}

fn prepare(
    features: &mut Features,
    world: &mut World,
    host: &mut Inventory,
    from: i32,
    spell_id: u32,
    book_slot: u16,
) -> Vec<u8> {
    let wire = servers::server_type(ServerProtocol::Quarm).wire();
    let command = ClientCommand::MoveInventory(InventoryMove {
        session_id: 5,
        revision: world.inventory.revision(),
        from: InventorySlot(from),
        to: InventorySlot::CURSOR,
        quantity: MoveQuantity::Whole,
        created: Instant::now(),
    });
    assert!(features.refusal(&command, world, Instant::now()).is_none());
    let output = testing::run_on(wire, |out| features.handle(&command, world, out));
    assert!(output.result.unwrap());
    assert_eq!(output.sent.len(), 1);
    assert_eq!(output.sent[0].opcode, 0x2c41);
    mirror(host, &output.events);
    let output = testing::run_on(wire, |out| {
        features.tick(Instant::now() + Duration::from_secs(3), world, out)
    });
    output.result.unwrap();
    mirror(host, &output.events);
    assert_eq!(host, &*world.inventory);
    let command = ClientCommand::ScribeSpell {
        session_id: 5,
        revision: world.inventory.revision(),
        slot: book_slot,
        spell_id,
        created: Instant::now(),
    };
    send(features, world, &command);
    let output = testing::run_on(wire, |out| {
        features.tick(Instant::now() + Duration::from_secs(6), world, out)
    });
    output.result.unwrap();
    assert_eq!(output.sent.len(), 1);
    assert_eq!(output.sent[0].opcode, 0x8241);
    output.sent[0].body.clone()
}

fn removal() -> Vec<u8> {
    [0u32, u32::MAX, u32::MAX]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect()
}

#[test]
fn two_scribes_consume_only_their_scrolls_and_recover_a_current_movement_grant() {
    let (mut features, mut world, mut host) = scrolls();
    for (from, spell, book_slot, weight) in [(22, 73, 1, 30), (23, 74, 2, 0)] {
        let book = prepare(&mut features, &mut world, &mut host, from, spell, book_slot);
        reply(&mut features, &mut world, &mut host, 0x8241, &book);
        // A duplicate book result does not create another consumption receipt.
        reply(&mut features, &mut world, &mut host, 0x8241, &book);
        let events = reply(&mut features, &mut world, &mut host, 0x2c41, &removal());
        assert!(events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Inventory(InventoryUpdate::Consume {
                slot: InventorySlot(30),
                charge: false
            }))
        )));
        assert!(!host.predicted());
        assert!(!host.stale());
        assert_eq!(
            host.carried_weight_tenths(GameDialect::EqMac, 0),
            Ok(weight)
        );
        let output = testing::run_on(servers::server_type(ServerProtocol::Quarm).wire(), |out| {
            features.tick(Instant::now(), &mut world, out)?;
            settle(&mut features, &mut world, out, true)
        });
        assert!(output.result.unwrap().is_none());
        let revision = output
            .events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::SpeedInputsRequired { revision, .. }) => {
                    Some(*revision)
                }
                _ => None,
            })
            .next_back()
            .expect("consumption requests fresh speed inputs");
        let command = ClientCommand::SpeedInputs {
            session_id: 5,
            revision,
            report_id: 1,
            state: SpeedInputState::Ready(SpeedInputs {
                rate: 0,
                weight: u16::try_from(weight / 10).unwrap(),
                capacity: 90,
            }),
        };
        assert!(features.refusal(&command, &world, Instant::now()).is_none());
        send(&mut features, &mut world, &command);
        assert!(world.body.motion_mut().unwrap().calibration().is_some());
        let duplicate = reply(&mut features, &mut world, &mut host, 0x2c41, &removal());
        assert!(!duplicate.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Inventory(InventoryUpdate::Consume { .. }))
        )));
        assert_eq!(
            host.carried_weight_tenths(GameDialect::EqMac, 0),
            Ok(weight)
        );
    }
}

#[test]
fn cursor_refill_before_or_after_consumption_is_retained_without_a_false_correction() {
    for before in [true, false] {
        let (mut features, mut world, mut host) = scrolls();
        let book = prepare(&mut features, &mut world, &mut host, 22, 73, 1);
        reply(&mut features, &mut world, &mut host, 0x8241, &book);
        let refill = record(0, 92, 75, 40);
        if before {
            reply(&mut features, &mut world, &mut host, 0x7841, &refill);
        }
        reply(&mut features, &mut world, &mut host, 0x2c41, &removal());
        if !before {
            reply(&mut features, &mut world, &mut host, 0x7841, &refill);
        }
        assert_eq!(host.items()[&InventorySlot::CURSOR].details.id, 92);
        assert_eq!(host.queued().count(), 0);
        assert!(!host.stale());
        assert!(!host.predicted());
        assert_eq!(host.carried_weight_tenths(GameDialect::EqMac, 0), Ok(70));
    }
}

#[test]
fn removal_before_book_confirmation_remains_a_correction_even_after_the_reply() {
    let (mut features, mut world, mut host) = scrolls();
    let book = prepare(&mut features, &mut world, &mut host, 22, 73, 1);
    reply(&mut features, &mut world, &mut host, 0x2c41, &removal());
    reply(&mut features, &mut world, &mut host, 0x8241, &book);
    assert!(host.stale());
    assert_eq!(
        host.carried_weight_tenths(GameDialect::EqMac, 0),
        Err(WeightUnavailable::Inventory)
    );
}
