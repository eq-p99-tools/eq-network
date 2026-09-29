use super::*;

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
            units_per_second: 6.0,
            velocity_scale: 0.05,
            animation: 12,
            backward: None,
            walk: None,
            strafe: Some(StrafeCalibration {
                units_per_second: 3.0,
                velocity_scale: 0.1,
                animation: 8,
            }),
        };
        session.calibrate(calibration, start).unwrap();
        motion.position.x = direction * 0.6;
        assert!(session
            .send_move(&motion, now, |_| panic!("forward budget used for strafe"))
            .is_err());
        motion.position.x = direction * 0.3;
        session
            .send_move(&motion, now, |body| {
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
        .send_move(&motion, now, |body| {
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
        .send_move(&motion, now, |body| {
            assert_eq!(word(body, 20) & 0x3ff, 1016); // signed -8
            assert!((f32::from_bits(word(body, 12)) + 0.2).abs() < 0.00001);
            assert_eq!(u16::from_le_bytes(body[2..4].try_into().unwrap()), 0);
            Ok(())
        })
        .unwrap();
    let now = now + Duration::from_millis(100);
    let forward = request(now, 0.4);
    session
        .send_move(&forward, now, |body| {
            assert_eq!(word(body, 20) & 0x3ff, 12);
            Ok(())
        })
        .unwrap();
    for animation in [0, 1, -513] {
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
fn request(created: Instant, x: f32) -> MovementRequest {
    MovementRequest {
        mode: MovementMode::Forward,
        session_id: 11,
        position: Position {
            x,
            ..Position::default()
        },
        created,
    }
}
fn word(body: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(body[offset..offset + 4].try_into().unwrap())
}
#[test]
fn stationary_turns_encode_rate_and_expire_to_a_zero_rate_stop() {
    let start = Instant::now();
    let mut session = setup(start);
    let now = start + Duration::from_millis(100);
    let mut motion = request(now, 0.0);
    motion.position.heading = 488.0;
    session
        .send_move(&motion, now, |body| {
            assert_eq!(word(body, 20) & 0x3ff, 0);
            assert_eq!((word(body, 20) >> 10) & 0x3ff, 784);
            assert_eq!(word(body, 12), 0);
            assert_eq!(word(body, 16), 0);
            Ok(())
        })
        .unwrap();
    assert!(session
        .tick(now + Duration::from_millis(250), |body| {
            assert_eq!((word(body, 20) >> 10) & 0x3ff, 0);
            Ok(())
        })
        .unwrap());
    let before = session.position();
    let mut snap = request(now + Duration::from_millis(350), 0.0);
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
    let mut session = setup(start).with_falls(true);
    session
        .send_move(&fall, now, |body| {
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
        .send_move(&motion, now, |body| {
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
        .send_move(&motion, later, |body| {
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
        .send_move(&motion, now, |body| {
            assert_eq!(&body[..4], &[7, 0, 0, 0]);
            assert!((f32::from_bits(word(body, 12)) - 0.25).abs() < 0.0001);
            assert_eq!(word(body, 20) & 0x3ff, 12);
            Ok(())
        })
        .unwrap();
}
#[test]
fn input_expiry_sends_one_stop_then_stationary_heartbeats() {
    let start = Instant::now();
    let mut session = setup(start);
    let moving_at = start + Duration::from_millis(100);
    session
        .send_move(&request(moving_at, 0.5), moving_at, |_| Ok(()))
        .unwrap();
    assert!(!session
        .tick(start + Duration::from_millis(349), |_| panic!("early stop"))
        .unwrap());
    assert!(session
        .tick(start + Duration::from_millis(350), |body| {
            assert_eq!(&body[2..4], &[1, 0]);
            assert_eq!(word(body, 24), 0.5f32.to_bits());
            assert_eq!(word(body, 20) & 0x3ff, 0);
            assert_eq!(&body[8..20], &[0; 12]);
            Ok(())
        })
        .unwrap());
    assert!(!session
        .tick(start + Duration::from_millis(600), |_| panic!(
            "duplicate stop"
        ))
        .unwrap());
    assert!(session
        .tick(start + Duration::from_millis(2350), |body| {
            assert_eq!(&body[2..4], &[2, 0]);
            Ok(())
        })
        .unwrap());
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
    session.sequence = u16::MAX;
    session
        .send_move(&request(now, 0.5), now, |_| Ok(()))
        .unwrap();
    assert_eq!(session.sequence, 0);
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
        .tick(start + Duration::from_millis(350), |body| {
            assert_eq!(word(body, 24), 0.25f32.to_bits());
            assert_eq!(word(body, 20) & 0x3ff, 0);
            assert_eq!(&body[8..20], &[0; 12]);
            Ok(())
        })
        .unwrap());
    assert!(session.calibration.is_none());
}

#[test]
fn queued_calibration_requires_fresh_valid_data_without_mutating_on_failure() {
    let start = Instant::now();
    let mut session = setup(start);
    let calibration = session.calibration.unwrap();
    let now = start + Duration::from_millis(300);
    assert!(session.calibrate_fresh(calibration, start, now).is_err());
    assert!(session
        .calibrate_fresh(calibration, now + Duration::from_millis(1), now)
        .is_err());
    for invalid in [
        MotionCalibration {
            walk: None,
            strafe: None,
            backward: None,
            units_per_second: f32::NAN,
            ..calibration
        },
        MotionCalibration {
            velocity_scale: 0.0,
            ..calibration
        },
        MotionCalibration {
            animation: 512,
            ..calibration
        },
    ] {
        assert!(session.calibrate_fresh(invalid, now, now).is_err());
    }
    assert_eq!(session.calibration, Some(calibration));
    assert_eq!(session.last_sample, start);
    session.calibrate_fresh(calibration, now, now).unwrap();
    assert_eq!(session.last_sample, now);
}
