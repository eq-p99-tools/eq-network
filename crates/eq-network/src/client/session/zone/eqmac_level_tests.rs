//! Synthetic bytes through the production decoder, session and AA gate.
use super::*;
use crate::client::{
    session::{feature::testing, wire},
    ServerProtocol,
};
use eq_network_game::{food::AutoEat, listing::ListingChange, world::WorldEvent};

fn features() -> Features {
    let mut setup = servers::Setup::new("Tester", AutoEat::default());
    setup.local_only = true;
    Features::from_setup(servers::server_type(ServerProtocol::Takp), &setup)
}

fn profile(level: u8, points: u32) -> crate::world::PlayerState {
    let mut player = testing::player(7);
    player.level = level;
    player.practice_points = Some(points);
    player
}

fn enter(
    features: &mut Features,
    world: &mut World,
    level: u8,
    points: u32,
    out: &mut Out<'_, '_>,
) -> Result<()> {
    world.own_spawn = Some(7);
    admit(
        profile(level, points),
        &Zone {
            name: "qeynos".into(),
            far_clip: None,
            sky: None,
            description: None,
        },
        features,
        world,
        out,
    )
}

fn level(current: u32, previous: u32, experience: u32) -> Vec<Message> {
    let body: Vec<_> = [current, previous, experience]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    eq_network_game::message::eqmac(0x9841, &body)
}

fn appearance(spawn_id: u16, value: u32) -> Vec<Message> {
    let mut body = spawn_id.to_le_bytes().to_vec();
    body.extend(1_u16.to_le_bytes());
    body.extend(value.to_le_bytes());
    eq_network_game::message::eqmac(eq_network_game::quarm::ZONE_SPAWN_APPEARANCE, &body)
}

fn split(percentage: u8) -> ClientCommand {
    ClientCommand::SetAlternateAdvancementSplit {
        session_id: 5,
        percentage,
    }
}

#[test]
fn eqmac_level_rise_unlocks_the_ordinary_aa_split_without_reentry() {
    let (mut features, mut world) = (features(), World::new(5));
    let outcome = testing::run_on(&wire::EqMac, |out| -> Result<()> {
        enter(&mut features, &mut world, 50, 3, out)?;
        hear(
            vec![Message::Event(WorldEvent::Spawns(vec![testing::spawn(
                7,
                crate::world::SpawnKind::Player,
            )]))],
            &mut features,
            &mut world,
            out,
            true,
        )?;
        assert!(features.handle(&split(50), &mut world, out)?);
        hear(level(51, 50, 123), &mut features, &mut world, out, true)?;
        assert_eq!(world.player.as_ref().unwrap().level, 51);
        assert_eq!(world.spawns.get(7).unwrap().level, 51);
        assert!(features.handle(&split(50), &mut world, out)?);
        Ok(())
    });
    outcome.result.unwrap();
    assert!(outcome.events.iter().any(|e| matches!(
        e,
        ClientEvent::World(WorldEvent::AlternateAdvancementRefused {
            refusal: Some(eq_network_game::alternate_advancement::Refusal::LevelTooLow),
            ..
        })
    )));
    assert!(outcome
        .events
        .iter()
        .any(|e| matches!(e, ClientEvent::World(WorldEvent::PracticePoints(8)))));
    let sent: Vec<_> = outcome.sent.iter().filter(|p| p.opcode == 0x1442).collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(&sent[0].body[..6], b"on 50\0");
}

#[test]
fn eqmac_level_loss_and_own_appearance_do_not_lower_the_character_gate() {
    let (mut features, mut world) = (features(), World::new(5));
    let outcome = testing::run_on(&wire::EqMac, |out| -> Result<()> {
        enter(&mut features, &mut world, 55, 275, out)?;
        hear(level(1, 55, 0), &mut features, &mut world, out, true)?;
        hear(level(55, 1, 999), &mut features, &mut world, out, true)?;
        hear(
            vec![Message::Event(WorldEvent::Spawns(vec![testing::spawn(
                8,
                crate::world::SpawnKind::Player,
            )]))],
            &mut features,
            &mut world,
            out,
            true,
        )?;
        hear(appearance(8, 2), &mut features, &mut world, out, true)?;
        assert_eq!(world.spawns.get(8).unwrap().level, 2);
        hear(appearance(7, 1), &mut features, &mut world, out, true)?;
        assert_eq!(world.player.as_ref().unwrap().level, 55);
        assert!(features.handle(&split(50), &mut world, out)?);
        Ok(())
    });
    outcome.result.unwrap();
    assert!(!outcome.events.iter().any(|e| matches!(
        e,
        ClientEvent::World(WorldEvent::Level { .. } | WorldEvent::PracticePoints(_))
    )));
    assert!(outcome.events.iter().any(|e| matches!(
        e,
        ClientEvent::World(WorldEvent::Listing {
            spawn_id: 7,
            change: ListingChange::Level(1)
        })
    )));
    assert_eq!(
        outcome.sent.iter().filter(|p| p.opcode == 0x1442).count(),
        1
    );
}

#[test]
fn eqmac_level_high_water_recovery_and_multi_level_jump_add_at_most_five() {
    for (previous, expected) in [(55, 275), (1, 280)] {
        let (mut features, mut world) = (features(), World::new(5));
        let outcome = testing::run_on(&wire::EqMac, |out| -> Result<()> {
            enter(&mut features, &mut world, 1, 275, out)?;
            hear(
                level(55, previous, 777),
                &mut features,
                &mut world,
                out,
                true,
            )?;
            hear(level(55, previous, 1), &mut features, &mut world, out, true)?;
            assert_eq!(world.player.as_ref().unwrap().level, 55);
            Ok(())
        });
        outcome.result.unwrap();
        let points: Vec<_> = outcome
            .events
            .iter()
            .filter_map(|e| match e {
                ClientEvent::World(WorldEvent::PracticePoints(points)) => Some(*points),
                _ => None,
            })
            .collect();
        assert_eq!(points, [expected]);
        assert_eq!(
            outcome
                .events
                .iter()
                .filter(|e| matches!(
                    e,
                    ClientEvent::World(WorldEvent::Level {
                        experience: 777,
                        ..
                    })
                ))
                .count(),
            1
        );
    }
}

#[test]
fn eqmac_level_staging_uses_the_new_profile_and_resets_on_the_next_admission() {
    let (mut features, mut world) = (features(), World::new(5));
    let outcome = testing::run_on(&wire::EqMac, |out| -> Result<()> {
        world.own_spawn = Some(7);
        hear(
            vec![Message::Event(WorldEvent::Spawns(vec![testing::spawn(
                7,
                crate::world::SpawnKind::Player,
            )]))],
            &mut features,
            &mut world,
            out,
            true,
        )?;
        for messages in [
            level(51, 50, 100),
            level(51, 50, 101),
            level(53, 51, 120),
            level(1, 55, 0),
            vec![Message::Event(WorldEvent::Experience(250))],
        ] {
            hear(messages, &mut features, &mut world, out, true)?;
        }
        enter(&mut features, &mut world, 50, 275, out)?;
        assert_eq!(world.player.as_ref().unwrap().level, 53);
        assert_eq!(world.spawns.get(7).unwrap().level, 53);
        assert_eq!(world.player.as_ref().unwrap().practice_points, Some(285));
        Ok(())
    });
    outcome.result.unwrap();
    assert!(outcome
        .events
        .iter()
        .any(|e| matches!(e, ClientEvent::World(WorldEvent::Experience(250)))));
    let mut next = World::new(6);
    let outcome = testing::run_on(&wire::EqMac, |out| {
        enter(&mut features, &mut next, 1, 4, out)
    });
    outcome.result.unwrap();
    assert_eq!(next.player.as_ref().unwrap().level, 1);
    assert_eq!(next.player.as_ref().unwrap().practice_points, Some(4));
    assert!(!outcome.events.iter().any(|e| matches!(
        e,
        ClientEvent::World(WorldEvent::Experience(_) | WorldEvent::Level { .. })
    )));
}

#[test]
fn eqmac_level_profile_below_the_aa_floor_still_allows_turning_the_split_off() {
    let (mut features, mut world) = (features(), World::new(5));
    let outcome = testing::run_on(&wire::EqMac, |out| -> Result<()> {
        enter(&mut features, &mut world, 1, 4, out)?;
        assert!(features.handle(&split(0), &mut world, out)?);
        Ok(())
    });
    outcome.result.unwrap();
    let sent: Vec<_> = outcome.sent.iter().filter(|p| p.opcode == 0x1442).collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(&sent[0].body[..4], b"off\0");
}
