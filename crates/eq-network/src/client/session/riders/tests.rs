use super::*;
use crate::client::session::{feature::testing, motion::Motion};
use eq_network_game::{
    boats::riders::{titanium_board, titanium_leave, titanium_missing, Transfer},
    world::SpawnKind,
    zoning::{self, Death},
    GameDialect,
};
use std::sync::Arc;

const BOAT: &str = "SirensBane00";

/// The player, spawn 7, admitted at the origin.
fn admitted(out: &mut Out<'_, '_>) -> World {
    admitted_at(out, Position::default())
}

/// The player, spawn 7, admitted where the profile put them.
fn admitted_at(out: &mut Out<'_, '_>, position: Position) -> World {
    let mut world = World::new(5);
    world.own_spawn = Some(7);
    let mut player = testing::player(7);
    player.position = position;
    world.player.admit(player);
    world.admitted = Some(Instant::now());
    Motion::new(false, false, None)
        .admitted(&mut world, out)
        .unwrap();
    world
}

/// A boat of this name at `position`, as the zone's spawns report it.
fn boat(world: &mut World, spawn_id: u16, name: &str, position: Position) -> CarrierSample {
    let mut boat = testing::spawn(spawn_id, SpawnKind::Npc);
    boat.race = 72;
    boat.name = name.into();
    boat.position = position;
    world.spawns.insert(boat.clone());
    world.carriers.observe(
        &WorldEvent::Spawns(vec![boat]),
        GameDialect::Titanium,
        Instant::now(),
    );
    world.carriers.latest(spawn_id).unwrap().sample()
}

fn transfer(name: &str) -> Message {
    Message::VehicleTransfer(Transfer {
        name: name.into(),
        zone_id: 69,
    })
}

fn placement(position: Option<Position>) -> ClientCommand {
    ClientCommand::ArrivalPlacement {
        session_id: 5,
        position,
        created: Instant::now(),
    }
}

fn arrivals(events: &[ClientEvent]) -> Vec<&Arrival> {
    events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::World(WorldEvent::Arrival { arrival, .. }) => Some(arrival),
            _ => None,
        })
        .collect()
}

fn diagnostics(events: &[ClientEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, ClientEvent::Diagnostic(_)))
        .count()
}

#[test]
fn the_first_linked_tick_boards_and_two_unlinked_ticks_leave() {
    let riding = Arc::new(Riding::default());
    let outcome = testing::run(|out| {
        let mut world = admitted(out);
        world.riding = Arc::clone(&riding);
        let sample = boat(&mut world, 8, BOAT, Position::default());
        let mut riders = Riders::default();
        riders.admitted(&mut world, out).unwrap();
        let start = Instant::now() + TICK;
        world.body.record_carrier_support(Some(sample));
        riders.tick(start, &mut world, out).unwrap();
        assert_eq!(world.vehicle.as_deref(), Some(BOAT));
        // Within the second, and on the next linked tick, nothing more.
        riders
            .tick(start + Duration::from_millis(500), &mut world, out)
            .unwrap();
        riders.tick(start + TICK, &mut world, out).unwrap();
        world.body.record_carrier_support(None);
        riders.tick(start + TICK * 2, &mut world, out).unwrap();
        // One unlinked tick keeps the name, as a jump on deck does.
        assert_eq!(world.vehicle.as_deref(), Some(BOAT));
        riders.tick(start + TICK * 3, &mut world, out).unwrap();
        assert_eq!(world.vehicle, None);
        riders.tick(start + TICK * 4, &mut world, out).unwrap();
    });
    assert_eq!(
        outcome.sent,
        [titanium_board(BOAT).unwrap(), titanium_leave()]
    );
    assert!(!riding.aboard.load(Ordering::Relaxed));
}

#[test]
fn a_boarding_kept_across_zones_leaves_once_no_boat_is_underfoot() {
    // The last word was a boarding in the zone before; this zone's floor
    // links nothing.
    let riding = Arc::new(Riding {
        aboard: AtomicBool::new(true),
        ..Riding::default()
    });
    let outcome = testing::run(|out| {
        let mut world = admitted(out);
        world.riding = Arc::clone(&riding);
        let mut riders = Riders::default();
        riders.admitted(&mut world, out).unwrap();
        let start = Instant::now() + TICK;
        for tick in 0..4 {
            riders.tick(start + TICK * tick, &mut world, out).unwrap();
        }
    });
    assert_eq!(outcome.sent, [titanium_leave()]);
    assert!(!riding.aboard.load(Ordering::Relaxed));
}

#[test]
fn a_nameless_carrier_names_nothing() {
    let outcome = testing::run(|out| {
        let mut world = admitted(out);
        let sample = boat(&mut world, 8, "", Position::default());
        world.body.record_carrier_support(Some(sample));
        let mut riders = Riders::default();
        riders.tick(Instant::now(), &mut world, out).unwrap();
        assert_eq!(world.vehicle, None);
    });
    assert_eq!(outcome.sent, []);
}

#[test]
fn only_a_transfer_naming_the_saved_boat_zones_with_it_and_only_once() {
    let outcome = testing::run(|out| {
        let mut world = admitted(out);
        let mut riders = Riders::default();
        // No boat saved: the program does nothing.
        riders.observe(&transfer(BOAT), &mut world, out).unwrap();
        world.vehicle = Some(BOAT.into());
        riders
            .observe(&transfer("Stormbreaker"), &mut world, out)
            .unwrap();
        assert!(!world.lifecycle.blocks_motion());
        // The name matches once digits and case are set aside.
        riders
            .observe(&transfer("sirensbane"), &mut world, out)
            .unwrap();
        assert!(world.lifecycle.blocks_motion());
        // Mid-zoning, a second transfer changes nothing.
        riders.observe(&transfer(BOAT), &mut world, out).unwrap();
    });
    let position = Position::default();
    assert_eq!(
        outcome.sent,
        [zoning::titanium_answer("Tester", (69, 0), position, riders::ZONE_REASON).unwrap()]
    );
}

#[test]
fn a_transfer_before_admission_is_not_taken() {
    let outcome = testing::run(|out| {
        let mut world = World::new(5);
        world.vehicle = Some(BOAT.into());
        Riders::default()
            .observe(&transfer(BOAT), &mut world, out)
            .unwrap();
    });
    assert_eq!(outcome.sent, []);
}

#[test]
fn a_profile_without_a_vehicle_arrives_as_ever() {
    let here = Position {
        x: 1.0,
        y: 2.0,
        z: 3.0,
        heading: 40.0,
    };
    let outcome = testing::run(|out| {
        let mut world = admitted_at(out, here);
        // Even with a departure z kept, an ordinary admission decodes as
        // ever: the loader's quirk applies only to a restore aboard.
        world.riding.depart_at(Some(-7.5));
        let mut riders = Riders::default();
        riders
            .admit(&Message::VehicleName(None), &mut world)
            .unwrap();
        riders.admitted(&mut world, out).unwrap();
        assert!(world.arrival.is_none() && world.vehicle.is_none());
        assert!(!world.motion_held());
        assert_eq!(world.body.position(), Some(here));
    });
    assert_eq!(outcome.sent, []);
    assert_eq!(arrivals(&outcome.events), Vec::<&Arrival>::new());
}

/// The boat at (100, 50, 0) facing 128, and the profile's place on its deck.
const DECK: Position = Position {
    x: 4.0,
    y: -2.0,
    z: 3.0,
    heading: 10.0,
};
const AT: Position = Position {
    x: 100.0,
    y: 50.0,
    z: 0.0,
    heading: 128.0,
};

/// An arrival whose profile names a boat that was in at the profile.
/// The profile's position fields that the program restores to [`DECK`],
/// with the departure's z: its loader keeps y and z as x and y.
const PROFILE: Position = Position {
    x: 99.0,
    y: DECK.x,
    z: DECK.y,
    heading: DECK.heading,
};

fn aboard(out: &mut Out<'_, '_>, riders: &mut Riders) -> (World, CarrierSample) {
    let mut world = admitted_at(out, PROFILE);
    world.riding.depart_at(Some(DECK.z));
    world.safe_point = Some(Position {
        x: -5.0,
        y: 6.0,
        z: 7.0,
        heading: 0.0,
    });
    let sample = boat(&mut world, 8, "SirensBane02", AT);
    riders
        .admit(&Message::VehicleName(Some(BOAT.into())), &mut world)
        .unwrap();
    riders.admitted(&mut world, out).unwrap();
    (world, sample)
}

#[test]
fn a_boat_in_at_the_profile_holds_the_player_for_one_placement_on_it() {
    let mut riders = Riders::default();
    let outcome = testing::run(|out| {
        let (mut world, sample) = aboard(out, &mut riders);
        assert!(matches!(
            world.arrival,
            Some(Pending::Aboard { sample: held, local, .. }) if held == sample && local == DECK
        ));
        assert!(world.motion_held());
        // The host swept to the deck a little below the restored point.
        let restored = carrier_world(AT, DECK);
        let swept = Position {
            z: restored.z - 0.75,
            ..restored
        };
        riders
            .handle(&placement(Some(swept)), &mut world, out)
            .unwrap();
        assert!(world.arrival.is_none() && !world.motion_held());
        assert_eq!(world.vehicle.as_deref(), Some(BOAT));
        assert_eq!(world.body.position(), Some(swept));
        // The grant is spent: a second placement is refused.
        riders
            .handle(&placement(Some(DECK)), &mut world, out)
            .unwrap();
        assert_eq!(world.body.position(), Some(swept));
        swept
    });
    let swept = outcome.result;
    assert_eq!(outcome.sent, []);
    let arrivals = arrivals(&outcome.events);
    assert!(
        matches!(arrivals[0], Arrival::Aboard { name, profile, local, .. }
        if name == BOAT && *profile == PROFILE && *local == DECK)
    );
    assert_eq!(arrivals[1], &Arrival::Placed { position: swept });
    assert_eq!(arrivals.len(), 2);
    assert_eq!(diagnostics(&outcome.events), 1);
}

#[test]
fn the_retry_point_is_a_placement_too() {
    let mut riders = Riders::default();
    let outcome = testing::run(|out| {
        let (mut world, _) = aboard(out, &mut riders);
        let retry = carrier_world(
            AT,
            Position {
                x: RETRY[0],
                y: RETRY[1],
                z: RETRY[2],
                heading: DECK.heading,
            },
        );
        riders
            .handle(&placement(Some(retry)), &mut world, out)
            .unwrap();
        world.body.position()
    });
    assert!(matches!(
        arrivals(&outcome.events).last(),
        Some(Arrival::Placed { position }) if Some(*position) == outcome.result
    ));
}

#[test]
fn a_placement_away_from_the_boat_or_none_goes_to_the_safe_point() {
    for offered in [
        Some(Position {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            heading: 0.0,
        }),
        // Above where the sweep starts.
        Some(Position {
            z: carrier_world(AT, DECK).z + 2.0,
            ..carrier_world(AT, DECK)
        }),
        None,
    ] {
        let mut riders = Riders::default();
        let outcome = testing::run(|out| {
            let (mut world, _) = aboard(out, &mut riders);
            riders.handle(&placement(offered), &mut world, out).unwrap();
            assert!(world.arrival.is_none() && world.vehicle.is_none());
            world.body.position()
        });
        let safe = Position {
            x: -5.0,
            y: 6.0,
            z: 7.0,
            heading: 0.0,
        };
        assert_eq!(outcome.result, Some(safe));
        assert_eq!(
            arrivals(&outcome.events).last(),
            Some(&&Arrival::SafePoint { position: safe })
        );
        assert_eq!(outcome.sent, []);
    }
}

#[test]
fn a_missing_boat_is_asked_for_and_the_wait_ends_at_the_safe_point() {
    let outcome = testing::run(|out| {
        let mut world = admitted(out);
        world.safe_point = Some(Position::default());
        let mut riders = Riders::default();
        // The boat comes in only after the profile: the one lookup missed it.
        riders
            .admit(&Message::VehicleName(Some(BOAT.into())), &mut world)
            .unwrap();
        boat(&mut world, 8, BOAT, AT);
        riders.admitted(&mut world, out).unwrap();
        let Some(Pending::Waiting { until }) = world.arrival else {
            panic!("the arrival waits");
        };
        assert!(world.motion_held());
        // No boat word while held, and nothing before the wait runs out.
        world
            .body
            .record_carrier_support(Some(world.carriers.latest(8).unwrap().sample()));
        riders
            .tick(
                until.checked_sub(Duration::from_secs(1)).unwrap(),
                &mut world,
                out,
            )
            .unwrap();
        assert!(world.motion_held());
        riders.tick(until, &mut world, out).unwrap();
        assert!(world.arrival.is_none() && world.vehicle.is_none());
    });
    assert_eq!(outcome.sent, [titanium_missing(BOAT).unwrap()]);
    let arrivals = arrivals(&outcome.events);
    assert_eq!(
        arrivals,
        [
            &Arrival::Waiting { name: BOAT.into() },
            &Arrival::SafePoint {
                position: Position::default()
            }
        ]
    );
}

#[test]
fn the_server_carrying_a_waiting_rider_on_zones_them_with_the_boat_and_no_save() {
    let outcome = testing::run(|out| {
        let mut world = admitted(out);
        let mut riders = Riders::default();
        riders
            .admit(&Message::VehicleName(Some(BOAT.into())), &mut world)
            .unwrap();
        riders.admitted(&mut world, out).unwrap();
        riders.observe(&transfer(BOAT), &mut world, out).unwrap();
        assert!(world.arrival.is_none());
        assert!(world.lifecycle.blocks_motion());
        assert_eq!(world.vehicle.as_deref(), Some(BOAT));
        // The zone approves: the departure goes, with no save before it,
        // and the boat's name is kept for the next zone.
        let mut answer = world
            .lifecycle
            .pending()
            .unwrap()
            .response("Tester")
            .unwrap()
            .body;
        answer[84..88].copy_from_slice(&1i32.to_le_bytes());
        let answer = zoning::ZoneAnswer::titanium(&answer).unwrap();
        Transfers::new("Tester", super::super::transfers::Home::Offered)
            .observe(&Message::ZoneAnswer(answer), &mut world, out)
            .unwrap();
        assert!(world.lifecycle.departing());
        assert_eq!(world.vehicle.as_deref(), Some(BOAT));
    });
    assert_eq!(
        outcome.sent,
        [
            titanium_missing(BOAT).unwrap(),
            zoning::titanium_answer("Tester", (69, 0), Position::default(), riders::ZONE_REASON)
                .unwrap(),
            zoning::titanium_depart(7),
        ]
    );
}

#[test]
fn a_transfer_to_a_zone_no_id_can_name_is_dropped_with_a_word() {
    let outcome = testing::run(|out| {
        let mut world = admitted(out);
        world.vehicle = Some(BOAT.into());
        let message = Message::VehicleTransfer(Transfer {
            name: BOAT.into(),
            zone_id: 70_000,
        });
        Riders::default().observe(&message, &mut world, out)?;
        assert!(!world.lifecycle.blocks_motion());
        anyhow::Ok(())
    });
    outcome.result.unwrap();
    assert_eq!(outcome.sent, []);
    assert_eq!(diagnostics(&outcome.events), 1);
}

#[test]
fn death_forgets_the_boat_and_the_arrival() {
    let mut riders = Riders::default();
    let outcome = testing::run(|out| {
        let (mut world, _) = aboard(out, &mut riders);
        let died = Message::Event(WorldEvent::Death(Death {
            spawn_id: 7,
            killer_id: 0,
            corpse_id: 9,
            bind_zone_id: 0,
            corpse_name: None,
        }));
        riders.observe(&died, &mut world, out).unwrap();
        assert!(world.arrival.is_none() && world.vehicle.is_none());
        // A placement after death has no grant to spend.
        riders
            .handle(&placement(Some(carrier_world(AT, DECK))), &mut world, out)
            .unwrap();
        assert_eq!(world.body.position(), Some(PROFILE));
    });
    assert_eq!(diagnostics(&outcome.events), 1);
}

#[test]
fn a_host_that_never_places_the_player_aboard_ends_at_the_safe_point() {
    let mut riders = Riders::default();
    let outcome = testing::run(|out| {
        let (mut world, _) = aboard(out, &mut riders);
        let Some(Pending::Aboard { until, .. }) = world.arrival else {
            panic!("the arrival is held aboard");
        };
        riders
            .tick(
                until.checked_sub(Duration::from_secs(1)).unwrap(),
                &mut world,
                out,
            )
            .unwrap();
        assert!(world.motion_held());
        riders.tick(until, &mut world, out).unwrap();
        assert!(world.arrival.is_none() && world.vehicle.is_none());
        world.body.position()
    });
    assert_eq!(
        outcome.result,
        Some(Position {
            x: -5.0,
            y: 6.0,
            z: 7.0,
            heading: 0.0,
        })
    );
    assert_eq!(outcome.sent, []);
}

#[test]
fn a_refused_carry_on_from_the_wait_ends_at_the_safe_point() {
    let safe = Position {
        x: -5.0,
        y: 6.0,
        z: 7.0,
        heading: 0.0,
    };
    let outcome = testing::run(|out| {
        let mut world = admitted(out);
        world.safe_point = Some(safe);
        let mut riders = Riders::default();
        riders
            .admit(&Message::VehicleName(Some(BOAT.into())), &mut world)
            .unwrap();
        riders.admitted(&mut world, out).unwrap();
        riders.observe(&transfer(BOAT), &mut world, out).unwrap();
        // The zone refuses the request.
        let answer = world
            .lifecycle
            .pending()
            .unwrap()
            .response("Tester")
            .unwrap()
            .body;
        let answer = zoning::ZoneAnswer::titanium(&answer).unwrap();
        Transfers::new("Tester", super::super::transfers::Home::Offered)
            .observe(&Message::ZoneAnswer(answer), &mut world, out)
            .unwrap();
        assert!(!world.lifecycle.blocks_motion() && !world.motion_held());
        assert_eq!(world.vehicle, None);
        assert_eq!(world.carried_from, CarriedFrom::Elsewhere);
        world.body.position()
    });
    assert_eq!(outcome.result, Some(safe));
    assert_eq!(
        arrivals(&outcome.events).last(),
        Some(&&Arrival::SafePoint { position: safe })
    );
}

#[test]
fn the_restore_keeps_the_profiles_y_and_z_and_the_departures_z() {
    let profile = Position {
        x: 1.0,
        y: 2.0,
        z: 3.0,
        heading: 40.0,
    };
    assert_eq!(
        restored(profile, Some(-7.5)),
        Position {
            x: 2.0,
            y: 3.0,
            z: -7.5,
            heading: 40.0,
        }
    );
    // No boat save this session: the record's third field is zero.
    assert_eq!(restored(profile, None).z, 0.0);
}

#[test]
fn the_departure_z_is_kept_by_a_boat_save_and_forgotten_with_the_boat() {
    let riding = Riding::default();
    riding.depart_at(Some(4.0));
    assert_eq!(riding.departure_z(), Some(4.0));
    let mut world = World::new(5);
    world.riding = Arc::new(riding);
    world.vehicle = Some(BOAT.into());
    world.forget_vehicle();
    assert_eq!(
        (world.vehicle.as_deref(), world.riding.departure_z()),
        (None, None)
    );
}
