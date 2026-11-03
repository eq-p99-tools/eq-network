//! The Quarm feature assembly, using synthetic state and an in-memory wire.
mod spells;

use super::super::feature::testing;
use super::*;
use crate::client::ServerProtocol;
use eq_network_game::{
    food::AutoEat,
    inventory::InventoryUpdate,
    movement::{MovementMode, MovementRequest},
    world::{Capability, Position, WorldEvent},
    zoning::{BindPoint, Death},
};

fn admitted() -> (Features, World) {
    let server = servers::server_type(ServerProtocol::Quarm);
    let mut features = Features::new(server, "Tester", AutoEat::default());
    let mut world = World::new(5);
    world.own_spawn = Some(7);
    let mut player = testing::player(7);
    (player.run_speed, player.walk_speed, player.level) = (0.7, 0.46, 50);
    world.player.admit(player);
    world.admitted = Some(Instant::now());
    features
        .admit(
            &Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(vec![]))),
            &mut world,
        )
        .unwrap();
    testing::run_on(server.wire(), |out| features.admitted(&mut world, out))
        .result
        .unwrap();
    (features, world)
}

#[test]
fn quarm_targets_visible_spawns_and_follows_server_changes_without_attacking() {
    use eq_network_game::world::SpawnKind;
    let server = servers::server_type(ServerProtocol::Quarm);
    let (mut features, mut world) = admitted();
    assert!(features.capabilities().contains(&Capability::Targeting));
    assert!(!features.capabilities().contains(&Capability::Combat));
    world.spawns.insert(testing::spawn(8, SpawnKind::Npc));
    let mut hidden = testing::spawn(9, SpawnKind::Npc);
    hidden.invisible = true;
    world.spawns.insert(hidden);
    for spawn_id in [Some(7), Some(8), None] {
        let output = testing::run_on(server.wire(), |out| {
            features.handle(
                &ClientCommand::SelectTarget {
                    session_id: 5,
                    spawn_id,
                },
                &mut world,
                out,
            )
        });
        assert!(output.result.unwrap());
        assert_eq!(output.sent.len(), 1);
        assert_eq!(output.sent[0].opcode, 0x6241);
        assert_eq!(output.sent[0].body, spawn_id.unwrap_or(0).to_le_bytes());
        assert_eq!(*world.target, spawn_id);
    }
    for spawn_id in [9, 10] {
        let output = testing::run_on(server.wire(), |out| {
            features.handle(
                &ClientCommand::SelectTarget {
                    session_id: 5,
                    spawn_id: Some(spawn_id),
                },
                &mut world,
                out,
            )
        });
        assert!(output.result.unwrap());
        assert_eq!(output.sent.len(), 0);
        assert_eq!(*world.target, None);
        assert!(output
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::TargetRejected { .. }))));
    }
    let output = testing::run_on(server.wire(), |out| {
        features.observe(
            &Message::Event(WorldEvent::Target(Some(7))),
            &mut world,
            out,
        )
    });
    output.result.unwrap();
    assert_eq!(*world.target, Some(7));
    assert_eq!(output.sent.len(), 0);
}

#[test]
fn quarm_moves_and_jumps_with_eqmac_packets_and_obeys_corrections() {
    let server = servers::server_type(ServerProtocol::Quarm);
    let (mut features, mut world) = admitted();
    assert!(features.capabilities().contains(&Capability::Moving));
    assert!(!features.capabilities().contains(&Capability::Inventory));
    let mut position = world.body.position().unwrap();
    let report = ClientCommand::SpeedInputs {
        session_id: 5,
        rate: 0,
        weight: 0,
        capacity: 100,
    };
    let output = testing::run_on(server.wire(), |out| {
        features.handle(&report, &mut world, out)
    });
    assert!(output.result.unwrap());
    // Leave a physics step between calibration and input without sleeping.
    let motion = world.body.motion_mut().unwrap();
    motion
        .calibrate(
            motion.calibration().unwrap(),
            Instant::now()
                .checked_sub(Duration::from_millis(200))
                .unwrap(),
        )
        .unwrap();
    // A small same-position heading update does not depend on wall-clock
    // displacement allowances and must use the Mac wire, not Titanium's.
    position.heading = 2.0;
    let output = testing::run_on(server.wire(), |out| {
        features.handle(
            &ClientCommand::Move(MovementRequest {
                session_id: 5,
                mode: MovementMode::Forward,
                position,
                created: Instant::now(),
            }),
            &mut world,
            out,
        )
    });
    assert!(output.result.unwrap());
    assert!(
        output.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::MotionSent { refused: None, .. })
        )),
        "{:?}",
        output.events
    );
    assert_eq!(output.unreliable.len(), 1);
    assert_eq!(output.unreliable[0].opcode, 0xf340);
    assert_eq!(output.unreliable[0].body.len(), 15);
    assert_eq!(&output.unreliable[0].body[..2], &7u16.to_le_bytes());
    let output = testing::run_on(server.wire(), |out| {
        features.handle(
            &ClientCommand::Jump {
                session_id: 5,
                created: Instant::now(),
            },
            &mut world,
            out,
        )
    });
    assert!(output.result.unwrap());
    assert_eq!(output.sent.len(), 1);
    assert_eq!(output.sent[0].opcode, 0x2040);
    assert_eq!(output.sent[0].body, Vec::<u8>::new());
    let corrected = Position {
        x: 15.0,
        y: -6.0,
        z: 2.5,
        heading: 32.0,
    };
    let output = testing::run_on(server.wire(), |out| {
        features.observe(
            &Message::Event(WorldEvent::Position {
                spawn_id: 7,
                position: corrected,
                velocity: [0.0; 3],
                forward: 0.0,
            }),
            &mut world,
            out,
        )
    });
    output.result.unwrap();
    assert_eq!(world.body.position(), Some(corrected));
    assert_eq!(world.player.as_ref().unwrap().position, corrected);
}

#[test]
fn quarm_server_reported_death_stops_motion_and_requests_the_saved_bind() {
    let server = servers::server_type(ServerProtocol::Quarm);
    let (mut features, mut world) = admitted();
    let bind = BindPoint {
        zone_id: 2,
        position: Position {
            x: 4.0,
            y: 5.0,
            z: 6.0,
            heading: 0.0,
        },
    };
    let output = testing::run_on(server.wire(), |out| -> Result<()> {
        features.observe(&Message::Bind(bind), &mut world, out)?;
        features.observe(
            &Message::Event(WorldEvent::Death(Death {
                spawn_id: 7,
                killer_id: 8,
                corpse_id: 7,
                bind_zone_id: 0,
                corpse_name: None,
            })),
            &mut world,
            out,
        )
    });
    output.result.unwrap();
    assert!(world.lifecycle.is_dead());
    assert!(world.lifecycle.blocks_motion());
    assert!(world.lifecycle.pending().is_none());
    let output = testing::run_on(server.wire(), |out| {
        features.tick(Instant::now() + Duration::from_secs(10), &mut world, out)
    });
    output.result.unwrap();
    assert_eq!(world.lifecycle.pending(), Some(&bind.offer()));
    assert_eq!(output.sent.len(), 1);
    assert_eq!(output.sent[0].opcode, 0xa340);
    assert_eq!(output.sent[0].body.len(), 76);
    let output = testing::run_on(server.wire(), |out| {
        features.tick(Instant::now() + Duration::from_secs(11), &mut world, out)
    });
    output.result.unwrap();
    assert_eq!(output.sent.len(), 0);
}
