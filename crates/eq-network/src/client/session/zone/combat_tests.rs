//! Normal combat through the actual server feature assembly and `EQMac` wire.
use super::*;
use crate::client::{session::feature::testing, ServerProtocol};
use eq_network_game::{
    combat::{self, CombatRequest, CombatRequestStatus, DamageOutcome},
    world::{SpawnKind, WorldEvent},
};

const SERVERS: &[ServerProtocol] = &[ServerProtocol::Takp, ServerProtocol::Quarm];

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
