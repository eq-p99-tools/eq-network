//! Ordinary doors through their real owner and selected generation's wire.
use super::*;
use crate::client::{session::feature::testing, ServerProtocol};
use eq_network_game::{
    doors::Door,
    inventory::{Inventory, InventoryUpdate},
    world::Position,
};
use std::time::Duration;

fn door() -> Door {
    Door {
        id: 7,
        model: "DOOR".into(),
        position: Position::default(),
        incline: 0,
        size: 100,
        open_type: 0,
        state_at_spawn: 0,
        invert_state: 0,
        parameter: 0,
        action: None,
    }
}

fn setup() -> (Doors, World) {
    let mut owner = Doors::eqmac();
    let mut world = World::new(5);
    world.player.admit(testing::player(7));
    let mut inventory = Inventory::default();
    inventory.apply(InventoryUpdate::Snapshot(Vec::new()));
    world.inventory = inventory.into();
    owner.apply(&DoorUpdate::Spawn(vec![door()]), Instant::now(), &mut world);
    (owner, world)
}

fn click() -> ClientCommand {
    ClientCommand::ClickDoor {
        session_id: 5,
        door_id: 7,
        created: Instant::now(),
    }
}

#[test]
fn an_ordinary_send_never_opens_the_door_without_a_received_action() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let wire = super::super::servers::server_type(protocol).wire();
        let (mut owner, mut world) = setup();
        let outcome = testing::run_on(wire, |out| owner.handle(&click(), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [eqmac::click(7, 7).unwrap()]);
        assert!(matches!(
            outcome.events.as_slice(),
            [ClientEvent::World(DoorAction {
                door_id: 7,
                error: None,
                ..
            })]
        ));
        assert_eq!(owner.table.entries()[&7].action, None);
        // Public server DoorKeyCheck sends formatted string130 for a missing
        // key. It is a chat refusal, not a door movement or request ACK.
        let mut locked = vec![0; 6];
        locked[2..4].copy_from_slice(&130_u16.to_le_bytes());
        locked.push(0);
        let refusal = wire.chat(0x3642, &locked, false).unwrap().unwrap();
        assert_eq!(refusal.string_id, Some(130));
        assert_eq!(
            refusal.channel_name,
            eq_network_game::chat::ChannelName::System
        );
        let outcome = testing::run_on(wire, |out| -> Result<()> {
            for message in wire.messages(
                0x3642,
                &locked,
                super::super::servers::server_type(protocol).item_rules(),
            ) {
                owner.observe(&message, &mut world, out)?;
            }
            Ok(())
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert!(outcome.events.is_empty());
        let future = Instant::now() + Duration::from_secs(60);
        let outcome = testing::run_on(wire, |out| owner.tick(future, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert!(outcome.events.is_empty());
        // A locked/nonmovable server may send text or nothing and no MoveDoor.
        // Silence never animates the door, retries, or declares it unlocked.
        assert_eq!(owner.table.entries()[&7].action, None);
        assert_eq!(owner.table.entries()[&7].active_endpoint(), Some(false));
    }
}

#[test]
fn received_motion_blocks_repeated_use_until_the_local_hinge_settles() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let wire = super::super::servers::server_type(protocol).wire();
        let (mut owner, mut world) = setup();
        let started = Instant::now();
        owner.apply(&DoorUpdate::Move { id: 7, action: 2 }, started, &mut world);
        let outcome = testing::run_on(wire, |out| owner.handle(&click(), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert!(
            matches!(&outcome.events[..], [ClientEvent::World(DoorAction { error: Some(reason), .. })] if reason == "door is moving")
        );
        let outcome = testing::run_on(wire, |out| {
            owner.tick(started + Duration::from_millis(800), &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        let outcome = testing::run_on(wire, |out| owner.handle(&click(), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [eqmac::click(7, 7).unwrap()]);
        assert_eq!(owner.table.entries()[&7].action, Some(2));
    }
}

#[test]
fn an_unloaded_or_invalidated_cursor_never_sends_an_ordinary_click() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let wire = super::super::servers::server_type(protocol).wire();
        for invalidated in [false, true] {
            let (mut owner, mut world) = setup();
            let mut inventory = Inventory::default();
            if invalidated {
                inventory.apply(InventoryUpdate::Snapshot(Vec::new()));
                inventory.apply(InventoryUpdate::Invalidated);
            }
            world.inventory = inventory.into();
            let outcome = testing::run_on(wire, |out| owner.handle(&click(), &mut world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent, []);
            assert!(
                matches!(&outcome.events[..], [ClientEvent::World(DoorAction { error: Some(reason), .. })] if reason == "the inventory is not loaded")
            );
            assert_eq!(owner.table.entries()[&7].action, None);
        }
    }
}

#[test]
fn special_unknown_stale_and_held_item_requests_are_refused_without_a_packet() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let wire = super::super::servers::server_type(protocol).wire();
        let (mut owner, mut world) = setup();
        for open_type in [9, 54, 55, 56, 57, 58, 120, 255] {
            let mut definition = door();
            definition.open_type = open_type;
            owner.apply(
                &DoorUpdate::Spawn(vec![definition]),
                Instant::now(),
                &mut world,
            );
            let outcome = testing::run_on(wire, |out| owner.handle(&click(), &mut world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent, []);
            assert!(
                matches!(&outcome.events[..], [ClientEvent::World(DoorAction { error: Some(reason), .. })] if reason == "this door mechanism is not supported")
            );
        }
        let old = click();
        owner.changed_at = Instant::now() + Duration::from_secs(1);
        let outcome = testing::run_on(wire, |out| owner.handle(&old, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert!(
            matches!(&outcome.events[..], [ClientEvent::World(DoorAction { error: Some(reason), .. })] if reason == "stale door request")
        );
        owner.changed_at = Instant::now();
        owner.apply(&DoorUpdate::Spawn(vec![door()]), Instant::now(), &mut world);
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![testing::item(30)]));
        world.inventory = inventory.into();
        let outcome = testing::run_on(wire, |out| owner.handle(&click(), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert!(
            matches!(&outcome.events[..], [ClientEvent::World(DoorAction { error: Some(reason), .. })] if reason == "put down the item on your cursor first")
        );
    }
}
