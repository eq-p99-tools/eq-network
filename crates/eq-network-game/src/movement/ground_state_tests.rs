//! State sampling retains the ordinary transaction, budgets and native gate.
#![allow(
    clippy::float_cmp,
    reason = "Exact assigned samples and unchanged ownership are the assertion"
)]
use super::{
    tests::{request, setup},
    *,
};

#[test]
fn ground_state_separates_travel_velocity_and_forward_input() {
    let start = Instant::now();
    let mut owner = setup(start);
    for (ms, x, velocity) in [(100, 0.5, 2.), (200, 0.7, 0.)] {
        let sample = request(start + Duration::from_millis(ms), x);
        owner
            .try_send_ground_sample(
                &sample,
                GroundState::new([velocity, 0.], false).unwrap(),
                sample.created,
                |p| {
                    assert_eq!(p.position, sample.position);
                    assert_eq!(p.delta, [velocity * 0.05, 0., 0.]);
                    assert_eq!(p.animation, 0);
                    Ok(())
                },
            )
            .unwrap();
    }
    assert!(
        !owner.stop_pending(),
        "no third zero-endpoint request needed"
    );
}

#[test]
fn ground_state_keeps_merged_history_recording_inside_the_same_transaction() {
    use crate::movement::{MovementPath, MovementPoint, TITANIUM_UPDATE_COUNTER};
    let start = Instant::now();
    let mut owner = setup(start)
        .with_counter(TITANIUM_UPDATE_COUNTER)
        .with_history(true);
    let mut sample = request(start + Duration::from_millis(100), 0.3);
    sample.history = Some(
        MovementPath::new(vec![
            MovementPoint {
                elapsed: Duration::ZERO,
                position: Position::default(),
            },
            MovementPoint {
                elapsed: Duration::from_millis(50),
                position: Position {
                    x: 0.1,
                    ..Position::default()
                },
            },
            MovementPoint {
                elapsed: Duration::from_millis(100),
                position: sample.position,
            },
        ])
        .unwrap(),
    );
    let state = GroundState::new([1., 0.], false).unwrap();
    assert!(owner
        .try_send_ground_sample(&sample, state, sample.created, |_| anyhow::bail!(
            "failed transport"
        ))
        .is_err());
    assert_eq!(owner.position(), Position::default());
    assert_eq!(owner.history.as_ref().unwrap().frames, 0);
    assert_eq!(owner.discarded_history_paths(), (0, None));
    owner
        .try_send_ground_sample(&sample, state, sample.created, |_| Ok(()))
        .unwrap();
    assert_eq!(owner.position(), sample.position);
    assert_eq!(owner.current_ground, Some(state));
    assert!(owner.history.as_ref().unwrap().frames > 0);
    assert_eq!(owner.discarded_history_paths(), (0, None));
}

#[test]
fn ground_state_waits_for_a_new_genuine_stamp_without_owner_changes() {
    let start = Instant::now();
    let mut owner = setup(start);
    let state = GroundState::new([1., 0.], true).unwrap();
    let first = request(start + Duration::from_millis(100), 0.1);
    owner
        .try_send_ground_sample(&first, state, first.created, |_| Ok(()))
        .unwrap();
    let before = (
        owner.position(),
        owner.sampled(),
        owner.last_sent,
        owner.counter.entries(),
        owner.stop_context(),
    );
    let early = request(start + Duration::from_millis(116), 0.15);
    for now in [early.created, start + Duration::from_millis(150)] {
        assert_eq!(
            owner
                .try_send_ground_sample(&early, state, now, |_| panic!("early transport"))
                .unwrap(),
            GroundSampleResult::WaitingForInterval
        );
        assert_eq!(
            (
                owner.position(),
                owner.sampled(),
                owner.last_sent,
                owner.counter.entries(),
                owner.stop_context()
            ),
            before
        );
    }
    let fresh = request(start + Duration::from_millis(150), 0.2);
    assert_eq!(
        owner
            .try_send_ground_sample(&fresh, state, fresh.created, |_| Ok(()))
            .unwrap(),
        GroundSampleResult::Accepted
    );
}

#[test]
fn ground_state_preserves_caps_and_transport_rollback() {
    let start = Instant::now();
    for velocity in [[6.01, 0.], [0., 6.01], [4.3, 4.3]] {
        let mut owner = setup(start);
        let sample = request(start + Duration::from_millis(100), 0.5);
        assert!(owner
            .try_send_ground_sample(
                &sample,
                GroundState::new(velocity, true).unwrap(),
                sample.created,
                |_| panic!("excess velocity")
            )
            .is_err());
        assert_eq!(
            (owner.position(), owner.sampled()),
            (Position::default(), start)
        );
    }
    let mut owner = setup(start);
    let sample = request(start + Duration::from_millis(100), 1.0);
    assert!(owner
        .try_send_ground_sample(
            &sample,
            GroundState::new([0.; 2], false).unwrap(),
            sample.created,
            |_| panic!("excess position")
        )
        .is_err());
    let valid = request(start + Duration::from_millis(100), 0.5);
    let state = GroundState::new([0.; 2], false).unwrap();
    assert!(owner
        .try_send_ground_sample(&valid, state, valid.created, |_| anyhow::bail!(
            "test transport refusal"
        ))
        .is_err());
    assert_eq!(
        (owner.position(), owner.sampled()),
        (Position::default(), start)
    );
    assert!(owner.last_update.is_none());
    owner
        .try_send_ground_sample(&valid, state, valid.created, |_| Ok(()))
        .unwrap();
    assert_eq!(owner.position(), valid.position);
    assert!(GroundState::new([f32::NAN, 0.], true).is_err());
    assert!(GroundState::new([0., f32::INFINITY], false).is_err());
}

fn native_owner(start: Instant) -> MotionSession {
    setup(start).with_gate(Some(UpdateGate::Flagged {
        pack: |p| p.packet(),
        rule: super::super::TITANIUM_FLAGS,
    }))
}

#[test]
fn ground_state_coalesces_a_gated_start_stop_into_one_native_report() {
    let start = Instant::now();
    let mut owner = native_owner(start);
    for (ms, x, velocity, input) in [(100, 0.4, 6., true), (200, 0.6, 0., false)] {
        let sample = request(start + Duration::from_millis(ms), x);
        owner
            .try_send_ground_sample(
                &sample,
                GroundState::new([velocity, 0.], input).unwrap(),
                sample.created,
                |_| panic!("inside native spacing"),
            )
            .unwrap();
    }
    let mut reports = Vec::new();
    assert!(owner
        .tick_sample(start + Duration::from_millis(281), |p| {
            reports.push(*p);
            Ok(())
        })
        .unwrap());
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].position.x, 0.6);
    assert_eq!(reports[0].delta, [0.; 3]);
    assert_eq!(reports[0].animation, 0);
    assert!(!owner
        .tick_sample(start + Duration::from_millis(300), |_| panic!(
            "forced final report"
        ))
        .unwrap());
}

#[test]
fn ground_state_separate_start_stop_have_two_reports_without_a_third() {
    let start = Instant::now();
    let mut owner = native_owner(start);
    let mut reports = Vec::new();
    for (ms, x, velocity, input) in [
        (300, 0.5, 6., true),
        (400, 0.6, 0.5, false),
        (500, 0.62, 0., false),
    ] {
        let sample = request(start + Duration::from_millis(ms), x);
        owner
            .try_send_ground_sample(
                &sample,
                GroundState::new([velocity, 0.], input).unwrap(),
                sample.created,
                |p| {
                    reports.push(*p);
                    Ok(())
                },
            )
            .unwrap();
    }
    assert_eq!(reports.len(), 1);
    assert!(owner
        .tick_sample(start + Duration::from_millis(581), |p| {
            reports.push(*p);
            Ok(())
        })
        .unwrap());
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[1].position.x, 0.62);
    assert_eq!(reports[1].delta, [0.; 3]);
    assert_eq!(reports[1].animation, 0);
    assert!(!owner
        .tick_sample(start + Duration::from_millis(582), |_| panic!(
            "forced third report"
        ))
        .unwrap());
}

#[test]
fn ground_state_matches_the_legacy_float_interval_boundary_and_packet() {
    let start = Instant::now();
    for nanos in [49_999_990, 49_999_999, 50_000_000, 50_000_001] {
        let (mut old, mut new) = (setup(start), setup(start));
        let sample = request(start + Duration::from_nanos(nanos), 0.1);
        let state = GroundState::new(
            [sample.position.x / old.guard.interval(sample.created), 0.],
            true,
        )
        .unwrap();
        let mut old_packet = None;
        let legacy = old.send_move_sample(&sample, sample.created, |p| {
            old_packet = Some(p.packet()?.body);
            Ok(())
        });
        let mut new_packet = None;
        let result = new
            .try_send_ground_sample(&sample, state, sample.created, |p| {
                new_packet = Some(p.packet()?.body);
                Ok(())
            })
            .unwrap();
        assert_eq!(legacy.is_ok(), result == GroundSampleResult::Accepted);
        assert_eq!(old_packet, new_packet);
    }
}

#[test]
fn ground_state_does_not_cross_calibration_or_correction_epochs() {
    let start = Instant::now();
    let mut owner = setup(start);
    let mut changed = owner.calibration().unwrap();
    changed.units_per_second = 3.;
    owner
        .calibrate(changed, start + Duration::from_millis(50))
        .unwrap();
    let sample = request(start + Duration::from_millis(100), 0.1);
    let state = GroundState::new([1., 0.], true).unwrap();
    assert!(owner
        .try_send_ground_sample(&sample, state, sample.created, |_| panic!(
            "calibration seam"
        ))
        .is_err());
    owner
        .send_move_sample(&sample, sample.created, |_| Ok(()))
        .unwrap();
    let next = request(start + Duration::from_millis(200), 0.2);
    owner
        .try_send_ground_sample(&next, state, next.created, |_| Ok(()))
        .unwrap();
    let corrected = Position {
        x: -1.,
        ..Position::default()
    };
    let now = start + Duration::from_millis(250);
    owner.correct(corrected, now).unwrap();
    owner.calibrate(changed, now).unwrap();
    assert!(owner
        .try_send_ground_sample(&next, state, now, |_| panic!("old correction epoch"))
        .is_err());
    assert_eq!(owner.position(), corrected);
}

#[test]
fn ground_state_input_request_survives_a_tap_inside_the_minimum_interval() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut owner = native_owner(start);
    let idle = request(at(300), 0.);
    owner
        .try_send_ground_sample(
            &idle,
            GroundState::new([0.; 2], false).unwrap(),
            at(300),
            |_| Ok(()),
        )
        .unwrap();
    for (ms, x, velocity, input) in [(316, 0.03, 2., true), (333, 0.05, 1., false)] {
        let sample = request(at(ms), x);
        owner
            .observe_ground_input(11, sample.created, input, sample.created)
            .unwrap();
        assert_eq!(
            owner
                .try_send_ground_sample(
                    &sample,
                    GroundState::new([velocity, 0.], input).unwrap(),
                    sample.created,
                    |_| panic!("too-early pose sent")
                )
                .unwrap(),
            GroundSampleResult::WaitingForInterval
        );
        assert_eq!(owner.position(), idle.position);
    }
    let final_state = request(at(350), 0.06);
    owner
        .observe_ground_input(11, final_state.created, false, final_state.created)
        .unwrap();
    owner
        .try_send_ground_sample(
            &final_state,
            GroundState::new([0.; 2], false).unwrap(),
            final_state.created,
            |_| panic!("inside native spacing"),
        )
        .unwrap();
    let mut count = 0;
    assert!(owner
        .tick_sample(at(581), |p| {
            count += 1;
            assert_eq!(p.position, final_state.position);
            assert_eq!(p.delta, [0.; 3]);
            Ok(())
        })
        .unwrap());
    assert_eq!(count, 1);
    assert!(!owner
        .tick_sample(at(582), |_| panic!("extra final report"))
        .unwrap());
}

#[test]
fn ground_state_waits_instead_of_sending_a_stale_current_state() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut owner = native_owner(start);
    let mut sent = Vec::new();
    let first = request(at(300), 0.4);
    owner
        .try_send_ground_sample(
            &first,
            GroundState::new([6., 0.], true).unwrap(),
            at(300),
            |p| {
                sent.push(*p);
                Ok(())
            },
        )
        .unwrap();
    let early = request(at(316), 0.45);
    let released = GroundState::new([2., 0.], false).unwrap();
    assert_eq!(
        owner
            .try_send_ground_sample(&early, released, at(316), |_| panic!("unadmitted"))
            .unwrap(),
        GroundSampleResult::WaitingForInterval
    );
    owner
        .observe_ground_input(11, early.created, false, at(316))
        .unwrap();
    assert!(!owner
        .tick_sample(at(581), |_| panic!("stale running snapshot"))
        .unwrap());
    assert_eq!(owner.sampled(), first.created);
    let rest = request(at(600), 0.5);
    owner
        .try_send_ground_sample(
            &rest,
            GroundState::new([0.; 2], false).unwrap(),
            at(600),
            |p| {
                sent.push(*p);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].position, rest.position);
    assert_eq!(sent[1].delta, [0.; 3]);
}

#[test]
fn ground_state_redundant_idle_observations_do_not_strand_the_pending_report() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut owner = native_owner(start);
    for (ms, x, state) in [
        (300, 0.4, GroundState::new([6., 0.], true).unwrap()),
        (400, 0.5, GroundState::new([0.; 2], false).unwrap()),
    ] {
        let sample = request(at(ms), x);
        owner
            .try_send_ground_sample(&sample, state, at(ms), |_| Ok(()))
            .unwrap();
    }
    let duplicate = request(at(416), 0.5);
    assert_eq!(
        owner
            .try_send_ground_sample(
                &duplicate,
                GroundState::new([0.; 2], false).unwrap(),
                at(416),
                |_| panic!("inside minimum interval")
            )
            .unwrap(),
        GroundSampleResult::WaitingForInterval
    );
    assert_eq!(owner.sampled(), at(400), "no acceptance of redundant time");
    assert!(owner
        .tick_sample(at(581), |p| {
            assert_eq!(p.position, duplicate.position);
            assert_eq!(p.delta, [0.; 3]);
            Ok(())
        })
        .unwrap());
    assert!(!owner.stop_pending());
}

#[test]
fn ground_state_refusals_preserve_waiting_and_correction_root_clear_it() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut owner = native_owner(start);
    let first = request(at(300), 0.4);
    owner
        .try_send_ground_sample(
            &first,
            GroundState::new([6., 0.], true).unwrap(),
            at(300),
            |_| Ok(()),
        )
        .unwrap();
    let waiting = request(at(316), 0.45);
    owner
        .try_send_ground_sample(
            &waiting,
            GroundState::new([2., 0.], false).unwrap(),
            at(316),
            |_| Ok(()),
        )
        .unwrap();
    let before = (
        owner.position(),
        owner.sampled(),
        owner.ground_waiting,
        owner.current_ground,
    );
    let invalid = request(at(400), 2.0);
    assert!(owner
        .try_send_ground_sample(
            &invalid,
            GroundState::new([2., 0.], false).unwrap(),
            at(400),
            |_| Ok(())
        )
        .is_err());
    assert_eq!(
        (
            owner.position(),
            owner.sampled(),
            owner.ground_waiting,
            owner.current_ground
        ),
        before
    );
    owner.root(at(420)).unwrap();
    assert_eq!(owner.ground_waiting, None);
    assert_eq!(owner.current_ground, None);
    owner.correct(Position::default(), at(500)).unwrap();
    assert_eq!(owner.ground_waiting, None);
}
