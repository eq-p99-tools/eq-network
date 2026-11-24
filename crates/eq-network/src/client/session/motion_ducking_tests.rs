//! Ducked move and host-decided jump posture on both programs, with
//! fabricated values and no live traffic (read 2026-11-24).
use super::super::posture::Stances;
use super::*;
use crate::client::session::feature::testing;
use eq_network_game::command;
use eq_network_game::movement::{titanium_jump, MovementMode};
use std::time::Duration;

const EQMAC: &dyn super::super::wire::Wire = &super::super::wire::EqMac;
const TITANIUM: &dyn super::super::wire::Wire = &super::super::wire::Titanium;

/// The ducked forward rate, half the walk speed: 0.23 units of input.
const DUCKED: f32 = 0.23 * 42.667;

/// The standing run rate, 0.7 units of input.
const RUNNING: f32 = 0.7 * 42.667;

/// A level 50 player admitted on `wire`, granted with nothing slowing them,
/// and then given `posture`.
fn admitted(wire: &'static dyn super::super::wire::Wire, posture: PostureState) -> (World, Motion) {
    let mut world = World::new(5);
    world.own_spawn = Some(7);
    let mut player = testing::player(7);
    (player.run_speed, player.walk_speed, player.level) = (0.7, 0.46, 50);
    world.player.admit(player);
    world.admitted = Some(Instant::now());
    let mut motion = Motion::new(true, true, None);
    motion
        .admit(
            &Message::Event(WorldEvent::Nourishment(
                eq_network_game::food::Nourishment {
                    fatigue: Some(0),
                    ..Default::default()
                },
            )),
            &mut world,
        )
        .unwrap();
    testing::run_on(wire, |out| motion.admitted(&mut world, out))
        .result
        .unwrap();
    let outcome = testing::run_on(wire, |out| motion.handle(&inputs(0), &mut world, out));
    outcome.result.unwrap();
    assert!(close(speed(&world), RUNNING), "{:?}", outcome.events);
    world.posture.observed(posture);
    (world, motion)
}

fn inputs(rate: i16) -> ClientCommand {
    ClientCommand::SpeedInputs {
        session_id: 5,
        revision: 1,
        report_id: 1,
        state: SpeedInputState::Ready(SpeedInputs {
            eqmac_fatigue: Some(eq_network_game::movement::EqMacFatigueInputs {
                strength: 100,
                stamina: Some(100),
            }),
            rate,
            weight: 0,
            capacity: 100,
        }),
    }
}

fn close(left: f32, right: f32) -> bool {
    (left - right).abs() < 0.05
}

fn speed(world: &World) -> f32 {
    world
        .body
        .motion
        .as_ref()
        .and_then(MotionSession::calibration)
        .map_or(0.0, |speeds| speeds.units_per_second)
}

/// Moves forward to `x`, in a sample made at `created`.
fn forward(
    wire: &'static dyn super::super::wire::Wire,
    world: &mut World,
    x: f32,
    created: Instant,
) -> testing::Outcome<Result<()>> {
    let request = MovementRequest::new(
        MovementMode::Forward,
        5,
        Position {
            x,
            ..Position::default()
        },
        created,
    );
    testing::run_on(wire, |out| Motion::step_at(&request, created, world, out))
}

/// One session tick, which keeps the grant in step with the stance.
fn tick(motion: &mut Motion, world: &mut World) -> testing::Outcome<Result<()>> {
    testing::run(|out| motion.tick(Instant::now(), world, out))
}

/// Ticks once and checks the tick granted `expected` alone, in place,
/// sending nothing reliable.
fn ticks_to(motion: &mut Motion, world: &mut World, expected: f32) {
    let output = tick(motion, world);
    output.result.unwrap();
    assert!(output.sent.is_empty(), "{:?}", output.sent);
    let (speeds, withdrawn) = grants(&output.events);
    assert!(!withdrawn, "{:?}", output.events);
    assert!(
        speeds.len() == 1 && close(speeds[0], expected),
        "{:?}",
        output.events
    );
}

fn refused(events: &[ClientEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            ClientEvent::World(WorldEvent::MotionSent {
                refused: Some(_),
                ..
            })
        )
    })
}

/// The speeds the events grant, and whether any withdraws movement.
fn grants(events: &[ClientEvent]) -> (Vec<f32>, bool) {
    let mut speeds = Vec::new();
    let mut withdrawn = false;
    for event in events {
        if let ClientEvent::World(WorldEvent::MotionState {
            units_per_second, ..
        }) = event
        {
            match units_per_second {
                Some(speed) => speeds.push(*speed),
                None => withdrawn = true,
            }
        }
    }
    (speeds, withdrawn)
}

fn stand() -> ClientCommand {
    ClientCommand::SetPosture {
        session_id: 5,
        spawn_id: 7,
        posture: command::Posture::Standing,
        created: Instant::now(),
    }
}

fn jump() -> ClientCommand {
    ClientCommand::Jump {
        session_id: 5,
        posture: JumpPosture::StandIfDucked,
        created: Instant::now(),
    }
}

#[test]
fn titanium_moves_a_ducking_player_ducked_at_the_sneaking_speed() {
    let (mut world, mut motion) = admitted(TITANIUM, PostureState::Ducking);
    // The grant follows the duck at the next tick, before any move.
    ticks_to(&mut motion, &mut world, DUCKED);
    let start = Instant::now();
    let half = start + Duration::from_millis(250);
    let output = forward(TITANIUM, &mut world, 1.0, half);
    output.result.unwrap();
    // No stand goes out and the step is taken.
    assert!(output.sent.is_empty(), "{:?}", output.sent);
    assert!(!refused(&output.events), "{:?}", output.events);
    assert!(world.body.position().is_some_and(|at| close(at.x, 1.0)));
    assert_eq!(world.posture.current(), Some(PostureState::Ducking));
    // A quarter second more at most of the standing run rate is beyond the
    // ducked one (the guard counts at most a quarter second a sample).
    let later = half + Duration::from_millis(250);
    let output = forward(TITANIUM, &mut world, 1.0 + RUNNING * 0.2, later);
    output.result.unwrap();
    assert!(refused(&output.events), "{:?}", output.events);
    assert!(output.sent.is_empty(), "{:?}", output.sent);
    // Standing, the same two samples are taken.
    let (mut world, _) = admitted(TITANIUM, PostureState::Standing);
    let start = Instant::now();
    let half = start + Duration::from_millis(250);
    let later = half + Duration::from_millis(250);
    for (x, created) in [(1.0, half), (1.0 + RUNNING * 0.2, later)] {
        let output = forward(TITANIUM, &mut world, x, created);
        output.result.unwrap();
        assert!(!refused(&output.events), "{:?}", output.events);
    }
}

#[test]
fn a_duck_at_rest_slows_the_first_sample() {
    // Ducked at rest, the first quarter second goes at the ducked rate from
    // the start: at the run rate it is refused, at the ducked rate taken.
    // A sample straddling the change takes each speed for its own span,
    // which the game crate's in-place tests cover.
    for (rate, taken) in [(RUNNING, false), (DUCKED, true)] {
        let (mut world, mut motion) = admitted(TITANIUM, PostureState::Ducking);
        ticks_to(&mut motion, &mut world, DUCKED);
        let created = Instant::now() + Duration::from_millis(250);
        let output = forward(TITANIUM, &mut world, rate * 0.24, created);
        output.result.unwrap();
        assert_eq!(!refused(&output.events), taken, "{:?}", output.events);
    }
}

#[test]
fn the_grant_follows_a_duck_and_a_stand_in_place() {
    let (mut world, mut motion) = admitted(TITANIUM, PostureState::Standing);
    // Our own duck, then a tick: the ducked grant, nothing withdrawn.
    let duck = ClientCommand::SetPosture {
        session_id: 5,
        spawn_id: 7,
        posture: command::Posture::Ducking,
        created: Instant::now(),
    };
    let output = testing::run(|out| Stances.handle(&duck, &mut world, out));
    output.result.unwrap();
    assert_eq!(
        output.sent,
        [command::titanium_posture(7, command::Posture::Ducking).unwrap()]
    );
    ticks_to(&mut motion, &mut world, DUCKED);
    // A stand, then a tick: the running grant, nothing withdrawn.
    let output = testing::run(|out| Stances.handle(&stand(), &mut world, out));
    output.result.unwrap();
    assert_eq!(
        output.sent,
        [command::titanium_posture(7, command::Posture::Standing).unwrap()]
    );
    ticks_to(&mut motion, &mut world, RUNNING);
    // A second tick grants nothing.
    let output = tick(&mut motion, &mut world);
    output.result.unwrap();
    assert!(grants(&output.events).0.is_empty(), "{:?}", output.events);
    // A server's report of a duck is followed the same way.
    world.posture.observed(PostureState::Ducking);
    ticks_to(&mut motion, &mut world, DUCKED);
}

#[test]
fn the_mac_era_client_moves_ducked_without_sending_a_stand() {
    let (mut world, mut motion) = admitted(EQMAC, PostureState::Ducking);
    let tick = testing::run_on(EQMAC, |out| motion.tick(Instant::now(), &mut world, out));
    tick.result.unwrap();
    assert!(close(speed(&world), DUCKED));
    let output = forward(
        EQMAC,
        &mut world,
        1.0,
        Instant::now() + Duration::from_millis(200),
    );
    output.result.unwrap();
    assert!(output.sent.is_empty(), "{:?}", output.sent);
    assert!(
        !output.unreliable.is_empty(),
        "movement must reach the wire"
    );
    assert!(!refused(&output.events), "{:?}", output.events);
    assert!(world.body.position().is_some_and(|at| close(at.x, 1.0)));
    assert_eq!(world.posture.current(), Some(PostureState::Ducking));
    assert!(grants(&output.events).0.is_empty(), "{:?}", output.events);
    let output = testing::run_on(EQMAC, |out| motion.tick(Instant::now(), &mut world, out));
    output.result.unwrap();
    assert!(close(speed(&world), DUCKED));
}

#[test]
fn a_ducked_jump_stands_the_player_then_launches() {
    for (wire, expected) in [
        (
            TITANIUM,
            vec![
                command::titanium_posture(7, command::Posture::Standing).unwrap(),
                titanium_jump(),
            ],
        ),
        (
            EQMAC,
            vec![
                eq_network_game::quarm::posture(7, command::Posture::Standing).unwrap(),
                eq_network_game::quarm::jump(),
            ],
        ),
    ] {
        let (mut world, mut motion) = admitted(wire, PostureState::Ducking);
        let output = testing::run_on(wire, |out| motion.handle(&jump(), &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent, expected);
        assert_eq!(world.posture.current(), Some(PostureState::Standing));
        assert!(output.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Posture {
                spawn_id: 7,
                posture: PostureState::Standing
            })
        )));
    }
    // A jump from standing sends only the jump, once the spacing has passed.
    let mut world_standing = admitted(TITANIUM, PostureState::Standing);
    let output = testing::run(|out| world_standing.1.handle(&jump(), &mut world_standing.0, out));
    output.result.unwrap();
    assert_eq!(output.sent, [titanium_jump()]);
}

#[test]
fn a_host_refused_stand_keeps_the_jump_ducked() {
    for (wire, packet) in [
        (TITANIUM, titanium_jump()),
        (EQMAC, eq_network_game::quarm::jump()),
    ] {
        let (mut world, mut motion) = admitted(wire, PostureState::Ducking);
        let jump = ClientCommand::Jump {
            session_id: 5,
            posture: JumpPosture::Keep,
            created: Instant::now(),
        };
        let output = testing::run_on(wire, |out| motion.handle(&jump, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent, [packet]);
        assert_eq!(world.posture.current(), Some(PostureState::Ducking));
        assert!(!output
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::Posture { .. }))));
    }
}

#[test]
fn a_refused_jump_stands_no_one() {
    // Titanium refuses a rooted jump (R22 c), so the ducking player stays
    // ducked and nothing goes out.
    let (mut world, mut motion) = admitted(TITANIUM, PostureState::Ducking);
    testing::run(|out| motion.handle(&inputs(-1000), &mut world, out))
        .result
        .unwrap();
    let output = testing::run(|out| motion.handle(&jump(), &mut world, out));
    output.result.unwrap();
    assert!(output.sent.is_empty(), "{:?}", output.sent);
    assert_eq!(world.posture.current(), Some(PostureState::Ducking));
}

/// A seated player's or a looter's jump goes nowhere and stands no one, on
/// both generations: Titanium's jump waits on the can-move flag (read R19 c),
/// and the Mac-era client's is inferred the same (6.599).
#[test]
fn a_seated_or_kneeling_player_does_not_jump_or_stand() {
    for wire in [TITANIUM, EQMAC] {
        for posture in [PostureState::Sitting, PostureState::Looting] {
            let (mut world, mut motion) = admitted(wire, posture);
            let output = testing::run_on(wire, |out| motion.handle(&jump(), &mut world, out));
            output.result.unwrap();
            assert!(output.sent.is_empty(), "{posture:?}: {:?}", output.sent);
            assert_eq!(world.posture.current(), Some(posture));
            assert!(
                !output
                    .events
                    .iter()
                    .any(|event| matches!(event, ClientEvent::World(WorldEvent::Posture { .. }))),
                "{posture:?}: {:?}",
                output.events
            );
        }
    }
}

/// Once the player stands, the same press jumps.
#[test]
fn a_player_who_stood_up_jumps_again() {
    let (mut world, mut motion) = admitted(TITANIUM, PostureState::Sitting);
    world.posture.observed(PostureState::Standing);
    let output = testing::run(|out| motion.handle(&jump(), &mut world, out));
    output.result.unwrap();
    assert_eq!(output.sent, [titanium_jump()]);
}

/// A jump while a feign is held standing (6.454) sends the jump alone and
/// keeps the feign: no posture goes, and the next stand is still owed.
#[test]
fn a_jump_keeps_a_feign_held_standing() {
    let (mut world, mut motion) = admitted(TITANIUM, PostureState::Standing);
    testing::run(|out| world.posture.hold_feign(out.log))
        .result
        .unwrap();
    let output = testing::run(|out| motion.handle(&jump(), &mut world, out));
    output.result.unwrap();
    assert_eq!(output.sent, [titanium_jump()]);
    assert!(world.posture.feign_held() && world.posture.stand_owed());
}
