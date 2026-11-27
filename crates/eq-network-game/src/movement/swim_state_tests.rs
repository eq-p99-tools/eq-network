//! Synthetic reset-matched controls; no client, socket or installed data.
use super::*;
use crate::movement::{
    water::*, Pace, SpeedState, StrafeSide, SwimControl, SwimControls, SwimInput,
};

fn calibration(rate: i16) -> MotionCalibration {
    super::super::titanium_speeds(&SpeedState {
        run: 0.7,
        walk: 0.46,
        level: 50,
        hit_points: Some((500, 500)),
        rate,
        weight: 0,
        capacity: 100,
        sneaking: false,
        ducking: false,
        zone_id: 1,
        swimming_skill: Some(100),
    })
    .motion
    .unwrap()
}

fn owner(start: Instant, rate: i16) -> MotionSession {
    let mut session = MotionSession::new(11, 7, Position::default(), start)
        .unwrap()
        .with_swimming(true)
        .with_heading_snapshots(true)
        .with_falls(true);
    session.calibrate(calibration(rate), start).unwrap();
    session
}

fn request(at: Position, moved: [f32; 3], heading: f32, now: Instant) -> MovementRequest {
    let mut request = MovementRequest::new(
        MovementMode::Swim,
        11,
        Position {
            x: at.x + moved[0],
            y: at.y + moved[1],
            z: at.z + moved[2],
            heading,
        },
        now,
    );
    request.heading = HeadingUpdate::Snapshot;
    request
}

fn piece(ms: u64, heading: f32, input: SwimInput) -> SwimControl {
    SwimControl::new(
        Duration::from_millis(ms),
        heading,
        WetKind::Normal,
        input,
        1.,
    )
    .unwrap()
}

// Independent oracle uses the actual recurrence, not the validator's input builder.
fn reference(
    velocity: &mut WetVelocity,
    calibrated: MotionCalibration,
    heading: f32,
    input: SwimInput,
    kind: WetKind,
    seconds: f32,
) -> [f32; 3] {
    let swim = calibrated.swim.unwrap();
    let forward = |pace| match pace {
        Pace::Run => swim.forward,
        Pace::Walk => swim.walk,
    };
    let sign = |side| match side {
        StrafeSide::Left => 1.,
        StrafeSide::Right => -1.,
    };
    let (side, along) = match input {
        SwimInput::Idle => (0., 0.),
        SwimInput::Forward(pace) => (0., forward(pace)),
        SwimInput::Backward => (0., -swim.backward),
        SwimInput::Strafe(side, pace) => {
            let strafe = calibrated.strafe.unwrap();
            let scale = match pace {
                Pace::Run => 1.,
                Pace::Walk => {
                    strafe
                        .walk_units_per_second
                        .unwrap_or(strafe.units_per_second)
                        / strafe.units_per_second
                }
            };
            (sign(side) * swim.strafe * scale, 0.)
        }
        SwimInput::ForwardStrafe(side, pace) => {
            let r = forward(pace) / 2_f32.sqrt();
            (sign(side) * r, r)
        }
    };
    let (sin, cos) = (heading / 512. * std::f32::consts::TAU).sin_cos();
    velocity.clamp(swim.caps());
    let (flat, up) = velocity.advance(
        swim.set_for(kind),
        WetInput {
            horizontal: [sin * along + cos * side, cos * along - sin * side],
            vertical: 0.,
            jump: false,
        },
        seconds,
    );
    [flat[0], flat[1], up]
}

fn advance(
    session: &mut MotionSession,
    velocity: &mut WetVelocity,
    frames: &[(Duration, f32, SwimInput, WetKind)],
) -> (MovementRequest, SwimControls) {
    let basis = session.swim_basis().unwrap();
    let mut moved = [0.; 3];
    let mut previous = Duration::ZERO;
    let mut controls = Vec::new();
    for &(until, heading, input, kind) in frames {
        let delta = reference(
            velocity,
            session.calibration().unwrap(),
            heading,
            input,
            kind,
            (until - previous).as_secs_f32(),
        );
        for (total, delta) in moved.iter_mut().zip(delta) {
            *total += delta;
        }
        controls.push(SwimControl::new(until, heading, kind, input, 1.).unwrap());
        previous = until;
    }
    let now = basis.started_at() + previous;
    let request = request(session.position(), moved, frames.last().unwrap().1, now);
    let controls = SwimControls::new(basis, controls).unwrap();
    session
        .send_swim_sample(&request, Some(&controls), now, |_| Ok(()))
        .unwrap();
    assert_eq!(session.position(), request.position);
    assert!(session.swim_basis().is_some());
    (request, controls)
}

#[test]
fn actual_left_and_right_turn_settle_reverse_controls_keep_earned_drift() {
    for side in [StrafeSide::Left, StrafeSide::Right] {
        let start = Instant::now();
        let mut session = owner(start, 0);
        let mut velocity = WetVelocity::default();
        for step in 1..=30_u16 {
            let heading = match step {
                1..=5 => 0.,
                6..=15 => f32::from(step - 5) * 12.,
                16..=20 => 120.,
                _ => (120. - f32::from(step - 20) * 16.).rem_euclid(512.),
            };
            advance(
                &mut session,
                &mut velocity,
                &[(
                    Duration::from_millis(100),
                    heading,
                    SwimInput::Strafe(side, Pace::Run),
                    WetKind::Normal,
                )],
            );
        }
        assert!(session.position().x.hypot(session.position().y) > 0.1);
    }
}

#[test]
fn three_angular_conditions_use_observed_frames_and_report_path_separately_from_chord() {
    for milliseconds in [50_u64, 100, 250] {
        for degrees in [0_f32, 18., 97.56] {
            let start = Instant::now();
            let mut session = owner(start, 0);
            let mut velocity = WetVelocity::default();
            let mut traveled = 0.;
            let mut elapsed = 0_u64;
            while elapsed < 10_000 {
                let before = session.position();
                // Multiple real input observations inside each report interval.
                let mut frames = Vec::new();
                let mut at = 0_u64;
                while at < milliseconds {
                    at = (at + 10).min(milliseconds);
                    let seconds = Duration::from_millis(elapsed + at).as_secs_f32();
                    let heading = (degrees * seconds / 360. * 512.).rem_euclid(512.);
                    frames.push((
                        Duration::from_millis(at),
                        heading,
                        SwimInput::Strafe(StrafeSide::Left, Pace::Run),
                        WetKind::Normal,
                    ));
                }
                let (request, _) = advance(&mut session, &mut velocity, &frames);
                traveled += (request.position.x - before.x).hypot(request.position.y - before.y);
                elapsed += milliseconds;
            }
            let chord = session.position().x.hypot(session.position().y);
            assert!(traveled > 50. && chord <= traveled + 0.01);
            if degrees > 1. {
                assert!(traveled > chord + 1.);
            }
            eprintln!("synthetic angular_rate={degrees} sample_ms={milliseconds} accepted_path={traveled} endpoint_chord={chord}; not a live replay");
        }
    }
}

#[test]
fn partial_frames_and_mixed_normal_controls_match_the_real_recurrence() {
    for rate in [0, 50] {
        let start = Instant::now();
        let mut session = owner(start, rate);
        let mut velocity = WetVelocity::default();
        let inputs = [
            SwimInput::Idle,
            SwimInput::Forward(Pace::Run),
            SwimInput::Backward,
            SwimInput::Strafe(StrafeSide::Left, Pace::Walk),
            SwimInput::Strafe(StrafeSide::Right, Pace::Run),
            SwimInput::ForwardStrafe(StrafeSide::Right, Pace::Run),
            SwimInput::Forward(Pace::Walk),
        ];
        let mut random = 71_u32;
        for sample in 0..120_u16 {
            let mut frames = Vec::new();
            for time in [7_u64, 19, 38, 73, 100] {
                random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let index = usize::try_from(random % 7).unwrap();
                let heading = f32::from(u16::try_from(random % 512).unwrap());
                let kind = if sample % 2 == 0 {
                    WetKind::Normal
                } else {
                    WetKind::Slower
                };
                frames.push((Duration::from_millis(time), heading, inputs[index], kind));
            }
            advance(&mut session, &mut velocity, &frames);
        }
    }
}

#[test]
fn repeated_fixed_heading_backward_obeys_the_existing_zero_and_buffed_rates() {
    for rate in [0, 50] {
        let start = Instant::now();
        let mut session = owner(start, rate);
        let mut velocity = WetVelocity::default();
        let maximum = WetCaps::new(
            calibration(rate).swim.unwrap().set,
            calibration(rate).swim.unwrap().backward,
            0.,
        )
        .horizontal;
        for _ in 0..120 {
            let before = session.position();
            let (request, _) = advance(
                &mut session,
                &mut velocity,
                &[(
                    Duration::from_millis(100),
                    0.,
                    SwimInput::Backward,
                    WetKind::Normal,
                )],
            );
            assert!(before.y - request.position.y <= maximum * 0.1 + 0.01);
        }
        let basis = session.swim_basis().unwrap();
        let controls = SwimControls::new(basis, vec![piece(100, 0., SwimInput::Backward)]).unwrap();
        let mut bad = request(
            session.position(),
            [0., -maximum * 0.1 - 0.03, 0.],
            0.,
            basis.started_at() + Duration::from_millis(100),
        );
        // This exceeds only the old backward limit, not an invented total rate.
        bad.position.x = session.position().x;
        assert!(session
            .send_swim_sample(&bad, Some(&controls), bad.created, |_| panic!(
                "excess backward sent"
            ))
            .is_err());
    }
}

#[test]
fn earned_coast_is_integrated_without_an_endpoint_rectangle_or_ninety_frame_cutoff() {
    let mut session = owner(Instant::now(), 0);
    let mut velocity = WetVelocity::default();
    for _ in 0..20 {
        advance(
            &mut session,
            &mut velocity,
            &[(
                Duration::from_millis(100),
                0.,
                SwimInput::Strafe(StrafeSide::Left, Pace::Run),
                WetKind::Normal,
            )],
        );
    }
    for milliseconds in [50, 100, 250, 250, 250, 250, 250, 250] {
        advance(
            &mut session,
            &mut velocity,
            &[(
                Duration::from_millis(milliseconds),
                384.,
                SwimInput::Idle,
                WetKind::Normal,
            )],
        );
    }
    let basis = session.swim_basis().unwrap();
    let controls = SwimControls::new(basis, vec![piece(100, 384., SwimInput::Idle)]).unwrap();
    let cap = calibration(0).swim.unwrap().caps().horizontal;
    let bad = request(
        session.position(),
        [cap * 0.1, 0., 0.],
        384.,
        basis.started_at() + Duration::from_millis(100),
    );
    assert!(session
        .send_swim_sample(&bad, Some(&controls), bad.created, |_| panic!(
            "sustained reversed excess sent"
        ))
        .is_err());
}

#[test]
fn failed_send_does_not_commit_trial_and_stale_receipts_cannot_restore_it() {
    let start = Instant::now();
    let mut session = owner(start, 0);
    let mut velocity = WetVelocity::default();
    let moved = reference(
        &mut velocity,
        calibration(0),
        0.,
        SwimInput::Forward(Pace::Run),
        WetKind::Normal,
        0.1,
    );
    let basis = session.swim_basis().unwrap();
    let controls =
        SwimControls::new(basis, vec![piece(100, 0., SwimInput::Forward(Pace::Run))]).unwrap();
    let request = request(
        session.position(),
        moved,
        0.,
        start + Duration::from_millis(100),
    );
    let before = format!("{session:?}");
    assert!(session
        .send_swim_sample(
            &request,
            Some(&controls),
            request.created,
            |_| anyhow::bail!("mock failed send")
        )
        .is_err());
    assert_eq!(format!("{session:?}"), before);
    session
        .send_swim_sample(&request, Some(&controls), request.created, |_| Ok(()))
        .unwrap();
    assert_ne!(session.swim_basis(), Some(basis));
    assert!(session
        .send_swim_sample(&request, Some(&controls), request.created, |_| panic!(
            "stale move sent"
        ))
        .is_err());
}

#[test]
fn uncertainty_does_not_gain_directional_credit_even_inside_the_total_cap() {
    for heading in [12., 64., 128., 256.] {
        let start = Instant::now();
        let mut session = owner(start, 0);
        let radians = heading / 512. * std::f32::consts::TAU;
        let bad = request(
            Position::default(),
            [-0.2 * radians.sin(), -0.2 * radians.cos(), 0.],
            heading,
            start + Duration::from_millis(100),
        );
        assert!(session
            .send_swim_sample(&bad, None, bad.created, |_| panic!(
                "unknown backward motion sent"
            ))
            .is_err());
        assert!(session.swim_basis().is_none());
    }
}

#[test]
fn omitted_or_unexplained_controls_use_legacy_checks_and_never_seed_from_average() {
    let start = Instant::now();
    let mut session = owner(start, 0);
    let basis = session.swim_basis().unwrap();
    let controls = SwimControls::new(basis, vec![piece(100, 0., SwimInput::Idle)]).unwrap();
    let request = request(
        Position::default(),
        [0., 0.2, 0.],
        0.,
        start + Duration::from_millis(100),
    );
    // The old ordinary forward endpoint is legal, though this trace cannot explain it.
    session
        .send_swim_sample(&request, Some(&controls), request.created, |_| Ok(()))
        .unwrap();
    assert!(session.swim_basis().is_none());
    session.correct(request.position, request.created).unwrap();
    session.calibrate(calibration(0), request.created).unwrap();
    assert!(session.swim_basis().is_some());
    let next = request.created + Duration::from_millis(100);
    let idle = super::super::MovementRequest::new(MovementMode::Swim, 11, request.position, next);
    session
        .send_swim_sample(&idle, None, next, |_| Ok(()))
        .unwrap();
    assert!(session.swim_basis().is_none());
}

#[test]
fn real_correction_replaces_the_anchor_but_local_unknown_entry_cannot_reset_it() {
    let start = Instant::now();
    let mut session = owner(start, 0);
    let old = session.swim_basis().unwrap();
    let corrected = Position {
        x: 10.,
        ..Position::default()
    };
    let now = start + Duration::from_secs(1);
    session.correct(corrected, now).unwrap();
    assert!(session.swim_basis().is_none()); // calibration revoked
    session.calibrate(calibration(0), now).unwrap();
    assert_ne!(session.swim_basis(), Some(old));
    let controls = SwimControls::new(old, vec![piece(100, 0., SwimInput::Idle)]).unwrap();
    let bad = request(
        corrected,
        [0., -0.1, 0.],
        0.,
        now + Duration::from_millis(100),
    );
    assert!(session
        .send_swim_sample(&bad, Some(&controls), bad.created, |_| panic!(
            "pre-correction state reused"
        ))
        .is_err());
    session.withdraw_swim_controls();
    session.calibrate(calibration(0), now).unwrap();
    assert!(session.swim_basis().is_none());
}

#[test]
fn complete_span_is_required_and_no_input_precedes_its_grant() {
    let start = Instant::now();
    let mut session = MotionSession::new(11, 7, Position::default(), start)
        .unwrap()
        .with_swimming(true)
        .with_heading_snapshots(true);
    session
        .calibrate(calibration(0), start + Duration::from_millis(50))
        .unwrap();
    let basis = session.swim_basis().unwrap();
    let controls = SwimControls::new(
        basis,
        vec![
            piece(50, 0., SwimInput::Idle),
            piece(150, 0., SwimInput::Forward(Pace::Run)),
        ],
    )
    .unwrap();
    let mut velocity = WetVelocity::default();
    let mut moved = reference(
        &mut velocity,
        calibration(0),
        0.,
        SwimInput::Idle,
        WetKind::Normal,
        0.05,
    );
    let after = reference(
        &mut velocity,
        calibration(0),
        0.,
        SwimInput::Forward(Pace::Run),
        WetKind::Normal,
        0.1,
    );
    for (a, b) in moved.iter_mut().zip(after) {
        *a += b;
    }
    let request = request(
        Position::default(),
        moved,
        0.,
        start + Duration::from_millis(150),
    );
    session
        .send_swim_sample(&request, Some(&controls), request.created, |_| Ok(()))
        .unwrap();
    assert!(session.swim_basis().is_some());
    let basis = session.swim_basis().unwrap();
    let incomplete = SwimControls::new(basis, vec![piece(50, 128., SwimInput::Idle)]).unwrap();
    let bad = super::super::MovementRequest::new(
        MovementMode::Swim,
        11,
        Position {
            x: session.position().x - 0.1,
            heading: 128.,
            ..session.position()
        },
        request.created + Duration::from_millis(100),
    );
    assert!(session
        .send_swim_sample(&bad, Some(&incomplete), bad.created, |_| panic!(
            "incomplete span credited"
        ))
        .is_err());
}

#[test]
fn malformed_observations_have_no_phase_or_velocity_escape_hatch() {
    let session = owner(Instant::now(), 0);
    let basis = session.swim_basis().unwrap();
    for heading in [f32::NAN, f32::INFINITY, -1., 512.] {
        assert!(SwimControl::new(
            Duration::from_millis(100),
            heading,
            WetKind::Normal,
            SwimInput::Idle,
            1.
        )
        .is_err());
    }
    for share in [-0.01, 1.01, f32::NAN] {
        assert!(SwimControl::new(
            Duration::from_millis(100),
            0.,
            WetKind::Normal,
            SwimInput::Idle,
            share
        )
        .is_err());
    }
    for pieces in [
        vec![],
        vec![piece(0, 0., SwimInput::Idle)],
        vec![
            piece(100, 0., SwimInput::Idle),
            piece(100, 0., SwimInput::Idle),
        ],
        vec![piece(251, 0., SwimInput::Idle)],
        vec![piece(1, 0., SwimInput::Idle); 65],
    ] {
        assert!(SwimControls::new(basis, pieces).is_err());
    }
}

#[test]
fn calibration_modes_holds_and_explicit_withdrawal_destroy_continuity() {
    let start = Instant::now();
    for seam in 0..5 {
        let mut session = owner(start, 0);
        let mut velocity = WetVelocity::default();
        advance(
            &mut session,
            &mut velocity,
            &[(
                Duration::from_millis(100),
                0.,
                SwimInput::Forward(Pace::Run),
                WetKind::Normal,
            )],
        );
        let now = start + Duration::from_millis(100);
        match seam {
            0 => session.calibrate(calibration(50), now).unwrap(),
            1 => session.withdraw_swim_controls(),
            2 => session.set_hold(Hold::Stunned),
            3 => session.stop_voluntary(),
            _ => session.withdraw(),
        }
        assert!(session.swim_basis().is_none());
    }
}
