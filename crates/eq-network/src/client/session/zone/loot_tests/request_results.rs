//! Local request results through the production action gate and loot owner.
use super::*;
use eq_network_game::loot::{LootRequest, LootRequestStatus};

fn command(
    features: &mut Features,
    world: &mut World,
    command: &ClientCommand,
    out: &mut Out<'_, '_>,
) -> Result<()> {
    if let Some(reason) = features.refusal(command, world, Instant::now()) {
        return actions::refuse(command, reason, out.log);
    }
    if !features.handle(command, world, out)? {
        unoffered(command, out.log)?;
    }
    Ok(())
}

fn result<'a>(
    events: &'a [ClientEvent],
    original: &ClientCommand,
) -> (&'a LootRequest, &'a LootRequestStatus) {
    let results: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::World(WorldEvent::LootRequest {
                session_id,
                created,
                request,
                status,
            }) => {
                assert_eq!(Some(*session_id), original.session_id());
                assert_eq!(*created, original.created());
                Some((request, status))
            }
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 1, "one local result per command");
    results[0]
}

fn refused(events: &[ClientEvent], original: &ClientCommand) {
    let (_, status) = result(events, original);
    let LootRequestStatus::Refused(reason) = status else {
        panic!("expected a local refusal: {status:?}");
    };
    let diagnostics: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::Diagnostic(text) => Some(text),
            _ => None,
        })
        .collect();
    let expected = format!("Rejected {} command: {reason}", original.name());
    assert_eq!(
        diagnostics,
        [&expected],
        "retain the command identity and original reason exactly once"
    );
}

#[test]
fn local_loot_refusals_retain_every_original_action_field() {
    let created = Instant::now();
    let cases = [
        (
            ClientCommand::Loot {
                session_id: 5,
                corpse_id: 71,
                created,
            },
            LootRequest::Open { corpse_id: 71 },
        ),
        (
            ClientCommand::LootItem {
                session_id: 5,
                corpse_id: 71,
                own_id: 17,
                place: 13,
                auto: true,
                created,
            },
            LootRequest::Item {
                corpse_id: 71,
                own_id: 17,
                place: 13,
                auto: true,
            },
        ),
        (
            ClientCommand::EndLoot {
                session_id: 5,
                corpse_id: 71,
            },
            LootRequest::Close { corpse_id: 71 },
        ),
    ];
    for (request, expected) in cases {
        let (mut features, mut world) = setup(SpawnKind::NpcCorpse);
        let output = testing::run_on(&EqMac, |out| {
            command(&mut features, &mut world, &request, out)
        });
        output.result.unwrap();
        assert_eq!(output.sent, []);
        assert_eq!(result(&output.events, &request).0, &expected);
        refused(&output.events, &request);
        assert!(world.inventory.items().is_empty());
    }
}

#[test]
fn shared_loot_validation_and_unoffered_policy_report_original_admission() {
    for request in [open(), take(), close()] {
        for case in 0..5 {
            let (mut features, mut world) = setup(SpawnKind::NpcCorpse);
            match case {
                0 => world.session_id = 6,
                1 => world.admitted = None,
                2 => world.lifecycle.mark_dead(),
                3 => {
                    features = Features::new(
                        servers::server_type(ServerProtocol::Takp),
                        "Tester",
                        AutoEat::default(),
                    );
                }
                4 => {
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
                _ => unreachable!(),
            }
            let output = testing::run_on(&EqMac, |out| {
                command(&mut features, &mut world, &request, out)
            });
            output.result.unwrap();
            assert_eq!(output.sent, []);
            refused(&output.events, &request);
        }
    }
}

#[test]
fn expired_and_newer_identical_loot_actions_keep_their_creation_context() {
    let now = Instant::now();
    let (mut features, mut world) = setup(SpawnKind::NpcCorpse);
    for created in [
        now.checked_sub(Duration::from_secs(2)).unwrap(),
        now,
        now + Duration::from_secs(2),
    ] {
        let request = ClientCommand::LootItem {
            session_id: 5,
            corpse_id: 9,
            own_id: 7,
            place: 0,
            auto: false,
            created,
        };
        let output = testing::run_on(&EqMac, |out| {
            command(&mut features, &mut world, &request, out)
        });
        output.result.unwrap();
        assert_eq!(output.sent, []);
        refused(&output.events, &request);
    }
}

#[test]
fn loot_controller_refusals_send_no_packet_or_inventory_change() {
    for case in 0..11 {
        let (mut features, mut world) = setup(SpawnKind::NpcCorpse);
        testing::run_on(&EqMac, |out| -> Result<()> {
            command(&mut features, &mut world, &open(), out)?;
            if case != 3 {
                list(&mut features, &mut world, out)?;
            }
            match case {
                4 => {
                    hear(
                        vec![Message::Event(WorldEvent::Inventory(
                            InventoryUpdate::Invalidated,
                        ))],
                        &mut features,
                        &mut world,
                        out,
                        true,
                    )?;
                }
                5 => packet(&mut features, &mut world, out, 0x7841, &item(0))?,
                6 => command(&mut features, &mut world, &take(), out)?,
                7 | 9 => command(&mut features, &mut world, &close(), out)?,
                _ => (),
            }
            Ok(())
        })
        .result
        .unwrap();
        let mut request = take();
        match (case, &mut request) {
            (0, ClientCommand::LootItem { corpse_id, .. }) => *corpse_id = 10,
            (1, ClientCommand::LootItem { own_id, .. }) => *own_id = 10,
            (2, ClientCommand::LootItem { place, .. }) => *place = 10,
            (8, _) => {
                request = ClientCommand::EndLoot {
                    session_id: 5,
                    corpse_id: 10,
                }
            }
            (9, _) => request = close(),
            (10, _) => request = open(),
            _ => (),
        }
        let inventory = world.inventory.items().clone();
        let coins = world.coins.purse;
        let output = testing::run_on(&EqMac, |out| {
            command(&mut features, &mut world, &request, out)
        });
        output.result.unwrap();
        assert_eq!(output.sent, [], "case {case}");
        refused(&output.events, &request);
        assert_eq!(world.inventory.items(), &inventory);
        assert_eq!(world.coins.purse, coins);
    }
}

#[test]
fn successful_loot_actions_report_submission_without_delivery() {
    let (mut features, mut world) = setup(SpawnKind::NpcCorpse);
    let request = open();
    let output = testing::run_on(&EqMac, |out| {
        command(&mut features, &mut world, &request, out)
    });
    output.result.unwrap();
    assert_eq!(
        result(&output.events, &request),
        (
            &LootRequest::Open { corpse_id: 9 },
            &LootRequestStatus::Sent
        )
    );
    assert_eq!(output.sent.len(), 1);
    testing::run_on(&EqMac, |out| list(&mut features, &mut world, out))
        .result
        .unwrap();
    let coins = world.coins.purse;
    for request in [take(), close()] {
        let output = testing::run_on(&EqMac, |out| {
            command(&mut features, &mut world, &request, out)
        });
        output.result.unwrap();
        assert_eq!(result(&output.events, &request).1, &LootRequestStatus::Sent);
        assert_eq!(output.sent.len(), 1);
        assert_eq!(output.events.len(), 1);
        assert!(world.inventory.items().is_empty());
        assert_eq!(world.coins.purse, coins);
    }
}

#[test]
fn encoding_loot_refusal_is_typed_once_and_does_not_open_a_corpse() {
    let (mut features, mut world) = setup(SpawnKind::NpcCorpse);
    world.spawns.insert(testing::spawn(0, SpawnKind::NpcCorpse));
    let request = ClientCommand::Loot {
        session_id: 5,
        corpse_id: 0,
        created: Instant::now(),
    };
    let output = testing::run_on(&EqMac, |out| {
        command(&mut features, &mut world, &request, out)
    });
    output.result.unwrap();
    assert_eq!(output.sent, []);
    refused(&output.events, &request);
    let output = testing::run_on(&EqMac, |out| {
        command(&mut features, &mut world, &open(), out)
    });
    output.result.unwrap();
    assert_eq!(
        output.sent.len(),
        1,
        "encoding refusal left no open request"
    );
}

#[test]
fn server_loot_denial_and_receipt_completion_are_not_local_refusals() {
    for response in [0, 2] {
        let (mut features, mut world) = setup(SpawnKind::NpcCorpse);
        testing::run_on(&EqMac, |out| {
            command(&mut features, &mut world, &open(), out)
        })
        .result
        .unwrap();
        let output = testing::run_on(&EqMac, |out| {
            packet(
                &mut features,
                &mut world,
                out,
                eqmac::MONEY_OPCODE,
                &money(response),
            )
        });
        output.result.unwrap();
        assert!(output.events.iter().any(|event| matches!(event, ClientEvent::World(WorldEvent::Loot(LootUpdate::Opened { response, .. })) if *response != eq_network_game::loot::LootResponse::Normal)));
        assert!(!output
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::LootRequest { .. }))));
    }
    let (mut features, mut world) = setup(SpawnKind::NpcCorpse);
    testing::run_on(&EqMac, |out| -> Result<()> {
        command(&mut features, &mut world, &open(), out)?;
        list(&mut features, &mut world, out)?;
        command(&mut features, &mut world, &take(), out)
    })
    .result
    .unwrap();
    let output = testing::run_on(&EqMac, |out| -> Result<()> {
        packet(
            &mut features,
            &mut world,
            out,
            eqmac::ITEM_OPCODE,
            &eqmac::item_request(9, 7, 0, false)?,
        )?;
        packet(&mut features, &mut world, out, eqmac::COMPLETE_OPCODE, &[])
    });
    output.result.unwrap();
    assert!(!output
        .events
        .iter()
        .any(|event| matches!(event, ClientEvent::World(WorldEvent::LootRequest { .. }))));
    assert!(output.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::Loot(LootUpdate::Acknowledged { place: 0 }))
    )));
    assert!(output.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::Loot(LootUpdate::Closed))
    )));
    assert!(world.inventory.items().is_empty());
}

#[test]
fn a_loot_transport_failure_is_not_reported_as_local_refusal_or_submission() {
    use super::super::super::feature::Sink;
    use eq_network_game::command::EncodedCommand;
    struct Failing;
    impl Sink for Failing {
        fn send(&mut self, _packet: &EncodedCommand) -> Result<()> {
            anyhow::bail!("synthetic transport failure")
        }
        fn send_unreliable(&mut self, packet: &EncodedCommand) -> Result<()> {
            self.send(packet)
        }
        fn last_received_seconds(&self) -> u64 {
            0
        }
    }
    let (mut features, mut world) = setup(SpawnKind::NpcCorpse);
    let mut sink = Failing;
    let output = testing::run_on(&EqMac, |out| {
        let mut failing = Out {
            sink: &mut sink,
            log: out.log,
            wire: out.wire,
            sender: out.sender,
        };
        command(&mut features, &mut world, &open(), &mut failing)
    });
    assert!(output.result.is_err());
    assert!(output.events.is_empty());
    assert!(world.inventory.items().is_empty());
}
