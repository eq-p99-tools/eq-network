use super::*;
use crate::client::{ClientConfig, ServerProtocol};
use eq_network_game::{
    command::{self, EncodedCommand, Posture},
    hazards::LiquidObservation,
    movement::{MotionSession, MovementOptions, PostureOptions},
    world::PostureState,
};

fn seated_world(posture: PostureState, stay_seated: bool) -> World {
    let mut config = ClientConfig::new("Example", "Fixture", "Local", "Tester");
    config.local_only = true;
    config.protocol = ServerProtocol::EqEmu;
    let mut world = World::for_zone(5, &config);
    let player = testing::player(7);
    let now = Instant::now();
    let mut motion = MotionSession::new(5, 7, player.position, now).unwrap();
    motion
        .set_options(
            MovementOptions {
                posture: PostureOptions {
                    stay_seated_when_hit: stay_seated,
                    ..Default::default()
                },
                ..Default::default()
            },
            now,
        )
        .unwrap();
    world.body.admit(motion);
    world.own_spawn = Some(7);
    world.player.admit(player);
    world.admitted = Some(now);
    world.posture.observed(posture);
    world
}

fn enter(feature: &Hazards, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
    out.log.send(ClientEvent::World(WorldEvent::Entered {
        capabilities: feature.capabilities(),
        choices: Vec::new(),
        session_id: 5,
        zone: "example".into(),
        player: Box::new(world.player.as_ref().unwrap().clone()),
        far_clip: None,
        spell_landing_flag: None,
        zone_id: 0,
    }))
}

fn local_report(hazard: Hazard, amount: u32) -> ClientCommand {
    match hazard {
        Hazard::Falling => report(hazard, amount),
        Hazard::Drowning | Hazard::Lava => ClientCommand::LiquidDamage {
            session_id: 5,
            hazard: if hazard == Hazard::Drowning {
                LiquidHazard::Drowning
            } else {
                LiquidHazard::Lava
            },
            amount,
            observation: LiquidObservation {
                revision: 0,
                position: Position::default(),
                sampled_at: Instant::now(),
            },
        },
        Hazard::Freezing => unreachable!("this fixture reports only fall, drowning and lava"),
    }
}

fn all_hazards() -> Hazards {
    Hazards::new(&[Hazard::Falling, Hazard::Drowning, Hazard::Lava], false)
}

fn assert_no_damage_authority(events: &[ClientEvent]) {
    assert!(!events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::Damage(_) | WorldEvent::HitPoints { .. })
    )));
}

#[test]
fn local_fall_drowning_and_lava_stand_only_sitters_and_looters() {
    for hazard in [Hazard::Falling, Hazard::Drowning, Hazard::Lava] {
        for (posture, stands) in [
            (PostureState::Sitting, true),
            (PostureState::Looting, true),
            (PostureState::Lying, false),
            (PostureState::Ducking, false),
            (PostureState::Standing, false),
            (PostureState::Frozen, false),
        ] {
            let mut world = seated_world(posture, false);
            let mut feature = all_hazards();
            let outcome = testing::run(|out| -> Result<()> {
                enter(&feature, &world, out)?;
                feature.handle(&local_report(hazard, 13), &mut world, out)
            });
            outcome.result.unwrap();
            let mut expected = vec![hazards::titanium_damage(7, hazard, 13).unwrap()];
            if stands {
                expected.push(command::titanium_posture(7, Posture::Standing).unwrap());
            }
            assert_eq!(outcome.sent, expected, "{hazard:?}, {posture:?}");
            assert_eq!(
                world.posture.current(),
                Some(if stands {
                    PostureState::Standing
                } else {
                    posture
                }),
                "{hazard:?}, {posture:?}"
            );
            assert_no_damage_authority(&outcome.events);
        }
    }
}

#[test]
fn the_stay_seated_option_applies_to_every_local_report() {
    for hazard in [Hazard::Falling, Hazard::Drowning, Hazard::Lava] {
        for posture in [PostureState::Sitting, PostureState::Looting] {
            let mut world = seated_world(posture, true);
            let mut feature = all_hazards();
            let outcome = testing::run(|out| -> Result<()> {
                enter(&feature, &world, out)?;
                feature.handle(&local_report(hazard, 13), &mut world, out)
            });
            outcome.result.unwrap();
            assert_eq!(
                outcome.sent,
                [hazards::titanium_damage(7, hazard, 13).unwrap()]
            );
            assert_eq!(world.posture.current(), Some(posture));
            assert_no_damage_authority(&outcome.events);
        }
    }
}

#[test]
fn zero_unencodable_and_refused_reports_stand_no_one() {
    for command in [
        report(Hazard::Falling, 0),
        report(Hazard::Falling, u32::MAX),
        report(Hazard::Drowning, 13),
        report(Hazard::Lava, 13),
        local_report(Hazard::Drowning, 0),
        local_report(Hazard::Lava, 0),
        // Entered seeds revision 0; this observation names another revision.
        ClientCommand::LiquidDamage {
            session_id: 5,
            hazard: LiquidHazard::Lava,
            amount: 13,
            observation: LiquidObservation {
                revision: 1,
                position: Position::default(),
                sampled_at: Instant::now(),
            },
        },
    ] {
        let mut world = seated_world(PostureState::Sitting, false);
        let mut feature = all_hazards();
        let outcome = testing::run(|out| -> Result<()> {
            enter(&feature, &world, out)?;
            feature.handle(&command, &mut world, out)
        });
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty(), "{command:?}");
        assert_eq!(world.posture.current(), Some(PostureState::Sitting));
        assert_no_damage_authority(&outcome.events);
    }
    let mut world = seated_world(PostureState::Sitting, false);
    let mut unavailable = Hazards::new(&[], false);
    let outcome =
        testing::run(|out| unavailable.handle(&report(Hazard::Falling, 13), &mut world, out));
    outcome.result.unwrap();
    assert_eq!(outcome.sent, []);
    assert_eq!(world.posture.current(), Some(PostureState::Sitting));
}

#[test]
fn safe_fall_stands_only_when_damage_remains() {
    for amount in [0, 40] {
        let mut world = seated_world(PostureState::Sitting, false);
        let mut feature = all_hazards();
        let command = ClientCommand::EnvironmentalDamage {
            session_id: 5,
            hazard: Hazard::Falling,
            amount,
            safe_fall: true,
        };
        let outcome = testing::run(|out| feature.handle(&command, &mut world, out));
        outcome.result.unwrap();
        let mut expected = vec![hazards::safe_fall(true)];
        if amount > 0 {
            expected.extend([
                hazards::titanium_damage(7, Hazard::Falling, amount).unwrap(),
                command::titanium_posture(7, Posture::Standing).unwrap(),
            ]);
        }
        assert_eq!(outcome.sent, expected);
        assert_eq!(
            world.posture.current(),
            Some(if amount > 0 {
                PostureState::Standing
            } else {
                PostureState::Sitting
            })
        );
        assert_no_damage_authority(&outcome.events);
    }
}

#[test]
fn a_repeated_liquid_sample_cannot_stand_a_player_who_sat_again() {
    let mut world = seated_world(PostureState::Sitting, false);
    let mut feature = all_hazards();
    let mut command = local_report(Hazard::Lava, 13);
    let outcome = testing::run(|out| -> Result<()> {
        enter(&feature, &world, out)?;
        feature.handle(&command, &mut world, out)?;
        assert_eq!(world.posture.current(), Some(PostureState::Standing));
        world.posture.set(7, Posture::Sitting, out)?;
        // Standing and sitting advance the input revision. Keep the timestamp
        // but name the current revision to exercise the no-new-tick refusal.
        if let ClientCommand::LiquidDamage { observation, .. } = &mut command {
            observation.revision = 2;
        }
        feature.handle(&command, &mut world, out)
    });
    outcome.result.unwrap();
    assert_eq!(
        outcome.sent,
        [
            hazards::titanium_damage(7, Hazard::Lava, 13).unwrap(),
            command::titanium_posture(7, Posture::Standing).unwrap(),
            command::titanium_posture(7, Posture::Sitting).unwrap(),
        ]
    );
    assert_eq!(world.posture.current(), Some(PostureState::Sitting));
    assert!(outcome.events.iter().any(|event| matches!(
        event,
        ClientEvent::Diagnostic(line) if line.contains("no new damage")
    )));
}

struct FailsAt {
    sent: Vec<EncodedCommand>,
    fail_at: usize,
}

impl super::super::super::feature::Sink for FailsAt {
    fn send(&mut self, packet: &EncodedCommand) -> Result<()> {
        anyhow::ensure!(self.sent.len() != self.fail_at, "synthetic send failure");
        self.sent.push(packet.clone());
        Ok(())
    }

    fn send_unreliable(&mut self, _: &EncodedCommand) -> Result<()> {
        anyhow::bail!("unexpected unreliable hazard send")
    }

    fn last_received_seconds(&self) -> u64 {
        0
    }
}

#[test]
fn failed_safe_fall_damage_or_posture_submission_keeps_the_sent_prefix_explicit() {
    let expected = [
        hazards::safe_fall(true),
        hazards::titanium_damage(7, Hazard::Falling, 40).unwrap(),
        command::titanium_posture(7, Posture::Standing).unwrap(),
    ];
    for fail_at in 0..expected.len() {
        let mut world = seated_world(PostureState::Sitting, false);
        let mut feature = all_hazards();
        let command = ClientCommand::EnvironmentalDamage {
            session_id: 5,
            hazard: Hazard::Falling,
            amount: 40,
            safe_fall: true,
        };
        let mut sink = FailsAt {
            sent: Vec::new(),
            fail_at,
        };
        let outcome = testing::run(|out| {
            let mut failing = Out {
                sink: &mut sink,
                log: &mut *out.log,
                wire: out.wire,
                sender: out.sender,
            };
            feature.handle(&command, &mut world, &mut failing)
        });
        assert!(outcome.result.is_err());
        assert_eq!(sink.sent, expected[..fail_at]);
        assert_eq!(world.posture.current(), Some(PostureState::Sitting));
        assert!(outcome.events.is_empty());
    }
}

#[test]
fn local_fall_reports_add_no_unread_mac_posture_rule() {
    for posture in [
        PostureState::Sitting,
        PostureState::Looting,
        PostureState::Lying,
    ] {
        let mut world = seated_world(posture, false);
        let mut feature = Hazards::new(&[Hazard::Falling], true);
        let outcome = testing::run_on(&super::super::super::wire::EqMac, |out| {
            feature.handle(&report(Hazard::Falling, 40), &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [
                hazards::eqmac_notice(7, Hazard::Falling, 40).unwrap(),
                hazards::eqmac_damage(7, Hazard::Falling, 40).unwrap(),
            ]
        );
        assert_eq!(world.posture.current(), Some(posture));
        assert_no_damage_authority(&outcome.events);
    }
}
