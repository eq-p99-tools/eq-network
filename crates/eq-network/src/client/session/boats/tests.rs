use super::*;
use crate::client::session::{
    feature::testing,
    motion::Motion,
    servers::{server_type, Setup},
};
use eq_network_game::{
    boats::{Action, ControlBoat},
    movement::CarrierSample,
    world::Position,
    GameDialect,
};

fn admitted(out: &mut Out<'_, '_>, race: u32) -> (World, CarrierSample) {
    let mut world = World::new(5);
    let now = Instant::now();
    world.player.admit(testing::player(7));
    world.admitted = Some(now);
    Motion::new(false, false, None)
        .admitted(&mut world, out)
        .unwrap();
    let mut boat = testing::spawn(8, SpawnKind::Npc);
    boat.race = race;
    world.spawns.insert(boat.clone());
    world
        .carriers
        .observe(&WorldEvent::Spawns(vec![boat]), GameDialect::Titanium, now);
    let sample = world.carriers.latest(8).unwrap().sample();
    world.body.record_carrier_support(Some(sample));
    (world, sample)
}

fn acquire(sample: CarrierSample, position: Position) -> ClientCommand {
    ClientCommand::BoatControl {
        session_id: 5,
        request: BoatRequest::Acquire {
            support: sample,
            position,
        },
        created: Instant::now(),
    }
}

#[test]
fn acquire_and_support_loss_send_only_control_requests_and_late_echo_never_restores_control() {
    let outcome = testing::run(|out| {
        let (mut world, sample) = admitted(out, 141);
        let mut boats = Boats::default();
        boats
            .handle(
                &acquire(sample, world.body.position().unwrap()),
                &mut world,
                out,
            )
            .unwrap();
        assert!(matches!(
            boats.control.as_ref().unwrap().state(),
            ControlState::Acquiring { .. }
        ));
        let echo = |action| {
            Message::Event(WorldEvent::BoatControl(ControlBoat {
                boat_id: 8,
                action,
                unknown: [0; 3],
            }))
        };
        boats
            .observe(&echo(Action::Acquire), &mut world, out)
            .unwrap();
        assert!(matches!(
            boats.control.as_ref().unwrap().state(),
            ControlState::Acquired(_)
        ));
        boats
            .handle(
                &ClientCommand::BoatControl {
                    session_id: 5,
                    request: BoatRequest::Support(None),
                    created: Instant::now(),
                },
                &mut world,
                out,
            )
            .unwrap();
        boats
            .observe(&echo(Action::Acquire), &mut world, out)
            .unwrap();
        assert!(matches!(
            boats.control.as_ref().unwrap().state(),
            ControlState::Releasing(_)
        ));
        boats
            .observe(&echo(Action::Release), &mut world, out)
            .unwrap();
        assert!(matches!(
            boats.control.as_ref().unwrap().state(),
            ControlState::Released(_)
        ));
        boats
            .handle(
                &acquire(sample, world.body.position().unwrap()),
                &mut world,
                out,
            )
            .unwrap();
        boats
            .observe(&echo(Action::Acquire), &mut world, out)
            .unwrap();
        assert!(matches!(
            boats.control.as_ref().unwrap().state(),
            ControlState::Acquired(_)
        ));
    });
    assert_eq!(
        outcome
            .sent
            .iter()
            .map(|packet| (packet.opcode, packet.body.clone()))
            .collect::<Vec<_>>(),
        vec![
            (0x2c81, vec![8, 0, 0, 0, 1, 0, 0, 0]),
            (0x2c81, vec![8, 0, 0, 0, 0, 0, 0, 0]),
            (0x2c81, vec![8, 0, 0, 0, 1, 0, 0, 0])
        ]
    );
    assert_eq!(outcome.unreliable, []);
}

#[test]
fn replacement_or_changed_rider_position_cannot_acquire() {
    let outcome = testing::run(|out| {
        let (mut world, sample) = admitted(out, 141);
        let mut boats = Boats::default();
        let moved = Position {
            x: 10.0,
            ..world.body.position().unwrap()
        };
        boats
            .handle(&acquire(sample, moved), &mut world, out)
            .unwrap();
        assert!(boats.control.is_none());
        world.body.record_carrier_support(None);
        boats
            .handle(
                &acquire(sample, world.body.position().unwrap()),
                &mut world,
                out,
            )
            .unwrap();
        assert!(boats.control.is_none());
        world.body.record_carrier_support(Some(sample));
        let mut replacement = testing::spawn(8, SpawnKind::Npc);
        replacement.race = 141;
        world.carriers.observe(
            &WorldEvent::Spawns(vec![replacement]),
            GameDialect::Titanium,
            Instant::now(),
        );
        boats
            .handle(
                &acquire(sample, world.body.position().unwrap()),
                &mut world,
                out,
            )
            .unwrap();
        assert!(boats.control.is_none());
    });
    assert_eq!(outcome.sent, []);
}

#[test]
fn boat_capability_remains_an_eqemu_source_scope() {
    let setup = Setup::new("Tester", eq_network_game::food::AutoEat::default());
    for protocol in [
        super::super::super::ServerProtocol::Project1999,
        super::super::super::ServerProtocol::Quarm,
        super::super::super::ServerProtocol::Takp,
    ] {
        assert!(server_type(protocol).boats(&setup).is_none());
    }
    assert!(server_type(super::super::super::ServerProtocol::EqEmu)
        .boats(&setup)
        .is_some());
}

#[test]
fn accepted_worker_support_loss_releases_even_without_a_frontend_support_command() {
    let outcome = testing::run(|out| {
        let (mut world, sample) = admitted(out, 141);
        let mut boats = Boats::default();
        boats
            .handle(
                &acquire(sample, world.body.position().unwrap()),
                &mut world,
                out,
            )
            .unwrap();
        world.body.record_carrier_support(None);
        boats.tick(Instant::now(), &mut world, out).unwrap();
        assert!(matches!(
            boats.control.as_ref().unwrap().state(),
            ControlState::Releasing(_)
        ));
    });
    assert_eq!(outcome.sent.len(), 2);
    assert_eq!(outcome.sent[1].body, [8, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(outcome.unreliable, []);
}

#[test]
fn a_late_acquire_that_expires_during_receive_still_sends_release_cleanup() {
    let outcome = testing::run(|out| {
        let (mut world, sample) = admitted(out, 141);
        let vessel = Vessel {
            spawn_id: sample.spawn_id,
            incarnation: sample.incarnation,
        };
        let now = Instant::now();
        let mut control = Control::new(5);
        assert!(control.submitted(
            5,
            vessel,
            Intent::Acquire,
            now.checked_sub(WAIT).unwrap(),
            now.checked_sub(Duration::from_millis(1)).unwrap()
        ));
        let mut boats = Boats {
            control: Some(control),
        };
        boats
            .observe(
                &Message::Event(WorldEvent::BoatControl(ControlBoat {
                    boat_id: 8,
                    action: Action::Acquire,
                    unknown: [0; 3],
                })),
                &mut world,
                out,
            )
            .unwrap();
        assert_eq!(
            boats.control.as_ref().unwrap().state(),
            ControlState::Releasing(vessel)
        );
    });
    assert_eq!(outcome.sent.len(), 1);
    assert_eq!(outcome.sent[0].body, [8, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(outcome.unreliable, []);
}
