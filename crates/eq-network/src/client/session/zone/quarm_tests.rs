//! The Quarm feature assembly, using synthetic state and an in-memory wire.
mod spells;

#[test]
fn only_quarms_death_bind_path_classifies_death_and_stale_notices_do_not() {
    for protocol in [ServerProtocol::Quarm, ServerProtocol::Takp] {
        let server = servers::server_type(protocol);
        let (_, mut world) = admitted();
        let mut features = Features::new(server, "Tester", AutoEat::default());
        world.zone = (2, 0);
        testing::run_on(server.wire(), |out| features.admitted(&mut world, out))
            .result
            .unwrap();
        world.take_news();
        let offer = eq_network_game::zoning::ZoneOffer {
            zone_id: 2,
            instance_id: 0,
            position: Position::default(),
            reason: 0,
            to_bind: protocol == ServerProtocol::Takp,
            solicited: true,
        };
        let output = testing::run_on(server.wire(), |out| {
            hear(
                vec![
                    Message::Event(WorldEvent::DeathBind { session_id: 999 }),
                    Message::ZoneOffer(offer),
                ],
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        output.result.unwrap();
        assert!(!world.lifecycle.is_dead());
        assert!(output.events.iter().all(|event| !matches!(
            event,
            ClientEvent::World(WorldEvent::DeathBind { session_id: 5 })
        )));
    }
}

#[test]
fn quarm_death_bind_without_a_death_packet_notifies_before_transfer_and_follows_approval() {
    use eq_network_game::zoning::ZoneAnswer;
    for packet_first in [false, true] {
        let server = servers::server_type(ServerProtocol::Quarm);
        let (mut features, mut world) = admitted();
        world.zone = (2, 0);
        let mut body = [0; 152];
        body[132..136].copy_from_slice(&2u32.to_le_bytes());
        let offer = eq_network_game::quarm::to_bind(&body).unwrap();
        let mut messages = Vec::new();
        if packet_first {
            messages.push(Message::Event(WorldEvent::Death(Death {
                spawn_id: 7,
                killer_id: 8,
                corpse_id: 7,
                bind_zone_id: 0,
                corpse_name: None,
            })));
        }
        messages.push(Message::ZoneOffer(offer.clone()));
        messages.push(Message::ZoneOffer(offer.clone()));
        let output = testing::run_on(server.wire(), |out| {
            hear(messages, &mut features, &mut world, out, true)
        });
        output.result.unwrap();
        assert!(world.lifecycle.is_dead());
        assert!(world.lifecycle.blocks_motion());
        assert_eq!(world.lifecycle.pending(), Some(&offer));
        assert_eq!(
            output.sent.len(),
            1,
            "duplicate offer sends no second request"
        );
        let deaths: Vec<_> = output
            .events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| {
                matches!(
                    event,
                    ClientEvent::World(WorldEvent::Death(_) | WorldEvent::DeathBind { .. })
                )
                .then_some(index)
            })
            .collect();
        assert_eq!(deaths.len(), 1);
        assert_eq!(
            output
                .events
                .iter()
                .filter(|event| matches!(event, ClientEvent::World(WorldEvent::DeathBind { .. })))
                .count(),
            usize::from(!packet_first)
        );
        let transfer = output
            .events
            .iter()
            .position(|event| matches!(event, ClientEvent::World(WorldEvent::ZoneTransfer(_))))
            .unwrap();
        assert!(deaths[0] < transfer);
        let answer = Message::ZoneAnswer(ZoneAnswer {
            character: "Tester".into(),
            zone_id: 185,
            instance_id: 0,
            position: None,
            success: 1,
        });
        let output = testing::run_on(server.wire(), |out| {
            hear(vec![answer], &mut features, &mut world, out, true)
        });
        output.result.unwrap();
        assert!(world.lifecycle.departing());
        assert_eq!(output.sent.len(), 2);
        let output = testing::run_on(server.wire(), |out| {
            hear(
                vec![Message::ZoneOffer(offer)],
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        output.result.unwrap();
        assert_eq!(output.sent, Vec::new());
    }
}

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
    assert!(features.capabilities().contains(&Capability::Combat));
    world.spawns.insert(testing::spawn(8, SpawnKind::Npc));
    let mut hidden = testing::spawn(9, SpawnKind::Npc);
    hidden.invisible = true;
    world.spawns.insert(hidden);
    for (request_id, spawn_id) in (1..).zip([Some(7), Some(8), None]) {
        let output = testing::run_on(server.wire(), |out| {
            features.handle(
                &ClientCommand::SelectTarget {
                    request_id,
                    session_id: 5,
                    spawn_id,
                    expected_pet: None,
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
    for (request_id, spawn_id) in (4..).zip([9, 10]) {
        let output = testing::run_on(server.wire(), |out| {
            features.handle(
                &ClientCommand::SelectTarget {
                    request_id,
                    session_id: 5,
                    spawn_id: Some(spawn_id),
                    expected_pet: None,
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
    assert!(features.capabilities().contains(&Capability::Inventory));
    let mut position = world.body.position().unwrap();
    let report = ClientCommand::SpeedInputs {
        session_id: 5,
        revision: 1,
        report_id: 1,
        state: eq_network_game::movement::SpeedInputState::Ready(
            eq_network_game::movement::SpeedInputs {
                eqmac_fatigue: None,
                rate: 0,
                weight: 0,
                capacity: 100,
            },
        ),
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
                heading: eq_network_game::movement::HeadingUpdate::Rate,
                position,
                created: Instant::now(),
                support: None,
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
