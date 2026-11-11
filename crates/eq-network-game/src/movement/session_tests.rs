use super::*;
use crate::movement::RunSpeedMultiplier;

#[test]
fn stopping_voluntary_motion_replaces_a_paced_update_and_keeps_calibration_and_jump() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut motion = setup(start)
        .with_gate(Some(UpdateGate::Changed {
            spacing: Duration::from_millis(280),
        }))
        .with_jump(Some((44.55, 400.0)));
    motion
        .send_move_sample(&request(at(300), 0.5), at(300), |_| Ok(()))
        .unwrap();
    motion
        .send_move_sample(&request(at(400), 1.0), at(400), |_| {
            panic!("paced update escaped")
        })
        .unwrap();
    motion.jumped();
    let calibration = motion.calibration();
    motion.stop_voluntary();
    assert_eq!(motion.calibration(), calibration);
    let mut rising = request(at(500), 1.0);
    rising.position.z = 1.0;
    assert!(motion.is_passive(&rising));
    let mut turning = rising.clone();
    turning.position.heading = 1.0;
    assert!(!motion.is_passive(&turning));
    assert!(!motion
        .tick_sample(at(579), |_| panic!("stop bypassed pacing"))
        .unwrap());
    assert!(motion
        .tick_sample(at(580), |_| anyhow::bail!("synthetic transport failure"))
        .is_err());
    assert!(motion.stop_pending());
    assert!(motion
        .tick_sample(at(580), |sample| {
            assert_eq!(sample.position, motion_position(1.0));
            assert_eq!(sample.delta, [0.0; 3]);
            assert_eq!((sample.animation, sample.delta_heading), (0, 0));
            Ok(())
        })
        .unwrap());
    assert!(!motion.stop_pending());
    assert_eq!(motion.calibration(), calibration);
    let mut ordinary = setup(start);
    assert!(!ordinary.is_passive(&request(at(100), 0.1)));
    rising.position = Position {
        z: 0.1,
        ..Position::default()
    };
    assert!(!ordinary.is_passive(&rising));
    ordinary.stop_voluntary();
    assert_eq!(ordinary.position(), Position::default());
}

fn motion_position(x: f32) -> Position {
    Position {
        x,
        ..Position::default()
    }
}

#[test]
fn passive_jump_ceiling_uses_representable_world_coordinates_without_extra_rise() {
    let start = Instant::now();
    let now = start + Duration::from_millis(100);
    for z in [0.0, 1000.0, 20000.0] {
        let position = Position {
            z,
            ..Position::default()
        };
        let mut motion = MotionSession::new(11, 7, position, start)
            .unwrap()
            .with_jump(Some((44.55, 400.0)));
        motion.jumped();
        let ceiling = z + motion.guard.jump_rise(0.0, 0.1);
        let mut candidate = request(now, 0.0);
        candidate.position.z = ceiling;
        assert!(motion.is_passive(&candidate));
        candidate.position.z = ceiling.next_up();
        assert!(!motion.is_passive(&candidate));
    }
}

#[test]
fn camp_stop_query_requires_submission_and_does_not_wait_on_heartbeats() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut motion = mac_era(start);
    motion
        .send_move_sample(&request(at(100), 0.5), at(100), |_| Ok(()))
        .unwrap();
    assert!(motion.stop_pending());
    let context = motion.stop_context();
    let stop = request(at(200), 0.5);
    assert!(motion.is_trailing_stop(&stop));
    assert!(!motion.is_trailing_stop(&request(at(100), 0.5)));
    assert!(!motion.is_trailing_stop(&request(at(200), 0.6)));
    let mut foreign = stop.clone();
    foreign.session_id += 1;
    assert!(!motion.is_trailing_stop(&foreign));
    motion
        .send_move_sample(&stop, at(200), |_| panic!("stop bypassed spacing"))
        .unwrap();
    assert!(motion.accepted_stop(at(200)));
    assert!(motion.stop_pending());
    assert_eq!(motion.stop_context(), context);
    assert!(!motion
        .tick_sample(at(250), |_| panic!("early stop"))
        .unwrap());
    assert!(motion
        .tick_sample(at(251), |_| anyhow::bail!("synthetic transport failure"))
        .is_err());
    assert!(motion.stop_pending());
    assert!(motion.tick_sample(at(251), |_| Ok(())).unwrap());
    assert!(!motion.stop_pending());
    assert!(!motion.is_trailing_stop(&stop));
    motion.tick_sample(at(1300), |_| Ok(())).unwrap();
    assert!(!motion.stop_pending(), "stationary repeats are not a stop");
    motion.withdraw();
    assert_eq!(motion.stop_context(), None);
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "Exact equality verifies unchanged or explicitly assigned state"
)]
fn strafing_requires_its_own_calibration_and_preserves_heading() {
    for direction in [-1.0, 1.0] {
        let start = Instant::now();
        let now = start + Duration::from_millis(100);
        let mut session = setup(start);
        let mut motion = request(now, direction * 0.3);
        motion.mode = MovementMode::Strafe;
        assert!(session
            .send_move(&motion, now, |_| panic!("uncalibrated strafe"))
            .is_err());
        let calibration = MotionCalibration {
            swim_backward_units_per_second: None,
            diagonal: DiagonalRates::default(),
            units_per_second: 6.0,
            velocity_scale: 0.05,
            animation: 12,
            backward: None,
            walk: None,
            strafe: Some(StrafeCalibration {
                units_per_second: 3.0,
                velocity_scale: 0.1,
                animation: 8,
                walk_units_per_second: None,
            }),
        };
        session.calibrate(calibration, start).unwrap();
        motion.position.x = direction * 0.6;
        assert!(session
            .send_move(&motion, now, |_| panic!("forward budget used for strafe"))
            .is_err());
        motion.position.x = direction * 0.3;
        session
            .send_move(&motion, now, |packet| {
                let body = &packet.body[..];
                assert_eq!(word(body, 20) & 0x3ff, 8);
                assert!((f32::from_bits(word(body, 12)) - direction * 0.3).abs() < 0.00001);
                Ok(())
            })
            .unwrap();
        assert_eq!(session.position().heading, 0.0);
        session.correct(Position::default(), now).unwrap();
        motion.created = now + Duration::from_millis(100);
        assert!(session
            .send_move(&motion, motion.created, |_| panic!(
                "strafe survived correction"
            ))
            .is_err());
    }
}

#[test]
fn walking_uses_its_own_budget_and_commits_only_after_transport() {
    let start = Instant::now();
    let now = start + Duration::from_millis(100);
    let mut session = setup(start);
    let mut motion = request(now, 0.2);
    motion.mode = MovementMode::Walk;
    assert!(session
        .send_move(&motion, now, |_| panic!("missing walk calibration"))
        .is_err());
    let calibration = MotionCalibration {
        swim_backward_units_per_second: None,
        diagonal: DiagonalRates::default(),
        units_per_second: 6.0,
        velocity_scale: 0.05,
        animation: 12,
        strafe: None,
        backward: None,
        walk: Some(WalkCalibration {
            units_per_second: 2.0,
            velocity_scale: 0.1,
            animation: 4,
        }),
    };
    session.calibrate(calibration, start).unwrap();
    motion.position.x = 0.6;
    assert!(session
        .send_move(&motion, now, |_| panic!("run budget used for walking"))
        .is_err());
    motion.position.x = 0.2;
    assert!(session
        .send_move(&motion, now, |_| anyhow::bail!(
            "synthetic transport failure"
        ))
        .is_err());
    assert_eq!(session.position(), Position::default());
    session
        .send_move(&motion, now, |packet| {
            let body = &packet.body[..];
            assert_eq!(word(body, 20) & 0x3ff, 4);
            assert!((f32::from_bits(word(body, 12)) - 0.2).abs() < 0.00001);
            Ok(())
        })
        .unwrap();
    // Changing mode does not reset the sample clock or grant extra distance.
    let mut run = request(now, 0.8);
    assert!(session
        .send_move(&run, now, |_| panic!("extra time budget"))
        .is_err());
    run.created = now + Duration::from_millis(100);
    session.send_move(&run, run.created, |_| Ok(())).unwrap();
    session.correct(Position::default(), run.created).unwrap();
    motion.created = run.created + Duration::from_millis(100);
    assert!(session
        .send_move(&motion, motion.created, |_| panic!(
            "walk resumed after correction"
        ))
        .is_err());
    for speed in [0.0, -1.0, 7.0, f32::NAN] {
        let mut invalid = calibration;
        invalid.walk.as_mut().unwrap().units_per_second = speed;
        assert!(invalid.validate().is_err());
    }
}

#[test]
fn backward_motion_has_its_own_budget_animation_and_transport_commit() {
    let start = Instant::now();
    let mut session = setup(start);
    let now = start + Duration::from_millis(100);
    let mut motion = request(now, -0.2);
    motion.mode = MovementMode::Backward;
    assert!(session
        .send_move(&motion, now, |_| panic!("uncalibrated send"))
        .is_err());
    let calibration = MotionCalibration {
        swim_backward_units_per_second: None,
        diagonal: DiagonalRates::default(),
        units_per_second: 6.0,
        velocity_scale: 0.05,
        animation: 12,
        walk: None,
        strafe: None,
        backward: Some(BackwardCalibration {
            units_per_second: 2.0,
            velocity_scale: 0.1,
            animation: -8,
        }),
    };
    session.calibrate(calibration, start).unwrap();
    motion.position.x = -0.4;
    assert!(session
        .send_move(&motion, now, |_| panic!(
            "forward budget used for backward motion"
        ))
        .is_err());
    motion.position.x = -0.2;
    assert!(session
        .send_move(&motion, now, |_| anyhow::bail!("synthetic send failure"))
        .is_err());
    assert_eq!(session.position(), Position::default());
    session
        .send_move(&motion, now, |packet| {
            let body = &packet.body[..];
            assert_eq!(word(body, 20) & 0x3ff, 1016); // signed -8
            assert!((f32::from_bits(word(body, 12)) + 0.2).abs() < 0.00001);
            assert_eq!(u16::from_le_bytes(body[2..4].try_into().unwrap()), 0);
            Ok(())
        })
        .unwrap();
    let now = now + Duration::from_millis(100);
    let forward = request(now, 0.4);
    session
        .send_move(&forward, now, |packet| {
            let body = &packet.body[..];
            assert_eq!(word(body, 20) & 0x3ff, 12);
            Ok(())
        })
        .unwrap();
    for animation in [1, -513] {
        let mut invalid = calibration;
        invalid.backward.as_mut().unwrap().animation = animation;
        assert!(invalid.validate().is_err());
    }
}
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

#[test]
fn grounded_diagonals_budget_each_axis_and_keep_the_longitudinal_wire_value() {
    use crate::movement::{DiagonalAlong, GroundedDiagonal, MovementOptions, StrafeSide};
    let start = Instant::now();
    let now = start + Duration::from_millis(100);
    for (along, side, x, y, animation) in [
        (DiagonalAlong::Run, StrafeSide::Right, -0.3, 0.72, 12),
        (DiagonalAlong::Run, StrafeSide::Left, 0.3, 0.72, 12),
        (DiagonalAlong::Walk, StrafeSide::Right, -0.1, 0.2, 7),
        (
            DiagonalAlong::Backward { walking: false },
            StrafeSide::Left,
            0.3,
            -0.2,
            -7,
        ),
        (
            DiagonalAlong::Backward { walking: true },
            StrafeSide::Right,
            -0.1,
            -0.2,
            -7,
        ),
    ] {
        let mut session = setup(start);
        let mut motion = request(now, x);
        motion.mode = MovementMode::GroundedDiagonal(GroundedDiagonal { along, side });
        motion.position.y = y;
        assert!(session
            .send_move_sample(&motion, now, |_| panic!("ungranted diagonal"))
            .is_err());
        grant_grounded_diagonal(&mut session, start);
        session
            .set_options(
                MovementOptions {
                    run_speed_multiplier: RunSpeedMultiplier::from_percent(120).unwrap(),
                    ..Default::default()
                },
                start,
            )
            .unwrap();
        for position in [
            Position {
                x: 0.0,
                y: y * 1.10,
                ..motion.position
            },
            Position {
                x: x * 1.25,
                y: 0.0,
                ..motion.position
            },
            Position {
                x: -x,
                ..motion.position
            },
            Position {
                heading: 1.0,
                ..motion.position
            },
        ] {
            let refused = MovementRequest {
                position,
                ..motion.clone()
            };
            assert!(session
                .send_move_sample(&refused, now, |_| panic!("invalid diagonal"))
                .is_err());
            assert_eq!(session.position(), Position::default());
        }
        session
            .send_move_sample(&motion, now, |sample| {
                assert_eq!(sample.animation, animation);
                assert_eq!(sample.delta_heading, 0);
                assert!((sample.delta[0] - x * 0.5).abs() < 0.0001);
                assert!((sample.delta[1] - y * 0.5).abs() < 0.0001);
                let encoded = sample.encode()?;
                assert_eq!(word(&encoded, 12), sample.delta[0].to_bits());
                assert_eq!(word(&encoded, 16), sample.delta[1].to_bits());
                Ok(())
            })
            .unwrap();
        let later = now + Duration::from_millis(100);
        let stop = MovementRequest {
            mode: MovementMode::Forward,
            created: later,
            ..motion
        };
        session
            .send_move_sample(&stop, later, |sample| {
                assert_eq!(sample.animation, 0);
                assert!(sample
                    .delta
                    .into_iter()
                    .all(|axis| axis.abs() < f32::EPSILON));
                Ok(())
            })
            .unwrap();
    }
}
fn grant_grounded_diagonal(session: &mut MotionSession, start: Instant) {
    let original = session.calibration().unwrap();
    session
        .calibrate(
            MotionCalibration {
                swim_backward_units_per_second: None,
                diagonal: DiagonalRates {
                    run: Some(3.0),
                    walk: Some(1.0),
                },
                walk: Some(WalkCalibration {
                    units_per_second: 2.0,
                    velocity_scale: original.velocity_scale,
                    animation: 7,
                }),
                backward: Some(BackwardCalibration {
                    units_per_second: 2.0,
                    velocity_scale: original.velocity_scale,
                    animation: -7,
                }),
                ..original
            },
            start,
        )
        .unwrap();
}

fn request(created: Instant, x: f32) -> MovementRequest {
    MovementRequest::new(
        MovementMode::Forward,
        11,
        Position {
            x,
            ..Position::default()
        },
        created,
    )
}

#[test]
fn titanium_heading_snapshots_keep_absolute_heading_and_ordinary_cadence() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut session =
        setup(start)
            .with_heading_snapshots(true)
            .with_gate(Some(UpdateGate::Changed {
                spacing: Duration::from_millis(280),
            }));
    let mut half_turn = request(at(300), 0.0);
    half_turn.position.heading = 256.0;
    assert!(session
        .send_move(&half_turn, at(300), |_| panic!(
            "unrepresentable paced turn"
        ))
        .is_err());
    half_turn.heading = HeadingUpdate::Snapshot;
    session
        .send_move(&half_turn, at(300), |packet| {
            assert_eq!(word(&packet.body, 20) & 0x000f_ffff, 0);
            assert_eq!(word(&packet.body, 32) & 0xfff, 1024);
            assert_eq!(word(&packet.body, 12), 0);
            assert_eq!(word(&packet.body, 16), 0);
            Ok(())
        })
        .unwrap();
    assert!(!session.stop_pending());

    for (ms, x, heading) in [(400, 0.5, 0.0), (500, 0.75, 384.0)] {
        let now = at(ms);
        let mut step = request(now, x);
        step.heading = HeadingUpdate::Snapshot;
        step.position.heading = heading;
        session
            .send_move(&step, now, |_| panic!("heading bypassed position cadence"))
            .unwrap();
    }
    assert!(!session
        .tick(at(579), |_| panic!("early position update"))
        .unwrap());
    assert!(session
        .tick(at(580), |packet| {
            assert_eq!(word(&packet.body, 32) & 0xfff, 1536);
            assert_eq!(word(&packet.body, 20) >> 10 & 0x3ff, 0);
            assert_ne!(word(&packet.body, 20) & 0x3ff, 0);
            assert_eq!(word(&packet.body, 24), 0.75_f32.to_bits());
            Ok(())
        })
        .unwrap());
}

#[test]
fn heading_snapshot_requires_titanium_and_preserves_movement_guards() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut snapshot = request(at(100), 0.0);
    snapshot.position.heading = 256.0;
    snapshot.heading = HeadingUpdate::Snapshot;
    let mut session = setup(start);
    assert!(session
        .send_move(&snapshot, at(100), |_| panic!("snapshot not enabled"))
        .is_err());
    let mut session = session.with_heading_snapshots(true);
    assert!(!session.is_passive(&snapshot));
    for invalid in [
        MovementRequest {
            created: at(40),
            ..snapshot.clone()
        },
        MovementRequest {
            session_id: 12,
            ..snapshot.clone()
        },
        MovementRequest {
            position: Position {
                x: 2.0,
                ..snapshot.position
            },
            ..snapshot.clone()
        },
        MovementRequest {
            mode: MovementMode::GroundedDiagonal(super::super::GroundedDiagonal {
                along: super::super::DiagonalAlong::Run,
                side: super::super::StrafeSide::Right,
            }),
            ..snapshot.clone()
        },
    ] {
        assert!(session
            .send_move(&invalid, at(100), |_| panic!("invalid snapshot"))
            .is_err());
        assert_eq!(session.position(), Position::default());
    }
    session.withdraw();
    assert!(session
        .send_move(&snapshot, at(100), |_| panic!("withdrawn heading input"))
        .is_err());
}

#[test]
fn completed_heading_snapshot_budgets_and_paces_a_chord_in_its_new_heading() {
    use crate::movement::{DiagonalAlong, GroundedDiagonal, StrafeSide};
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    for (side, sign) in [(StrafeSide::Left, -1.0), (StrafeSide::Right, 1.0)] {
        let mut session =
            setup(start)
                .with_heading_snapshots(true)
                .with_gate(Some(UpdateGate::Changed {
                    spacing: Duration::from_millis(280),
                }));
        grant_grounded_diagonal(&mut session, start);
        let initial = request(at(300), 0.0);
        session
            .send_move_sample(&initial, at(300), |_| Ok(()))
            .unwrap();
        let chord = MovementRequest {
            mode: MovementMode::GroundedDiagonal(GroundedDiagonal {
                along: DiagonalAlong::Run,
                side,
            }),
            position: Position {
                x: 0.6,
                y: sign * 0.3,
                heading: 128.0,
                ..initial.position
            },
            heading: HeadingUpdate::Snapshot,
            ..request(at(400), 0.0)
        };
        // At heading 128, +X is forward and signed Y is lateral. The
        // previous heading's basis and a changed Rate heading both fail.
        for (x, y, heading) in [
            (0.6, sign * 0.3, HeadingUpdate::Rate),
            (-sign * 0.3, 0.6, HeadingUpdate::Snapshot),
            (-0.2, sign * 0.3, HeadingUpdate::Snapshot),
            (0.6, -sign * 0.3, HeadingUpdate::Snapshot),
            (0.65, sign * 0.2, HeadingUpdate::Snapshot),
            (0.4, sign * 0.4, HeadingUpdate::Snapshot),
        ] {
            let invalid = MovementRequest {
                position: Position {
                    x,
                    y,
                    ..chord.position
                },
                heading,
                ..chord.clone()
            };
            assert!(session
                .send_move_sample(&invalid, at(400), |_| panic!("invalid chord"))
                .is_err());
            assert_eq!(session.position(), initial.position);
        }
        session
            .send_move_sample(&chord, at(400), |_| panic!("chord bypassed cadence"))
            .unwrap();
        let later = MovementRequest {
            position: Position {
                x: 1.2,
                y: sign * 0.6,
                ..chord.position
            },
            heading: HeadingUpdate::Rate,
            created: at(500),
            ..chord
        };
        session
            .send_move_sample(&later, at(500), |_| panic!("chord bypassed cadence"))
            .unwrap();
        assert!(!session
            .tick_sample(at(579), |_| panic!("early chord"))
            .unwrap());
        assert!(session
            .tick_sample(at(580), |sample| {
                assert_eq!(sample.position, later.position);
                assert_eq!((sample.animation, sample.delta_heading), (12, 0));
                assert!((sample.delta[0] - 0.3).abs() < 0.001);
                assert!((sample.delta[1] - sign * 0.15).abs() < 0.001);
                Ok(())
            })
            .unwrap());
    }
}
fn word(body: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(body[offset..offset + 4].try_into().unwrap())
}
#[test]
fn stationary_turns_encode_rate_and_the_hosts_stop_sends_a_zero_rate() {
    let start = Instant::now();
    let mut session = setup(start);
    let now = start + Duration::from_millis(100);
    let mut motion = request(now, 0.0);
    motion.position.heading = 488.0;
    session
        .send_move(&motion, now, |packet| {
            let body = &packet.body[..];
            assert_eq!(word(body, 20) & 0x3ff, 0);
            assert_eq!((word(body, 20) >> 10) & 0x3ff, 784);
            assert_eq!(word(body, 12), 0);
            assert_eq!(word(body, 16), 0);
            Ok(())
        })
        .unwrap();
    // A gap without input stops nothing; the host's own stop does.
    assert!(!session
        .tick(now + Duration::from_millis(250), |_| panic!(
            "inferred stop"
        ))
        .unwrap());
    let stopped = now + Duration::from_millis(250);
    let mut stop = request(stopped, 0.0);
    stop.position.heading = 488.0;
    session
        .send_move(&stop, stopped, |packet| {
            assert_eq!((word(&packet.body, 20) >> 10) & 0x3ff, 0);
            Ok(())
        })
        .unwrap();
    // The rate is cut toward zero: 23.99 over 250 ms, 95.96 a second, sends
    // 95.
    let later = stopped + Duration::from_millis(250);
    let mut slower = request(later, 0.0);
    slower.position.heading = 488.0 + 23.99;
    session
        .send_move(&slower, later, |packet| {
            assert_eq!((word(&packet.body, 20) >> 10) & 0x3ff, 95);
            Ok(())
        })
        .unwrap();
    let before = session.position();
    let mut snap = request(later + Duration::from_millis(100), 0.0);
    snap.position.heading = 200.0;
    assert!(session
        .send_move(&snap, snap.created, |_| panic!(
            "invalid turn must not send"
        ))
        .is_err());
    assert_eq!(session.position(), before);
}

#[test]
fn falls_need_permission_and_carry_the_descent_in_position_only() {
    let start = Instant::now();
    let now = start + Duration::from_millis(100);
    let mut fall = request(now, 0.5);
    fall.mode = MovementMode::Fall;
    fall.position.z = -4.0;
    let mut session = setup(start);
    assert!(session
        .send_move(&fall, now, |_| panic!("falls are off by default"))
        .is_err());
    // Jumps share the falls' client-side physics and permission.
    assert!(session.jump().is_err());
    let mut session = setup(start).with_falls(true);
    assert!(session.jump().is_ok());
    session
        .send_move(&fall, now, |packet| {
            let body = &packet.body[..];
            assert_eq!(word(body, 8), 0.0f32.to_bits());
            assert_eq!(word(body, 28), (-4.0f32).to_bits());
            assert_eq!(word(body, 20) & 0x3ff, 12);
            Ok(())
        })
        .unwrap();
    assert_eq!(session.position().z.to_bits(), (-4.0f32).to_bits());
}

#[test]
fn grounded_height_changes_do_not_encode_airborne_velocity() {
    let start = Instant::now();
    let mut session = setup(start);
    let now = start + Duration::from_millis(100);
    let mut motion = request(now, 0.5);
    motion.position.z = 0.75;
    session
        .send_move(&motion, now, |packet| {
            let body = &packet.body[..];
            assert_eq!(word(body, 8), 0.0f32.to_bits());
            assert_eq!(word(body, 28), 0.75f32.to_bits());
            assert_eq!(word(body, 20) & 0x3ff, 12);
            Ok(())
        })
        .unwrap();
    let later = now + Duration::from_millis(100);
    motion.created = later;
    motion.position.z = 1.0;
    session
        .send_move(&motion, later, |packet| {
            let body = &packet.body[..];
            assert_eq!(word(body, 8), 0.0f32.to_bits());
            assert_eq!(word(body, 20) & 0x3ff, 0);
            Ok(())
        })
        .unwrap();
}
#[test]
fn failed_submission_preserves_position_sequence_and_budget() {
    let start = Instant::now();
    let mut session = setup(start);
    let now = start + Duration::from_millis(100);
    let motion = request(now, 0.5);
    assert!(session
        .send_move(&motion, now, |_| anyhow::bail!("synthetic send failure"))
        .is_err());
    session
        .send_move(&motion, now, |packet| {
            let body = &packet.body[..];
            assert_eq!(&body[..4], &[7, 0, 0, 0]);
            assert!((f32::from_bits(word(body, 12)) - 0.25).abs() < 0.0001);
            assert_eq!(word(body, 20) & 0x3ff, 12);
            Ok(())
        })
        .unwrap();
}
#[test]
fn a_gap_without_input_stops_nothing_and_the_hosts_stop_goes_out_then_heartbeats() {
    let start = Instant::now();
    let mut session = setup(start);
    let moving_at = start + Duration::from_millis(100);
    session
        .send_move(&request(moving_at, 0.5), moving_at, |_| Ok(()))
        .unwrap();
    // A stalled host, however long, leaves the player moving: the program's
    // one loop only delays its next update.
    for ms in [349, 350, 600, 1_350, 5_000] {
        assert!(!session
            .tick(start + Duration::from_millis(ms), |_| panic!(
                "inferred stop"
            ))
            .unwrap());
    }
    assert!(session.stop_pending());
    let stopped = start + Duration::from_millis(5_100);
    session
        .send_move(&request(stopped, 0.5), stopped, |packet| {
            let body = &packet.body[..];
            assert_eq!(&body[2..4], &[1, 0]);
            assert_eq!(word(body, 24), 0.5f32.to_bits());
            assert_eq!(word(body, 20) & 0x3ff, 0);
            assert_eq!(&body[8..20], &[0; 12]);
            Ok(())
        })
        .unwrap();
    assert!(!session.stop_pending());
    assert!(!session
        .tick(stopped + Duration::from_millis(250), |_| panic!(
            "duplicate stop"
        ))
        .unwrap());
    assert!(!session
        .tick(stopped + Duration::from_millis(999), |_| panic!(
            "early heartbeat"
        ))
        .unwrap());
    assert!(session
        .tick(stopped + Duration::from_millis(1000), |packet| {
            let body = &packet.body[..];
            assert_eq!(&body[2..4], &[2, 0]);
            Ok(())
        })
        .unwrap());
}
#[test]
fn a_withdrawal_or_correction_stops_a_moving_player_as_the_speed_falls() {
    let start = Instant::now();
    for correct in [false, true] {
        let mut session = setup(start);
        let moving_at = start + Duration::from_millis(100);
        session
            .send_move(&request(moving_at, 0.5), moving_at, |_| Ok(()))
            .unwrap();
        let at = start + Duration::from_millis(150);
        let place = Position {
            x: 2.0,
            ..Position::default()
        };
        if correct {
            session.correct(place, at).unwrap();
        } else {
            session.withdraw();
        }
        assert!(session.stop_pending());
        let mut stops = Vec::new();
        session
            .tick(start + Duration::from_millis(400), |packet| {
                stops.push(packet.body.clone());
                Ok(())
            })
            .unwrap();
        let [stop] = &stops[..] else {
            panic!("{stops:?}")
        };
        assert_eq!(word(stop, 20) & 0x3ff, 0);
        assert_eq!(&stop[8..20], &[0; 12]);
        let x = if correct { 2.0_f32 } else { 0.5 };
        assert_eq!(word(stop, 24), x.to_bits());
        assert!(!session.stop_pending());
    }
}

#[test]
fn correction_revokes_motion_and_old_input_cannot_replay_after_recalibration() {
    let start = Instant::now();
    let mut session = setup(start);
    let old = request(start + Duration::from_millis(100), 0.5);
    let correction_at = start + Duration::from_millis(150);
    session.correct(Position::default(), correction_at).unwrap();
    assert!(session
        .send_move(&old, correction_at, |_| panic!("uncalibrated send"))
        .is_err());
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
            correction_at,
        )
        .unwrap();
    assert!(session
        .send_move(&old, start + Duration::from_millis(250), |_| panic!(
            "replayed input"
        ))
        .is_err());
}
#[test]
fn death_or_transfer_suppresses_heartbeats_and_cannot_resume_old_admission() {
    let start = Instant::now();
    let mut session = setup(start);
    session.suspend();
    assert!(!session
        .tick(start + Duration::from_secs(5), |_| panic!("dead heartbeat"))
        .unwrap());
    assert!(session
        .send_move(&request(start, 0.0), start, |_| panic!("dead movement"))
        .is_err());
    assert!(session
        .calibrate(
            MotionCalibration {
                swim_backward_units_per_second: None,
                diagonal: DiagonalRates::default(),
                walk: None,
                strafe: None,
                backward: None,
                units_per_second: 6.0,
                velocity_scale: 0.05,
                animation: 12
            },
            start
        )
        .is_err());
    let mut new_zone = MotionSession::new(12, 9, Position::default(), start).unwrap();
    new_zone
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
    let now = start + Duration::from_millis(100);
    assert!(new_zone
        .send_move(&request(now, 0.5), now, |_| panic!("old zone movement"))
        .is_err());
}
#[test]
fn malformed_and_excessive_requests_never_reach_transport() {
    let start = Instant::now();
    let mut session = setup(start);
    let now = start + Duration::from_millis(100);
    for x in [f32::NAN, f32::INFINITY, 100.0] {
        assert!(session
            .send_move(&request(now, x), now, |_| panic!("invalid send"))
            .is_err());
    }
    session.counter.set_entries(u16::MAX);
    session
        .send_move(&request(now, 0.5), now, |_| Ok(()))
        .unwrap();
    assert_eq!(session.counter.entries(), 0);
}

#[test]
fn correction_preserves_prompt_stop_and_rejects_queued_calibration() {
    let start = Instant::now();
    let mut session = setup(start);
    let calibration = session.calibration.unwrap();
    let moving_at = start + Duration::from_millis(100);
    session
        .send_move(&request(moving_at, 0.5), moving_at, |_| Ok(()))
        .unwrap();
    let corrected_at = start + Duration::from_millis(200);
    let corrected = Position {
        x: 0.25,
        ..Position::default()
    };
    session.correct(corrected, corrected_at).unwrap();
    assert!(session
        .calibrate_fresh(calibration, moving_at, corrected_at)
        .is_err());
    assert!(session
        .tick(start + Duration::from_millis(350), |packet| {
            let body = &packet.body[..];
            assert_eq!(word(body, 24), 0.25f32.to_bits());
            assert_eq!(word(body, 20) & 0x3ff, 0);
            assert_eq!(&body[8..20], &[0; 12]);
            Ok(())
        })
        .unwrap());
    assert!(session.calibration.is_none());
}

#[test]
fn queued_calibration_requires_valid_data_from_after_the_last_reset() {
    let start = Instant::now();
    let mut session = setup(start);
    let calibration = session.calibration.unwrap();
    let now = start + Duration::from_millis(300);
    let before_reset = session
        .guard
        .input_epoch
        .checked_sub(Duration::from_millis(1))
        .unwrap();
    assert!(session
        .calibrate_fresh(calibration, before_reset, now)
        .is_err());
    for invalid in [
        MotionCalibration {
            swim_backward_units_per_second: None,
            diagonal: DiagonalRates::default(),
            walk: None,
            strafe: None,
            backward: None,
            units_per_second: f32::NAN,
            ..calibration
        },
        MotionCalibration {
            swim_backward_units_per_second: None,
            velocity_scale: 0.0,
            ..calibration
        },
        MotionCalibration {
            swim_backward_units_per_second: None,
            animation: 512,
            ..calibration
        },
    ] {
        assert!(session.calibrate_fresh(invalid, now, now).is_err());
    }
    assert_eq!(session.calibration, Some(calibration));
    assert_eq!(session.guard.sampled, start);
    // A grant changed in place keeps the last accepted sample's stamp, so
    // the sample under way is neither stale nor refused.
    session.calibrate_fresh(calibration, now, now).unwrap();
    assert_eq!(session.guard.sampled, start);
}
#[test]
fn a_spacing_holds_the_latest_sample_until_it_passes_and_stops_no_sooner() {
    let start = Instant::now();
    let mut session = setup(start).with_gate(Some(UpdateGate::Changed {
        spacing: Duration::from_millis(280),
    }));
    let at = |ms| start + Duration::from_millis(ms);
    let mut sent = Vec::new();
    // The first sample goes out at once; the session began 300 ms earlier.
    let first = at(300);
    session
        .send_move(&request(first, 0.5), first, |packet| {
            sent.push(packet.body.clone());
            Ok(())
        })
        .unwrap();
    // Two more inside the spacing are accepted but held; the later wins.
    for (ms, x) in [(400, 1.0), (500, 1.5)] {
        session
            .send_move(&request(at(ms), x), at(ms), |_| {
                panic!("sent inside the spacing")
            })
            .unwrap();
        assert!((session.position().x - x).abs() < f32::EPSILON);
    }
    assert!(!session.tick(at(579), |_| panic!("early")).unwrap());
    assert!(session
        .tick(at(580), |packet| {
            sent.push(packet.body.clone());
            Ok(())
        })
        .unwrap());
    let [first, held] = &sent[..] else {
        panic!("{sent:?}")
    };
    assert_eq!(&first[2..4], &[0, 0]);
    // The held sample takes the next sequence and keeps its motion.
    assert_eq!(&held[2..4], &[1, 0]);
    assert_eq!(word(held, 24), 1.5f32.to_bits());
    assert_eq!(word(held, 12), 0.25f32.to_bits());
    assert_ne!(word(held, 20) & 0x3ff, 0);
    // The host's stop waits for the spacing too, then heartbeats resume.
    session
        .send_move(&request(at(700), 1.5), at(700), |_| {
            panic!("stop sent inside the spacing")
        })
        .unwrap();
    assert!(!session.tick(at(859), |_| panic!("early stop")).unwrap());
    assert!(session
        .tick(at(860), |packet| {
            assert_eq!(&packet.body[2..4], &[2, 0]);
            assert_eq!(word(&packet.body, 20) & 0x3ff, 0);
            Ok(())
        })
        .unwrap());
}

#[test]
fn a_correction_drops_a_held_sample() {
    let start = Instant::now();
    let mut session = setup(start).with_gate(Some(UpdateGate::Changed {
        spacing: Duration::from_millis(280),
    }));
    let at = |ms| start + Duration::from_millis(ms);
    session
        .send_move(&request(at(300), 0.5), at(300), |_| Ok(()))
        .unwrap();
    session
        .send_move(&request(at(400), 1.0), at(400), |_| {
            panic!("inside the spacing")
        })
        .unwrap();
    session.correct(Position::default(), at(450)).unwrap();
    // Only the stop for the sample that went out follows, never the held one.
    assert!(session
        .tick(at(600), |packet| {
            assert_eq!(word(&packet.body, 24), 0.0f32.to_bits());
            assert_eq!(word(&packet.body, 20) & 0x3ff, 0);
            Ok(())
        })
        .unwrap());
}

#[test]
fn a_generation_may_fall_faster_than_the_provisional_limit() {
    let start = Instant::now();
    let now = start + Duration::from_millis(100);
    let mut fall = request(now, 0.0);
    fall.mode = MovementMode::Fall;
    // 20 units in 0.1 s is past 40 units a second, within EQMac's falls.
    fall.position.z = -20.0;
    let mut provisional = setup(start).with_falls(true);
    assert!(provisional
        .send_move(&fall, now, |_| panic!("faster than the provisional limit"))
        .is_err());
    let mut eqmac = setup(start)
        .with_falls(true)
        .with_fall_speed(Some(super::super::EQMAC_FALL_SPEED));
    eqmac.send_move(&fall, now, |_| Ok(())).unwrap();
    assert_eq!(eqmac.position().z.to_bits(), (-20.0f32).to_bits());
}
#[test]
fn a_jump_rises_past_a_step_once_the_session_announces_it() {
    let start = Instant::now();
    let now = start + Duration::from_millis(50);
    // The first 50 ms of a 44.55 u/s jump rise 2.03 units, past a step.
    let mut rise = request(now, 0.0);
    rise.mode = MovementMode::Fall;
    rise.position.z = 2.03;
    let jump = Some((
        super::super::JUMP_SPEED,
        super::super::gravity_per_second(None),
    ));
    let mut session = setup(start).with_falls(true).with_jump(jump);
    assert!(session
        .send_move(&rise, now, |_| panic!("no jump was announced"))
        .is_err());
    session.jumped();
    session.send_move(&rise, now, |_| Ok(())).unwrap();
    assert_eq!(session.position().z.to_bits(), 2.03f32.to_bits());
    // Without the generation's jump read, the announcement admits nothing.
    let mut unread = setup(start).with_falls(true);
    unread.jumped();
    assert!(unread
        .send_move(&rise, now, |_| panic!("the jump is unread"))
        .is_err());
}

#[test]
fn an_announced_jump_refuses_diagonal_credit_without_consuming_the_sample() {
    use crate::movement::{DiagonalAlong, GroundedDiagonal, MovementOptions, StrafeSide};

    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut session = setup(start).with_falls(true).with_jump(Some((
        super::super::JUMP_SPEED,
        super::super::gravity_per_second(None),
    )));
    grant_grounded_diagonal(&mut session, start);
    let mode = MovementMode::GroundedDiagonal(GroundedDiagonal {
        along: DiagonalAlong::Run,
        side: StrafeSide::Right,
    });
    let mut step = request(at(100), -0.3);
    step.mode = mode;
    step.position.y = 0.6;
    step.position.z = 0.5;
    let submit = |session: &mut MotionSession, sample: &MovementRequest| {
        session
            .send_move_sample(sample, sample.created, |_| Ok(()))
            .unwrap();
    };
    submit(&mut session, &step);
    assert_eq!(session.position(), step.position);

    // The host marks the jump only after its notice was submitted.
    assert_eq!(session.jump().unwrap().opcode, JUMP_OPCODE);
    session.jumped();
    let diagonal = |accepted: Position, now| MovementRequest {
        mode,
        position: Position {
            x: accepted.x - 0.15,
            y: accepted.y + 0.3,
            ..accepted
        },
        ..request(now, accepted.x)
    };
    let reject = |session: &mut MotionSession, now| {
        let accepted = session.position();
        let sampled = session.guard.sampled;
        let jump = session.guard.jumped_at;
        let sequence = session.counter.entries();
        let combined = diagonal(accepted, now);
        let refusal = session.send_move_sample(&combined, now, |_| panic!("diagonal during jump"));
        assert_eq!(
            refusal.unwrap_err().to_string(),
            "grounded diagonal movement is unavailable during a tracked jump"
        );
        assert_eq!(session.position(), accepted);
        assert_eq!(session.guard.sampled, sampled);
        assert_eq!(session.guard.jumped_at, jump);
        assert_eq!(session.counter.entries(), sequence);
    };

    let ordinary = MovementOptions::default();
    let flight = MovementOptions {
        fly: true,
        ..ordinary
    };
    // Toggling flight twice before the first jump sample cannot make the
    // pending jump eligible for ground credit, even with unchanged Z.
    session.set_options(flight, at(105)).unwrap();
    session.set_options(ordinary, at(110)).unwrap();
    reject(&mut session, at(150));
    let ascent = MovementRequest {
        mode: MovementMode::Fall,
        position: Position {
            z: step.position.z + 2.03,
            ..step.position
        },
        ..request(at(150), step.position.x)
    };
    submit(&mut session, &ascent);
    session
        .set_options(
            MovementOptions {
                run_speed_multiplier: RunSpeedMultiplier::from_percent(110).unwrap(),
                walk_while_stunned: true,
                ..ordinary
            },
            at(155),
        )
        .unwrap();
    // An unrelated option change may retire voluntary input, but it
    // cannot make a known airborne step eligible for a ground chord.
    reject(&mut session, at(200));
    session.set_options(flight, at(205)).unwrap();
    let cardinal = MovementRequest {
        position: Position {
            y: ascent.position.y + 0.2,
            ..ascent.position
        },
        ..request(at(250), ascent.position.x)
    };
    submit(&mut session, &cardinal);
    assert_eq!(session.position(), cardinal.position);
    // A sample accepted under the flight budget does not establish landing.
    session.set_options(ordinary, at(255)).unwrap();
    reject(&mut session, at(300));

    // Ordinary sample progression still retires the ascent tracker at its
    // apex. This only ends the jump-specific guard against a ground chord.
    let past_apex = MovementRequest {
        created: at(400),
        ..cardinal
    };
    submit(&mut session, &past_apex);
    assert!(session.guard.jumped_at.is_none());
    let grounded = diagonal(past_apex.position, at(450));
    submit(&mut session, &grounded);
    assert_eq!(session.position(), grounded.position);
}

#[test]
fn the_wire_velocity_covers_the_stamp_interval_not_the_processing_one() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut session = setup(start);
    let mut sent = Vec::new();
    // 0.6 units over a 100 ms stamp interval, processed 30 ms late.
    session
        .send_move_sample(&request(at(100), 0.6), at(130), |sample| {
            sent.push(*sample);
            Ok(())
        })
        .unwrap();
    // Processed 75 ms after the first, stamped 100 ms after it.
    session
        .send_move_sample(&request(at(200), 1.2), at(205), |sample| {
            sent.push(*sample);
            Ok(())
        })
        .unwrap();
    // 6 units per second at a velocity scale of 0.05.
    for sample in sent {
        assert!((sample.delta[0] - 0.3).abs() < 1e-4, "{sample:?}");
    }
}

fn mac_era(start: Instant) -> MotionSession {
    setup(start).with_gate(Some(UpdateGate::Flagged {
        pack: crate::quarm::client_update,
        rule: crate::movement::EQMAC_FLAGS,
    }))
}

/// Feeds a host sample (when `x` is some) and a pass of the gate at each
/// 100 ms step from `from` to `to`, counting the updates that go out.
fn passes(
    session: &mut MotionSession,
    start: Instant,
    steps: std::ops::RangeInclusive<u64>,
    x: impl Fn(u64) -> Option<f32>,
) -> Vec<u64> {
    let mut sent = Vec::new();
    for step in steps {
        let now = start + Duration::from_millis(step * 100);
        if let Some(x) = x(step) {
            // Every host sample is accepted: 100 ms apart is past the
            // guard's 50 ms floor, and 0.5 units fits its budget.
            session
                .send_move_sample(&request(now, x), now, |_| {
                    sent.push(step * 100);
                    Ok(())
                })
                .unwrap();
        }
        if session
            .tick_sample(now, |sample| {
                crate::quarm::client_update(sample)?;
                Ok(())
            })
            .unwrap()
        {
            sent.push(step * 100);
        }
    }
    sent
}

#[test]
fn the_mac_era_gate_sends_a_steady_run_once_a_second() {
    let start = Instant::now();
    let mut session = mac_era(start);
    // Running straight at 5 units a second: the start goes out at once,
    // then only the 1 s timer lets the next out, each more than 1000 ms on.
    #[allow(clippy::cast_precision_loss)]
    let sent = passes(&mut session, start, 1..=25, |step| Some(step as f32 * 0.5));
    assert_eq!(sent, [100, 1200, 2300]);
    // A gap without input sends nothing new for a moving player.
    let gap = passes(&mut session, start, 26..=29, |_| None);
    assert_eq!(gap, Vec::<u64>::new());
    // The host's stop is a speed change: due at once, out on the first pass
    // more than 150 ms after the last update.
    let stop = passes(&mut session, start, 30..=33, |_| Some(12.5));
    assert_eq!(stop, [3000]);
}

#[test]
fn the_mac_era_gate_repeats_a_still_update_six_times_then_holds_it() {
    let start = Instant::now();
    let mut session = mac_era(start);
    let sent = passes(&mut session, start, 1..=100, |_| None);
    // The first, then six repeats a timer apart; then nothing.
    assert_eq!(sent, [100, 1200, 2300, 3400, 4500, 5600, 6700]);
    // Still due, so the first change goes out at once.
    let moved = passes(&mut session, start, 101..=101, |_| Some(0.5));
    assert_eq!(moved, [10_100]);
}

#[test]
fn the_mac_era_gate_lets_a_turn_out_no_sooner_than_150_ms_after_the_last() {
    let start = Instant::now();
    let mut session = mac_era(start);
    let at = |ms| start + Duration::from_millis(ms);
    let mut sent = 0;
    session
        .tick_sample(at(100), |_| {
            sent += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(sent, 1);
    // A 3-unit turn 100 ms later is due at once but held by the spacing.
    let mut turn = request(at(200), 0.0);
    turn.position.heading = 3.0;
    session
        .send_move_sample(&turn, at(200), |_| panic!("inside 150 ms"))
        .unwrap();
    assert!(!session
        .tick_sample(at(250), |_| panic!("at 150 ms"))
        .unwrap());
    assert!(session.tick_sample(at(251), |_| Ok(())).unwrap());
}

#[test]
fn the_mac_era_gate_sends_after_a_jump_and_the_titanium_gate_skips_no_change() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut session = mac_era(start).with_falls(true);
    assert!(session.tick_sample(at(100), |_| Ok(())).unwrap());
    // Nothing is due until the timer, unless a jump marks it.
    assert!(!session.tick_sample(at(400), |_| panic!("not due")).unwrap());
    session.jumped();
    assert!(session.tick_sample(at(500), |_| Ok(())).unwrap());

    let mut titanium = setup(start).with_gate(Some(UpdateGate::Changed {
        spacing: Duration::from_millis(280),
    }));
    titanium
        .send_move(&request(at(300), 0.5), at(300), |_| Ok(()))
        .unwrap();
    // Standing there is a change (the stop); standing there again isn't,
    // so nothing is sent or held.
    titanium
        .send_move(&request(at(700), 0.5), at(700), |_| Ok(()))
        .unwrap();
    titanium
        .send_move(&request(at(1100), 0.5), at(1100), |_| panic!("no change"))
        .unwrap();
    assert!(titanium.held.is_none());
}
#[test]
fn a_grounded_climb_rises_the_bodys_reach_each_step_once_set() {
    let start = Instant::now();
    let now = start + Duration::from_millis(250);
    // Five steps at a human's Titanium reach of 3.15 are 15.75 units.
    let mut climb = request(now, 1.0);
    climb.position.z = 15.75;
    let mut session = setup(start);
    assert!(session
        .send_move(&climb, now, |_| panic!("no climb was set"))
        .is_err());
    session.set_climb(3.15);
    let mut over = request(now, 1.0);
    over.position.z = 15.8;
    assert!(session
        .send_move(&over, now, |_| panic!("past the body's reach"))
        .is_err());
    session.send_move(&climb, now, |_| Ok(())).unwrap();
    assert_eq!(session.position().z.to_bits(), 15.75f32.to_bits());
}

#[test]
fn withdrawn_input_allows_only_fresh_vertical_descent_and_no_jump() {
    let start = Instant::now();
    let mut session = setup(start).with_falls(true);
    session.withdraw();
    let epoch = Instant::now();
    assert!(session.check_jump().is_err());
    let mut falling = request(epoch + Duration::from_millis(100), 0.0);
    falling.mode = MovementMode::Fall;
    falling.position.z = -1.0;
    let now = falling.created;
    for position in [
        Position {
            x: 0.001,
            ..falling.position
        },
        Position {
            heading: 1.0,
            ..falling.position
        },
        Position {
            z: 0.001,
            ..falling.position
        },
    ] {
        let bad = MovementRequest {
            position,
            ..falling
        };
        assert!(session
            .send_move(&bad, now, |_| panic!("input while withdrawn"))
            .is_err());
    }
    session.send_move(&falling, now, |_| Ok(())).unwrap();
    let before = session.position();
    session
        .calibrate(setup(start).calibration().unwrap(), now)
        .unwrap();
    let stale = request(start, 0.3);
    assert!(session
        .send_move(&stale, now, |_| panic!("stale input restored"))
        .is_err());
    assert_eq!(session.position(), before);
    assert!(session.check_jump().is_ok());
}

#[test]
#[allow(clippy::float_cmp, reason = "Exact checked and assigned positions")]
fn prepared_move_drop_preserves_owner_and_rejects_a_foreign_admission() {
    let start = Instant::now();
    let now = start + Duration::from_millis(100);
    let mut session = setup(start);
    let motion = request(now, 0.5);
    let mut foreign = motion.clone();
    foreign.session_id += 1;
    assert!(session.prepare_move_sample(&foreign, now).is_err());
    {
        let prepared = session.prepare_move_sample(&motion, now).unwrap();
        assert_eq!(prepared.sample.position, motion.position);
        assert_eq!(prepared.sample.sequence, 0);
        assert!(matches!(prepared.release, Release::Send));
    }
    assert_eq!(session.position(), Position::default());
    assert_eq!(session.counter.entries(), 0);
    assert_eq!(session.last_sent, start);
    // Dropping a checked candidate did not consume its guard time budget.
    session
        .send_move_sample(&motion, now, |sample| {
            assert_eq!(sample.position, motion.position);
            assert_eq!(sample.sequence, 0);
            Ok(())
        })
        .unwrap();
}

#[test]
#[allow(clippy::float_cmp, reason = "Exact checked and assigned positions")]
fn prepared_held_sample_commits_locally_then_releases_the_same_sample() {
    let start = Instant::now();
    let now = start + Duration::from_millis(100);
    let mut session = setup(start).with_gate(Some(UpdateGate::Changed {
        spacing: Duration::from_millis(280),
    }));
    let motion = request(now, 0.5);
    let prepared = session.prepare_move_sample(&motion, now).unwrap();
    assert!(matches!(prepared.release, Release::Hold));
    let expected = prepared.sample;
    prepared
        .submit(|_| panic!("held acceptance must not send"))
        .unwrap();
    assert_eq!(session.position(), expected.position);
    assert_eq!(session.counter.entries(), 0);
    assert_eq!(session.last_sent, start);
    session
        .tick_sample(start + Duration::from_millis(280), |sample| {
            assert_eq!(*sample, expected);
            Ok(())
        })
        .unwrap();
    assert_eq!(session.counter.entries(), 1);
}

#[test]
#[allow(clippy::float_cmp, reason = "Exact checked and assigned positions")]
fn unchanged_candidate_accepts_input_once_without_a_send_or_extra_budget() {
    let start = Instant::now();
    let mut session = setup(start).with_gate(Some(UpdateGate::Changed {
        spacing: Duration::from_millis(50),
    }));
    let first = start + Duration::from_millis(100);
    let still = request(first, 0.0);
    session.send_move_sample(&still, first, |_| Ok(())).unwrap();
    let now = first + Duration::from_millis(100);
    let next = request(now, 0.0);
    let prepared = session.prepare_move_sample(&next, now).unwrap();
    assert!(matches!(prepared.release, Release::Unchanged));
    assert_eq!(prepared.sample.position, next.position);
    prepared
        .submit(|_| panic!("unchanged sample must not send"))
        .unwrap();
    assert_eq!(session.counter.entries(), 1);
    assert_eq!(session.last_sent, first);
    assert!(session
        .send_move_sample(&next, now, |_| panic!("duplicate input stamp"))
        .is_err());
}

#[test]
#[allow(clippy::float_cmp, reason = "Exact checked and assigned positions")]
fn failed_prepared_send_preserves_motion_without_undoing_external_effects() {
    let start = Instant::now();
    let now = start + Duration::from_millis(100);
    let mut session = setup(start);
    let motion = request(now, 0.5);
    let mut external_effects = Vec::new();
    assert!(session
        .send_move_sample(&motion, now, |sample| {
            external_effects.push(sample.position);
            anyhow::bail!("synthetic terminal execution failure")
        })
        .is_err());
    assert_eq!(external_effects, [motion.position]);
    assert_eq!(session.position(), Position::default());
    assert_eq!(session.counter.entries(), 0);
    assert_eq!(session.last_sent, start);
    // Test-only inspection of preserved state; session-level execution errors
    // may still require terminal teardown rather than a production retry.
    session
        .send_move_sample(&motion, now, |sample| {
            assert_eq!(sample.sequence, 0);
            Ok(())
        })
        .unwrap();
}

#[test]
fn run_speed_multiplier_changes_the_worker_budget_until_reset() {
    use crate::movement::MovementOptions;

    let start = Instant::now();
    let now = start + Duration::from_millis(100);
    assert_eq!(RunSpeedMultiplier::default().percent(), 100);
    assert!(RunSpeedMultiplier::from_percent(99).is_none());
    assert!(RunSpeedMultiplier::from_percent(141).is_none());
    assert!(RunSpeedMultiplier::from_percent(u16::MAX).is_none());

    // The host's 6 units/second forward grant covers these distances in 100 ms.
    for (percent, distance) in [(100, 0.60), (110, 0.66), (120, 0.72), (140, 0.84)] {
        let mut session = setup(start);
        session
            .set_options(
                MovementOptions {
                    run_speed_multiplier: RunSpeedMultiplier::from_percent(percent).unwrap(),
                    ..Default::default()
                },
                start,
            )
            .unwrap();
        assert!(session
            .send_move(&request(now, distance + 0.02), now, |_| panic!(
                "over budget"
            ))
            .is_err());
        session
            .send_move(&request(now, distance), now, |_| Ok(()))
            .unwrap();
        session
            .set_options(MovementOptions::default(), now)
            .unwrap();
        let later = now + Duration::from_millis(100);
        assert!(session
            .send_move(&request(later, distance + 0.65), later, |_| panic!(
                "over reset budget"
            ))
            .is_err());
        session
            .send_move(&request(later, distance + 0.60), later, |_| Ok(()))
            .unwrap();
    }
}

#[test]
fn changing_run_speed_keeps_the_moving_sample_and_its_send_deadline() {
    use crate::movement::MovementOptions;

    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut session = setup(start).with_gate(Some(UpdateGate::Changed {
        spacing: Duration::from_millis(280),
    }));
    session
        .set_options(
            MovementOptions {
                run_speed_multiplier: RunSpeedMultiplier::MAX,
                ..Default::default()
            },
            start,
        )
        .unwrap();
    session
        .send_move_sample(&request(at(300), 0.5), at(300), |_| Ok(()))
        .unwrap();
    session
        .send_move_sample(&request(at(400), 1.3), at(400), |_| {
            panic!("moving sample bypassed pacing")
        })
        .unwrap();
    let held = session.held.unwrap();
    assert!(MotionSession::moving_update(&held));
    session
        .set_options(MovementOptions::default(), at(450))
        .unwrap();
    assert!(session.moving);
    assert_eq!(session.held, Some(held));
    assert!(!session
        .tick_sample(at(579), |_| panic!("speed change bypassed pacing"))
        .unwrap());
    assert!(session
        .tick_sample(at(580), |sample| {
            assert_eq!(*sample, held);
            Ok(())
        })
        .unwrap());

    let now = at(600);
    assert!(session
        .send_move_sample(&request(now, 2.6), now, |_| panic!("old speed budget"))
        .is_err());
    session
        .send_move_sample(&request(now, 2.5), now, |_| {
            panic!("next moving sample bypassed pacing")
        })
        .unwrap();
    assert!(MotionSession::moving_update(&session.held.unwrap()));
    session
        .set_options(
            MovementOptions {
                fly: true,
                ..Default::default()
            },
            at(610),
        )
        .unwrap();
    assert!(!session.moving);
    assert!(!MotionSession::moving_update(&session.held.unwrap()));
}

#[test]
fn the_run_speed_multiplier_scales_flight_travel_only_horizontally() {
    use crate::movement::MovementOptions;

    let start = Instant::now();
    let now = start + Duration::from_millis(100);
    let mut session = setup(start);
    session
        .set_options(
            MovementOptions {
                run_speed_multiplier: RunSpeedMultiplier::MAX,
                fly: true,
                ..Default::default()
            },
            start,
        )
        .unwrap();
    // At 6 units/s the horizontal limit is 0.84 and the vertical limit is 0.60.
    let mut flight = request(now, 0.8);
    flight.position.z = 0.8;
    let refused = session
        .send_move_sample(&flight, now, |_| {
            panic!("boosted ascent escaped the vertical gate")
        })
        .unwrap_err();
    assert_eq!(
        refused.to_string(),
        "vertical motion requires a supported movement mode"
    );
    assert_eq!(session.position(), Position::default());
    flight.position.z = 0.6;
    session
        .send_move_sample(&flight, now, |sample| {
            assert_eq!(sample.position, flight.position);
            Ok(())
        })
        .unwrap();
    assert_eq!(session.position(), flight.position);
}

#[test]
fn flying_changes_only_the_vertical_gate_and_still_needs_current_valid_motion() {
    let start = Instant::now();
    let now = start + Duration::from_millis(100);
    let mut proposal = request(now, 0.0);
    proposal.position.z = 2.5;
    let mut session = setup(start);
    let mut calibration = session.calibration().unwrap();
    calibration.units_per_second = 30.0;
    session.calibrate(calibration, start).unwrap();
    assert!(session.send_move(&proposal, now, |_| Ok(())).is_err());
    session
        .set_options(
            super::super::MovementOptions {
                fly: true,
                ..Default::default()
            },
            start,
        )
        .unwrap();
    let mut foreign = proposal.clone();
    foreign.session_id = 99;
    assert!(session.send_move(&foreign, now, |_| Ok(())).is_err());
    let mut excessive = proposal.clone();
    excessive.position.z = 3.1;
    assert!(session.send_move(&excessive, now, |_| Ok(())).is_err());
    let mut invalid = proposal.clone();
    invalid.position.z = f32::NAN;
    assert!(session.send_move(&invalid, now, |_| Ok(())).is_err());
    session.guard.bounds = Some(super::super::MovementBounds::new([-1.0; 3], [1.0; 3]).unwrap());
    assert!(session.send_move(&proposal, now, |_| Ok(())).is_err());
    session.guard.bounds = None;
    session.send_move(&proposal, now, |_| Ok(())).unwrap();
    let later = now + Duration::from_millis(100);
    let mut descent = request(later, 0.0);
    descent.position.z = 0.0;
    session.send_move(&descent, later, |_| Ok(())).unwrap();
    session.withdraw();
    let mut no_speed = request(later + Duration::from_millis(100), 0.0);
    no_speed.position.z = 1.0;
    assert!(session
        .send_move(&no_speed, no_speed.created, |_| Ok(()))
        .is_err());
}

#[test]
fn caller_stamped_options_preserve_same_frame_motion_without_idle_credit() {
    let start = Instant::now();
    let caller = start + Duration::from_millis(100);
    let processing = caller + Duration::from_millis(25);
    let mut session = setup(start);
    let options = super::super::MovementOptions {
        run_speed_multiplier: RunSpeedMultiplier::from_percent(110).unwrap(),
        ..Default::default()
    };
    session.set_options(options, caller).unwrap();
    let stale = request(caller.checked_sub(Duration::from_millis(1)).unwrap(), 0.2);
    assert!(session.send_move(&stale, processing, |_| Ok(())).is_err());
    session
        .send_move(&request(caller, 0.65), processing, |_| Ok(()))
        .unwrap();
    assert!(session
        .set_options(
            super::super::MovementOptions::default(),
            caller.checked_sub(Duration::from_millis(1)).unwrap()
        )
        .is_err());
    let later = caller + Duration::from_millis(100);
    assert!(session
        .send_move(
            &request(later, 1.4),
            processing + Duration::from_millis(100),
            |_| Ok(())
        )
        .is_err());
    assert_eq!(session.position().x, 0.65);
}

#[test]
fn a_boosted_forward_step_can_finish_as_a_fall_within_the_same_budget() {
    let start = Instant::now();
    let now = start + Duration::from_millis(100);
    let mut fall = request(now, 0.84);
    fall.mode = MovementMode::Fall;
    fall.position.z = -0.5;
    let mut session = setup(start).with_falls(true);
    assert!(session
        .send_move(&fall, now, |_| panic!("unboosted fall exceeded its budget"))
        .is_err());
    session
        .set_options(
            super::super::MovementOptions {
                run_speed_multiplier: RunSpeedMultiplier::MAX,
                ..Default::default()
            },
            start,
        )
        .unwrap();
    let mut too_far = fall.clone();
    too_far.position.x = 0.87;
    assert!(session
        .send_move(&too_far, now, |_| panic!(
            "boosted fall exceeded its budget"
        ))
        .is_err());
    session.send_move(&fall, now, |_| Ok(())).unwrap();
    assert_eq!(session.position(), fall.position);
}

#[test]
fn swim_budget_is_three_dimensional_and_vertical_strokes_are_voluntary() {
    let start = Instant::now();
    let now = start + Duration::from_millis(100);
    let mut proposal = request(now, 0.0);
    proposal.mode = MovementMode::Swim;
    proposal.position.z = -0.08;

    let mut session = setup(start);
    assert!(session
        .send_move_sample(&proposal, now, |_| panic!("ungranted stroke"))
        .is_err());
    let mut session = session.with_swimming(true);
    assert!(!session.is_passive(&proposal), "a dive is not gravity");
    proposal.position.x = 0.08;
    assert!(session
        .send_move_sample(&proposal, now, |_| panic!("axes exceed the shared budget"))
        .is_err());
    assert_eq!(session.position(), Position::default());

    proposal.position.x = 0.0;
    assert!(session
        .send_move_sample(&proposal, now, |_| anyhow::bail!("failed transport"))
        .is_err());
    assert_eq!(session.position(), Position::default());
    session
        .send_move_sample(&proposal, now, |sample| {
            assert_eq!(sample.position, proposal.position);
            assert!((sample.delta[2] + 0.04).abs() < 1e-6);
            assert_eq!(sample.animation, 0);
            assert_eq!(sample.packet()?.body.len(), 36);
            Ok(())
        })
        .unwrap();
    assert!(session.stop_pending(), "vertical motion requires a stop");
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "One chronological trace checks accept, rejection, correction, and rearm"
)]
fn shoreline_replays_wet_and_dry_frames_without_borrowing_from_another_sample() {
    use crate::movement::{
        ShorelineFrame, ShorelineStroke, ShorelineTrace, ShorelineVelocity, SHORELINE_FRAMES,
    };

    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut session = setup(start)
        .with_swimming(true)
        .with_heading_snapshots(true)
        .with_jump(Some((44.55, 160.0)));
    let grant = session.calibration().unwrap().units_per_second;
    let swim = MovementRequest::new(MovementMode::Swim, 11, Position::default(), at(100));
    let make = |start, finish, times: &[ShorelineFrame]| {
        let mut frames = [ShorelineFrame::EMPTY; SHORELINE_FRAMES];
        frames[..times.len()].copy_from_slice(times);
        MovementMode::Shoreline(ShorelineTrace {
            start,
            finish,
            frames,
        })
    };
    let wet = ShorelineFrame::new(16, true, false, ShorelineStroke::new([0.0, 0.9, 0.4])).unwrap();
    let turned =
        ShorelineFrame::new(16, true, false, ShorelineStroke::new([0.9, 0.0, -0.4])).unwrap();
    let released = ShorelineFrame::new(0, true, false, ShorelineStroke::default()).unwrap();
    let dry = ShorelineFrame::new(7, false, false, ShorelineStroke::default()).unwrap();
    let mut physics = ShorelineVelocity::start(0.0);
    physics.advance(wet, grant, 160.0);
    let ([x, y], z) = physics.travel(wet);
    let proposal = MovementRequest::new(
        make(true, false, &[wet]),
        11,
        Position {
            x,
            y,
            z,
            ..Position::default()
        },
        at(200),
    );
    assert!(session
        .send_move_sample(&proposal, at(200), |_| panic!("start without swim"))
        .is_err());
    session
        .send_move_sample(&swim, at(100), |_| Ok(()))
        .unwrap();
    // A frame longer than the bound cannot be built at all.
    assert!(ShorelineFrame::new(51, true, false, ShorelineStroke::new([0.0, 0.9, 0.4])).is_none());
    // A start must be one wet, upward frame.
    let level =
        ShorelineFrame::new(16, true, false, ShorelineStroke::new([0.0, 1.0, 0.0])).unwrap();
    for bad in [
        make(true, false, &[level]),
        make(true, false, &[dry]),
        make(true, false, &[wet, wet]),
    ] {
        let request = MovementRequest::new(bad, 11, Position::default(), at(200));
        assert!(session
            .send_move_sample(&request, at(200), |_| panic!("bad start frame"))
            .is_err());
    }
    // Motion beyond the replayed travel is refused on every axis. Shorter
    // motion inside the box stays allowed: collision may stop a frame early.
    let travel = proposal.position;
    for (x, y, z) in [
        (travel.x + 0.5, travel.y, travel.z),
        (travel.x, travel.y + 0.5, travel.z),
        (travel.x, travel.y, travel.z + 0.5),
        (travel.x, travel.y, -0.5),
    ] {
        let beyond = Position { x, y, z, ..travel };
        let request = MovementRequest::new(proposal.mode, 11, beyond, at(200));
        assert!(session
            .send_move_sample(&request, at(200), |_| panic!("beyond replayed travel"))
            .is_err());
    }
    assert_eq!(session.position(), swim.position);
    assert!(session
        .send_move_sample(&proposal, at(200), |_| anyhow::bail!("transport"))
        .is_err());
    assert_eq!(session.position(), swim.position);
    session
        .send_move_sample(&proposal, at(200), |packet| {
            assert!((packet.delta[2] - physics.vertical).abs() < 1e-6);
            assert_eq!(packet.packet()?.body.len(), 36);
            Ok(())
        })
        .unwrap();

    let old_position = session.position();
    let mut too_long =
        MovementRequest::new(make(false, false, &[wet; 7]), 11, old_position, at(300));
    assert!(session
        .send_move_sample(&too_long, at(300), |_| panic!("borrowed time"))
        .is_err());
    too_long.session_id = 12;
    assert!(session
        .send_move_sample(&too_long, at(300), |_| panic!("other admission"))
        .is_err());

    let mut next = old_position;
    let frames = [turned, released, dry, dry];
    for frame in frames {
        physics.advance(frame, grant, 160.0);
        let ([x, y], z) = physics.travel(frame);
        next.x += x;
        next.y += y;
        next.z += z;
    }
    assert!(next.x > old_position.x);
    next.heading = 128.0;
    let mut continued = MovementRequest::new(make(false, false, &frames), 11, next, at(300));
    continued.heading = HeadingUpdate::Snapshot;
    session
        .send_move_sample(&continued, at(300), |packet| {
            assert_eq!(packet.position.heading, 128.0);
            assert_eq!(
                packet.delta,
                [
                    physics.horizontal[0],
                    physics.horizontal[1],
                    physics.vertical,
                ]
            );
            Ok(())
        })
        .unwrap();
    let swim = MovementRequest::new(MovementMode::Swim, 11, next, at(400));
    session
        .send_move_sample(&swim, at(400), |_| Ok(()))
        .unwrap();
    let early = MovementRequest::new(proposal.mode, 11, next, at(500));
    assert!(session
        .send_move_sample(&early, at(500), |_| panic!("cooldown bypass"))
        .is_err());

    let mut corrected = setup(start)
        .with_swimming(true)
        .with_jump(Some((44.55, 160.0)));
    let calibration = corrected.calibration().unwrap();
    let initial = MovementRequest::new(MovementMode::Swim, 11, Position::default(), at(100));
    corrected
        .send_move_sample(&initial, at(100), |_| Ok(()))
        .unwrap();
    corrected
        .send_move_sample(&proposal, at(200), |_| Ok(()))
        .unwrap();
    corrected.correct(next, at(250)).unwrap();
    corrected.calibrate(calibration, at(250)).unwrap();
    let stale = MovementRequest::new(make(false, true, &[dry]), 11, next, at(350));
    assert!(corrected
        .send_move_sample(&stale, at(350), |_| panic!("correction retained arc"))
        .is_err());
}

#[test]
fn correction_drops_held_swim_and_requires_a_new_grant_and_input() {
    let start = Instant::now();
    let stroke = start + Duration::from_millis(100);
    let correction = start + Duration::from_millis(200);
    let stop = start + Duration::from_millis(300);
    let later = start + Duration::from_millis(600);
    let mut session = setup(start)
        .with_swimming(true)
        .with_gate(Some(UpdateGate::Changed {
            spacing: Duration::from_millis(280),
        }));
    let calibration = session.calibration().unwrap();
    let mut old = request(stroke, 0.0);
    old.mode = MovementMode::Swim;
    old.position.z = 0.08;
    session
        .send_move_sample(&old, stroke, |_| panic!("stroke bypassed pacing"))
        .unwrap();
    let placed = Position {
        x: 25.0,
        z: -12.0,
        ..Position::default()
    };
    session.correct(placed, correction).unwrap();
    assert!(session
        .tick_sample(stop, |sample| {
            assert_eq!(sample.position, placed);
            assert_eq!(sample.delta, [0.0; 3]);
            Ok(())
        })
        .unwrap());
    let mut fresh = MovementRequest::new(MovementMode::Swim, 11, placed, later);
    fresh.position.z += 0.08;
    assert!(session
        .send_move_sample(&fresh, later, |_| panic!("correction did not revoke input"))
        .is_err());
    session.calibrate(calibration, correction).unwrap();
    assert!(session
        .send_move_sample(&old, correction, |_| panic!(
            "replayed a pre-correction stroke"
        ))
        .is_err());
    assert!(session
        .send_move_sample(&fresh, later, |sample| {
            assert_eq!(sample.position, fresh.position);
            assert!(sample.delta[2] > 0.0);
            Ok(())
        })
        .is_ok());
}

#[test]
fn titanium_counter_carries_its_frames_and_earlier_updates_from_each_admission() {
    use crate::movement::TITANIUM_UPDATE_COUNTER;
    let titanium = |start: Instant| {
        setup(start)
            .with_gate(Some(UpdateGate::Changed {
                spacing: Duration::from_millis(280),
            }))
            .with_counter(TITANIUM_UPDATE_COUNTER)
    };
    let sent = |motion: &mut MotionSession, at: Instant, x: f32| {
        let mut counter = None;
        motion
            .send_move_sample(&request(at, x), at, |sample| {
                counter = Some(sample.sequence);
                Ok(())
            })
            .unwrap();
        counter
    };
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut motion = titanium(start);
    // 300 ms is 19 frames at 65 a second; no update has gone out yet.
    assert_eq!(sent(&mut motion, at(300), 0.5), Some(19));
    // 600 ms is 39 frames, and the first update recorded one entry.
    assert_eq!(sent(&mut motion, at(600), 1.0), Some(40));
    // A held update takes the counter as it goes out, not as it was held.
    assert_eq!(sent(&mut motion, at(700), 1.2), None);
    let mut ticked = None;
    assert!(motion
        .tick_sample(at(900), |sample| {
            ticked = Some(sample.sequence);
            Ok(())
        })
        .unwrap());
    assert_eq!(ticked, Some(58 + 2));
    // A failed send records nothing.
    assert!(motion
        .send_move_sample(&request(at(1200), 1.4), at(1200), |_| {
            anyhow::bail!("synthetic transport failure")
        })
        .is_err());
    assert_eq!(sent(&mut motion, at(1200), 1.4), Some(78 + 3));

    // A new admission starts again from 0.
    let readmitted = at(5000);
    let mut motion = titanium(readmitted);
    assert_eq!(
        sent(&mut motion, readmitted + Duration::from_millis(300), 0.5),
        Some(19)
    );
}

#[test]
fn titanium_counter_records_the_pad_region_and_its_warp() {
    use crate::{movement::TITANIUM_UPDATE_COUNTER, zoning::LocalPad};
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut motion = setup(start)
        .with_gate(Some(UpdateGate::Changed {
            spacing: Duration::from_millis(280),
        }))
        .with_counter(TITANIUM_UPDATE_COUNTER);
    motion
        .send_move_sample(&request(at(300), 0.5), at(300), |_| Ok(()))
        .unwrap();
    let entry = motion.position();
    let landing = Position {
        x: -20.0,
        z: 6.0,
        ..entry
    };
    let mut reported = None;
    motion
        .cross_pad(LocalPad::new(entry, landing), at(400), at(400), |sample| {
            reported = Some(sample.sequence);
            Ok(())
        })
        .unwrap();
    // 26 frames, the first update's entry, the region's two and the warp's
    // entry at the old place.
    assert_eq!(reported, Some(26 + 1 + 2 + 1));
    let mut landed = None;
    assert!(motion
        .tick_sample(at(700), |sample| {
            assert_eq!(sample.position, landing);
            landed = Some(sample.sequence);
            Ok(())
        })
        .unwrap());
    // 45 frames, and the entry report's own entry and the landing's.
    assert_eq!(landed, Some(45 + 4 + 2));
}

fn titanium_flagged(start: Instant) -> MotionSession {
    setup(start).with_gate(Some(UpdateGate::Flagged {
        pack: |sample| sample.packet(),
        rule: crate::movement::TITANIUM_FLAGS,
    }))
}

/// As [`passes`], with the host's request built by `make` at each step.
fn titanium_passes(
    session: &mut MotionSession,
    start: Instant,
    steps: std::ops::RangeInclusive<u64>,
    make: impl Fn(u64, Instant) -> Option<MovementRequest>,
) -> Vec<(u64, u16)> {
    let mut sent = Vec::new();
    for step in steps {
        let now = start + Duration::from_millis(step * 100);
        if let Some(request) = make(step, now) {
            session
                .send_move_sample(&request, now, |sample| {
                    sent.push((step * 100, sample.sequence));
                    Ok(())
                })
                .unwrap();
        }
        let mut ticked = None;
        if session
            .tick_sample(now, |sample| {
                sample.packet()?;
                ticked = Some(sample.sequence);
                Ok(())
            })
            .unwrap()
        {
            sent.push((step * 100, ticked.unwrap()));
        }
    }
    sent
}

fn times(sent: &[(u64, u16)]) -> Vec<u64> {
    sent.iter().map(|(at, _)| *at).collect()
}

#[test]
fn titanium_flag_sends_a_steady_run_once_a_second_and_a_turn_more_often() {
    let start = Instant::now();
    let mut session = titanium_flagged(start);
    // Running straight at 5 units a second: the start's speed change goes
    // out more than 280 ms after admission, then only the 1 s tick.
    #[allow(clippy::cast_precision_loss)]
    let sent = titanium_passes(&mut session, start, 1..=25, |step, now| {
        Some(request(now, step as f32 * 0.5))
    });
    assert_eq!(times(&sent), [300, 1200, 2300]);

    // Turning 4 units a step: each turn of 2 or more marks one due, and it
    // goes out more than 280 ms after the last.
    let mut session = titanium_flagged(start);
    #[allow(clippy::cast_precision_loss)]
    let sent = titanium_passes(&mut session, start, 1..=12, |step, now| {
        let mut turn = request(now, 0.0);
        turn.position.heading = step as f32 * 4.0;
        Some(turn)
    });
    assert_eq!(times(&sent), [300, 600, 900, 1200]);
}

#[test]
fn titanium_a_stall_mid_run_sends_no_stop_and_the_run_goes_on_by_the_tick() {
    let start = Instant::now();
    let mut session = titanium_flagged(start);
    let mut sent = Vec::new();
    let mut x = 0.0;
    // Running 0.5 a step, with no input from 900 ms to 1300 ms: the host
    // stalled, as a locked desktop stalls a client's frames, and goes on
    // from where it was.
    for step in 1..=25_u64 {
        let now = start + Duration::from_millis(step * 100);
        if !(9..=13).contains(&step) {
            x += 0.5;
            let moved = request(now, x);
            session
                .send_move_sample(&moved, now, |sample| {
                    sent.push((step * 100, sample.animation));
                    Ok(())
                })
                .unwrap();
        }
        session
            .tick_sample(now, |sample| {
                sent.push((step * 100, sample.animation));
                Ok(())
            })
            .unwrap();
    }
    // No still went out: the tick in the gap sends the last move held, and
    // the run goes on by the 1 s tick, not by a speed change from a stop.
    assert_eq!(sent, [(300, 12), (1200, 12), (2300, 12)]);
    assert!(session.stop_pending());
}

#[test]
fn titanium_flag_heading_wraps_and_drift_adds_up_from_the_last_flagged_value() {
    use crate::movement::{gate::Flags, TITANIUM_FLAGS};
    let start = Instant::now();
    let sample = |heading: f32| PositionPacket {
        spawn_id: 7,
        sequence: 0,
        position: Position {
            heading,
            ..Position::default()
        },
        delta: [0.0; 3],
        animation: 0,
        delta_heading: 0,
    };
    let mut flags = Flags::default();
    // The first observation is the tick's; 0 to 510 is a turn of 2 back
    // across the wrap, not 510.
    flags.observe(&sample(510.0), start, &TITANIUM_FLAGS, false);
    assert!(flags.release_frozen(Duration::from_secs(1), Duration::ZERO));
    flags.observe(&sample(511.0), start, &TITANIUM_FLAGS, false);
    assert!(!flags.release_frozen(Duration::from_secs(1), Duration::ZERO));
    // 510 to 0 is a turn of 2 forward across the wrap.
    flags.observe(&sample(0.0), start, &TITANIUM_FLAGS, false);
    assert!(flags.release_frozen(Duration::from_secs(1), Duration::ZERO));
    // Three drifts of 0.75 add up from the last flagged heading.
    for (heading, due) in [(0.75, false), (1.5, false), (2.25, true)] {
        flags.observe(&sample(heading), start, &TITANIUM_FLAGS, false);
        assert_eq!(
            flags.release_frozen(Duration::from_secs(1), Duration::ZERO),
            due
        );
    }
    // The speed is tracked apart: 8 in the field is 0.2 of the speed.
    for (animation, due) in [(7, false), (8, true), (15, false), (16, true)] {
        let moving = PositionPacket {
            animation,
            ..sample(2.25)
        };
        flags.observe(&moving, start, &TITANIUM_FLAGS, false);
        assert_eq!(
            flags.release_frozen(Duration::from_secs(1), Duration::ZERO),
            due
        );
    }
}

#[test]
fn titanium_flag_sends_strafing_and_a_jump_more_often_than_the_tick() {
    let start = Instant::now();
    let mut session = MotionSession::new(11, 7, Position::default(), start)
        .unwrap()
        .with_gate(Some(UpdateGate::Flagged {
            pack: |sample| sample.packet(),
            rule: crate::movement::TITANIUM_FLAGS,
        }));
    session
        .calibrate(
            MotionCalibration {
                swim_backward_units_per_second: None,
                diagonal: DiagonalRates::default(),
                walk: None,
                strafe: Some(StrafeCalibration {
                    units_per_second: 6.0,
                    velocity_scale: 0.05,
                    animation: 12,
                    walk_units_per_second: None,
                }),
                backward: None,
                units_per_second: 6.0,
                velocity_scale: 0.05,
                animation: 12,
            },
            start,
        )
        .unwrap();
    // A strafe at a steady speed: every sample marks one due.
    #[allow(clippy::cast_precision_loss)]
    let sent = titanium_passes(&mut session, start, 1..=12, |step, now| {
        Some(MovementRequest::new(
            MovementMode::Strafe,
            11,
            Position {
                y: step as f32 * 0.5,
                ..Position::default()
            },
            now,
        ))
    });
    assert_eq!(times(&sent), [300, 600, 900, 1200]);

    // A steady run, then a jump at 1500 ms: it goes out at once.
    let mut session = titanium_flagged(start).with_jump(Some((44.55, 400.0)));
    #[allow(clippy::cast_precision_loss)]
    let sent = titanium_passes(&mut session, start, 1..=16, |step, now| {
        Some(request(now, step as f32 * 0.5))
    });
    assert_eq!(times(&sent), [300, 1200]);
    session.jumped();
    let now = start + Duration::from_millis(1700);
    let mut jumped = false;
    session
        .send_move_sample(&request(now, 8.5), now, |_| {
            jumped = true;
            Ok(())
        })
        .unwrap();
    assert!(jumped);
}

#[test]
fn titanium_frozen_sends_from_the_tick_with_the_last_counter_and_a_stun_sends_nothing() {
    use crate::movement::{Hold, TITANIUM_UPDATE_COUNTER};
    let start = Instant::now();
    let mut session = titanium_flagged(start).with_counter(TITANIUM_UPDATE_COUNTER);
    // One ordinary update first, at 300 ms: 19 frames.
    let sent = titanium_passes(&mut session, start, 1..=3, |step, now| {
        (step == 1).then(|| request(now, 0.5))
    });
    assert_eq!(sent, [(300, 19)]);
    session.set_hold(Hold::Frozen);
    let sent = titanium_passes(&mut session, start, 4..=40, |_, _| None);
    // Only the tick marks one due, more than 560 ms after the last update,
    // and each repeats the last counter written without recording one.
    assert_eq!(sent, [(1200, 19), (2300, 19), (3400, 19)]);
    // Unfrozen, the frames counted all along: 4,500 ms is 292 frames, and
    // the frozen updates recorded nothing.
    session.set_hold(Hold::None);
    let sent = titanium_passes(&mut session, start, 45..=45, |_, now| {
        Some(request(now, 1.0))
    });
    assert_eq!(sent, [(4500, 292 + 1)]);

    // Stunned and not frozen: nothing goes out, not even the tick's.
    session.set_hold(Hold::Stunned);
    assert_eq!(
        titanium_passes(&mut session, start, 46..=80, |_, _| None),
        []
    );
}

#[test]
fn titanium_backs_up_in_water_at_its_rate_and_on_land_at_once_again() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut session = MotionSession::new(11, 7, Position::default(), start)
        .unwrap()
        .with_swimming(true);
    session
        .calibrate(
            MotionCalibration {
                diagonal: DiagonalRates::default(),
                walk: None,
                strafe: None,
                backward: Some(BackwardCalibration {
                    units_per_second: 6.0,
                    velocity_scale: 0.05,
                    animation: -12,
                }),
                units_per_second: 6.0,
                velocity_scale: 0.05,
                animation: 12,
                // Unbuffed: no backing up in water.
                swim_backward_units_per_second: Some(0.0),
            },
            start,
        )
        .unwrap();
    let to = |mode, y: f32, ms| {
        MovementRequest::new(
            mode,
            11,
            Position {
                y,
                ..Position::default()
            },
            at(ms),
        )
    };
    // Heading 0 faces +y. Swimming forward is fine; backing into the water
    // from land takes the water's rate at once.
    session
        .send_move_sample(&to(MovementMode::Swim, 0.05, 100), at(100), |_| Ok(()))
        .unwrap();
    let refused = session
        .send_move_sample(&to(MovementMode::Swim, 0.0, 200), at(200), |_| {
            panic!("backing up in water without a buff")
        })
        .unwrap_err();
    assert!(
        refused.to_string().contains("backing up in water"),
        "{refused}"
    );
    // A swim stroke that only sinks or rises still goes.
    let mut sink = to(MovementMode::Swim, 0.05, 200);
    sink.position.z = -0.05;
    session
        .send_move_sample(&sink, at(200), |_| Ok(()))
        .unwrap();
    // Backing out of the water onto land resumes the land rate at once.
    let mut ashore = to(MovementMode::Backward, -0.2, 300);
    ashore.position.z = -0.05;
    session
        .send_move_sample(&ashore, at(300), |_| Ok(()))
        .unwrap();
    // And back into the water, the water's rate again.
    let mut wet = to(MovementMode::Swim, -0.25, 400);
    wet.position.z = -0.05;
    let refused = session
        .send_move_sample(&wet, at(400), |_| panic!("backing into water"))
        .unwrap_err();
    assert!(
        refused.to_string().contains("backing up in water"),
        "{refused}"
    );
    // With a buff's rate, backing up in water goes, within the water's share.
    let mut buffed = session.calibration().unwrap();
    buffed.swim_backward_units_per_second = Some(6.0);
    session.calibrate(buffed, at(400)).unwrap();
    let share = super::super::swim_speed(6.0) * 0.1;
    let mut back = to(MovementMode::Swim, -0.2 - share, 500);
    back.position.z = -0.05;
    session
        .send_move_sample(&back, at(500), |_| Ok(()))
        .unwrap();
}

fn slower(units_per_second: f32) -> MotionCalibration {
    MotionCalibration {
        swim_backward_units_per_second: None,
        diagonal: DiagonalRates::default(),
        walk: None,
        strafe: None,
        backward: None,
        units_per_second,
        velocity_scale: 0.05,
        animation: 12,
    }
}

fn ms(start: Instant, millis: u64) -> Instant {
    start + Duration::from_millis(millis)
}

fn sends(session: &mut MotionSession, created: Instant, x: f32) -> bool {
    session
        .send_move(&request(created, x), created, |_| Ok(()))
        .is_ok()
}

#[test]
fn a_speed_change_in_place_bounds_each_span_at_its_own_speed() {
    // Run 2026-11-11 rows 4 and 5: each change of speed reset the guard's
    // stamps, so the sample under way was refused and the player held.
    let start = Instant::now();
    let mut session = setup(start);
    assert!(sends(&mut session, ms(start, 100), 0.6));
    session.calibrate(slower(3.0), ms(start, 150)).unwrap();
    // 50 ms at 6 and 50 ms at 3: 0.45, never the 0.6 the old speed allows.
    assert!(!sends(&mut session, ms(start, 200), 0.6 + 0.47));
    assert!(sends(&mut session, ms(start, 200), 0.6 + 0.44));
    // The change is spent: the next sample runs at 3 alone.
    assert!(!sends(&mut session, ms(start, 300), 1.04 + 0.32));
    assert!(sends(&mut session, ms(start, 300), 1.04 + 0.29));
}

#[test]
fn a_sample_made_before_a_change_is_neither_stale_nor_too_frequent() {
    let start = Instant::now();
    let mut session = setup(start);
    assert!(sends(&mut session, ms(start, 100), 0.6));
    session.calibrate(slower(3.0), ms(start, 170)).unwrap();
    // Made before the change and taken after it, it earns 60 ms at 6 alone.
    let made = ms(start, 160);
    assert!(session
        .send_move(&request(made, 0.6 + 0.38), ms(start, 180), |_| Ok(()))
        .is_err());
    session
        .send_move(&request(made, 0.6 + 0.35), ms(start, 180), |_| Ok(()))
        .unwrap();
    // The next earns 10 ms at 6 and 60 at 3: 0.24. Made 60 ms after the
    // change, it is not too frequent.
    assert!(!sends(&mut session, ms(start, 230), 0.95 + 0.26));
    assert!(sends(&mut session, ms(start, 230), 0.95 + 0.23));
}

#[test]
fn two_changes_in_one_interval_each_count_for_their_own_span() {
    let start = Instant::now();
    let mut session = setup(start);
    assert!(sends(&mut session, ms(start, 100), 0.6));
    session.calibrate(slower(3.0), ms(start, 130)).unwrap();
    session.calibrate(slower(12.0), ms(start, 160)).unwrap();
    // 30 ms at 6, 30 at 3 and 40 at 12: 0.18 + 0.09 + 0.48 = 0.75.
    assert!(!sends(&mut session, ms(start, 200), 0.6 + 0.77));
    assert!(sends(&mut session, ms(start, 200), 0.6 + 0.74));
}

#[test]
fn a_root_earns_no_distance_but_takes_turns_and_keeps_what_came_before() {
    let start = Instant::now();
    let mut session = setup(start);
    assert!(sends(&mut session, ms(start, 100), 0.6));
    session.root(ms(start, 150)).unwrap();
    assert!(session.rooted());
    assert!(session.calibration().is_none());
    // The 50 ms before the root earn 0.3; the time rooted earns nothing.
    assert!(!sends(&mut session, ms(start, 200), 0.6 + 0.32));
    assert!(sends(&mut session, ms(start, 200), 0.6 + 0.29));
    // Rooted: a step is refused, a turn in place goes out at no speed.
    assert!(!sends(&mut session, ms(start, 300), 0.89 + 0.02));
    let mut turn = request(ms(start, 300), 0.89);
    turn.position.heading = 24.0;
    session
        .send_move(&turn, ms(start, 300), |packet| {
            let body = &packet.body[..];
            assert_eq!(word(body, 20) & 0x3ff, 0, "no animation rooted");
            assert_ne!((word(body, 20) >> 10) & 0x3ff, 0, "the turn rate goes out");
            Ok(())
        })
        .unwrap();
    assert_eq!(session.position().heading, 24.0);
    // No jump while rooted, unless the generation's program allows one.
    assert!(session.check_jump().is_err());
}

#[test]
fn time_rooted_earns_nothing_when_the_root_ends_in_the_same_interval() {
    let start = Instant::now();
    let mut session = setup(start);
    assert!(sends(&mut session, ms(start, 100), 0.6));
    session.root(ms(start, 120)).unwrap();
    session.calibrate(slower(6.0), ms(start, 200)).unwrap();
    assert!(!session.rooted());
    // 20 ms at 6, 80 rooted, 50 at 6: 0.12 + 0.30 = 0.42.
    assert!(!sends(&mut session, ms(start, 250), 0.6 + 0.44));
    assert!(sends(&mut session, ms(start, 250), 0.6 + 0.41));
}

#[test]
fn a_withdrawal_or_correction_ends_a_root_and_resets_the_stamps() {
    let start = Instant::now();
    let mut session = setup(start).with_falls(true).with_rooted_jumps(true);
    session.root(ms(start, 50)).unwrap();
    session.check_jump().unwrap();
    session.withdraw();
    assert!(!session.rooted());
    assert!(session.check_jump().is_err());
    session.root(ms(start, 100)).unwrap();
    session
        .correct(Position::default(), ms(start, 150))
        .unwrap();
    assert!(!session.rooted());
    // A grant after the reset starts afresh from it.
    session.calibrate(slower(6.0), ms(start, 200)).unwrap();
    assert_eq!(session.guard.sampled, ms(start, 200));
}
