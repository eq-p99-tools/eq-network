use super::super::feature::testing;
use super::*;
use eq_network_game::{
    movement::{CarrierSample, MovementMode},
    world::SpawnKind,
    GameDialect,
};

fn setup(start: Instant) -> (World, CarrierSample) {
    let mut world = World::new(5);
    world.player.admit(testing::player(7));
    world
        .body
        .admit(MotionSession::new(5, 7, world.player.as_ref().unwrap().position, start).unwrap());
    let mut boat = testing::spawn(8, SpawnKind::Npc);
    boat.race = 141;
    world.spawns.insert(boat.clone());
    world.carriers.observe(
        &WorldEvent::Spawns(vec![boat]),
        GameDialect::Titanium,
        start,
    );
    let sample = world.carriers.latest(8).unwrap().sample();
    (world, sample)
}

#[test]
fn accepted_boat_support_changes_only_on_an_accepted_proposal() {
    let start = Instant::now();
    let (mut world, sample) = setup(start);
    let at = start + Duration::from_millis(100);
    let accepted =
        MovementRequest::new(MovementMode::Forward, 5, world.body.position().unwrap(), at)
            .supported_by(Support::Spawn(sample));
    let outcome = testing::run(|out| Motion::step_at(&accepted, at, &mut world, out));
    outcome.result.unwrap();
    assert_eq!(world.body.carrier_support(), Some(sample));
    let later = at + Duration::from_millis(100);
    let rejected = MovementRequest::new(
        MovementMode::Forward,
        5,
        Position {
            x: 1000.0,
            ..accepted.position
        },
        later,
    );
    let outcome = testing::run(|out| Motion::step_at(&rejected, later, &mut world, out));
    outcome.result.unwrap();
    assert!(outcome.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::MotionSent {
            refused: Some(_),
            ..
        })
    )));
    assert_eq!(world.body.carrier_support(), Some(sample));
    world
        .body
        .motion_mut()
        .unwrap()
        .calibrate(
            MotionCalibration {
                units_per_second: 6.0,
                velocity_scale: 0.05,
                animation: 12,
                backward: None,
                walk: None,
                strafe: None,
                diagonal: DiagonalRates::default(),
                swim_backward_units_per_second: None,
            },
            later,
        )
        .unwrap();
    let later = later + Duration::from_millis(100);
    let unlinked = MovementRequest::new(MovementMode::Forward, 5, accepted.position, later);
    let outcome = testing::run(|out| Motion::step_at(&unlinked, later, &mut world, out));
    outcome.result.unwrap();
    assert!(outcome.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::MotionSent { refused: None, .. })
    )));
    assert_eq!(world.body.carrier_support(), None);
}

#[test]
fn invalid_correction_preserves_support_but_correction_suspend_and_admission_clear_it() {
    let now = Instant::now();
    let (mut world, sample) = setup(now);
    world.body.record_carrier_support(Some(sample));
    let mut player = testing::player(7);
    let invalid = Position {
        x: f32::NAN,
        ..player.position
    };
    assert!(world.body.correct(&mut player, invalid, now).is_err());
    assert_eq!(world.body.carrier_support(), Some(sample));
    let position = player.position;
    world.body.correct(&mut player, position, now).unwrap();
    assert_eq!(world.body.carrier_support(), None);
    world.body.record_carrier_support(Some(sample));
    world.body.suspend();
    assert_eq!(world.body.carrier_support(), None);
    world.body.record_carrier_support(Some(sample));
    world
        .body
        .admit(MotionSession::new(6, 7, position, now).unwrap());
    assert_eq!(world.body.carrier_support(), None);
}

#[test]
fn no_move_is_sent_while_an_arrival_aboard_is_held() {
    let start = Instant::now();
    let (mut world, sample) = setup(start);
    world.arrival = Some(super::super::riders::Pending::Aboard {
        sample,
        local: Position::default(),
    });
    let held = MovementRequest::new(
        MovementMode::Forward,
        5,
        world.body.position().unwrap(),
        start,
    );
    let outcome = testing::run(|out| Motion::step(&held, &mut world, out));
    outcome.result.unwrap();
    assert_eq!(outcome.sent, []);
    assert_eq!(outcome.unreliable, []);
    assert!(outcome
        .events
        .iter()
        .any(|event| matches!(event, ClientEvent::World(WorldEvent::MotionRefused { .. }))));
}
