//! Following a tracked spawn through the zone session's real features: a
//! zone change ends it with the session, as the program's does (read
//! R49-30 c, 6.636).
use super::super::feature::testing;
use super::*;
use crate::client::ServerProtocol;
use eq_network_game::{
    food::AutoEat,
    tracking::{TrackAction, TrackingUpdate},
    world::{SpawnKind, WorldEvent},
};

/// A stock `EQEmu` zone session's features and world, with spawn 42 in view.
fn zone() -> (Features, World) {
    let server = servers::server_type(ServerProtocol::EqEmu);
    let features = Features::new(server, "Synthetic", AutoEat::default());
    let mut world = World::new(5);
    world.admitted = Some(Instant::now());
    world.own_spawn = Some(7);
    world.player.admit(testing::player(7));
    let mut spawn = testing::spawn(42, SpawnKind::Npc);
    spawn.position.y = 10.0;
    world.spawns.insert(spawn);
    (features, world)
}

fn tracking(events: &[ClientEvent]) -> Vec<TrackingUpdate> {
    events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::World(WorldEvent::Tracking(update)) => Some(update.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_zone_change_ends_following_with_the_zone_session() {
    let (mut features, mut world) = zone();
    let follow = ClientCommand::Track {
        session_id: 5,
        action: TrackAction::Follow(42),
    };
    testing::run(|out| {
        apply_commands(&[follow], &mut features, &mut world, out, true)
            .map(|exit| assert!(exit.is_none()))
    })
    .result
    .unwrap();
    let outcome = testing::run(|out| features.tick(Instant::now(), &mut world, out));
    outcome.result.unwrap();
    assert!(matches!(
        tracking(&outcome.events)[..],
        [TrackingUpdate::Bearing { .. }]
    ));
    // The next zone's session builds its features afresh: nothing is
    // followed there, and no line says so, as the program's zoning clears
    // it silently.
    let (mut features, mut world) = zone();
    let outcome = testing::run(|out| features.tick(Instant::now(), &mut world, out));
    outcome.result.unwrap();
    assert_eq!(tracking(&outcome.events), []);
}
