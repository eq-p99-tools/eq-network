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
        .send_move(&motion, now, |packet| {
            let body = &packet.body[..];
            assert_eq!(word(body, 20) & 0x3ff, 0);
            assert_eq!((word(body, 20) >> 10) & 0x3ff, 784);
            assert_eq!(word(body, 12), 0);
            assert_eq!(word(body, 16), 0);
            Ok(())
        })
        .unwrap();
    assert!(session
        .tick(now + Duration::from_millis(250), |packet| {
            let body = &packet.body[..];
            assert_eq!((word(body, 20) >> 10) & 0x3ff, 0);
            Ok(())
        })
        .unwrap());
    // The rate is cut toward zero: 23.99 over 250 ms, 95.96 a second, sends
    // 95.
    let later = now + Duration::from_millis(350);
    let mut slower = request(later, 0.0);
    slower.position.heading = 488.0 + 23.99;
    session
        .send_move(&slower, later, |packet| {
            assert_eq!((word(&packet.body, 20) >> 10) & 0x3ff, 95);
            Ok(())
        })
        .unwrap();
    let before = session.position();
    let mut snap = request(now + Duration::from_millis(450), 0.0);
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
        .tick(start + Duration::from_millis(350), |packet| {
            let body = &packet.body[..];
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
    assert!(!session
        .tick(start + Duration::from_millis(1349), |_| panic!(
            "early heartbeat"
        ))
        .unwrap());
    assert!(session
        .tick(start + Duration::from_millis(1350), |packet| {
            let body = &packet.body[..];
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
    assert_eq!(session.guard.sampled, start);
    session.calibrate_fresh(calibration, now, now).unwrap();
    assert_eq!(session.guard.sampled, now);
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
    // The stop waits for the spacing too, then heartbeats resume.
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
    // The stop is a speed change: due at once, out on the first pass more
    // than 150 ms after the last update.
    let stop = passes(&mut session, start, 26..=29, |_| None);
    assert_eq!(stop, [2800]);
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
