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

#[test]
fn a_take_asked_again_while_pending_goes_again_and_never_extends_the_wait() {
    let mut first_sent = None;
    let outcome = testing::run(|out| {
        let (mut world, sample) = admitted(out, 141);
        let mut boats = Boats::default();
        let take = acquire(sample, world.body.position().unwrap());
        boats.handle(&take, &mut world, out).unwrap();
        first_sent = Some(Instant::now());
        std::thread::sleep(Duration::from_millis(5));
        for _ in 0..9 {
            boats.handle(&take, &mut world, out).unwrap();
        }
        assert!(matches!(
            boats.control.as_ref().unwrap().state(),
            ControlState::Acquiring { .. }
        ));
        // The deadline is still the first take's: at it, the wait ends.
        let mut later = boats.control.take().unwrap();
        later.expire(first_sent.unwrap() + WAIT);
        assert!(matches!(later.state(), ControlState::Unconfirmed(_)));
        boats.control = Some(Control::new(5));
    });
    // Ten clicks, ten identical takes, and nothing else.
    assert_eq!(
        outcome
            .sent
            .iter()
            .map(|packet| (packet.opcode, packet.body.clone()))
            .collect::<Vec<_>>(),
        vec![(0x2c81, vec![8, 0, 0, 0, 1, 0, 0, 0]); 10]
    );
}

#[test]
fn one_echo_grants_a_repeated_take_and_later_echoes_change_nothing() {
    let echo = |boat_id, action| {
        Message::Event(WorldEvent::BoatControl(ControlBoat {
            boat_id,
            action,
            unknown: [0; 3],
        }))
    };
    let outcome = testing::run(|out| {
        let (mut world, sample) = admitted(out, 141);
        let mut boats = Boats::default();
        let take = acquire(sample, world.body.position().unwrap());
        boats.handle(&take, &mut world, out).unwrap();
        boats.handle(&take, &mut world, out).unwrap();
        boats
            .observe(&echo(8, Action::Acquire), &mut world, out)
            .unwrap();
        let granted = boats.control.as_ref().unwrap().state();
        assert!(matches!(granted, ControlState::Acquired(_)));
        boats
            .observe(&echo(8, Action::Acquire), &mut world, out)
            .unwrap();
        assert_eq!(boats.control.as_ref().unwrap().state(), granted);
    });
    // One state change told for the grant, and no line for the second echo.
    let states = outcome
        .events
        .iter()
        .filter(|event| {
            matches!(
                event,
                ClientEvent::World(WorldEvent::BoatControlState {
                    state: ControlState::Acquired(_),
                    ..
                })
            )
        })
        .count();
    assert_eq!(states, 1);
    assert!(!outcome
        .events
        .iter()
        .any(|event| matches!(event, ClientEvent::Diagnostic(_))));
}

#[test]
fn another_boats_take_while_one_is_pending_stays_refused() {
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
        // Another incarnation of the same number is another vessel.
        let mut other = sample;
        other.incarnation += 1;
        boats
            .handle(
                &acquire(other, world.body.position().unwrap()),
                &mut world,
                out,
            )
            .unwrap();
    });
    assert_eq!(outcome.sent.len(), 1);
}

/// A control echo as TAKP's or Quarm's wire reads it.
fn mac_echo(wire: &'static dyn super::super::wire::Wire, body: &[u8]) -> Message {
    let messages = wire.messages(0x2641, body, eq_network_game::inventory::ItemRules::Takp);
    assert!(matches!(
        messages.as_slice(),
        [Message::Event(WorldEvent::BoatControl(_))]
    ));
    messages.into_iter().next().unwrap()
}

const MAC: [super::super::super::ServerProtocol; 2] = [
    super::super::super::ServerProtocol::Takp,
    super::super::super::ServerProtocol::Quarm,
];

/// TAKP's and Quarm's 4-byte control (6.602): built and tested, but not
/// offered until the `EQMac` program's steering and riding are read.
#[test]
fn mac_control_is_four_bytes_and_releases_on_leaving_zoning_or_dying() {
    let offer = Message::ZoneOffer(eq_network_game::zoning::ZoneOffer {
        zone_id: 2,
        instance_id: 0,
        position: Position::default(),
        reason: 0,
        to_bind: false,
        solicited: true,
    });
    let death = Message::Event(WorldEvent::Death(eq_network_game::zoning::Death {
        spawn_id: 7,
        killer_id: 0,
        corpse_id: 0,
        bind_zone_id: 0,
        corpse_name: None,
    }));
    for protocol in MAC {
        let wire = server_type(protocol).wire();
        // Granted, then each way of leaving gives control back.
        for ending in ["leave", "zone", "death"] {
            let outcome = testing::run_on(wire, |out| {
                let (mut world, sample) = admitted(out, 141);
                let mut boats = Boats::default();
                boats
                    .handle(
                        &acquire(sample, world.body.position().unwrap()),
                        &mut world,
                        out,
                    )
                    .unwrap();
                boats
                    .observe(&mac_echo(wire, &[8, 0, 1, 0]), &mut world, out)
                    .unwrap();
                assert!(matches!(
                    boats.control.as_ref().unwrap().state(),
                    ControlState::Acquired(_)
                ));
                match ending {
                    "leave" => {
                        world.body.record_carrier_support(None);
                        boats.tick(Instant::now(), &mut world, out).unwrap();
                    }
                    "zone" => boats.observe(&offer, &mut world, out).unwrap(),
                    _ => {
                        world.own_spawn = Some(7);
                        boats.observe(&death, &mut world, out).unwrap();
                    }
                }
                assert!(
                    matches!(
                        boats.control.as_ref().unwrap().state(),
                        ControlState::Releasing(_)
                    ),
                    "{ending}"
                );
                boats
                    .observe(&mac_echo(wire, &[8, 0, 0, 0]), &mut world, out)
                    .unwrap();
                assert!(matches!(
                    boats.control.as_ref().unwrap().state(),
                    ControlState::Released(_)
                ));
            });
            assert_eq!(
                outcome
                    .sent
                    .iter()
                    .map(|packet| (packet.opcode, packet.body.clone()))
                    .collect::<Vec<_>>(),
                [(0x2641, vec![8, 0, 1, 0]), (0x2641, vec![8, 0, 0, 0])],
                "{protocol:?} {ending}"
            );
        }
    }
}

/// In use: the server sends `IN_USE` and no echo. The wait runs out and the
/// session gives up the take, as on Titanium.
#[test]
fn a_mac_take_refused_in_use_is_given_up_when_its_wait_runs_out() {
    for protocol in MAC {
        let outcome = testing::run_on(server_type(protocol).wire(), |out| {
            let (mut world, sample) = admitted(out, 141);
            let mut boats = Boats::default();
            boats
                .handle(
                    &acquire(sample, world.body.position().unwrap()),
                    &mut world,
                    out,
                )
                .unwrap();
            boats
                .tick(
                    Instant::now() + WAIT + Duration::from_millis(1),
                    &mut world,
                    out,
                )
                .unwrap();
            assert!(!matches!(
                boats.control.as_ref().unwrap().state(),
                ControlState::Acquired(_) | ControlState::Acquiring { .. }
            ));
        });
        assert_eq!(
            outcome
                .sent
                .iter()
                .map(|packet| packet.body.clone())
                .collect::<Vec<_>>(),
            [vec![8, 0, 1, 0], vec![8, 0, 0, 0]],
            "{protocol:?}"
        );
    }
}
