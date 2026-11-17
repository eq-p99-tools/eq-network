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
