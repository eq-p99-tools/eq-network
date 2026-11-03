//! Quarm's spell requests and authoritative replies through its actual features.
mod consumption;
use super::*;
use eq_network_game::{command::Posture, inventory::InventorySlot, spells::SpellBook};

/// An admitted character with an eight-gem bar and a 256-slot `EQMac` book.
fn caster() -> (Features, World) {
    let (mut features, mut world) = admitted();
    let mut player = world.player.as_ref().unwrap().clone();
    player.class = Some(2);
    player.memorized_spells[0] = Some(202);
    world.player.admit(player);
    let mut profile = vec![0; 8460];
    profile[1846..1848].copy_from_slice(&202i16.to_le_bytes());
    features
        .admit(
            &Message::Event(WorldEvent::SpellBook(
                SpellBook::eqmac_profile(&profile).unwrap(),
            )),
            &mut world,
        )
        .unwrap();
    (features, world)
}

fn cast(spell_id: u32, target_id: u16) -> ClientCommand {
    ClientCommand::CastSpell {
        session_id: 5,
        gem: 0,
        spell_id,
        target_id,
        created: Instant::now(),
    }
}

/// Feed wire replies through the same explanation and observation path as a zone.
fn receive(features: &mut Features, world: &mut World, opcode: u16, body: &[u8]) {
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
    assert_eq!(output.sent.len(), 0);
}

fn send(
    features: &mut Features,
    world: &mut World,
    command: &ClientCommand,
) -> Vec<eq_network_game::command::EncodedCommand> {
    let wire = servers::server_type(ServerProtocol::Quarm).wire();
    assert!(actions::Held::new(features.holds(world, Instant::now()))
        .conflict(command)
        .is_none());
    let output = testing::run_on(wire, |out| features.handle(command, world, out));
    assert!(output.result.unwrap());
    output.sent
}

#[test]
fn casting_uses_the_mac_wire_and_waits_for_a_matching_reply_or_interruption() {
    let (mut features, mut world) = caster();
    let command = cast(202, 7);
    let sent = send(&mut features, &mut world, &command);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].opcode, 0x7e41);
    assert_eq!(sent[0].body, [0, 0, 202, 0, 255, 255, 7, 0, 0, 0, 0, 0]);
    receive(
        &mut features,
        &mut world,
        0xa940,
        &[7, 0, 202, 0, 0xe8, 3, 0, 0],
    );
    // A cast holds further casts until an answer for that cast arrives.
    receive(&mut features, &mut world, 0x7f41, &[90, 0, 203, 0]);
    assert!(actions::Held::new(features.holds(&world, Instant::now()))
        .conflict(&command)
        .is_some());
    receive(&mut features, &mut world, 0x7f41, &[90, 0, 202, 0]);
    assert_eq!(send(&mut features, &mut world, &command), sent);
    // EQMac interruptions carry no caster ID. The feature assembly must
    // resolve them to this character before releasing the cast guard.
    receive(&mut features, &mut world, 0x3542, &[0x21, 0x01, 0, 0]);
    assert_eq!(send(&mut features, &mut world, &command), sent);
}

#[test]
fn item_casts_use_current_inventory_without_predicting_charge_consumption() {
    use eq_network_game::inventory::{ClickEffect, ClickKind, ItemUse};
    let wire = servers::server_type(ServerProtocol::Quarm).wire();
    let (mut features, mut world) = caster();
    let slot = InventorySlot(22);
    let mut item = testing::item(slot.0);
    item.charges = 2;
    item.activation.maximum_charges = 5;
    item.activation.effect = Some(ClickEffect {
        spell_id: 202,
        kind: ClickKind::Expendable,
        required_level: 1,
        effect_level: 1,
        cast_time_ms: 1000,
        recast_delay_seconds: 0,
        recast_type: 0,
    });
    testing::run_on(wire, |out| {
        features.observe(
            &Message::Event(WorldEvent::Inventory(InventoryUpdate::Set(vec![item]))),
            &mut world,
            out,
        )
    })
    .result
    .unwrap();
    let request = ItemUse {
        request_id: 1,
        session_id: 5,
        revision: world.inventory.revision(),
        slot,
        target_id: 7,
        created: Instant::now(),
    };
    let stale = ClientCommand::UseItem(ItemUse {
        revision: request.revision - 1,
        ..request.clone()
    });
    let output = testing::run_on(wire, |out| features.handle(&stale, &mut world, out));
    assert!(output.result.unwrap());
    assert_eq!(output.sent.len(), 0);
    assert!(output.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::ItemUseAction { error: Some(_), .. })
    )));
    let sent = send(&mut features, &mut world, &ClientCommand::UseItem(request));
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].opcode, 0x7e41);
    assert_eq!(sent[0].body, [10, 0, 202, 0, 22, 0, 7, 0, 0, 0, 0, 0]);
    assert_eq!(world.inventory.items()[&slot].charges, 2);
    assert!(features.capabilities().contains(&Capability::Inventory));
}

#[test]
fn casting_refuses_a_changed_gem_or_unavailable_target_without_sending() {
    let wire = servers::server_type(ServerProtocol::Quarm).wire();
    let (mut features, mut world) = caster();
    for command in [cast(203, 7), cast(202, 99)] {
        let output = testing::run_on(wire, |out| features.handle(&command, &mut world, out));
        assert!(output.result.unwrap());
        assert_eq!(output.sent.len(), 0);
        assert!(output
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::CastRejected { .. }))));
        assert_eq!(features.holds(&world, Instant::now()).len(), 0);
    }
}

#[test]
fn memorizing_and_forgetting_wait_for_the_server_before_changing_the_gem() {
    let wire = servers::server_type(ServerProtocol::Quarm).wire();
    let (mut features, mut world) = caster();
    let memorize = ClientCommand::MemorizeSpell {
        session_id: 5,
        gem: 2,
        spell_id: 202,
        created: Instant::now(),
    };
    assert_eq!(
        send(&mut features, &mut world, &memorize),
        [eq_network_game::quarm::posture(7, Posture::Sitting).unwrap()]
    );
    let output = testing::run_on(wire, |out| {
        features.tick(Instant::now() + Duration::from_secs(6), &mut world, out)
    });
    output.result.unwrap();
    assert_eq!(output.sent.len(), 1);
    assert_eq!(output.sent[0].opcode, 0x8241);
    assert_eq!(output.sent[0].body, [2, 0, 0, 0, 202, 0, 0, 0, 1, 0, 0, 0]);
    assert_eq!(world.player.as_ref().unwrap().memorized_spells[2], None);
    receive(&mut features, &mut world, 0x8241, &output.sent[0].body);
    assert_eq!(
        world.player.as_ref().unwrap().memorized_spells[2],
        Some(202)
    );
    let sent = send(
        &mut features,
        &mut world,
        &ClientCommand::ForgetSpell {
            session_id: 5,
            gem: 2,
            spell_id: 202,
            created: Instant::now(),
        },
    );
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].opcode, 0x8241);
    assert_eq!(sent[0].body, [2, 0, 0, 0, 202, 0, 0, 0, 2, 0, 0, 0]);
    assert_eq!(
        world.player.as_ref().unwrap().memorized_spells[2],
        Some(202)
    );
    receive(&mut features, &mut world, 0x8241, &sent[0].body);
    assert_eq!(world.player.as_ref().unwrap().memorized_spells[2], None);
}

#[test]
fn standing_cancels_a_waiting_memorization_without_enabling_unchecked_book_edits() {
    let wire = servers::server_type(ServerProtocol::Quarm).wire();
    let (mut features, mut world) = caster();
    for capability in [Capability::MovingSpells, Capability::DeletingSpells] {
        assert!(!features.capabilities().contains(&capability));
    }
    assert!(features.capabilities().contains(&Capability::Abilities));
    send(
        &mut features,
        &mut world,
        &ClientCommand::MemorizeSpell {
            session_id: 5,
            gem: 2,
            spell_id: 202,
            created: Instant::now(),
        },
    );
    send(
        &mut features,
        &mut world,
        &ClientCommand::SetPosture {
            session_id: 5,
            spawn_id: 7,
            posture: Posture::Standing,
            created: Instant::now(),
        },
    );
    let output = testing::run_on(wire, |out| {
        features.tick(Instant::now() + Duration::from_secs(6), &mut world, out)
    });
    output.result.unwrap();
    assert_eq!(output.sent.len(), 0);
    assert_eq!(features.holds(&world, Instant::now()).len(), 0);
    assert_eq!(world.player.as_ref().unwrap().memorized_spells[2], None);
}

#[test]
fn scribing_uses_read_only_inventory_and_waits_for_authoritative_scroll_consumption() {
    let wire = servers::server_type(ServerProtocol::Quarm).wire();
    let (mut features, mut world) = caster();
    assert!(features.capabilities().contains(&Capability::Inventory));
    let mut scroll = testing::item(30);
    scroll.scroll_spell = Some(73);
    testing::run_on(wire, |out| {
        features.observe(
            &Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(vec![
                scroll,
            ]))),
            &mut world,
            out,
        )
    })
    .result
    .unwrap();
    let revision = world.inventory.revision();
    send(
        &mut features,
        &mut world,
        &ClientCommand::ScribeSpell {
            session_id: 5,
            revision,
            slot: 1,
            spell_id: 73,
            created: Instant::now(),
        },
    );
    let output = testing::run_on(wire, |out| {
        features.tick(Instant::now() + Duration::from_secs(6), &mut world, out)
    });
    output.result.unwrap();
    assert_eq!(output.sent.len(), 1);
    assert_eq!(output.sent[0].opcode, 0x8241);
    assert_eq!(output.sent[0].body, [1, 0, 0, 0, 73, 0, 0, 0, 0, 0, 0, 0]);
    receive(&mut features, &mut world, 0x8241, &output.sent[0].body);
    assert!(features
        .holds(&world, Instant::now())
        .iter()
        .any(|(held, _)| *held == actions::Resource::Inventory));
    let output = testing::run_on(wire, |out| {
        hear(
            vec![Message::Event(WorldEvent::Inventory(
                InventoryUpdate::Remove(InventorySlot::CURSOR),
            ))],
            &mut features,
            &mut world,
            out,
            true,
        )
    });
    assert!(output.result.unwrap().is_none());
    assert_eq!(world.inventory.items().len(), 0);
    assert_eq!(features.holds(&world, Instant::now()).len(), 0);
    assert!(output.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::Inventory(InventoryUpdate::Consume { .. }))
    )));
}
