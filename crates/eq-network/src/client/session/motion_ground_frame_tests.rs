//! Current-state producer through the ordinary owner and existing wire codecs.
#![allow(
    clippy::float_cmp,
    reason = "Exact assigned samples and unchanged ownership are the assertion"
)]
use super::*;
use crate::client::session::{
    control::Stuns,
    feature::testing,
    wire::{EqMac, Titanium, Wire},
};
use eq_network_game::movement::{HeadingUpdate, MovementMode};

fn world(start: Instant, wire: &'static dyn Wire) -> World {
    let mut world = World::new(5);
    world.admitted = Some(start);
    world.own_spawn = Some(7);
    world.player.admit(testing::player(7));
    let mut owner = MotionSession::new(5, 7, Position::default(), start)
        .unwrap()
        .with_gate(wire.update_gate());
    owner
        .calibrate(
            MotionCalibration {
                units_per_second: 6.,
                velocity_scale: 0.05,
                animation: 12,
                backward: None,
                walk: None,
                strafe: None,
                swim: None,
                swim_backward_units_per_second: None,
                diagonal: DiagonalRates::default(),
            },
            start,
        )
        .unwrap();
    world.body.admit(owner);
    world
}

fn request(start: Instant, ms: u64, x: f32) -> MovementRequest {
    MovementRequest {
        session_id: 5,
        mode: MovementMode::Forward,
        heading: HeadingUpdate::Rate,
        created: start + Duration::from_millis(ms),
        position: Position {
            x,
            ..Position::default()
        },
        support: None,
        vertical_speed: None,
        history: None,
    }
}

fn received(events: &[ClientEvent]) -> Vec<(Instant, Position, bool)> {
    events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::World(WorldEvent::MotionSent {
                created,
                position,
                refused,
                ..
            }) => Some((*created, *position, refused.is_none())),
            _ => None,
        })
        .collect()
}

#[test]
fn ground_frame_waits_without_publishing_false_acceptance_and_coalesces_a_tap() {
    let start = Instant::now();
    let mut world = world(start, &Titanium);
    for (ms, x, speed, forward, accepted) in [
        (100, 0.10, 2., true, true),
        (116, 0.12, 1., false, false),
        (150, 0.14, 0., false, true),
    ] {
        let request = request(start, ms, x);
        let output = testing::run(|out| {
            Motion::step_with_ground(
                &request,
                Some(GroundState::new([speed, 0.], forward).unwrap()),
                request.created,
                &mut world,
                out,
            )
        });
        output.result.unwrap();
        assert!(output.sent.is_empty() && output.unreliable.is_empty());
        if accepted {
            assert_eq!(
                received(&output.events),
                [(request.created, request.position, true)]
            );
        } else {
            assert!(
                received(&output.events).is_empty(),
                "waiting is not acceptance"
            );
            assert_eq!(world.body.position().unwrap().x, 0.10);
        }
    }
    let output = testing::run(|out| {
        world
            .body
            .heartbeat(start + Duration::from_millis(281), out)
    });
    output.result.unwrap();
    assert_eq!(output.unreliable.len(), 1);
    let owner = world.body.motion_mut().unwrap();
    assert_eq!(owner.position().x, 0.14);
    assert!(!owner.stop_pending());
    let next = testing::run(|out| {
        world
            .body
            .heartbeat(start + Duration::from_millis(282), out)
    });
    next.result.unwrap();
    assert!(next.unreliable.is_empty(), "no special final report");
}

#[test]
fn ground_frame_uses_identical_ordinary_reporting_when_the_observed_fields_match() {
    let start = Instant::now();
    for wire in [&Titanium as &dyn Wire, &EqMac] {
        let (mut old, mut current) = (world(start, wire), world(start, wire));
        for step in 1_u8..=11 {
            let ms = u64::from(step) * 100;
            let released = step == 11;
            let x = f32::from(step.min(10)) * 0.5;
            let request = request(start, ms, x);
            let legacy = testing::run_on(wire, |out| {
                Motion::step_at(&request, request.created, &mut old, out)
            });
            let framed = testing::run_on(wire, |out| {
                Motion::step_with_ground(
                    &request,
                    Some(
                        GroundState::new([if released { 0. } else { 5. }, 0.], !released).unwrap(),
                    ),
                    request.created,
                    &mut current,
                    out,
                )
            });
            legacy.result.unwrap();
            framed.result.unwrap();
            assert_eq!(legacy.sent, framed.sent);
            assert_eq!(
                legacy.unreliable, framed.unreliable,
                "native packet/cadence changed at {ms}"
            );
            assert_eq!(received(&legacy.events), received(&framed.events));
        }
        for ms in [1101, 1200, 1280, 1281, 1400, 2000] {
            let now = start + Duration::from_millis(ms);
            let legacy = testing::run_on(wire, |out| old.body.heartbeat(now, out));
            let framed = testing::run_on(wire, |out| current.body.heartbeat(now, out));
            legacy.result.unwrap();
            framed.result.unwrap();
            assert_eq!(legacy.unreliable, framed.unreliable);
        }
    }
}

#[test]
fn ground_frame_controls_cannot_hide_velocity_behind_an_unmoved_position() {
    let start = Instant::now();
    for lying in [false, true] {
        let mut world = world(start, &Titanium);
        if lying {
            world.posture.observed(PostureState::Lying);
        } else {
            testing::run(|out| {
                Stuns::observe_at(
                    &Message::Event(WorldEvent::Stun(Some(Duration::from_secs(2)))),
                    start,
                    &mut world,
                    out,
                )
            })
            .result
            .unwrap();
        }
        let request = request(start, 100, 0.);
        let output = testing::run(|out| {
            Motion::step_with_ground(
                &request,
                Some(GroundState::new([1., 0.], false).unwrap()),
                request.created,
                &mut world,
                out,
            )
        });
        output.result.unwrap();
        assert!(output.sent.is_empty() && output.unreliable.is_empty());
        assert_eq!(
            received(&output.events),
            [(request.created, Position::default(), false)]
        );
        assert_eq!(world.body.position(), Some(Position::default()));
    }
}

#[test]
fn ground_frame_correction_rejects_queued_old_samples_before_any_publication() {
    let start = Instant::now();
    let mut world = world(start, &Titanium);
    let old = request(start, 100, 0.1);
    let corrected = Position {
        x: -1.,
        ..Position::default()
    };
    world
        .correct_own(corrected, start + Duration::from_millis(150))
        .unwrap();
    let output = testing::run(|out| {
        Motion::step_with_ground(
            &old,
            Some(GroundState::new([1., 0.], true).unwrap()),
            start + Duration::from_millis(160),
            &mut world,
            out,
        )
    });
    output.result.unwrap();
    assert_eq!(
        output.unreliable,
        Vec::<eq_network_game::command::EncodedCommand>::new()
    );
    assert_eq!(received(&output.events), [(old.created, corrected, false)]);
    assert_eq!(world.body.position(), Some(corrected));
}

#[test]
fn ground_frame_is_owned_by_the_normal_feature_entry_without_a_new_packet_type() {
    let now = Instant::now();
    let start = now.checked_sub(Duration::from_secs(1)).unwrap();
    let mut world = world(start, &Titanium);
    let motion = Motion::new(false, false, None);
    let command = ClientCommand::GroundFrame {
        request: request(start, 1000, 0.1),
        state: GroundState::new([0.; 2], false).unwrap(),
    };
    assert!(motion.owns(&command));
    let mut motion = motion;
    // The fixture has already applied this grant for these inventory inputs.
    motion.belongings = Some((
        world.inventory.revision(),
        world.coins.purse,
        world.coins.cursor,
    ));
    let output = testing::run(|out| motion.handle(&command, &mut world, out));
    output.result.unwrap();
    assert_eq!(output.unreliable.len(), 1);
    assert_eq!(
        output.unreliable[0].opcode,
        eq_network_game::movement::POSITION_OPCODE
    );
    assert_eq!(received(&output.events).len(), 1);
}

#[test]
fn ground_frame_packet_counter_matches_transport_submissions_not_local_acceptances() {
    let start = Instant::now();
    let mut world = world(start, &Titanium);
    let output = testing::run(|out| {
        for frame in 1_u32..=600 {
            let created = start + Duration::from_nanos(u64::from(frame) * 16_666_667);
            let request = MovementRequest {
                created,
                ..request(start, 0, f32::from(u16::try_from(frame).unwrap()) * 0.1)
            };
            Motion::step_with_ground(
                &request,
                Some(GroundState::new([6., 0.], true).unwrap()),
                created,
                &mut world,
                out,
            )
            .unwrap();
            world.body.heartbeat(created, out).unwrap();
        }
        out.log.movement_packets_sent
    });
    assert!((1..40).contains(&output.result));
    assert_eq!(
        output.result,
        u64::try_from(output.unreliable.len()).unwrap()
    );
    assert_eq!(
        output.sent,
        Vec::<eq_network_game::command::EncodedCommand>::new()
    );
    assert!(received(&output.events).len() > output.unreliable.len());
}
