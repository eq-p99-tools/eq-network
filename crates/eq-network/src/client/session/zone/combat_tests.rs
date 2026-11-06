//! Normal combat through the actual server feature assembly and wire codecs.
use super::*;
use crate::client::{session::feature::testing, ServerProtocol};
use eq_network_game::{
    combat::{
        self,
        ranged::{RangedAttack, RangedKind, RangedLoadout},
        CombatRequest, CombatRequestStatus, DamageOutcome,
    },
    inventory::InventorySlot,
    world::{SpawnKind, WorldEvent},
};

const SERVERS: &[ServerProtocol] = &[ServerProtocol::Takp, ServerProtocol::Quarm];

fn ranged_setup(protocol: ServerProtocol, kind: RangedKind) -> (Features, World) {
    let (features, mut world) = setup(protocol);
    let mut player = testing::player(7);
    let mut skills = vec![0; 100];
    skills[usize::from(kind.skill())] = 20;
    player.skills = Some(skills);
    world.player.admit(player);
    let mut target = testing::spawn(8, SpawnKind::Npc);
    target.position.x = 50.0;
    world.spawns.insert(target);
    world.target = 8.into();
    world.inventory = super::super::combat::test_ranged_inventory(kind).into();
    (features, world)
}

fn ranged_attack(world: &World) -> RangedAttack {
    RangedAttack {
        session_id: world.session_id,
        target: world.target.state(),
        loadout: RangedLoadout::current(&world.inventory).unwrap(),
        created: Instant::now(),
    }
}

fn set_ranged_rank(world: &mut World, kind: RangedKind, rank: Option<u32>) {
    let player = world.player.corrected().unwrap();
    player.class = Some(4); // Ranger
    player.level = 10;
    player.skills = rank.map(|rank| {
        let mut skills = vec![0; 100];
        skills[usize::from(kind.skill())] = rank;
        skills
    });
}

#[test]
fn eqemu_rank_zero_first_shots_leave_skill_eligibility_to_the_server() {
    let protocol = ServerProtocol::EqEmu;
    let wire = servers::server_type(protocol).wire();
    for (kind, rank) in [
        (RangedKind::Archery, Some(0)),
        (RangedKind::Throwing, Some(0)),
        // Neither missing data nor a marker-like value supplies a class/level cap.
        (RangedKind::Archery, None),
        (RangedKind::Throwing, Some(u32::MAX)),
    ] {
        let (mut features, mut world) = ranged_setup(protocol, kind);
        set_ranged_rank(&mut world, kind, rank);
        let request = ClientCommand::RangedAttack(ranged_attack(&world));
        let output = testing::run_on(wire, |out| {
            command(&mut features, &mut world, &request, out)
        });
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1, "{kind:?} {rank:?}");
        assert_eq!(output.sent[0].body[8], kind.skill());
        assert_eq!(output.events.len(), 1);
        assert_eq!(status(&output.events).1, &CombatRequestStatus::Sent);
        let skills = &world.player.as_ref().unwrap().skills;
        assert_eq!(
            skills
                .as_ref()
                .map(|skills| skills[usize::from(kind.skill())]),
            rank
        );

        let output = testing::run_on(wire, |out| {
            command(&mut features, &mut world, &request, out)
        });
        output.result.unwrap();
        assert_eq!(
            output.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );
        assert!(
            matches!(status(&output.events).1, CombatRequestStatus::Refused(reason)
            if reason.contains("recovering"))
        );
    }
}

#[test]
fn p99_zero_rank_and_unknown_rank_are_distinct_local_refusals() {
    let protocol = ServerProtocol::Project1999;
    let wire = servers::server_type(protocol).wire();
    let kind = RangedKind::Archery;
    for (rank, expected) in [
        (Some(0), "positive ranged skill rank"),
        (None, "rank is unavailable"),
        (Some(254), "rank is unavailable"),
        (Some(255), "rank is unavailable"),
        (Some(u32::MAX), "rank is unavailable"),
    ] {
        let (mut features, mut world) = ranged_setup(protocol, kind);
        set_ranged_rank(&mut world, kind, rank);
        let request = ClientCommand::RangedAttack(ranged_attack(&world));
        let output = testing::run_on(wire, |out| {
            command(&mut features, &mut world, &request, out)
        });
        output.result.unwrap();
        assert_eq!(
            output.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );
        assert!(
            matches!(status(&output.events).1, CombatRequestStatus::Refused(reason)
            if reason.contains(expected))
        );
    }
}

#[test]
fn titanium_ranged_sends_one_request_and_only_received_inventory_changes_ammo() {
    for (kind, slot, wire_skill) in [(RangedKind::Archery, 21, 7), (RangedKind::Throwing, 11, 51)] {
        let protocol = ServerProtocol::EqEmu;
        let server = servers::server_type(protocol);
        let wire = server.wire();
        let (mut features, mut world) = ranged_setup(protocol, kind);
        let request = ClientCommand::RangedAttack(ranged_attack(&world));
        let output = testing::run_on(wire, |out| {
            command(&mut features, &mut world, &request, out)
        });
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
        assert_eq!(output.sent[0].opcode, 0x5ee8);
        assert_eq!(
            output.sent[0].body,
            [8, 0, 0, 0, 11, 0, 0, 0, wire_skill, 0, 0, 0]
        );
        assert_eq!(output.events.len(), 1);
        assert_eq!(status(&output.events).1, &CombatRequestStatus::Sent);
        assert_eq!(
            world.inventory.items()[&InventorySlot(slot)].stack_count,
            Some(2)
        );

        // A second input inside the local wait sends nothing and leaves ammo alone.
        let output = testing::run_on(wire, |out| {
            command(&mut features, &mut world, &request, out)
        });
        output.result.unwrap();
        assert_eq!(
            output.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );
        assert!(
            matches!(status(&output.events).1, CombatRequestStatus::Refused(reason)
            if reason.contains("recovering"))
        );
        assert_eq!(
            world.inventory.items()[&InventorySlot(slot)].stack_count,
            Some(2)
        );

        let mut delete = [255; 12];
        delete[..4].copy_from_slice(&slot.to_le_bytes());
        for (opcode, remaining) in [(0x4d81, Some(1)), (0x420f, None)] {
            let output = testing::run_on(wire, |out| {
                hear(
                    wire.messages(opcode, &delete, server.item_rules()),
                    &mut features,
                    &mut world,
                    out,
                    true,
                )
            });
            output.result.unwrap();
            assert_eq!(
                output.sent,
                [] as [eq_network_game::command::EncodedCommand; 0]
            );
            assert_eq!(
                world
                    .inventory
                    .items()
                    .get(&InventorySlot(slot))
                    .and_then(|item| item.stack_count),
                remaining
            );
        }
        assert!(RangedLoadout::current(&world.inventory).is_err());
    }
}

#[test]
fn ranged_refuses_stale_target_loadout_and_admission_and_unavailable_actions() {
    let protocol = ServerProtocol::Project1999;
    let server = servers::server_type(protocol);
    let wire = server.wire();
    for case in [
        "new target revision",
        "new item revision",
        "old admission",
        "dead",
        "sitting",
        "near",
        "far",
    ] {
        let (mut features, mut world) = ranged_setup(protocol, RangedKind::Archery);
        let mut attack = ranged_attack(&world);
        match case {
            "new target revision" => {
                let output = testing::run_on(wire, |out| {
                    hear(
                        wire.messages(
                            eq_network_game::targets::TARGET_COMMAND_OPCODE,
                            &8u32.to_le_bytes(),
                            server.item_rules(),
                        ),
                        &mut features,
                        &mut world,
                        out,
                        true,
                    )
                });
                output.result.unwrap();
                assert_eq!(*world.target, attack.target.spawn_id);
                assert_ne!(world.target.state().revision, attack.target.revision);
            }
            "new item revision" => {
                let mut refreshed = (*world.inventory).clone();
                let identical = refreshed.items().values().cloned().collect();
                refreshed.apply(eq_network_game::inventory::InventoryUpdate::Snapshot(
                    identical,
                ));
                world.inventory = refreshed.into();
            }
            "old admission" => attack.session_id -= 1,
            "dead" => world.lifecycle.mark_dead(),
            "sitting" => world
                .posture
                .observed(eq_network_game::world::PostureState::Sitting),
            "near" | "far" => {
                let mut target = testing::spawn(8, SpawnKind::Npc);
                target.position.x = if case == "near" { 20.0 } else { 300.0 };
                world.spawns.insert(target);
            }
            _ => unreachable!(),
        }
        let output = testing::run_on(wire, |out| {
            command(
                &mut features,
                &mut world,
                &ClientCommand::RangedAttack(attack),
                out,
            )
        });
        output.result.unwrap();
        assert!(output.sent.is_empty(), "{case}");
        assert!(
            output.events.iter().any(|event| matches!(
                event,
                ClientEvent::World(WorldEvent::CombatRequest {
                    status: CombatRequestStatus::Refused(_),
                    ..
                })
            )),
            "{case}"
        );
    }

    for &protocol in SERVERS {
        let (mut features, mut world) = ranged_setup(protocol, RangedKind::Throwing);
        let request = ClientCommand::RangedAttack(ranged_attack(&world));
        let output = testing::run_on(servers::server_type(protocol).wire(), |out| {
            command(&mut features, &mut world, &request, out)
        });
        output.result.unwrap();
        assert_eq!(
            output.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );
        assert!(matches!(
            status(&output.events).1,
            CombatRequestStatus::Refused(_)
        ));
    }
}

fn setup(protocol: ServerProtocol) -> (Features, World) {
    let features = Features::new(
        servers::server_type(protocol),
        "Synthetic",
        eq_network_game::food::AutoEat::default(),
    );
    let mut world = World::new(5);
    world.own_spawn = Some(7);
    world.player.admit(testing::player(7));
    world.admitted = Some(Instant::now());
    world.spawns.insert(testing::spawn(8, SpawnKind::Npc));
    (features, world)
}

fn attack(enabled: bool) -> ClientCommand {
    ClientCommand::AutoAttack {
        session_id: 5,
        enabled,
        created: Instant::now(),
    }
}

fn consider(own_id: u16, target_id: u16) -> ClientCommand {
    ClientCommand::Consider {
        session_id: 5,
        own_id,
        target_id,
        created: Instant::now(),
    }
}

/// The same shared stale/hold/owner/refusal path used by the zone loop.
fn command(
    features: &mut Features,
    world: &mut World,
    command: &ClientCommand,
    out: &mut Out<'_, '_>,
) -> Result<()> {
    let now = Instant::now();
    if let Some(reason) = actions::stale(command, world.session_id, now)
        .or_else(|| actions::Held::new(features.holds(world, now)).conflict(command))
    {
        return actions::refuse(command, reason, out.log);
    }
    if !features.handle(command, world, out)? {
        unoffered(command, out.log)?;
    }
    Ok(())
}

fn status(events: &[ClientEvent]) -> (&CombatRequest, &CombatRequestStatus) {
    let mut requests = events.iter().filter_map(|event| match event {
        ClientEvent::World(WorldEvent::CombatRequest {
            session_id: 5,
            request,
            status,
        }) => Some((request, status)),
        _ => None,
    });
    let result = requests.next().expect("one request disposition");
    assert!(requests.next().is_none());
    result
}

#[test]
fn eqmac_toggles_send_notice_then_toggle_without_inventing_a_hit() {
    for &protocol in SERVERS {
        let (mut features, mut world) = setup(protocol);
        assert!(features
            .capabilities()
            .contains(&crate::world::Capability::Combat));
        // No target is required to toggle. Range, equipment, recovery and
        // swing eligibility remain with their existing owners and the server.
        for enabled in [true, false] {
            let output = testing::run_on(servers::server_type(protocol).wire(), |out| {
                command(&mut features, &mut world, &attack(enabled), out)
            });
            output.result.unwrap();
            assert_eq!(output.sent.len(), 2);
            assert_eq!(output.sent[0].opcode, combat::EQMAC_AUTO_ATTACK2_OPCODE);
            assert_eq!(output.sent[1].opcode, combat::EQMAC_AUTO_ATTACK_OPCODE);
            assert!(output
                .sent
                .iter()
                .all(|p| p.body == combat::auto_attack(enabled)));
            assert_eq!(
                status(&output.events),
                (
                    &CombatRequest::AutoAttack { enabled },
                    &CombatRequestStatus::Sent
                )
            );
            assert_eq!(output.events.len(), 1);
            assert!(world.target.is_none());
        }
    }
}

#[test]
fn considering_self_or_a_distant_visible_spawn_does_not_change_the_target() {
    for &protocol in SERVERS {
        let (mut features, mut world) = setup(protocol);
        world.target = super::super::targeting::Target::from(8);
        let mut distant = testing::spawn(9, SpawnKind::Npc);
        distant.position.x = 10_000.0;
        world.spawns.insert(distant);
        for target_id in [7, 9] {
            let output = testing::run_on(servers::server_type(protocol).wire(), |out| {
                command(&mut features, &mut world, &consider(7, target_id), out)
            });
            output.result.unwrap();
            assert_eq!(output.sent.len(), 1);
            assert_eq!(output.sent[0].opcode, combat::EQMAC_CONSIDER_OPCODE);
            assert_eq!(
                output.sent[0].body,
                combat::eqmac_consider_request(7, target_id).unwrap()
            );
            assert_eq!(
                status(&output.events),
                (
                    &CombatRequest::Consider { target_id },
                    &CombatRequestStatus::Sent
                )
            );
            assert_eq!(output.events.len(), 1);
            assert_eq!(*world.target, Some(8));
        }
    }
}

#[test]
fn unavailable_stale_dead_and_zoning_requests_never_reach_the_wire() {
    for &protocol in SERVERS {
        for case in 0..11 {
            let (mut features, mut world) = setup(protocol);
            let mut request = attack(true);
            match case {
                0 => world.admitted = None,
                1 => world.own_spawn = None,
                2 => world.lifecycle.mark_dead(),
                3 => {
                    world
                        .lifecycle
                        .offer(
                            eq_network_game::zoning::ZoneOffer {
                                zone_id: 9,
                                instance_id: 0,
                                position: eq_network_game::world::Position::default(),
                                reason: 0,
                                to_bind: false,
                                solicited: true,
                            },
                            Instant::now(),
                        )
                        .unwrap();
                }
                4 => request = consider(6, 8),
                5 => request = consider(7, 10),
                6 => {
                    let mut hidden = testing::spawn(9, SpawnKind::Npc);
                    hidden.invisible = true;
                    world.spawns.insert(hidden);
                    request = consider(7, 9);
                }
                7 => world.session_id = 6,
                8 => {
                    request = ClientCommand::AutoAttack {
                        session_id: 5,
                        enabled: true,
                        created: Instant::now().checked_sub(Duration::from_secs(2)).unwrap(),
                    }
                }
                9 => {
                    request = ClientCommand::Consider {
                        session_id: 5,
                        own_id: 7,
                        target_id: 8,
                        created: Instant::now() + Duration::from_secs(2),
                    }
                }
                10 => world.player = super::super::character::PlayerRecord::default(),
                _ => unreachable!(),
            }
            let output = testing::run_on(servers::server_type(protocol).wire(), |out| {
                command(&mut features, &mut world, &request, out)
            });
            output.result.unwrap();
            assert_eq!(output.sent, [], "{protocol:?}, case {case}");
            assert!(matches!(
                status(&output.events).1,
                CombatRequestStatus::Refused(_)
            ));
        }
    }
}

#[test]
fn server_records_alone_report_considerations_hits_and_avoidance() {
    for &protocol in SERVERS {
        let (mut features, mut world) = setup(protocol);
        // A delayed assessment belongs to its response ID, not a newer target.
        world.target = super::super::targeting::Target::from(9);
        let wire = servers::server_type(protocol).wire();
        let item_rules = servers::server_type(protocol).item_rules();
        let mut response = [0u8; 24];
        response[..4].copy_from_slice(&[7, 0, 8, 0]);
        response[4..8].copy_from_slice(&9u32.to_le_bytes());
        response[8..12].copy_from_slice(&77u32.to_le_bytes());
        let output = testing::run_on(wire, |out| {
            hear(
                wire.messages(combat::EQMAC_CONSIDER_OPCODE, &response, item_rules),
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        output.result.unwrap();
        assert_eq!(output.sent, []);
        assert!(
            matches!(output.events[..], [ClientEvent::World(WorldEvent::Consideration(value))]
            if value.target_id == 8 && value.faction == 9
                && value.color == combat::ConColor::Other(77) && value.hit_points.is_none())
        );
        assert_eq!(*world.target, Some(9));
        for (raw, expected) in [
            (12i32, DamageOutcome::Hit(12)),
            (-4, DamageOutcome::Dodge),
            (-99, DamageOutcome::Other(-99)),
        ] {
            let mut damage = [0u8; 24];
            damage[..4].copy_from_slice(&[8, 0, 7, 0]);
            damage[8..12].copy_from_slice(&raw.to_le_bytes());
            let output = testing::run_on(wire, |out| {
                hear(
                    wire.messages(combat::EQMAC_DAMAGE_OPCODE, &damage, item_rules),
                    &mut features,
                    &mut world,
                    out,
                    true,
                )
            });
            output.result.unwrap();
            assert_eq!(output.sent, []);
            assert!(
                matches!(output.events[..], [ClientEvent::World(WorldEvent::Damage(value))]
                if value.target_id == 8 && value.source_id == 7 && value.outcome == expected)
            );
        }
        let mut animation = [0u8; 12];
        animation[..2].copy_from_slice(&8u16.to_le_bytes());
        animation[4..8].copy_from_slice(&5u32.to_le_bytes());
        animation[8..].copy_from_slice(&1.0f32.to_le_bytes());
        let output = testing::run_on(wire, |out| {
            hear(
                wire.messages(combat::EQMAC_ANIMATION_OPCODE, &animation, item_rules),
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        output.result.unwrap();
        assert_eq!(output.sent, []);
        assert!(
            matches!(output.events[..], [ClientEvent::World(WorldEvent::Animation(value))]
            if value.spawn_id == 8 && value.action == 5)
        );
        // A malformed response never becomes a synthetic successful result.
        let output = testing::run_on(wire, |out| {
            hear(
                wire.messages(combat::EQMAC_CONSIDER_OPCODE, &response[..23], item_rules),
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        output.result.unwrap();
        assert_eq!(output.sent, []);
        assert!(output
            .events
            .iter()
            .all(|e| matches!(e, ClientEvent::Diagnostic(_))));
    }
}

#[test]
fn an_incomplete_toggle_pair_does_not_report_submission_or_success() {
    use super::super::feature::Sink;
    use eq_network_game::command::EncodedCommand;
    struct FailingSink {
        attempts: usize,
        fail_at: usize,
    }
    impl Sink for FailingSink {
        fn send(&mut self, _packet: &EncodedCommand) -> Result<()> {
            self.attempts += 1;
            anyhow::ensure!(self.attempts != self.fail_at, "synthetic transport failure");
            Ok(())
        }
        fn send_unreliable(&mut self, packet: &EncodedCommand) -> Result<()> {
            self.send(packet)
        }
        fn last_received_seconds(&self) -> u64 {
            0
        }
    }
    for &protocol in SERVERS {
        for fail_at in [1, 2] {
            let (mut features, mut world) = setup(protocol);
            let mut sink = FailingSink {
                attempts: 0,
                fail_at,
            };
            let output = testing::run_on(servers::server_type(protocol).wire(), |out| {
                let mut failing = Out {
                    sink: &mut sink,
                    log: out.log,
                    wire: out.wire,
                    sender: out.sender,
                };
                command(&mut features, &mut world, &attack(true), &mut failing)
            });
            assert!(output.result.is_err());
            assert_eq!(sink.attempts, fail_at);
            assert!(output.events.is_empty());
        }
    }
}

#[test]
fn unoffered_combat_reports_a_structured_refusal() {
    let (mut features, mut world) = setup(ServerProtocol::Quarm);
    // Exercise the actual unavailable-policy path without another combat owner.
    features
        .provided
        .retain(|feature| !feature.owns(&attack(true)));
    let output = testing::run_on(servers::server_type(ServerProtocol::Quarm).wire(), |out| {
        command(&mut features, &mut world, &attack(true), out)
    });
    output.result.unwrap();
    assert_eq!(output.sent, []);
    assert_eq!(
        status(&output.events).1,
        &CombatRequestStatus::Refused(UNOFFERED.into())
    );
}
