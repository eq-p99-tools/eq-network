//! Carrier-relative budgets, yaw and existing update pacing.
use super::*;
use crate::movement::{carrier_local, carrier_world, CarrierSample, Support, TITANIUM_FLAGS};

fn sample(start: Instant, at: Instant) -> CarrierSample {
    CarrierSample {
        spawn_id: 8,
        reported: at,
        spawned: start,
        serial: 1,
        incarnation: 1,
    }
}

fn setup(start: Instant) -> MotionSession {
    let mut session = MotionSession::new(
        11,
        7,
        Position {
            x: 20.0,
            ..Position::default()
        },
        start,
    )
    .unwrap();
    session
        .calibrate(
            MotionCalibration {
                swim_backward_units_per_second: None,
                swim: None,
                diagonal: DiagonalRates::default(),
                walk: None,
                strafe: None,
                backward: None,
                units_per_second: 6.0,
                velocity_scale: 0.05,
                animation: 12,
            },
            start,
        )
        .unwrap();
    session
}

fn request(start: Instant, at: Instant, pose: Position, local: Position) -> MovementRequest {
    MovementRequest::new(MovementMode::Forward, 11, carrier_world(pose, local), at)
        .supported_by(Support::Spawn(sample(start, at)))
}

fn attach(session: &mut MotionSession, start: Instant) -> Position {
    let at = start + Duration::from_millis(100);
    let local = Position {
        x: 20.0,
        ..Position::default()
    };
    session.allow_carrier(Some((sample(start, at), Position::default())));
    session
        .send_move_sample(&request(start, at, Position::default(), local), at, |_| {
            Ok(())
        })
        .unwrap();
    local
}

#[test]
fn a_verified_carrier_keeps_passive_motion_without_granting_player_input() {
    let start = Instant::now();
    let mut session = MotionSession::new(
        11,
        7,
        Position {
            x: 20.0,
            ..Position::default()
        },
        start,
    )
    .unwrap();
    let local = attach(&mut session, start);
    assert!(!session.granted());
    let at = start + Duration::from_millis(200);
    let pose = Position {
        x: 100.0,
        heading: 128.0,
        ..Position::default()
    };
    let still = request(start, at, pose, local);
    // An unverified support name cannot bypass withdrawn input checks.
    session.allow_carrier(None);
    assert!(session
        .send_move_sample(&still, at, |_| panic!("unverified carrier"))
        .is_err());
    session.allow_carrier(Some((sample(start, at), pose)));
    session
        .send_move_sample(&still, at, |wire| {
            assert_eq!(wire.position, still.position);
            assert_eq!(wire.delta, [0.0; 3]);
            assert_eq!(wire.delta_heading, 0);
            assert_eq!(wire.animation, 0);
            Ok(())
        })
        .unwrap();
    assert!(!session.granted());
    let later = at + Duration::from_millis(100);
    let mut walked = local;
    walked.x += 0.1;
    session.allow_carrier(Some((sample(start, later), pose)));
    assert!(session
        .send_move_sample(&request(start, later, pose, walked), later, |_| {
            panic!("a carrier cannot grant the rider movement");
        })
        .is_err());
    assert_eq!(session.position(), still.position);
}

#[test]
fn translation_and_yaw_do_not_spend_the_riders_walk_or_own_turn_budget() {
    let start = Instant::now();
    let mut session = setup(start);
    let mut local = attach(&mut session, start);
    let at = start + Duration::from_millis(200);
    let boat = Position {
        x: 100.0,
        y: 20.0,
        z: 10.0,
        heading: 128.0,
    };
    session.allow_carrier(Some((sample(start, at), boat)));
    let still = request(start, at, boat, local);
    assert!(session.is_passive(&still));
    session
        .send_move_sample(&still, at, |wire| {
            assert_eq!(wire.position, still.position);
            assert_eq!(wire.delta, [0.0; 3]);
            assert_eq!(wire.delta_heading, 0);
            assert_eq!(wire.animation, 0);
            Ok(())
        })
        .unwrap();
    let later = at + Duration::from_millis(100);
    // 6 units/s still allows exactly 0.6 of walking on deck.
    local.x += 0.6;
    session.allow_carrier(Some((sample(start, later), boat)));
    session
        .send_move_sample(&request(start, later, boat, local), later, |wire| {
            assert_eq!(wire.animation, 12);
            assert!(wire.delta[0].abs() < 0.001);
            assert!((wire.delta[1] + 0.3).abs() < 0.001);
            Ok(())
        })
        .unwrap();
    let before = session.position();
    let too_fast = later + Duration::from_millis(100);
    local.x += 0.7;
    session.allow_carrier(Some((sample(start, too_fast), boat)));
    assert!(session
        .send_move_sample(
            &request(start, too_fast, boat, local),
            too_fast,
            |_| panic!("over budget")
        )
        .is_err());
    assert_eq!(session.position(), before);
}

#[test]
fn a_delayed_pose_catches_up_once_and_never_becomes_free_walking() {
    let start = Instant::now();
    let mut session = setup(start);
    let local = attach(&mut session, start);
    let at = start + Duration::from_millis(200);
    // The frontend still displays the previous report. The session offers
    // precisely that verified pose, not its newer boat pose plus tolerance.
    session.allow_carrier(Some((sample(start, at), Position::default())));
    session
        .send_move_sample(&request(start, at, Position::default(), local), at, |_| {
            Ok(())
        })
        .unwrap();
    let later = at + Duration::from_millis(100);
    let boat = Position {
        x: 100.0,
        heading: 128.0,
        ..Position::default()
    };
    session.allow_carrier(Some((sample(start, later), boat)));
    session
        .send_move_sample(&request(start, later, boat, local), later, |_| Ok(()))
        .unwrap();
    let denied = later + Duration::from_millis(100);
    let mut invented = local;
    invented.x += 30.0;
    session.allow_carrier(Some((sample(start, denied), boat)));
    assert!(session
        .send_move_sample(&request(start, denied, boat, invented), denied, |_| panic!(
            "free walking"
        ))
        .is_err());
}

#[test]
#[allow(clippy::cast_precision_loss)] // A bounded synthetic 30-step trace.
fn a_still_turning_rider_uses_the_tick_without_flagging_the_boats_yaw() {
    let start = Instant::now();
    let mut session = setup(start).with_gate(Some(UpdateGate::Flagged {
        pack: |sample| sample.packet(),
        rule: TITANIUM_FLAGS,
    }));
    let local = attach(&mut session, start);
    let mut sent = Vec::new();
    for step in 2..=30_u32 {
        let at = start + Duration::from_millis(u64::from(step) * 100);
        let pose = Position {
            x: step as f32 * 2.0,
            heading: (step as f32 * 10.0).rem_euclid(512.0),
            ..Position::default()
        };
        session.allow_carrier(Some((sample(start, at), pose)));
        session
            .send_move_sample(&request(start, at, pose, local), at, |sample| {
                sent.push((step, *sample));
                Ok(())
            })
            .unwrap();
        session
            .tick_sample(at, |sample| {
                sent.push((step, *sample));
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(sent.len(), 3, "{sent:?}");
    for (_, wire) in sent {
        assert_eq!(wire.delta_heading, 0);
        assert_eq!(wire.animation, 0);
        assert_eq!(wire.delta, [0.0; 3]);
        assert!(wire.position.heading > 0.0);
    }
}

#[test]
fn own_turns_accumulate_across_carrier_wrap_and_failed_sends_do_not_spend_them() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut session = setup(start).with_gate(Some(UpdateGate::Flagged {
        pack: |sample| sample.packet(),
        rule: TITANIUM_FLAGS,
    }));
    let mut local = attach(&mut session, start);
    assert!(session.tick_sample(at(300), |_| Ok(())).unwrap());
    for (ms, heading, own) in [(400, 500.0, 0.0), (500, 4.0, 1.0)] {
        let pose = Position {
            heading,
            ..Position::default()
        };
        local.heading = own;
        session.allow_carrier(Some((sample(start, at(ms)), pose)));
        session
            .send_move_sample(&request(start, at(ms), pose, local), at(ms), |_| {
                panic!("carrier yaw and sub-threshold own turn must wait for the timer");
            })
            .unwrap();
        assert!(!session
            .tick_sample(at(ms), |_| panic!("held yaw raised the input flag"))
            .unwrap());
    }
    let before = session.position();
    let pose = Position {
        heading: 8.0,
        ..Position::default()
    };
    local.heading = 2.25;
    let turn = request(start, at(600), pose, local);
    session.allow_carrier(Some((sample(start, at(600)), pose)));
    let mut failed = None;
    assert!(session
        .send_move_sample(&turn, at(600), |wire| {
            failed = Some(*wire);
            anyhow::bail!("synthetic transport failure");
        })
        .is_err());
    assert_eq!(session.position(), before);
    assert_eq!(session.counter.entries(), 1);
    session
        .send_move_sample(&turn, at(600), |wire| {
            assert_eq!(Some(*wire), failed);
            assert_eq!(wire.position, turn.position);
            assert_eq!(wire.delta, [0.0; 3]);
            assert_eq!(wire.animation, 0);
            assert_eq!(wire.delta_heading, 12);
            Ok(())
        })
        .unwrap();
    assert_eq!(session.counter.entries(), 2);
}

#[test]
fn a_fast_boat_walk_off_keeps_own_budget_and_the_next_step_is_unsupported() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut session = setup(start);
    let mut local = attach(&mut session, start);
    let before = session.position();
    let pose = Position {
        x: 1000.0,
        y: 200.0,
        heading: 128.0,
        ..Position::default()
    };
    local.x += 0.5;
    let leaving = request(start, at(200), pose, local);
    session.allow_carrier(Some((sample(start, at(200)), pose)));
    // A generic host that drops support one sample early must still pay
    // world motion. No carrier allowance is inferred from its old link.
    let unsupported = MovementRequest {
        support: None,
        ..leaving.clone()
    };
    assert!(session
        .send_move_sample(&unsupported, at(200), |_| panic!("unnamed carrier motion"))
        .is_err());
    assert_eq!(session.position(), before);
    assert!(session
        .send_move_sample(&leaving, at(200), |_| anyhow::bail!(
            "synthetic send failure"
        ))
        .is_err());
    assert_eq!(session.position(), before);
    session
        .send_move_sample(&leaving, at(200), |wire| {
            assert_eq!(wire.position, leaving.position);
            assert!(wire.delta[0].abs() < 0.001);
            assert!((wire.delta[1] + 0.25).abs() < 0.001);
            assert_eq!(wire.delta_heading, 0);
            assert_eq!(wire.animation, 12);
            Ok(())
        })
        .unwrap();
    // The accepted endpoint is now on ordinary ground. Its next request
    // names no carrier and spends only the ordinary walking budget.
    let next = MovementRequest::new(
        MovementMode::Forward,
        11,
        Position {
            x: leaving.position.x + 0.5,
            ..leaving.position
        },
        at(300),
    );
    session.allow_carrier(None);
    session
        .send_move_sample(&next, at(300), |_| Ok(()))
        .unwrap();
    let unsupported = session.position();
    let later_pose = Position { x: 2000.0, ..pose };
    session.allow_carrier(Some((sample(start, at(400)), later_pose)));
    assert!(session
        .send_move_sample(&request(start, at(400), later_pose, local), at(400), |_| {
            panic!("detached carrier allowance was reused");
        })
        .is_err());
    assert_eq!(session.position(), unsupported);
}

#[test]
fn a_fast_walk_off_rejects_stale_replaced_missing_and_unreachable_support() {
    let start = Instant::now();
    let at = start + Duration::from_millis(200);
    let pose = Position {
        x: 1000.0,
        heading: 128.0,
        ..Position::default()
    };
    for case in 0..4 {
        let mut session = setup(start);
        let mut local = attach(&mut session, start);
        local.x += 0.5;
        let mut leaving = request(start, at, pose, local);
        let mut named = sample(start, at);
        let mut checked = at;
        let mut supplied = true;
        match case {
            0 => checked += Duration::from_millis(251),
            1 => {
                named.incarnation += 1;
                leaving.support = Some(Support::Spawn(named));
            }
            2 => supplied = false,
            3 => leaving.position = carrier_world(pose, Position { x: 1000.0, ..local }),
            _ => unreachable!(),
        }
        session.allow_carrier(supplied.then_some((named, pose)));
        let before = session.position();
        assert!(session
            .send_move_sample(&leaving, checked, |_| panic!("invalid carrier support"))
            .is_err());
        assert_eq!(session.position(), before);
        assert_eq!(session.counter.entries(), 1);
    }
}

#[test]
fn walking_off_or_a_missing_report_removes_carrier_allowance() {
    let start = Instant::now();
    let mut session = setup(start);
    let local = attach(&mut session, start);
    let at = start + Duration::from_millis(200);
    let pose = Position {
        x: 100.0,
        ..Position::default()
    };
    session.allow_carrier(Some((sample(start, at), pose)));
    session
        .send_move_sample(&request(start, at, pose, local), at, |_| Ok(()))
        .unwrap();
    let later = at + Duration::from_millis(100);
    let walked = MovementRequest::new(
        MovementMode::Forward,
        11,
        Position {
            x: session.position().x + 0.5,
            ..session.position()
        },
        later,
    );
    session.allow_carrier(None);
    session
        .send_move_sample(&walked, later, |_| Ok(()))
        .unwrap();
    let missing = later + Duration::from_millis(100);
    assert!(session
        .send_move_sample(&request(start, missing, pose, local), missing, |_| panic!(
            "missing report"
        ))
        .is_err());
    let mut far = walked.clone();
    far.created = missing;
    far.position.x += 100.0;
    assert!(session
        .send_move_sample(&far, missing, |_| panic!("walked off"))
        .is_err());
}

#[test]
fn a_replaced_id_or_point_outside_the_radius_cannot_carry() {
    let start = Instant::now();
    let mut session = setup(start);
    let local = attach(&mut session, start);
    let at = start + Duration::from_millis(200);
    let pose = Position {
        x: 100.0,
        ..Position::default()
    };
    let mut replacement = sample(start, at);
    replacement.incarnation += 1;
    session.allow_carrier(Some((replacement, pose)));
    let mut proposed = request(start, at, pose, local);
    proposed.support = Some(Support::Spawn(replacement));
    assert!(session
        .send_move_sample(&proposed, at, |_| panic!("new incarnation"))
        .is_err());
    let near = Position { x: 1000.0, ..local };
    session.allow_carrier(Some((sample(start, at), pose)));
    assert!(session
        .send_move_sample(&request(start, at, pose, near), at, |_| panic!(
            "outside radius"
        ))
        .is_err());
    assert_eq!(
        carrier_local(Position::default(), session.position()),
        local
    );
}
