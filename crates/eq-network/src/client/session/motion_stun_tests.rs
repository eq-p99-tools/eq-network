//! Clock-driven session enforcement with fabricated values, no live traffic.
use super::*;
use crate::client::session::{control::Stuns, feature::testing};
use eq_network_game::movement::MovementMode;
use std::time::Duration;

fn setup(start: Instant) -> World {
    let mut world = World::new(12);
    world.player.admit(testing::player(7));
    world.own_spawn = Some(7);
    world.admitted = Some(start);
    let mut motion = MotionSession::new(12, 7, Position::default(), start)
        .unwrap()
        .with_falls(true)
        .with_jump(Some((44.55, 400.0)));
    motion
        .calibrate(
            MotionCalibration {
                diagonal: Default::default(),
                units_per_second: 6.0,
                velocity_scale: 0.05,
                animation: 12,
                strafe: None,
                walk: None,
                backward: None,
            },
            start,
        )
        .unwrap();
    world.body.admit(motion);
    world
}

fn stun(world: &mut World, now: Instant) {
    testing::run(|out| {
        Stuns::observe_at(
            &Message::Event(WorldEvent::Stun(Some(Duration::from_secs(2)))),
            now,
            world,
            out,
        )
    })
    .result
    .unwrap();
}

fn request(created: Instant, position: Position) -> MovementRequest {
    MovementRequest {
        session_id: 12,
        mode: MovementMode::Fall,
        created,
        position,
    }
}

#[test]
fn stun_refuses_horizontal_turn_new_jump_and_grounded_rise_before_any_send() {
    let start = Instant::now();
    let mut world = setup(start);
    stun(&mut world, start + Duration::from_millis(100));
    let now = start + Duration::from_millis(200);
    for position in [
        Position {
            x: 0.1,
            ..Position::default()
        },
        Position {
            heading: 10.0,
            ..Position::default()
        },
        Position {
            z: 0.1,
            ..Position::default()
        },
    ] {
        let output =
            testing::run(|out| Motion::step_at(&request(now, position), now, &mut world, out));
        output.result.unwrap();
        assert!(output.sent.is_empty() && output.unreliable.is_empty());
        assert!(matches!(
            output.events[..],
            [ClientEvent::World(WorldEvent::MotionSent {
                refused: Some(_),
                ..
            })]
        ));
        assert_eq!(world.body.position(), Some(Position::default()));
    }
    let motion = Motion::new(true, true, None);
    let output = testing::run(|out| motion.jump(now, &mut world, out));
    output.result.unwrap();
    assert_eq!(
        output.sent,
        Vec::<eq_network_game::command::EncodedCommand>::new()
    );
    // A vertical descent still goes through the original calibrated owner.
    let falling = request(
        now,
        Position {
            z: -1.0,
            ..Position::default()
        },
    );
    let output = testing::run(|out| Motion::step_at(&falling, now, &mut world, out));
    output.result.unwrap();
    assert_eq!(output.unreliable.len(), 1);
    assert_eq!(world.body.position(), Some(falling.position));
}

#[test]
fn prior_jump_and_corrections_survive_stun_but_transfer_and_death_still_suspend_motion() {
    let start = Instant::now();
    let mut world = setup(start);
    world.body.motion_mut().unwrap().jumped();
    let calibration = world.body.motion_mut().unwrap().calibration();
    stun(&mut world, start + Duration::from_millis(50));
    assert_eq!(world.body.motion_mut().unwrap().calibration(), calibration);
    let now = start + Duration::from_millis(100);
    let rising = request(
        now,
        Position {
            z: 1.0,
            ..Position::default()
        },
    );
    let output = testing::run(|out| Motion::step_at(&rising, now, &mut world, out));
    output.result.unwrap();
    assert_eq!(world.body.position(), Some(rising.position));
    assert_eq!(output.unreliable.len(), 1);

    let corrected = Position {
        x: 80.0,
        z: 10.0,
        ..Position::default()
    };
    world.correct_own(corrected, now).unwrap();
    assert!(world.control.stunned());
    assert_eq!(world.body.position(), Some(corrected));
    assert_eq!(world.player.as_ref().unwrap().position, corrected);
    world
        .transfer_offered(
            eq_network_game::zoning::ZoneOffer {
                zone_id: 2,
                instance_id: 0,
                position: corrected,
                reason: 1,
                to_bind: false,
                solicited: true,
            },
            now,
        )
        .unwrap();
    let suspended = request(
        start + Duration::from_millis(200),
        Position {
            z: 9.0,
            ..corrected
        },
    );
    let output =
        testing::run(|out| Motion::step_at(&suspended, suspended.created, &mut world, out));
    output.result.unwrap();
    assert_eq!(
        output.unreliable,
        Vec::<eq_network_game::command::EncodedCommand>::new()
    );
    world
        .transfer_refused(start + Duration::from_millis(250))
        .unwrap();
    assert!(world.control.stunned());
    world.died();
    let output = testing::run(|out| world.body.heartbeat(start + Duration::from_secs(5), out));
    output.result.unwrap();
    assert_eq!(
        output.unreliable,
        Vec::<eq_network_game::command::EncodedCommand>::new()
    );
    testing::run(|out| Stuns.tick(start + Duration::from_secs(6), &mut world, out))
        .result
        .unwrap();
    assert!(!world.control.stunned());
    assert!(world.lifecycle.is_dead());
    assert!(world.body.motion_mut().unwrap().calibration().is_none());
}

#[test]
fn expiry_does_not_replay_queued_move_or_jump_and_fresh_motion_uses_existing_calibration() {
    let start = Instant::now();
    let mut world = setup(start);
    stun(&mut world, start + Duration::from_millis(100));
    let end = start + Duration::from_millis(2101);
    testing::run(|out| Stuns.tick(end, &mut world, out))
        .result
        .unwrap();
    let queued = request(
        start + Duration::from_millis(200),
        Position {
            x: 0.1,
            ..Position::default()
        },
    );
    let output = testing::run(|out| Motion::step_at(&queued, end, &mut world, out));
    output.result.unwrap();
    assert_eq!(
        output.unreliable,
        Vec::<eq_network_game::command::EncodedCommand>::new()
    );
    let motion = Motion::new(true, true, None);
    let output = testing::run(|out| motion.jump(queued.created, &mut world, out));
    output.result.unwrap();
    assert_eq!(
        output.sent,
        Vec::<eq_network_game::command::EncodedCommand>::new()
    );
    let fresh = request(end + Duration::from_millis(100), queued.position);
    let output = testing::run(|out| Motion::step_at(&fresh, fresh.created, &mut world, out));
    output.result.unwrap();
    assert_eq!(output.unreliable.len(), 1);
    assert_eq!(world.body.position(), Some(fresh.position));
}

#[test]
fn opted_in_stun_movement_keeps_transition_freshness_and_jump_restriction() {
    let start = Instant::now();
    let mut world = setup(start);
    stun(&mut world, start + Duration::from_millis(100));
    world
        .body
        .motion
        .as_mut()
        .unwrap()
        .set_options(
            eq_network_game::movement::MovementOptions {
                walk_while_stunned: true,
                ..Default::default()
            },
            start + Duration::from_millis(110),
        )
        .unwrap();
    let now = start + Duration::from_millis(210);
    let output = testing::run(|out| {
        Motion::step_at(
            &request(
                now,
                Position {
                    x: 0.2,
                    ..Position::default()
                },
            ),
            now,
            &mut world,
            out,
        )
    });
    output.result.unwrap();
    assert_eq!(world.body.position().unwrap().x, 0.2);
    let output = testing::run(|out| {
        Motion::step_at(
            &request(
                start + Duration::from_millis(90),
                Position {
                    x: 0.3,
                    ..Position::default()
                },
            ),
            now,
            &mut world,
            out,
        )
    });
    output.result.unwrap();
    assert_eq!(world.body.position().unwrap().x, 0.2);
    assert_eq!(output.unreliable, []);
    let motion = Motion::new(true, true, None);
    let output = testing::run(|out| motion.jump(now, &mut world, out));
    output.result.unwrap();
    assert_eq!(output.sent, []);
    assert!(
        world.control.stunned(),
        "the option never clears control authority"
    );
}

#[test]
fn movement_policy_feedback_is_local_and_refusals_keep_prior_policy() {
    let start = Instant::now();
    let mut world = setup(start);
    let mut owner = Motion::new(true, true, None);
    owner.belongings = Some((
        world.inventory.revision(),
        world.coins.purse,
        world.coins.cursor,
    ));
    let options = eq_network_game::movement::MovementOptions {
        fly: true,
        ..Default::default()
    };
    let command = ClientCommand::SetMovementOptions {
        session_id: 12,
        request_id: 42,
        options,
        created: Instant::now(),
    };
    let output = testing::run(|out| owner.handle(&command, &mut world, out));
    output.result.unwrap();
    assert_eq!(output.sent, []);
    assert!(
        matches!(output.events[..], [ClientEvent::World(WorldEvent::MovementOptionsApplied {
        session_id: 12, request_id: 42, options: applied,
    })] if applied == options)
    );
    assert_eq!(world.body.motion.as_ref().unwrap().options(), options);
    // A stale local option result is a refusal, not a reset or server packet.
    let old = ClientCommand::SetMovementOptions {
        session_id: 12,
        request_id: 43,
        options: eq_network_game::movement::MovementOptions::default(),
        created: start.checked_sub(Duration::from_millis(1)).unwrap(),
    };
    let output = testing::run(|out| owner.handle(&old, &mut world, out));
    output.result.unwrap();
    assert_eq!(output.sent, []);
    assert!(matches!(
        output.events[..],
        [
            ClientEvent::World(WorldEvent::MovementOptionsRefused {
                session_id: 12,
                request_id: 43,
                ..
            }),
            ClientEvent::Diagnostic(_)
        ]
    ));
    assert_eq!(world.body.motion.as_ref().unwrap().options(), options);
}
