//! Controlled causal frames. Scripted geometry is synthetic, not live parity.
use super::*;
use crate::movement::{SwimScene, SwimSceneHandle, SwimSceneMotion};

#[derive(Clone)]
struct Bottom {
    slope: f32,
    blocked: bool,
    unknown: bool,
    unexpected_horizontal: bool,
}
impl SwimScene for Bottom {
    fn step_height(&self) -> f32 {
        3.75
    }
    fn floor(&self, at: [f32; 3]) -> anyhow::Result<Option<f32>> {
        anyhow::ensure!(!self.unknown, "unknown fixture floor");
        Ok(Some(-3.75 + self.slope * at[0]))
    }
    fn kind(&self, _: [f32; 3]) -> anyhow::Result<WetKind> {
        Ok(WetKind::Normal)
    }
    fn sweep(&self, from: [f32; 3], mut target: [f32; 3]) -> anyhow::Result<[f32; 3]> {
        if self.blocked {
            target[0] = from[0];
            target[1] = from[1];
        }
        if self.unexpected_horizontal {
            target[0] += 0.01;
        }
        Ok(target)
    }
    fn clear_lift(&self, _: [f32; 3], _: [f32; 3]) -> anyhow::Result<bool> {
        Ok(true)
    }
}
fn scene(slope: f32) -> SwimSceneHandle {
    SwimSceneHandle::new(Bottom {
        slope,
        blocked: false,
        unknown: false,
        unexpected_horizontal: false,
    })
}
fn native(cal: MotionCalibration, input: SwimInput, heading: f32, pitch: f32) -> WetInput {
    let swim = cal.swim.unwrap();
    let (sin, cos) = (heading / 512.0 * std::f32::consts::TAU).sin_cos();
    let (side, along) = match input {
        SwimInput::Idle => (0.0, 0.0),
        SwimInput::Backward => (0.0, -swim.backward),
        SwimInput::Forward(_) => (0.0, swim.forward),
        SwimInput::Strafe(StrafeSide::Left, _) => (swim.strafe, 0.0),
        SwimInput::Strafe(StrafeSide::Right, _) => (-swim.strafe, 0.0),
        SwimInput::ForwardStrafe(..) => unreachable!(),
    };
    WetInput {
        horizontal: [cos * side + sin * along, cos * along - sin * side],
        vertical: if matches!(input, SwimInput::Forward(_)) {
            swim.forward * pitch.sin()
        } else {
            0.0
        },
        jump: false,
    }
}
fn sample(
    session: &MotionSession,
    model: &mut SwimSceneMotion,
    input: SwimInput,
    turn: f32,
    pitch: f32,
    at: Instant,
) -> (MovementRequest, SwimControls) {
    let mut pieces = Vec::new();
    let mut until = Duration::ZERO;
    let calibration = calibration(0);
    let mut heading = session.position().heading;
    let mut observed = session.sampled();
    let physical_start = at
        .checked_sub(Duration::from_millis(100))
        .expect("fixture start");
    for millis in [16, 33, 50, 66, 83, 100] {
        heading = (heading + turn).rem_euclid(512.0);
        let ended = physical_start + Duration::from_millis(millis);
        let seconds = ended.duration_since(observed).as_secs_f32();
        observed = ended;
        until += Duration::from_secs_f32(seconds);
        let control = SwimControl::physical(
            until,
            heading,
            WetKind::Normal,
            input,
            seconds,
            pitch,
            ended,
        )
        .unwrap();
        *model = model
            .frame(
                calibration.swim.unwrap().set,
                native(calibration, input, heading, pitch),
                seconds,
            )
            .unwrap()
            .0;
        pieces.push(control);
    }
    let [x, y, z] = model.origin();
    let mut request =
        MovementRequest::new(MovementMode::Swim, 11, Position { x, y, z, heading }, at);
    request.heading = HeadingUpdate::Snapshot;
    let tape = SwimControls::from_physical_frames(
        session.swim_basis().unwrap(),
        pieces,
        heading,
        model.scene().clone(),
    )
    .unwrap();
    (request, tape)
}

#[test]
fn scene_frames_replay_both_strafe_and_turn_signs_with_pitch_controls() {
    for side in [StrafeSide::Left, StrafeSide::Right] {
        for turn in [-2.0_f32, 2.0] {
            for pitch in [-0.55_f32, 0.0, 0.55] {
                let start = Instant::now();
                let mut session = owner(start, 0);
                let mut model = SwimSceneMotion::at_reset(scene(0.05), [0.0; 3]).unwrap();
                let mut path = 0.0_f64;
                for tick in 1..=100 {
                    let before = session.position();
                    let (request, tape) = sample(
                        &session,
                        &mut model,
                        SwimInput::Strafe(side, Pace::Run),
                        turn,
                        pitch,
                        start + Duration::from_millis(tick * 100),
                    );
                    session.inspect_swim_replay(&request, &tape).unwrap();
                    session
                        .send_swim_sample(&request, Some(&tape), request.created, |_| Ok(()))
                        .unwrap();
                    path += f64::from(
                        (request.position.x - before.x).hypot(request.position.y - before.y),
                    );
                }
                let net = session.position().x.hypot(session.position().y);
                assert!(path > 40.0 && f64::from(net) <= path + 0.001);
            }
        }
    }
}

#[test]
fn scene_keeps_true_backward_zero_and_earned_release_carry() {
    let start = Instant::now();
    let mut session = owner(start, 0);
    let mut model = SwimSceneMotion::at_reset(scene(0.0), [0.0; 3]).unwrap();
    for tick in 1..=4 {
        let (request, tape) = sample(
            &session,
            &mut model,
            SwimInput::Backward,
            0.0,
            0.0,
            start + Duration::from_millis(tick * 100),
        );
        session
            .send_swim_sample(&request, Some(&tape), request.created, |_| Ok(()))
            .unwrap();
        assert_eq!([session.position().x, session.position().y], [0.0; 2]);
    }
    let (request, tape) = sample(
        &session,
        &mut model,
        SwimInput::Forward(Pace::Run),
        0.0,
        0.0,
        start + Duration::from_millis(500),
    );
    session
        .send_swim_sample(&request, Some(&tape), request.created, |_| Ok(()))
        .unwrap();
    let before = session.position();
    let (request, tape) = sample(
        &session,
        &mut model,
        SwimInput::Backward,
        0.0,
        0.0,
        start + Duration::from_millis(600),
    );
    session
        .send_swim_sample(&request, Some(&tape), request.created, |_| Ok(()))
        .unwrap();
    assert!(
        session.position().y > before.y,
        "earned forward carry is retained"
    );
    for tick in 7..=100 {
        let (request, tape) = sample(
            &session,
            &mut model,
            SwimInput::Backward,
            0.0,
            0.0,
            start + Duration::from_millis(tick * 100),
        );
        session
            .send_swim_sample(&request, Some(&tape), request.created, |_| Ok(()))
            .unwrap();
    }
    assert!(model
        .velocity()
        .units_per_second()
        .0
        .into_iter()
        .all(|v| v.abs() < 0.001));
}

#[test]
fn changed_scene_and_correction_cannot_manufacture_an_anchor() {
    let start = Instant::now();
    let mut session = owner(start, 0);
    let mut model = SwimSceneMotion::at_reset(scene(0.0), [0.0; 3]).unwrap();
    let (request, tape) = sample(
        &session,
        &mut model,
        SwimInput::Idle,
        0.0,
        0.0,
        start + Duration::from_millis(100),
    );
    session
        .send_swim_sample(&request, Some(&tape), request.created, |_| Ok(()))
        .unwrap();
    let mut replaced = SwimSceneMotion::at_reset(scene(0.0), model.origin()).unwrap();
    let (changed, tape) = sample(
        &session,
        &mut replaced,
        SwimInput::Idle,
        0.0,
        0.0,
        start + Duration::from_millis(200),
    );
    assert!(session.inspect_swim_replay(&changed, &tape).is_err());
    session
        .correct(session.position(), start + Duration::from_millis(250))
        .unwrap();
    let reset = session.swim_basis();
    assert!(session
        .send_swim_sample(
            &changed,
            Some(&tape),
            start + Duration::from_millis(250),
            |_| panic!("stale scene submitted")
        )
        .is_err());
    assert_eq!(session.swim_basis(), reset);
}

#[test]
fn unknown_floor_and_unearned_horizontal_results_are_not_replay_credit() {
    let unknown = SwimSceneHandle::new(Bottom {
        slope: 0.0,
        blocked: false,
        unknown: true,
        unexpected_horizontal: false,
    });
    assert!(SwimSceneMotion::at_reset(unknown, [0.0; 3]).is_err());
    let unsupported = SwimSceneHandle::new(Bottom {
        slope: 0.0,
        blocked: false,
        unknown: false,
        unexpected_horizontal: true,
    });
    let model = SwimSceneMotion::at_reset(unsupported, [0.0; 3]).unwrap();
    assert!(model.frame(LIQUID_5, WetInput::default(), 0.016).is_err());
    assert_eq!(model.origin(), [0.0; 3]);
}

#[test]
fn refused_send_does_not_commit_scene_trial_or_reset() {
    let start = Instant::now();
    let mut session = owner(start, 0);
    let mut model = SwimSceneMotion::at_reset(scene(0.0), [0.0; 3]).unwrap();
    let reset = session.swim_basis();
    let (request, tape) = sample(
        &session,
        &mut model,
        SwimInput::Forward(Pace::Run),
        0.0,
        0.0,
        start + Duration::from_millis(100),
    );
    assert!(session
        .send_swim_sample(&request, Some(&tape), request.created, |_| anyhow::bail!(
            "failed send"
        ))
        .is_err());
    assert_eq!(session.swim_basis(), reset);
    assert_eq!(session.position(), Position::default());
    session
        .send_swim_sample(&request, Some(&tape), request.created, |_| Ok(()))
        .unwrap();
}

#[test]
fn a_closed_endpoint_cannot_hide_unearned_physical_path() {
    let mut budget = crate::movement::swim_scene::PathBudget::new();
    budget.add([0.0; 3], [1.0, 0.0, 0.0], [0.1, 0.0, 0.0]);
    budget.add([1.0, 0.0, 0.0], [0.0; 3], [-0.1, 0.0, 0.0]);
    assert!(budget
        .check(
            WetCaps {
                horizontal: 100.0,
                up: 100.0,
                down: 100.0
            },
            0.1
        )
        .is_err());
}

#[test]
fn actual_frame_clock_keeps_fractional_phase_without_retiming_older_input() {
    let start = Instant::now();
    let mut session = owner(start, 0);
    let mut model = SwimSceneMotion::at_reset(scene(0.0), [0.0; 3]).unwrap();
    let mut now = start;
    let mut fractional_seen = 0;
    for _ in 0..60 {
        let mut until = Duration::ZERO;
        let mut pieces = Vec::new();
        for _ in 0..9 {
            now += Duration::from_micros(16_667);
            let seconds = 0.016_667;
            until += Duration::from_secs_f32(seconds);
            let input = SwimInput::Strafe(StrafeSide::Left, Pace::Run);
            model = model
                .frame(LIQUID_5, native(calibration(0), input, 0.0, 0.0), seconds)
                .unwrap()
                .0;
            fractional_seen += usize::from(
                model
                    .velocity()
                    .frame_phase()
                    .is_some_and(|phase| !phase.at_boundary),
            );
            pieces.push(
                SwimControl::physical(until, 0.0, WetKind::Normal, input, seconds, 0.0, now)
                    .unwrap(),
            );
        }
        let [x, y, z] = model.origin();
        let mut request = MovementRequest::new(
            MovementMode::Swim,
            11,
            Position {
                x,
                y,
                z,
                heading: 0.0,
            },
            now,
        );
        request.heading = HeadingUpdate::Snapshot;
        let tape = SwimControls::from_physical_frames(
            session.swim_basis().unwrap(),
            pieces,
            0.0,
            model.scene().clone(),
        )
        .unwrap();
        session
            .send_swim_sample(&request, Some(&tape), now, |_| Ok(()))
            .unwrap();
        assert!(session.swim_basis().is_some());
    }
    assert!(
        fractional_seen > 0,
        "actual ordinary calls must retain fractional phase across reports"
    );
}

#[test]
fn a_missing_observed_interval_requires_ordinary_fallback_without_scene_credit() {
    let start = Instant::now();
    let mut session = owner(start, 0);
    let mut model = SwimSceneMotion::at_reset(scene(0.0), [0.0; 3]).unwrap();
    let (request, tape) = sample(
        &session,
        &mut model,
        SwimInput::Idle,
        0.0,
        0.0,
        start + Duration::from_millis(100),
    );
    session
        .send_swim_sample(&request, Some(&tape), request.created, |_| Ok(()))
        .unwrap();
    let mut pieces = Vec::new();
    let mut until = Duration::ZERO;
    for frame in 1..=6 {
        until += Duration::from_secs_f32(0.016);
        pieces.push(
            SwimControl::physical(
                until,
                0.0,
                WetKind::Normal,
                SwimInput::Idle,
                0.016,
                0.0,
                start + Duration::from_millis(104 + frame * 16),
            )
            .unwrap(),
        );
    }
    let mut request = MovementRequest::new(
        MovementMode::Swim,
        11,
        session.position(),
        start + Duration::from_millis(200),
    );
    request.heading = HeadingUpdate::Snapshot;
    let tape = SwimControls::from_physical_frames(
        session.swim_basis().unwrap(),
        pieces,
        0.0,
        model.scene().clone(),
    )
    .unwrap();
    assert!(session.inspect_swim_replay(&request, &tape).is_err());
    assert!(session
        .send_swim_sample(&request, Some(&tape), request.created, |_| panic!(
            "unqualified scene sent"
        ))
        .is_err());
    session
        .send_move_sample(&request, request.created, |_| Ok(()))
        .unwrap();
    assert!(
        session.swim_basis().is_none(),
        "ordinary rest acceptance cannot certify the missing frame history"
    );
}

#[test]
fn a_reported_stall_keeps_the_scene_and_earns_only_the_next_physical_frame() {
    let start = Instant::now();
    let mut session = owner(start, 0);
    let mut model = SwimSceneMotion::at_reset(scene(0.05), [0.0; 3]).unwrap();
    let input = SwimInput::Strafe(StrafeSide::Right, Pace::Run);
    let stopped = start + Duration::from_millis(100);
    let (first, tape) = sample(&session, &mut model, input, 0.0, 0.0, stopped);
    session
        .send_swim_sample(&first, Some(&tape), stopped, |_| Ok(()))
        .unwrap();
    let resumed = stopped + Duration::from_micros(231_838);
    let dt = Duration::from_millis(16);
    let end = resumed + dt;
    let cal = calibration(0);
    let (next, _) = model
        .frame(
            cal.swim.unwrap().set,
            native(cal, input, 0.0, 0.0),
            dt.as_secs_f32(),
        )
        .unwrap();
    let [x, y, z] = next.origin();
    let mut request = MovementRequest::new(
        MovementMode::Swim,
        11,
        Position {
            x,
            y,
            z,
            heading: 0.0,
        },
        end,
    );
    request.heading = HeadingUpdate::Snapshot;
    let piece = SwimControl::physical(
        Duration::from_secs_f32(dt.as_secs_f32()),
        0.0,
        WetKind::Normal,
        input,
        dt.as_secs_f32(),
        0.0,
        end,
    )
    .unwrap();
    let basis = session.swim_basis().unwrap();
    let tape =
        |p| SwimControls::from_physical_frames(basis, vec![p], 0.0, model.scene().clone()).unwrap();
    assert!(session.inspect_swim_replay(&request, &tape(piece)).is_err());
    assert!(piece.after_stall(resumed, stopped).is_err());
    assert!(piece.after_stall(stopped, resumed + dt).is_err());
    let overlapping = piece.after_stall(start, resumed).unwrap();
    assert!(session
        .inspect_swim_replay(&request, &tape(overlapping))
        .is_err());
    let valid = tape(piece.after_stall(stopped, resumed).unwrap());
    let mut extra = request.clone();
    extra.position.x += 1.0;
    assert!(session.inspect_swim_replay(&extra, &valid).is_err());
    session
        .send_swim_sample(&request, Some(&valid), end, |_| Ok(()))
        .unwrap();
    assert_eq!(session.position(), request.position);
    assert!(session.inspect_swim_replay(&request, &valid).is_err());
}

#[test]
fn scene_backing_uses_its_positive_calibrated_rate() {
    let start = Instant::now();
    let mut session = owner(start, 25);
    let calibration = calibration(25);
    let swim = calibration.swim.unwrap();
    assert!(swim.backward > 0.0);
    let cap = WetCaps::new(swim.set, swim.backward, 0.0).horizontal;
    let mut model = SwimSceneMotion::at_reset(scene(0.0), [0.0; 3]).unwrap();
    let mut now = start;
    for _ in 0..100 {
        let mut pieces = Vec::new();
        let mut until = Duration::ZERO;
        for _ in 0..5 {
            now += Duration::from_millis(20);
            let seconds = 0.02;
            until += Duration::from_secs_f32(seconds);
            let input = SwimInput::Backward;
            model = model
                .frame(swim.set, native(calibration, input, 0.0, 0.0), seconds)
                .unwrap()
                .0;
            pieces.push(
                SwimControl::physical(until, 0.0, WetKind::Normal, input, seconds, 0.0, now)
                    .unwrap(),
            );
        }
        let [x, y, z] = model.origin();
        let mut request = MovementRequest::new(
            MovementMode::Swim,
            11,
            Position {
                x,
                y,
                z,
                heading: 0.0,
            },
            now,
        );
        request.heading = HeadingUpdate::Snapshot;
        let tape = SwimControls::from_physical_frames(
            session.swim_basis().unwrap(),
            pieces,
            0.0,
            model.scene().clone(),
        )
        .unwrap();
        session.inspect_swim_replay(&request, &tape).unwrap();
        session
            .send_swim_sample(&request, Some(&tape), now, |_| Ok(()))
            .unwrap();
        assert!(session.swim_basis().is_some());
    }
    assert!(session.position().y < -0.5);
    assert!(-session.position().y <= cap * 10.0 + 0.01);
}
