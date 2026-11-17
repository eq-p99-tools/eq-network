use super::{super::feature::testing, *};
use crate::client::{ClientConfig, ServerProtocol};
use eq_network_game::{
    combat::{Damage, DamageOutcome},
    command::{self, Posture},
    hazards::{self, Hazard, LiquidHazard, LiquidObservation},
    world::PostureState,
};

fn fixture() -> (World, Features) {
    let mut config = ClientConfig::new("Example", "Fixture", "Local", "Tester");
    config.local_only = true;
    config.protocol = ServerProtocol::EqEmu;
    let mut world = World::for_zone(5, &config);
    world.own_spawn = Some(7);
    world.player.admit(testing::player(7));
    world.admitted = Some(Instant::now());
    world.posture.observed(PostureState::Sitting);
    let features = Features {
        provided: vec![
            Box::new(super::super::posture::Stances),
            Box::new(super::super::hazards::Hazards::new(
                &[Hazard::Falling, Hazard::Drowning, Hazard::Lava],
                false,
            )),
        ],
        choices: Vec::new(),
        spell_landing_flag: None,
        answers_list_polls: false,
        gameplay_departures: false,
    };
    (world, features)
}

fn enter(world: &World, features: &Features, out: &mut Out<'_, '_>) -> Result<()> {
    out.log.send(ClientEvent::World(WorldEvent::Entered {
        capabilities: features.capabilities(),
        choices: Vec::new(),
        session_id: 5,
        zone: "example".into(),
        player: Box::new(world.player.as_ref().unwrap().clone()),
        far_clip: None,
        spell_landing_flag: None,
        zone_id: 0,
    }))
}

fn report(hazard: Hazard, session_id: u64) -> ClientCommand {
    match hazard {
        Hazard::Falling => ClientCommand::EnvironmentalDamage {
            session_id,
            hazard,
            amount: 13,
            safe_fall: false,
        },
        Hazard::Drowning | Hazard::Lava => ClientCommand::LiquidDamage {
            session_id,
            hazard: if hazard == Hazard::Drowning {
                LiquidHazard::Drowning
            } else {
                LiquidHazard::Lava
            },
            amount: 13,
            observation: LiquidObservation {
                revision: 0,
                position: eq_network_game::world::Position::default(),
                sampled_at: Instant::now(),
            },
        },
        Hazard::Freezing => unreachable!("this fixture reports only fall, drowning and lava"),
    }
}

#[test]
fn stale_admission_and_unready_local_reports_never_stand_the_player() {
    for hazard in [Hazard::Falling, Hazard::Drowning, Hazard::Lava] {
        for (ready, session_id) in [(true, 4), (false, 5)] {
            let (mut world, mut features) = fixture();
            if !ready {
                world.admitted = None;
            }
            let outcome = testing::run(|out| -> Result<()> {
                enter(&world, &features, out)?;
                assert!(apply_commands(
                    &[report(hazard, session_id)],
                    &mut features,
                    &mut world,
                    out,
                    true,
                )?
                .is_none());
                Ok(())
            });
            outcome.result.unwrap();
            assert!(outcome.sent.is_empty(), "{hazard:?}, ready={ready}");
            assert_eq!(world.posture.current(), Some(PostureState::Sitting));
            assert!(!outcome.events.iter().any(|event| matches!(
                event,
                ClientEvent::World(
                    WorldEvent::Posture { .. }
                        | WorldEvent::Damage(_)
                        | WorldEvent::HitPoints { .. }
                )
            )));
        }
    }
}

#[test]
fn an_accepted_local_report_and_repeated_server_echo_send_only_one_stand() {
    for hazard in [Hazard::Falling, Hazard::Drowning, Hazard::Lava] {
        let (mut world, mut features) = fixture();
        let echo = || {
            Message::Event(WorldEvent::Damage(Damage {
                target_id: 7,
                source_id: 0,
                kind: 231,
                spell_id: None,
                outcome: DamageOutcome::Hit(13),
                push: None,
            }))
        };
        let outcome = testing::run(|out| -> Result<()> {
            enter(&world, &features, out)?;
            assert!(
                apply_commands(&[report(hazard, 5)], &mut features, &mut world, out, true,)?
                    .is_none()
            );
            hear(vec![echo(), echo()], &mut features, &mut world, out, true)?;
            Ok(())
        });
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [
                hazards::titanium_damage(7, hazard, 13).unwrap(),
                command::titanium_posture(7, Posture::Standing).unwrap(),
            ]
        );
        assert_eq!(world.posture.current(), Some(PostureState::Standing));
        assert_eq!(
            outcome
                .events
                .iter()
                .filter(|event| matches!(event, ClientEvent::World(WorldEvent::Damage(_))))
                .count(),
            2,
            "only the two received damage events reach the host"
        );
        assert!(!outcome
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::HitPoints { .. }))));
    }
}
