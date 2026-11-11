//! A player lying on the ground, as the Titanium program holds them still
//! (read R19 c), with fabricated values and no live traffic.
use super::*;
use crate::client::session::feature::testing;
use eq_network_game::movement::{DiagonalRates, MovementMode, WalkCalibration};
use std::time::Duration;

const EQMAC: &dyn super::super::wire::Wire = &super::super::wire::EqMac;

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
                swim_backward_units_per_second: None,
                diagonal: DiagonalRates::default(),
                units_per_second: 6.0,
                velocity_scale: 0.05,
                animation: 12,
                strafe: None,
                walk: Some(WalkCalibration {
                    units_per_second: 3.0,
                    velocity_scale: 0.05,
                    animation: 6,
                }),
                backward: None,
            },
            start,
        )
        .unwrap();
    world.body.admit(motion);
    world.posture.observed(PostureState::Lying);
    world
}

fn request(mode: MovementMode, created: Instant, position: Position) -> MovementRequest {
    MovementRequest {
        session_id: 12,
        mode,
        heading: eq_network_game::movement::HeadingUpdate::Rate,
        created,
        position,
        support: None,
    }
}

fn ahead(distance: f32) -> Position {
    Position {
        x: distance,
        ..Position::default()
    }
}

fn refused(events: &[ClientEvent]) -> bool {
    matches!(
        events[..],
        [ClientEvent::World(WorldEvent::MotionSent {
            refused: Some(_),
            ..
        })]
    )
}

#[test]
fn titanium_moves_no_lying_player_and_stands_no_one_to_move() {
    let start = Instant::now();
    let mut world = setup(start);
    let now = start + Duration::from_millis(500);
    for mode in [
        MovementMode::Forward,
        MovementMode::Backward,
        MovementMode::Strafe,
        MovementMode::Walk,
    ] {
        let output = testing::run(|out| {
            Motion::step_at(&request(mode, now, ahead(0.5)), now, &mut world, out)
        });
        output.result.unwrap();
        assert!(refused(&output.events), "{mode:?}");
        assert!(
            output.sent.is_empty() && output.unreliable.is_empty(),
            "{mode:?}"
        );
        assert_eq!(world.posture.current(), Some(PostureState::Lying));
        assert_eq!(world.body.position(), Some(Position::default()));
    }
    // A grant is not taken away: the motion session keeps its calibration.
    assert!(world.body.motion_mut().unwrap().calibration().is_some());
    let motion = Motion::new(true, true, None);
    let output = testing::run(|out| motion.jump(now, &mut world, out));
    output.result.unwrap();
    assert!(output.sent.is_empty(), "{:?}", output.sent);
}

#[test]
fn a_lying_player_still_turns_and_falls() {
    let start = Instant::now();
    let mut world = setup(start);
    let now = start + Duration::from_millis(500);
    // Mouse look turns a lying player in the chase views (read R19 c).
    let turn = Position {
        heading: 10.0,
        ..Position::default()
    };
    let output = testing::run(|out| {
        Motion::step_at(
            &request(MovementMode::Forward, now, turn),
            now,
            &mut world,
            out,
        )
    });
    output.result.unwrap();
    assert!(!refused(&output.events));
    assert_eq!(world.body.position(), Some(turn));
    let later = now + Duration::from_millis(500);
    let falling = Position { z: -1.0, ..turn };
    let output = testing::run(|out| {
        Motion::step_at(
            &request(MovementMode::Fall, later, falling),
            later,
            &mut world,
            out,
        )
    });
    output.result.unwrap();
    assert_eq!(output.unreliable.len(), 1);
    assert!(output.sent.is_empty(), "no stand goes out");
    assert_eq!(world.body.position(), Some(falling));
    assert_eq!(world.posture.current(), Some(PostureState::Lying));
}

#[test]
fn the_mac_client_still_stands_a_lying_player_to_move() {
    let start = Instant::now();
    let mut world = setup(start);
    let now = start + Duration::from_millis(500);
    let output = testing::run_on(EQMAC, |out| {
        Motion::step_at(
            &request(MovementMode::Forward, now, ahead(0.5)),
            now,
            &mut world,
            out,
        )
    });
    output.result.unwrap();
    assert!(!refused(&output.events));
    assert_eq!(output.sent.len(), 1, "the stand");
    assert_eq!(world.posture.current(), Some(PostureState::Standing));
}

/// A feign spell landing on a walking player lies them down and stops the
/// movement in hand, as the press does (read R21); the next step is refused
/// and the grant is kept.
#[test]
fn a_feign_spell_landing_stops_a_walking_player_and_keeps_the_grant() {
    use super::super::{feature::Feature, posture::Stances};
    let start = Instant::now();
    let mut world = setup(start);
    world.posture.observed(PostureState::Standing);
    let now = start + Duration::from_millis(500);
    let output = testing::run(|out| {
        Motion::step_at(
            &request(MovementMode::Forward, now, ahead(0.5)),
            now,
            &mut world,
            out,
        )
    });
    output.result.unwrap();
    assert!(!refused(&output.events));
    let walked = world.body.position();
    let landed = ClientCommand::FeignLanded {
        session_id: 12,
        spawn_id: 7,
        created: now,
    };
    let output = testing::run(|out| Stances.handle(&landed, &mut world, out));
    output.result.unwrap();
    assert_eq!(
        output.sent,
        [eq_network_game::feign::titanium_posture(7).unwrap()]
    );
    assert_eq!(world.posture.current(), Some(PostureState::Lying));
    let later = now + Duration::from_millis(500);
    let output = testing::run(|out| {
        Motion::step_at(
            &request(MovementMode::Forward, later, ahead(1.0)),
            later,
            &mut world,
            out,
        )
    });
    output.result.unwrap();
    assert!(refused(&output.events));
    assert!(output.sent.is_empty() && output.unreliable.is_empty());
    assert_eq!(world.body.position(), walked);
    assert!(world.body.motion_mut().unwrap().calibration().is_some());
}
