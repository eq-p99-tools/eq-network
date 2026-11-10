//! A lift carrying the player: the guard's vertical allowance for the door
//! the client names as underfoot, and how the risen height goes out.
use super::*;
use crate::movement::{DiagonalRates, MotionCalibration, UpdateGate, STATIONARY_HEARTBEAT};

fn setup(now: Instant) -> MotionSession {
    let mut session = MotionSession::new(11, 7, Position::default(), now).unwrap();
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
            now,
        )
        .unwrap();
    session
}

fn titanium_flagged(start: Instant) -> MotionSession {
    setup(start).with_gate(Some(UpdateGate::Flagged {
        pack: |sample| sample.packet(),
        rule: crate::movement::TITANIUM_FLAGS,
    }))
}

/// Standing on door 3 at `x`, risen to `z`.
fn riding(created: Instant, x: f32, z: f32) -> MovementRequest {
    MovementRequest::new(
        MovementMode::Forward,
        11,
        Position {
            x,
            z,
            ..Position::default()
        },
        created,
    )
    .on_door(3)
}

#[test]
fn a_named_lift_lets_one_sample_rise_as_far_as_it_rose_and_no_further() {
    let start = Instant::now();
    let now = start + Duration::from_millis(250);
    let mut session = setup(start);
    // A riser's worth is any grounded sample's; 4.5 is past it.
    assert!(session
        .send_move(&riding(now, 0.0, 4.5), now, |_| panic!("no lift rose"))
        .is_err());
    session.allow_lift_rise(4.5);
    assert!(session
        .send_move(&riding(now, 0.0, 6.6), now, |_| panic!(
            "past the lift and a riser"
        ))
        .is_err());
    session
        .send_move(&riding(now, 0.0, 4.5), now, |_| Ok(()))
        .unwrap();
    assert_eq!(session.position().z, 4.5);
    // The allowance is spent with the sample that used it.
    let later = now + Duration::from_millis(250);
    assert!(session
        .send_move(&riding(later, 0.0, 9.0), later, |_| panic!("spent"))
        .is_err());
    for nothing in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        session.allow_lift_rise(nothing);
        assert!(session
            .send_move(&riding(later, 0.0, 9.0), later, |_| panic!("none"))
            .is_err());
    }
}

#[test]
fn the_lifts_rise_composes_with_walking_and_a_still_rider_stays_passive() {
    let start = Instant::now();
    let now = start + Duration::from_millis(250);
    let mut session = setup(start);
    session.allow_lift_rise(4.5);
    // Walking 1.5 in 0.25 s at 6 a second while the lift rises 4.5.
    assert!(!session.is_passive(&riding(now, 1.5, 4.5)));
    session
        .send_move(&riding(now, 1.5, 4.5), now, |_| Ok(()))
        .unwrap();
    // Walking further than the speed allows is still refused on a lift.
    let later = now + Duration::from_millis(250);
    session.allow_lift_rise(4.5);
    assert!(session
        .send_move(&riding(later, 3.1, 9.0), later, |_| panic!("too fast"))
        .is_err());
    // Standing still, the rise alone is passive; any horizontal change or
    // a rise past the lift is not.
    assert!(session.is_passive(&riding(later, 1.5, 9.0)));
    assert!(!session.is_passive(&riding(later, 1.5, 9.1)));
    assert!(!session.is_passive(&riding(later, 1.6, 9.0)));
    session.allow_lift_rise(0.0);
    assert!(!session.is_passive(&riding(later, 1.5, 9.0)));
}

#[test]
fn a_lift_still_carries_a_player_whose_input_is_withdrawn() {
    let start = Instant::now();
    let mut session = setup(start);
    session.withdraw();
    let now = Instant::now() + Duration::from_millis(100);
    session.allow_lift_rise(0.9);
    let unnamed = MovementRequest {
        support: None,
        ..riding(now, 0.0, 0.9)
    };
    assert!(session
        .send_move(&unnamed, now, |_| panic!("no lift named"))
        .is_err());
    assert!(session
        .send_move(&riding(now, 0.1, 0.9), now, |_| panic!("moved across"))
        .is_err());
    session
        .send_move(&riding(now, 0.0, 0.9), now, |_| Ok(()))
        .unwrap();
    assert_eq!(session.position().z, 0.9);
}

#[test]
fn titanium_sends_a_still_riders_risen_height_on_its_one_second_tick() {
    let start = Instant::now();
    let mut session = titanium_flagged(start);
    let mut sent = Vec::new();
    // Standing on a lift rising 9 a second, sampled every 100 ms.
    for step in 1..=30_u64 {
        let now = start + Duration::from_millis(step * 100);
        session.allow_lift_rise(0.9);
        #[allow(clippy::cast_precision_loss)]
        let z = 0.9 * step as f32;
        session
            .send_move_sample(&riding(now, 0.0, z), now, |sample| {
                sent.push((step * 100, sample.position.z, sample.animation));
                Ok(())
            })
            .unwrap();
        session
            .tick_sample(now, |sample| {
                sent.push((step * 100, sample.position.z, sample.animation));
                Ok(())
            })
            .unwrap();
    }
    // Nothing a still rider changes sets a flag, so the tick sends the
    // latest height each second, still and with no animation.
    let times: Vec<u64> = sent.iter().map(|(at, ..)| *at).collect();
    assert_eq!(times.len(), 3, "{sent:?}");
    for (at, z, animation) in &sent {
        #[allow(clippy::cast_precision_loss)]
        let expected = 0.9 * (*at / 100) as f32;
        assert!((z - expected).abs() < 1e-3, "{sent:?}");
        assert_eq!(*animation, 0);
    }
    assert!(!session.stop_pending());
}

#[test]
fn walking_off_a_lift_flags_its_update_as_on_the_ground() {
    let start = Instant::now();
    let mut session = titanium_flagged(start);
    let mut sent = Vec::new();
    for step in 1..=20_u64 {
        let now = start + Duration::from_millis(step * 100);
        let request = if step <= 10 {
            session.allow_lift_rise(0.9);
            #[allow(clippy::cast_precision_loss)]
            riding(now, 0.0, 0.9 * step as f32)
        } else {
            // At the top, the rider walks off at 6 a second.
            #[allow(clippy::cast_precision_loss)]
            MovementRequest::new(
                MovementMode::Forward,
                11,
                Position {
                    x: 0.6 * (step - 10) as f32,
                    z: 9.0,
                    ..Position::default()
                },
                now,
            )
        };
        session
            .send_move_sample(&request, now, |sample| {
                sent.push((step * 100, sample.animation));
                Ok(())
            })
            .unwrap();
        session
            .tick_sample(now, |sample| {
                sent.push((step * 100, sample.animation));
                Ok(())
            })
            .unwrap();
    }
    // The speed change of walking off flags at once, past the 280 ms
    // spacing from the tick's send.
    let walking: Vec<_> = sent.iter().filter(|(at, _)| *at > 1000).collect();
    assert_eq!(
        walking.first().map(|(_, animation)| *animation),
        Some(12),
        "{sent:?}"
    );
    assert!(
        walking.first().is_some_and(|(at, _)| *at <= 1400),
        "{sent:?}"
    );
}

#[test]
fn the_plain_path_heartbeats_a_still_riders_risen_height() {
    let start = Instant::now();
    let mut session = setup(start);
    let now = start + Duration::from_millis(250);
    session.allow_lift_rise(4.5);
    let mut sent = Vec::new();
    session
        .send_move_sample(&riding(now, 0.0, 4.5), now, |sample| {
            sent.push(sample.position.z);
            Ok(())
        })
        .unwrap();
    assert!(!session.stop_pending());
    let beat = now + STATIONARY_HEARTBEAT;
    assert!(session
        .tick_sample(beat, |sample| {
            sent.push(sample.position.z);
            Ok(())
        })
        .unwrap());
    assert_eq!(sent, [4.5, 4.5]);
}
