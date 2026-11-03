//! Synthetic decoder failures through the actual receive diagnostic path.
use super::*;
use crate::client::{session::feature::testing, ServerProtocol};
use eq_network_game::{
    food::AutoEat,
    inventory::InventoryUpdate,
    message::Part,
    world::{SpawnKind, WorldEvent},
};

fn receive(opcode: u16, body: &[u8]) -> Vec<ClientEvent> {
    let server = servers::server_type(ServerProtocol::Quarm);
    let mut features = Features::new(server, "Tester", AutoEat::default());
    let mut world = World::new(5);
    world.own_spawn = Some(7);
    world.player.admit(testing::player(7));
    world.admitted = Some(Instant::now());
    world.spawns.insert(testing::spawn(9, SpawnKind::Npc));
    features
        .admit(
            &Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(Vec::new()))),
            &mut world,
        )
        .unwrap();
    testing::run_on(server.wire(), |out| features.admitted(&mut world, out))
        .result
        .unwrap();
    // Admission queues the initial readiness request. Settle and verify that
    // lifecycle step before checking exactly what the received packet emits.
    let readiness = testing::run_on(server.wire(), |out| {
        settle(&mut features, &mut world, out, true)
    });
    assert!(readiness.result.unwrap().is_none());
    assert_eq!(readiness.sent, [], "admission settlement sent packets");
    assert!(
        matches!(
            readiness.events.as_slice(),
            [ClientEvent::World(WorldEvent::SpeedInputsRequired {
                session_id: 5,
                revision: 1,
            })]
        ),
        "expected only the initial admission readiness request: {:?}",
        readiness.events
    );
    let output = testing::run_on(server.wire(), |out| {
        hear(
            read_messages(server.wire(), opcode, body, server.item_rules()),
            &mut features,
            &mut world,
            out,
            true,
        )
    });
    assert!(output.result.unwrap().is_none());
    assert_eq!(output.sent, [], "received packet sent packets");
    output.events
}

fn one_diagnostic(events: &[ClientEvent]) -> &str {
    let [ClientEvent::Diagnostic(reason)] = events else {
        panic!("expected one existing diagnostic and no fabricated world event: {events:?}");
    };
    reason
}

#[test]
fn a_required_zero_death_id_keeps_its_reason_and_adds_only_packet_metadata_once() {
    let events = receive(0x4a40, &[0; 20]);
    assert_eq!(
        one_diagnostic(&events),
        "World update rejected: zero EQMac spawn ID (opcode=0x4a40, payload_len=20)"
    );
}

#[test]
fn the_same_reason_from_a_movement_batch_identifies_its_opcode_and_body_length() {
    let mut body = vec![0; 4 + 2 * 15];
    body[..4].copy_from_slice(&2u32.to_le_bytes());
    body[4..6].copy_from_slice(&9u16.to_le_bytes());
    // The second record names no spawn; the decoder still rejects the batch
    // rather than publishing the valid first position or accepting spawn zero.
    let events = receive(0x9f40, &body);
    assert_eq!(
        one_diagnostic(&events),
        "World update rejected: zero EQMac spawn ID (opcode=0x9f40, payload_len=34)"
    );
}

#[test]
fn normal_optional_zero_death_fields_stay_accepted_without_a_new_diagnostic() {
    let mut body = [0; 20];
    body[..2].copy_from_slice(&9u16.to_le_bytes());
    let events = receive(0x4a40, &body);
    let [ClientEvent::World(WorldEvent::Death(death))] = events.as_slice() else {
        panic!("expected the unchanged accepted death: {events:?}");
    };
    assert_eq!(
        (death.spawn_id, death.killer_id, death.corpse_id),
        (9, 0, 0)
    );
}

#[test]
fn an_inventory_rejection_keeps_its_existing_category_reason_and_message_count() {
    let server = servers::server_type(ServerProtocol::Quarm);
    let original = server.wire().messages(0x6441, &[], server.item_rules());
    let contextual = read_messages(server.wire(), 0x6441, &[], server.item_rules());
    let [Message::Unreadable {
        part: Part::Inventory,
        error: original,
    }] = original.as_slice()
    else {
        panic!("expected the real inventory decoder to reject the empty record");
    };
    let [Message::Unreadable {
        part: Part::Inventory,
        error: contextual,
    }] = contextual.as_slice()
    else {
        panic!("the diagnostic wrapper changed the inventory message category/count");
    };
    assert_eq!(contextual, original);
    assert!(!contextual.contains("opcode="));
}
