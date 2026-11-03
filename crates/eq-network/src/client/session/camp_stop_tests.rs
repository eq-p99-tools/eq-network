//! Deterministic production-owner timing; no sleeps, captures or server ACKs.
use super::*;
use crate::client::session::{
    feature::{testing, Sink},
    wire::{EqMac, Wire},
};
use crate::client::{ClientConfig, Events};
use eq_network_game::{
    command::EncodedCommand,
    movement::{MotionCalibration, MotionSession, MovementMode, MovementRequest, UpdateGate},
    quarm,
    request::Sender,
    world::Position,
    GameDialect,
};

#[derive(Default)]
struct Recording {
    /// One order across reliable/unreliable submissions, including failures.
    order: Vec<(bool, EncodedCommand)>,
    fail_stop: bool,
    fail_camp: bool,
}

impl Sink for Recording {
    fn send(&mut self, packet: &EncodedCommand) -> Result<()> {
        self.order.push((true, packet.clone()));
        if self.fail_camp {
            anyhow::bail!("synthetic camp transport failure");
        }
        Ok(())
    }

    fn send_unreliable(&mut self, packet: &EncodedCommand) -> Result<()> {
        self.order.push((false, packet.clone()));
        if self.fail_stop {
            anyhow::bail!("synthetic stop transport failure");
        }
        Ok(())
    }

    fn last_received_seconds(&self) -> u64 {
        0
    }
}

/// Real feature output with one ordered sink; tests inspect local submission.
fn with_out<R>(
    wire: &'static dyn Wire,
    sink: &mut Recording,
    step: impl FnOnce(&mut Out<'_, '_>) -> R,
) -> (R, Vec<ClientEvent>) {
    let config = ClientConfig::new("ACCOUNT", "PASSWORD", "Test Server", "Tester");
    let mut events = Vec::new();
    let result = {
        let mut handler = |event| {
            events.push(event);
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        step(&mut Out {
            wire,
            sink,
            log: &mut log,
            sender: Sender {
                name: "Tester",
                spawn_id: Some(7),
            },
        })
    };
    (result, events)
}

fn at(start: Instant, ms: u64) -> Instant {
    start + Duration::from_millis(ms)
}

fn request(created: Instant, position: Position) -> ClientCommand {
    ClientCommand::Move(MovementRequest {
        session_id: 5,
        mode: MovementMode::Forward,
        position,
        created,
    })
}

/// Submit through the real movement owner and current generation/sink.
fn move_owner(
    world: &mut World,
    command: &ClientCommand,
    now: Instant,
    out: &mut Out<'_, '_>,
) -> Result<()> {
    let ClientCommand::Move(request) = command else {
        unreachable!()
    };
    world
        .body
        .motion_mut()
        .unwrap()
        .send_move_sample(request, now, |sample| {
            out.request_unreliable(&Request::Position(*sample))
        })
}

fn walking(start: Instant) -> World {
    let mut motion = MotionSession::new(5, 7, Position::default(), start)
        .unwrap()
        .with_gate(Some(UpdateGate::Flagged {
            pack: quarm::client_update,
        }));
    motion
        .calibrate(
            MotionCalibration {
                units_per_second: 6.0,
                velocity_scale: 0.05,
                animation: 12,
                backward: None,
                walk: None,
                strafe: None,
            },
            start,
        )
        .unwrap();
    let mut world = World::new(5);
    world.admitted = Some(start);
    world.player.admit(testing::player(7));
    world.own_spawn = Some(7);
    world.body.admit(motion);
    let command = request(
        at(start, 100),
        Position {
            x: 0.5,
            ..Position::default()
        },
    );
    with_out(&EqMac, &mut Recording::default(), |out| {
        move_owner(&mut world, &command, at(start, 100), out)
    })
    .0
    .unwrap();
    world
}

fn queued(start: Instant) -> (World, Camp) {
    let world = walking(start);
    let mut camp = Camp::default();
    let mut sink = Recording::default();
    let (result, events) = with_out(&EqMac, &mut sink, |out| {
        camp.request(at(start, 133), &world, out)
    });
    result.unwrap();
    assert_eq!(sink.order, Vec::new());
    assert!(events.is_empty(), "{events:?}");
    assert!(matches!(camp.phase, Phase::WaitingForStop { .. }));
    (world, camp)
}

/// The production server feature list runs Motion's timer before Camp's.
fn pass(camp: &mut Camp, world: &mut World, now: Instant, out: &mut Out<'_, '_>) -> Result<()> {
    world.body.heartbeat(now, out)?;
    camp.tick(now, world, out)
}

fn accept_trailing_stop(world: &mut World, camp: &mut Camp, start: Instant) {
    let stop = request(at(start, 200), world.body.position().unwrap());
    let mut sink = Recording::default();
    with_out(&EqMac, &mut sink, |out| {
        camp.notice(&stop, world, out)?;
        move_owner(world, &stop, at(start, 200), out)
    })
    .0
    .unwrap();
    assert_eq!(sink.order, Vec::new(), "held stop cannot bypass spacing");
}

#[test]
fn the_33ms_camp_waits_for_the_100ms_trailing_stop_and_paced_submission() {
    let start = Instant::now();
    let (mut world, mut camp) = queued(start);
    accept_trailing_stop(&mut world, &mut camp, start);
    let mut sink = Recording::default();
    with_out(&EqMac, &mut sink, |out| {
        pass(&mut camp, &mut world, at(start, 250), out)
    })
    .0
    .unwrap();
    assert_eq!(sink.order, Vec::new());
    let (result, events) = with_out(&EqMac, &mut sink, |out| {
        pass(&mut camp, &mut world, at(start, 251), out)
    });
    result.unwrap();
    assert_eq!(sink.order.len(), 2);
    assert!(!sink.order[0].0);
    assert_eq!(sink.order[0].1.opcode, quarm::ZONE_CLIENT_UPDATE);
    assert_eq!(sink.order[0].1.body[2], 0, "actual stop animation");
    assert_eq!(sink.order[1], (true, quarm::camp()));
    assert!(matches!(
        events.as_slice(),
        [ClientEvent::World(WorldEvent::Camp(CampStatus::Preparing))]
    ));
    assert!(!camp.logout_due(at(start, 30_250)));
    assert!(camp.logout_due(at(start, 30_251)));
}

#[test]
fn without_an_early_host_stop_the_existing_input_expiry_stop_precedes_camp() {
    let start = Instant::now();
    let (mut world, mut camp) = queued(start);
    let mut sink = Recording::default();
    with_out(&EqMac, &mut sink, |out| {
        pass(&mut camp, &mut world, at(start, 349), out)
    })
    .0
    .unwrap();
    assert_eq!(sink.order, Vec::new());
    with_out(&EqMac, &mut sink, |out| {
        pass(&mut camp, &mut world, at(start, 350), out)
    })
    .0
    .unwrap();
    assert_eq!(sink.order.len(), 2);
    assert_eq!(sink.order[1], (true, quarm::camp()));
}

#[test]
fn new_movement_including_same_stamp_intent_cancels_without_a_standing_packet() {
    let start = Instant::now();
    for (ms, x, heading) in [(100, 0.5, 0.0), (200, 0.6, 0.0), (200, 0.5, 1.0)] {
        let (mut world, mut camp) = queued(start);
        let command = request(
            at(start, ms),
            Position {
                x,
                heading,
                ..Position::default()
            },
        );
        let mut sink = Recording::default();
        with_out(&EqMac, &mut sink, |out| {
            camp.notice(&command, &mut world, out)
        })
        .0
        .unwrap();
        assert!(!camp.active());
        assert_eq!(sink.order, Vec::new());
        with_out(&EqMac, &mut sink, |out| {
            pass(&mut camp, &mut world, at(start, 350), out)
        })
        .0
        .unwrap();
        assert!(sink
            .order
            .iter()
            .all(|(_, packet)| packet.opcode != quarm::camp().opcode));
    }
}

#[test]
fn a_refused_trailing_stop_cannot_be_replaced_by_a_later_idle_stop() {
    let start = Instant::now();
    let (mut world, mut camp) = queued(start);
    let stop = request(at(start, 120), world.body.position().unwrap());
    let mut sink = Recording::default();
    let (result, _) = with_out(&EqMac, &mut sink, |out| {
        camp.notice(&stop, &mut world, out)?;
        move_owner(&mut world, &stop, at(start, 133), out)
    });
    assert!(
        result.is_err(),
        "20ms proposal violates the real sample guard"
    );
    with_out(&EqMac, &mut sink, |out| {
        camp.tick(at(start, 133), &mut world, out)
    })
    .0
    .unwrap();
    assert!(!camp.active());
    with_out(&EqMac, &mut sink, |out| {
        pass(&mut camp, &mut world, at(start, 350), out)
    })
    .0
    .unwrap();
    assert!(sink
        .order
        .iter()
        .all(|(_, packet)| packet.opcode != quarm::camp().opcode));
}

struct RefusePosition;
struct RefuseCamp;

impl Wire for RefusePosition {
    fn dialect(&self) -> GameDialect {
        GameDialect::EqMac
    }
    fn encode(&self, request: &Request, sender: Sender<'_>) -> Result<EncodedCommand> {
        if matches!(request, Request::Position(_)) {
            anyhow::bail!("synthetic position codec refusal");
        }
        EqMac.encode(request, sender)
    }
}

impl Wire for RefuseCamp {
    fn dialect(&self) -> GameDialect {
        GameDialect::EqMac
    }
    fn encode(&self, request: &Request, sender: Sender<'_>) -> Result<EncodedCommand> {
        if matches!(request, Request::Camp) {
            anyhow::bail!("synthetic camp codec refusal");
        }
        EqMac.encode(request, sender)
    }
}

#[test]
fn failed_stop_encoding_or_transport_never_reaches_camp_or_preparing() {
    let start = Instant::now();
    for wire in [&RefusePosition as &dyn Wire, &EqMac] {
        let (mut world, mut camp) = queued(start);
        accept_trailing_stop(&mut world, &mut camp, start);
        let mut sink = Recording {
            fail_stop: true,
            ..Recording::default()
        };
        let (result, events) = with_out(wire, &mut sink, |out| {
            pass(&mut camp, &mut world, at(start, 251), out)
        });
        assert!(result.is_err());
        assert!(events.is_empty(), "{events:?}");
        assert!(sink.order.iter().all(|(reliable, _)| !reliable));
        assert_eq!(world.body.stop_pending(), Some(true));
        with_out(&EqMac, &mut sink, |out| {
            camp.tick(at(start, 500), &mut world, out)
        })
        .0
        .unwrap();
        assert!(matches!(camp.phase, Phase::WaitingForStop { .. }));
        assert!(sink.order.iter().all(|(reliable, _)| !reliable));
    }
}

#[test]
fn failed_camp_encoding_or_transport_leaves_no_timer_or_replayable_intent() {
    let start = Instant::now();
    for wire in [&RefuseCamp as &dyn Wire, &EqMac] {
        let (mut world, mut camp) = queued(start);
        let mut sink = Recording::default();
        with_out(&EqMac, &mut sink, |out| {
            world.body.heartbeat(at(start, 350), out)
        })
        .0
        .unwrap();
        sink.fail_camp = true;
        let (result, events) = with_out(wire, &mut sink, |out| {
            camp.tick(at(start, 350), &mut world, out)
        });
        assert!(result.is_err());
        assert!(events.is_empty(), "{events:?}");
        assert!(!camp.active());
        let attempts = sink.order.len();
        sink.fail_camp = false;
        with_out(&EqMac, &mut sink, |out| {
            camp.tick(at(start, 500), &mut world, out)
        })
        .0
        .unwrap();
        assert_eq!(sink.order.len(), attempts);
    }
}

#[test]
fn admission_withdrawal_correction_and_owner_replacement_cancel_pending_camp() {
    let start = Instant::now();
    for boundary in 0..6 {
        let (mut world, mut camp) = queued(start);
        match boundary {
            0 => world = World::new(6),
            1 => world.admitted = Some(at(start, 1)),
            2 => world.body.motion_mut().unwrap().withdraw(),
            3 => world
                .body
                .motion_mut()
                .unwrap()
                .correct(Position::default(), at(start, 150))
                .unwrap(),
            4 => world.body.suspend(),
            5 => {
                let calibration = world.body.motion_mut().unwrap().calibration().unwrap();
                let mut replacement =
                    MotionSession::new(5, 7, Position::default(), at(start, 150)).unwrap();
                replacement.calibrate(calibration, at(start, 150)).unwrap();
                world.body.admit(replacement);
            }
            _ => unreachable!(),
        }
        let mut sink = Recording::default();
        with_out(&EqMac, &mut sink, |out| {
            camp.tick(at(start, 350), &mut world, out)
        })
        .0
        .unwrap();
        assert!(!camp.active());
        assert_eq!(sink.order, Vec::new());
    }
}

#[test]
fn real_death_or_zone_offer_discards_waiting_without_submitting_camp() {
    use eq_network_game::zoning::{Death, ZoneOffer};

    let start = Instant::now();
    for event in [
        WorldEvent::Death(Death {
            spawn_id: 7,
            killer_id: 0,
            corpse_id: 0,
            bind_zone_id: 0,
            corpse_name: None,
        }),
        WorldEvent::ZoneTransfer(ZoneOffer {
            zone_id: 4,
            instance_id: 0,
            position: Position::default(),
            reason: 0,
            to_bind: false,
            solicited: true,
        }),
    ] {
        let (mut world, mut camp) = queued(start);
        let mut sink = Recording::default();
        with_out(&EqMac, &mut sink, |out| {
            camp.observe(&Message::Event(event), &mut world, out)
        })
        .0
        .unwrap();
        assert!(!camp.active());
        with_out(&EqMac, &mut sink, |out| {
            pass(&mut camp, &mut world, at(start, 350), out)
        })
        .0
        .unwrap();
        assert!(sink
            .order
            .iter()
            .all(|(_, p)| p.opcode != quarm::camp().opcode));
    }
}

#[test]
fn withdrawn_motion_cannot_turn_a_new_camp_into_an_unguarded_submission() {
    let start = Instant::now();
    let mut world = walking(start);
    world.body.motion_mut().unwrap().withdraw();
    let mut camp = Camp::default();
    let mut sink = Recording::default();
    let (result, events) = with_out(&EqMac, &mut sink, |out| {
        camp.request(at(start, 133), &world, out)
    });
    result.unwrap();
    assert!(!camp.active());
    assert_eq!(sink.order, Vec::new());
    assert!(matches!(
        events.as_slice(),
        [ClientEvent::World(WorldEvent::Camp(CampStatus::Rejected(
            _
        )))]
    ));
}

#[test]
fn duplicate_camp_keeps_one_intent_and_stationary_repeats_do_not_delay_camp() {
    let start = Instant::now();
    let (mut world, mut camp) = queued(start);
    let mut sink = Recording::default();
    with_out(&EqMac, &mut sink, |out| {
        camp.request(at(start, 140), &world, out)
    })
    .0
    .unwrap();
    with_out(&EqMac, &mut sink, |out| {
        pass(&mut camp, &mut world, at(start, 350), out)
    })
    .0
    .unwrap();
    with_out(&EqMac, &mut sink, |out| {
        pass(&mut camp, &mut world, at(start, 1300), out)
    })
    .0
    .unwrap();
    assert_eq!(
        sink.order
            .iter()
            .filter(|(_, p)| p.opcode == quarm::camp().opcode)
            .count(),
        1
    );
    assert!(matches!(camp.phase, Phase::Preparing(_)));
    let mut next = Camp::default();
    let mut still_sink = Recording::default();
    with_out(&EqMac, &mut still_sink, |out| {
        next.request(at(start, 1400), &world, out)
    })
    .0
    .unwrap();
    assert_eq!(still_sink.order, [(true, quarm::camp())]);
}
