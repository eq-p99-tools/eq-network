//! Both complete `EQMac` feature assemblies, with synthetic state and no sockets.
use super::super::{feature::testing, targeting::Target};
use super::*;
use crate::client::ServerProtocol;
use eq_network_game::{
    abilities::{eqmac, Ability},
    appearance::AppearanceChange,
    command::Posture,
    food::AutoEat,
    inventory::InventoryUpdate,
    world::{Capability, PostureState, SpawnKind, WorldEvent},
};

const SERVERS: [ServerProtocol; 2] = [ServerProtocol::Takp, ServerProtocol::Quarm];

fn setup(protocol: ServerProtocol, class: u32) -> (Features, World) {
    let server = servers::server_type(protocol);
    let mut features = Features::new(server, "Tester", AutoEat::default());
    let mut world = World::new(5);
    world.own_spawn = Some(7);
    world.spawns.insert(testing::spawn(7, SpawnKind::Player));
    world.spawns.insert(testing::spawn(42, SpawnKind::Npc));
    let mut player = testing::player(7);
    player.class = Some(class);
    player.race = 1;
    player.skills = Some(vec![100; 100]);
    let mut shield = testing::item(14);
    shield.rules.item_type = 8;
    let mut weapon = testing::item(13);
    weapon.rules.item_type = 2;
    features
        .admit(
            &Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(vec![
                shield, weapon,
            ]))),
            &mut world,
        )
        .unwrap();
    let output = testing::run_on(server.wire(), |out| {
        admit(
            Ok(player),
            &Zone {
                name: "SyntheticZone".into(),
                far_clip: None,
                sky: None,
                description: None,
            },
            &mut features,
            &mut world,
            out,
        )
    });
    output.result.unwrap();
    world.admitted = Some(Instant::now());
    assert!(output.events.iter().any(|event| matches!(event,
        ClientEvent::World(WorldEvent::Entered { capabilities, .. }) if capabilities.contains(&Capability::Abilities))));
    assert!(output.events.iter().any(|event| matches!(event,
        ClientEvent::World(WorldEvent::AbilitiesOffered(list)) if list == &eqmac::SUPPORTED)));
    world.target = Target::from(42);
    (features, world)
}

fn use_ability(
    protocol: ServerProtocol,
    features: &mut Features,
    world: &mut World,
    ability: Ability,
) -> testing::Outcome<Result<bool>> {
    testing::run_on(servers::server_type(protocol).wire(), |out| {
        features.handle(
            &ClientCommand::UseAbility {
                session_id: 5,
                ability,
                created: Instant::now(),
            },
            world,
            out,
        )
    })
}

fn receive(
    protocol: ServerProtocol,
    features: &mut Features,
    world: &mut World,
    opcode: u16,
    body: &[u8],
) -> testing::Outcome<Result<Option<ZoneExit>>> {
    let server = servers::server_type(protocol);
    let wire = server.wire();
    testing::run_on(wire, |out| {
        hear(
            wire.messages(opcode, body, server.item_rules()),
            features,
            world,
            out,
            true,
        )
    })
}

fn refused(output: testing::Outcome<Result<bool>>) {
    assert!(output.result.unwrap());
    assert_eq!(
        output.sent,
        Vec::<eq_network_game::command::EncodedCommand>::new()
    );
    assert_eq!(
        output.unreliable,
        Vec::<eq_network_game::command::EncodedCommand>::new()
    );
    assert!(output
        .events
        .iter()
        .any(|event| matches!(event, ClientEvent::World(WorldEvent::AbilityRefused { .. }))));
    assert!(!output.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::AbilityUsed { .. } | WorldEvent::Posture { .. })
    )));
}

#[test]
fn eqmac_skill_changes_update_eligibility_without_releasing_recovery() {
    for protocol in SERVERS {
        let (mut features, mut world) = setup(protocol, 7);
        world.player.corrected().unwrap().skills.as_mut().unwrap()[32] = 254;
        refused(use_ability(
            protocol,
            &mut features,
            &mut world,
            Ability::Mend,
        ));
        let update = receive(
            protocol,
            &mut features,
            &mut world,
            0x8941,
            &[32, 0, 0, 0, 0, 0, 0, 0],
        );
        update.result.unwrap();
        assert_eq!(
            world.player.as_ref().unwrap().skills.as_ref().unwrap()[32],
            0
        );
        let used = use_ability(protocol, &mut features, &mut world, Ability::Mend);
        assert!(used.result.unwrap());
        assert_eq!(used.sent, [eqmac::encode(Ability::Mend, None, 7).unwrap()]);
        let update = receive(
            protocol,
            &mut features,
            &mut world,
            0x8941,
            &[32, 0, 0, 0, 1, 0, 0, 0],
        );
        update.result.unwrap();
        assert_eq!(
            world.player.as_ref().unwrap().skills.as_ref().unwrap()[32],
            1
        );
        refused(use_ability(
            protocol,
            &mut features,
            &mut world,
            Ability::Mend,
        ));
    }
}

#[test]
fn eqmac_abilities_admit_and_send_every_supported_action_without_claiming_success() {
    for protocol in SERVERS {
        for ability in eqmac::SUPPORTED {
            let class = match ability {
                Ability::Bash | Ability::Taunt => 1,
                Ability::Backstab => 9,
                _ => 7,
            };
            let (mut features, mut world) = setup(protocol, class);
            let output = use_ability(protocol, &mut features, &mut world, ability);
            assert!(output.result.unwrap());
            let mut expected = vec![];
            if ability == Ability::FeignDeath {
                expected.push(eqmac::feign_posture(7).unwrap());
            }
            expected.push(eqmac::encode(ability, Some(42), 7).unwrap());
            assert_eq!(output.sent, expected, "{protocol:?} {ability:?}");
            assert_eq!(
                output.unreliable,
                Vec::<eq_network_game::command::EncodedCommand>::new()
            );
            assert!(output.events.iter().any(|event| matches!(event, ClientEvent::World(WorldEvent::AbilityUsed { ability: used, .. }) if *used == ability)));
            assert!(!output.events.iter().any(|event| matches!(
                event,
                ClientEvent::World(
                    WorldEvent::HitPoints { .. }
                        | WorldEvent::Visibility { .. }
                        | WorldEvent::Appearance { .. }
                )
            )));
        }
    }
}

#[test]
fn eqmac_abilities_refuse_before_admission_and_reject_gaps_and_sentinels() {
    for protocol in SERVERS {
        let mut features =
            Features::new(servers::server_type(protocol), "Tester", AutoEat::default());
        refused(use_ability(
            protocol,
            &mut features,
            &mut World::new(5),
            Ability::Mend,
        ));
        let (mut features, mut world) = setup(protocol, 7);
        for ability in [Ability::Frenzy, Ability::Fishing, Ability::BindWound] {
            refused(use_ability(protocol, &mut features, &mut world, ability));
        }
        for rank in [253, 254, 255, 256, u32::MAX] {
            world.player.corrected().unwrap().skills.as_mut().unwrap()[32] = rank;
            refused(use_ability(
                protocol,
                &mut features,
                &mut world,
                Ability::Mend,
            ));
        }
        world.player.corrected().unwrap().skills.as_mut().unwrap()[32] = 0;
        world.player.corrected().unwrap().class = Some(1);
        refused(use_ability(
            protocol,
            &mut features,
            &mut world,
            Ability::Mend,
        ));
        refused(use_ability(
            protocol,
            &mut features,
            &mut world,
            Ability::FeignDeath,
        ));
        world.player.corrected().unwrap().class = Some(7);
        assert_eq!(
            use_ability(protocol, &mut features, &mut world, Ability::Mend)
                .sent
                .len(),
            1
        );
    }
}

#[test]
fn eqmac_abilities_validate_target_kind_visibility_range_and_equipment() {
    for protocol in SERVERS {
        for ability in [Ability::Kick, Ability::Taunt] {
            for target in [0, 7, 43, 44, 45, 46, 999] {
                let (mut features, mut world) = setup(protocol, 1);
                let mut far = testing::spawn(43, SpawnKind::Npc);
                far.position.x = 16.1;
                world.spawns.insert(far);
                let mut upstairs = testing::spawn(44, SpawnKind::Npc);
                upstairs.position.z = 16.1;
                world.spawns.insert(upstairs);
                world
                    .spawns
                    .insert(testing::spawn(45, SpawnKind::NpcCorpse));
                let mut hidden = testing::spawn(46, SpawnKind::Npc);
                hidden.invisible = true;
                world.spawns.insert(hidden);
                world.target = Target::from(target);
                refused(use_ability(protocol, &mut features, &mut world, ability));
            }
        }
        let (mut features, mut world) = setup(protocol, 1);
        world.spawns.insert(testing::spawn(43, SpawnKind::Player));
        world.target = Target::from(43);
        refused(use_ability(
            protocol,
            &mut features,
            &mut world,
            Ability::Taunt,
        ));
        for (ability, class) in [(Ability::Bash, 1), (Ability::Backstab, 9)] {
            let (mut features, mut world) = setup(protocol, class);
            testing::run_on(servers::server_type(protocol).wire(), |out| {
                features.observe(
                    &Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(vec![]))),
                    &mut world,
                    out,
                )
            })
            .result
            .unwrap();
            refused(use_ability(protocol, &mut features, &mut world, ability));
        }
    }
}

#[test]
fn eqmac_abilities_release_only_authoritative_resets_and_share_strike_recovery() {
    for protocol in SERVERS {
        let (mut features, mut world) = setup(protocol, 7);
        assert_eq!(
            use_ability(protocol, &mut features, &mut world, Ability::Mend)
                .sent
                .len(),
            1
        );
        assert_eq!(
            use_ability(protocol, &mut features, &mut world, Ability::Kick)
                .sent
                .len(),
            1
        );
        refused(use_ability(
            protocol,
            &mut features,
            &mut world,
            Ability::FlyingKick,
        ));
        for bytes in [
            &[32, 0][..],
            &[32, 0, 0, 0, 1, 0, 0, 0],
            &[0xff, 0xff, 0, 0, 0, 0, 0, 0],
        ] {
            let output = receive(protocol, &mut features, &mut world, 0x3d42, bytes);
            output.result.unwrap();
            assert_eq!(
                output.sent,
                Vec::<eq_network_game::command::EncodedCommand>::new()
            );
            refused(use_ability(
                protocol,
                &mut features,
                &mut world,
                Ability::Mend,
            ));
        }
        receive(
            protocol,
            &mut features,
            &mut world,
            0x3d42,
            &[10, 0, 0, 0, 0, 0, 0, 0],
        )
        .result
        .unwrap();
        assert_eq!(
            use_ability(protocol, &mut features, &mut world, Ability::FlyingKick)
                .sent
                .len(),
            1
        );
        refused(use_ability(
            protocol,
            &mut features,
            &mut world,
            Ability::Mend,
        ));
        receive(
            protocol,
            &mut features,
            &mut world,
            0x3d42,
            &[32, 0, 0, 0, 0, 0, 0, 0],
        )
        .result
        .unwrap();
        assert_eq!(
            use_ability(protocol, &mut features, &mut world, Ability::Mend)
                .sent
                .len(),
            1
        );
    }
}

#[test]
fn eqmac_abilities_keep_server_stealth_state_and_cancellation_separate_from_recovery() {
    for protocol in SERVERS {
        let (mut features, mut world) = setup(protocol, 7);
        use_ability(protocol, &mut features, &mut world, Ability::Sneak)
            .result
            .unwrap();
        assert!(!world.spawns.visible(7).unwrap().appearance.sneaking);
        // Server success; another spawn's failure does not cancel our sneak.
        for (spawn, on) in [(7, 1), (42, 0)] {
            receive(
                protocol,
                &mut features,
                &mut world,
                0xf540,
                &[spawn, 0, 15, 0, on, 0, 0, 0],
            )
            .result
            .unwrap();
        }
        assert!(world.spawns.visible(7).unwrap().appearance.sneaking);
        receive(
            protocol,
            &mut features,
            &mut world,
            0x3d42,
            &[42, 0, 0, 0, 0, 0, 0, 0],
        )
        .result
        .unwrap();
        refused(use_ability(
            protocol,
            &mut features,
            &mut world,
            Ability::Sneak,
        ));
        let output = receive(protocol, &mut features, &mut world, 0x5a41, &[]);
        output.result.unwrap();
        assert!(
            output.sent.is_empty(),
            "EQMac does not acknowledge cancellation"
        );
        assert!(output.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Appearance {
                spawn_id: 7,
                change: AppearanceChange::Sneaking(false)
            })
        )));
        assert!(!world.spawns.visible(7).unwrap().appearance.sneaking);
        use_ability(protocol, &mut features, &mut world, Ability::Sneak)
            .result
            .unwrap();
        receive(protocol, &mut features, &mut world, 0x5a41, &[])
            .result
            .unwrap();
        refused(use_ability(
            protocol,
            &mut features,
            &mut world,
            Ability::Sneak,
        ));
        // Only the visibility packet changes visibility, including deceptive
        // non-rogue hide appearances; neither the request nor cancel guesses.
        use_ability(protocol, &mut features, &mut world, Ability::Hide)
            .result
            .unwrap();
        assert!(world.spawns.visible(7).is_some());
        receive(
            protocol,
            &mut features,
            &mut world,
            0xf540,
            &[7, 0, 3, 0, 1, 0, 0, 0],
        )
        .result
        .unwrap();
        assert!(world.spawns.visible(7).is_none());
        receive(
            protocol,
            &mut features,
            &mut world,
            0xf540,
            &[7, 0, 3, 0, 0, 0, 0, 0],
        )
        .result
        .unwrap();
        assert!(world.spawns.visible(7).is_some());
    }
}

#[test]
fn eqmac_abilities_feign_attempt_lies_down_but_standing_and_server_postures_still_work() {
    for protocol in SERVERS {
        let (mut features, mut world) = setup(protocol, 7);
        use_ability(protocol, &mut features, &mut world, Ability::FeignDeath)
            .result
            .unwrap();
        assert_eq!(world.posture.current(), Some(PostureState::Lying));
        // A repeated attempt does not send another posture or reset recovery.
        refused(use_ability(
            protocol,
            &mut features,
            &mut world,
            Ability::FeignDeath,
        ));
        let output = testing::run_on(servers::server_type(protocol).wire(), |out| {
            features.handle(
                &ClientCommand::SetPosture {
                    session_id: 5,
                    spawn_id: 7,
                    posture: Posture::Standing,
                    created: Instant::now(),
                },
                &mut world,
                out,
            )
        });
        assert!(output.result.unwrap());
        assert_eq!(
            output.sent,
            [eq_network_game::quarm::posture(7, Posture::Standing).unwrap()]
        );
        assert_eq!(world.posture.current(), Some(PostureState::Standing));
        receive(
            protocol,
            &mut features,
            &mut world,
            0xf540,
            &[7, 0, 14, 0, 110, 0, 0, 0],
        )
        .result
        .unwrap();
        assert_eq!(world.posture.current(), Some(PostureState::Sitting));
        refused(use_ability(
            protocol,
            &mut features,
            &mut world,
            Ability::Forage,
        ));
    }
}
