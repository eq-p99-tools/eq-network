//! Rapid moves through the zone's real admission, policy, gate and wire.
use super::super::feature::testing;
use super::*;
use crate::client::ServerProtocol;
use eq_network_game::{
    food::AutoEat,
    inventory::{InventoryItem, InventoryMove, InventorySlot, InventoryUpdate, MoveQuantity},
    world::WorldEvent,
};
use std::num::NonZeroU32;

fn setup(protocol: ServerProtocol, items: Vec<InventoryItem>) -> (Features, World) {
    let server = servers::server_type(protocol);
    let mut features = Features::new(server, "Synthetic", AutoEat::Anything);
    let mut world = World::new(5);
    world.player.admit(testing::player(7));
    world.own_spawn = Some(7);
    world.admitted = Some(Instant::now());
    features
        .admit(
            &Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(items))),
            &mut world,
        )
        .unwrap();
    testing::run_on(server.wire(), |out| features.admitted(&mut world, out))
        .result
        .unwrap();
    (features, world)
}

fn command(world: &World, from: i32, to: i32, quantity: MoveQuantity) -> ClientCommand {
    ClientCommand::MoveInventory(InventoryMove {
        sequence: None,
        session_id: world.session_id,
        revision: world.inventory.revision(),
        from: InventorySlot(from),
        to: InventorySlot(to),
        quantity,
        created: Instant::now(),
    })
}

fn submit(
    protocol: ServerProtocol,
    features: &mut Features,
    world: &mut World,
    command: &ClientCommand,
) -> testing::Outcome<Result<bool>> {
    testing::run_on(servers::server_type(protocol).wire(), |out| {
        if let Some(reason) = features.refusal(command, world, Instant::now()) {
            actions::refuse(command, reason, out.log)?;
            Ok(true)
        } else {
            features.handle(command, world, out)
        }
    })
}

fn observe(
    protocol: ServerProtocol,
    features: &mut Features,
    world: &mut World,
    update: InventoryUpdate,
) {
    testing::run_on(servers::server_type(protocol).wire(), |out| {
        features.observe(&Message::Event(WorldEvent::Inventory(update)), world, out)
    })
    .result
    .unwrap();
}

fn stack(slot: i32, count: u32) -> InventoryItem {
    let mut item = testing::item(slot);
    item.stack_count = Some(count);
    item.rules.stack_size = 20;
    item
}

#[test]
fn an_equip_invalidates_a_liquid_sample_at_the_same_position() {
    use crate::client::ClientConfig;
    use eq_network_game::{
        hazards::{LiquidHazard, LiquidObservation},
        inventory::titanium_move,
        items::{EquipmentRules, ItemBonuses},
    };

    let mut config = ClientConfig::new("Example", "Fixture", "Local", "Tester");
    config.local_only = true;
    config.protocol = ServerProtocol::EqEmu;
    let mut world = World::for_zone(5, &config);
    let mut setup = servers::Setup::new("Tester", config.auto_eat);
    setup.local_only = true;
    let mut features = Features::from_setup(servers::server_type(config.protocol), &setup);
    let mut player = testing::player(7);
    player.class = Some(1);
    player.race = 1;
    player.level = 60;
    let position = player.position;
    world.own_spawn = Some(7);
    world.body.own(7);
    world.body.place(position);
    world.player.admit(player.clone());
    world.admitted = Some(Instant::now());

    let cursor = InventorySlot::CURSOR;
    let worn = InventorySlot(17);
    let mut item = testing::item(cursor.0);
    item.details.equipment = Some(EquipmentRules::default());
    item.details.bonuses = Some(ItemBonuses {
        fire_resistance: Some(15),
        ..ItemBonuses::default()
    });
    item.details.slots = 1 << 17;
    item.details.classes = 1;
    item.details.races = 1;

    let output = testing::run(|out| -> Result<()> {
        out.log.send(ClientEvent::World(WorldEvent::Entered {
            capabilities: features.capabilities(),
            choices: Vec::new(),
            session_id: 5,
            zone: "example".to_owned(),
            player: Box::new(player),
            far_clip: None,
            spell_landing_flag: None,
        }))?;
        hear(
            vec![Message::Event(WorldEvent::Inventory(
                InventoryUpdate::Snapshot(vec![item]),
            ))],
            &mut features,
            &mut world,
            out,
            true,
        )?;
        let old_heat = ClientCommand::LiquidDamage {
            session_id: 5,
            hazard: LiquidHazard::Lava,
            amount: 19,
            observation: LiquidObservation {
                revision: 1,
                position,
                sampled_at: Instant::now(),
            },
        };
        let equip = command(&world, cursor.0, worn.0, MoveQuantity::Whole);
        assert!(
            apply_commands(&[equip, old_heat], &mut features, &mut world, out, true)?.is_none()
        );
        Ok(())
    });
    output.result.unwrap();
    assert_eq!(
        output.sent,
        [titanium_move(cursor, worn, MoveQuantity::Whole).unwrap()]
    );
    assert!(output.unreliable.is_empty());
    assert!(world.inventory.items().contains_key(&worn));
    assert!(world.inventory.equipment_stats_uncertain());
    assert_eq!(world.player_at(), Some((7, position)));
    assert!(output.events.windows(3).any(|events| matches!(
        events,
        [
            ClientEvent::World(WorldEvent::InventoryAction {
                session_id: 5, update: Some(InventoryUpdate::Prediction(_)), error: None, ..
            }),
            ClientEvent::World(WorldEvent::LiquidHazardRevision { session_id: 5, revision: 2 }),
            ClientEvent::Diagnostic(line),
        ] if line.contains("The liquid observation is no longer current")
    )));
}

#[test]
fn explicit_destroy_is_single_submission_through_p99_and_eqemu_inventory_owner() {
    for protocol in [ServerProtocol::Project1999, ServerProtocol::EqEmu] {
        let cursor = stack(30, 7);
        let (mut features, mut world) = setup(protocol, vec![cursor.clone()]);
        let request =
            ClientCommand::DestroyItem(Box::new(eq_network_game::inventory::InventoryDestroy {
                session_id: world.session_id,
                revision: world.inventory.revision(),
                item: cursor,
                created: Instant::now(),
            }));
        let output = submit(protocol, &mut features, &mut world, &request);
        assert!(output.result.unwrap());
        assert_eq!(
            output.sent,
            [eq_network_game::inventory::titanium_destroy()]
        );
        assert!(output.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::InventoryAction {
                update: Some(InventoryUpdate::DestroyCursor),
                error: None,
                ..
            })
        )));
        assert!(world.inventory.predicted());
        assert!(!world.inventory.items().contains_key(&InventorySlot::CURSOR));
        let duplicate = submit(protocol, &mut features, &mut world, &request);
        duplicate.result.unwrap();
        assert_eq!(duplicate.sent, []);
        assert!(duplicate.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::InventoryAction {
                update: None,
                error: Some(_),
                ..
            })
        )));
        let after = Instant::now() + Duration::from_secs(30);
        testing::run_on(servers::server_type(protocol).wire(), |out| {
            features.tick(after, &mut world, out)
        })
        .result
        .unwrap();
        assert!(world.inventory.predicted());
        observe(
            protocol,
            &mut features,
            &mut world,
            InventoryUpdate::Remove(InventorySlot::CURSOR),
        );
        assert!(!world.inventory.predicted());
    }
}

#[test]
fn destroy_stale_admission_expiry_and_replacement_send_nothing() {
    for case in 0..4 {
        let protocol = ServerProtocol::Project1999;
        let cursor = stack(30, 7);
        let (mut features, mut world) = setup(protocol, vec![cursor.clone()]);
        let mut destroy = eq_network_game::inventory::InventoryDestroy {
            session_id: world.session_id,
            revision: world.inventory.revision(),
            item: cursor.clone(),
            created: Instant::now(),
        };
        match case {
            0 => destroy.session_id += 1,
            1 => destroy.created -= Duration::from_secs(2),
            2 => observe(
                protocol,
                &mut features,
                &mut world,
                InventoryUpdate::Set(vec![cursor]),
            ),
            _ => world.lifecycle.mark_dead(),
        }
        let before = world.inventory.clone();
        let output = submit(
            protocol,
            &mut features,
            &mut world,
            &ClientCommand::DestroyItem(Box::new(destroy)),
        );
        output.result.unwrap();
        assert!(output.sent.is_empty(), "case {case}");
        assert_eq!(&*world.inventory, &before, "case {case}");
        assert!(output.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::InventoryAction {
                update: None,
                error: Some(_),
                ..
            })
        )));
    }
}

#[test]
fn destroy_transport_failure_attempts_once_without_committing_a_prediction() {
    use super::super::feature::{Out, Sink};
    use eq_network_game::command::EncodedCommand;
    #[derive(Default)]
    struct FailingSink(usize);
    impl Sink for FailingSink {
        fn send(&mut self, _packet: &EncodedCommand) -> Result<()> {
            self.0 += 1;
            anyhow::bail!("Synthetic destroy transport failure")
        }
        fn send_unreliable(&mut self, _packet: &EncodedCommand) -> Result<()> {
            panic!("Destruction requires reliable transport")
        }
        fn last_received_seconds(&self) -> u64 {
            0
        }
    }
    let protocol = ServerProtocol::EqEmu;
    let cursor = stack(30, 7);
    let (mut features, mut world) = setup(protocol, vec![cursor.clone()]);
    let command =
        ClientCommand::DestroyItem(Box::new(eq_network_game::inventory::InventoryDestroy {
            session_id: world.session_id,
            revision: world.inventory.revision(),
            item: cursor,
            created: Instant::now(),
        }));
    let original = world.inventory.clone();
    let mut sink = FailingSink::default();
    let output = testing::run_on(servers::server_type(protocol).wire(), |out| {
        features.handle(
            &command,
            &mut world,
            &mut Out {
                sink: &mut sink,
                log: out.log,
                wire: out.wire,
                sender: out.sender,
            },
        )
    });
    assert!(output.result.is_err());
    assert_eq!(sink.0, 1);
    assert_eq!(&*world.inventory, &original);
    assert!(!output.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::InventoryAction { error: None, .. })
    )));
}

#[test]
fn root_stack_splits_keep_worker_mass_bounded_without_acknowledging_moves() {
    use eq_network_game::{
        items::{EquipmentRules, ItemBonuses},
        GameDialect,
    };
    for protocol in [
        ServerProtocol::Takp,
        ServerProtocol::Quarm,
        ServerProtocol::EqEmu,
    ] {
        let mut milk = stack(22, 19);
        milk.details.weight_tenths = 1;
        milk.details.equipment = Some(EquipmentRules::default());
        milk.details.bonuses = Some(ItemBonuses::default());
        let mut other = milk.clone();
        other.slot = InventorySlot(24);
        other.stack_count = None;
        other.details.id += 1;
        other.details.weight_tenths = 43;
        let (mut features, mut world) = setup(protocol, vec![milk, other]);
        let count = MoveQuantity::Count(NonZeroU32::new(1).unwrap());
        for cycle in 0..3 {
            for (from, to, quantity) in [
                (22, 30, count),
                (30, 29, MoveQuantity::Whole),
                (29, 30, MoveQuantity::Whole),
                (30, 22, count),
            ] {
                let request = command(&world, from, to, quantity);
                let output = submit(protocol, &mut features, &mut world, &request);
                assert!(output.result.unwrap());
                assert_eq!(output.sent.len(), 1);
                assert!(output.events.iter().any(|event| matches!(
                    event,
                    ClientEvent::World(WorldEvent::InventoryAction {
                        update: Some(InventoryUpdate::Prediction(_)),
                        error: None,
                        ..
                    })
                )));
                let bound = if cycle == 0 && from == 22 { 45 } else { 46 };
                assert_eq!(
                    world.inventory.carried_weight_tenths(GameDialect::EqMac, 0),
                    Ok(bound)
                );
                assert!(world.inventory.predicted());
                assert!(!world.inventory.stale());
                assert!(!world.inventory.equipment_stats_uncertain());
            }
        }
        assert_eq!(
            world.inventory.items()[&InventorySlot(22)].stack_count,
            Some(19)
        );
        observe(
            protocol,
            &mut features,
            &mut world,
            InventoryUpdate::Consume {
                slot: InventorySlot(22),
                charge: false,
            },
        );
        assert!(world
            .inventory
            .carried_weight_tenths(GameDialect::EqMac, 0)
            .is_err());
    }
}

#[test]
fn rapid_moves_send_in_order_on_takp_quarm_and_titanium() {
    for (protocol, opcode, cursor) in [
        (ServerProtocol::Takp, 0x2c41, 0),
        (ServerProtocol::Quarm, 0x2c41, 0),
        (ServerProtocol::EqEmu, 0x420f, 30),
    ] {
        let (mut features, mut world) = setup(protocol, vec![stack(22, 12)]);
        let mut sent = Vec::new();
        let start = Instant::now();
        for (index, (from, to)) in [(22, 30), (30, 22), (22, 30), (30, 23)]
            .into_iter()
            .enumerate()
        {
            if index == 1 {
                // Regression: returning a TAKP stack after 1.3 seconds must
                // not wait for our former two-second hold. No wall-clock sleep.
                testing::run_on(servers::server_type(protocol).wire(), |out| {
                    features.tick(start + Duration::from_millis(1300), &mut world, out)
                })
                .result
                .unwrap();
            }
            let request = command(&world, from, to, MoveQuantity::Whole);
            let outcome = submit(protocol, &mut features, &mut world, &request);
            outcome.result.unwrap();
            assert_eq!(outcome.sent.len(), 1, "{protocol:?}: {:?}", outcome.events);
            sent.extend(outcome.sent);
            // Replaying the same UI intent cannot duplicate the packet.
            assert_eq!(
                submit(protocol, &mut features, &mut world, &request).sent,
                []
            );
        }
        let words = |from: u32, to: u32| {
            [from, to, 0]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            sent.iter()
                .map(|packet| (packet.opcode, packet.body.clone()))
                .collect::<Vec<_>>(),
            [
                (opcode, words(22, cursor)),
                (opcode, words(cursor, 22)),
                (opcode, words(22, cursor)),
                (opcode, words(cursor, 23))
            ]
        );
        assert_eq!(world.inventory.unresolved_predictions(), 4);
        assert_eq!(
            world.inventory.items()[&InventorySlot(23)].stack_count,
            Some(12)
        );
        // Slot evidence, not invented per-move ACKs: the empty cursor alone
        // leaves every move's other changed slot uncertain.
        observe(
            protocol,
            &mut features,
            &mut world,
            InventoryUpdate::Remove(InventorySlot::CURSOR),
        );
        assert_eq!(world.inventory.unresolved_predictions(), 4);
        observe(
            protocol,
            &mut features,
            &mut world,
            InventoryUpdate::Remove(InventorySlot(22)),
        );
        assert_eq!(world.inventory.unresolved_predictions(), 1);
        let final_item = world.inventory.items()[&InventorySlot(23)].clone();
        observe(
            protocol,
            &mut features,
            &mut world,
            InventoryUpdate::Set(vec![final_item]),
        );
        assert_eq!(world.inventory.unresolved_predictions(), 0);
        assert!(!world.inventory.predicted());
        assert!(!world.inventory.stale());
    }
}

#[test]
fn eqmac_silence_and_out_of_order_corrections_never_confirm_moves() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let original = vec![stack(22, 12)];
        let (mut features, mut world) = setup(protocol, original.clone());
        for (from, to) in [(22, 30), (30, 23)] {
            let request = command(&world, from, to, MoveQuantity::Whole);
            assert_eq!(
                submit(protocol, &mut features, &mut world, &request)
                    .sent
                    .len(),
                1
            );
        }
        testing::run_on(servers::server_type(protocol).wire(), |out| {
            features.tick(Instant::now() + Duration::from_secs(60), &mut world, out)
        })
        .result
        .unwrap();
        assert_eq!(world.inventory.unresolved_predictions(), 2);
        // A late replacement describes an intermediate cursor, not the
        // current projection. Never assign it to the oldest pending move.
        observe(
            protocol,
            &mut features,
            &mut world,
            InventoryUpdate::Set(vec![stack(30, 12)]),
        );
        assert!(world.inventory.stale());
        for update in [
            InventoryUpdate::Remove(InventorySlot(23)),
            InventoryUpdate::Set(original.clone()),
            InventoryUpdate::Remove(InventorySlot::CURSOR),
        ] {
            observe(protocol, &mut features, &mut world, update.clone());
            observe(protocol, &mut features, &mut world, update);
        }
        assert!(world.inventory.stale());
        let request = command(&world, 22, 30, MoveQuantity::Whole);
        assert_eq!(
            submit(protocol, &mut features, &mut world, &request).sent,
            []
        );
        observe(
            protocol,
            &mut features,
            &mut world,
            InventoryUpdate::Snapshot(original),
        );
        assert!(!world.inventory.stale());
        assert_eq!(world.inventory.unresolved_predictions(), 0);
    }
}

#[test]
fn matching_stack_swap_sends_nothing_but_an_explicit_merge_still_works() {
    for protocol in [
        ServerProtocol::Takp,
        ServerProtocol::Quarm,
        ServerProtocol::EqEmu,
    ] {
        let (mut features, mut world) = setup(protocol, vec![stack(30, 3), stack(22, 4)]);
        let swap = command(&world, 30, 22, MoveQuantity::Whole);
        assert_eq!(submit(protocol, &mut features, &mut world, &swap).sent, []);
        let merge = command(
            &world,
            30,
            22,
            MoveQuantity::Count(NonZeroU32::new(3).unwrap()),
        );
        assert_eq!(
            submit(protocol, &mut features, &mut world, &merge)
                .sent
                .len(),
            1
        );
        assert_eq!(
            world.inventory.items()[&InventorySlot(22)].stack_count,
            Some(7)
        );
    }
}

/// Select all moves from a host projection before the worker sees any of them.
#[test]
fn projected_commands_enqueue_before_feedback_and_send_in_order() {
    use eq_network_game::inventory::{Inventory, InventoryActor, MoveSequence};
    for protocol in [
        ServerProtocol::Takp,
        ServerProtocol::Quarm,
        ServerProtocol::EqEmu,
    ] {
        let (mut features, mut world) = setup(protocol, vec![stack(22, 12)]);
        let mut projected: Inventory = (*world.inventory).clone();
        let actor = InventoryActor {
            bank_access: false,
            deity: None,
            dual_wield: None,
            class: Some(1),
            race: 1,
            level: 60,
            trade_slots: 0,
            trade_no_drop: false,
            world_container: false,
        };
        let mut commands = Vec::new();
        for (index, (from, to)) in [(22, 30), (30, 23), (23, 30), (30, 24)]
            .into_iter()
            .enumerate()
        {
            let id = u64::try_from(index).unwrap() + 1;
            let request = InventoryMove {
                session_id: world.session_id,
                revision: projected.revision(),
                sequence: Some(MoveSequence {
                    id,
                    after: (id > 1).then_some(id - 1),
                }),
                from: InventorySlot(from),
                to: InventorySlot(to),
                quantity: MoveQuantity::Whole,
                created: Instant::now(),
            };
            projected.apply(projected.plan_move(&request, actor).unwrap());
            commands.push(ClientCommand::MoveInventory(request));
        }
        for request in &commands {
            let outcome = submit(protocol, &mut features, &mut world, request);
            outcome.result.unwrap();
            assert_eq!(outcome.sent.len(), 1, "{protocol:?}: {:?}", outcome.events);
            assert!(matches!(
                outcome.events[..],
                [ClientEvent::World(WorldEvent::InventoryAction {
                    sequence: Some(_),
                    update: Some(InventoryUpdate::Prediction(_)),
                    error: None,
                    ..
                })]
            ));
        }
        assert_eq!(*world.inventory, projected);
        assert_eq!(
            submit(protocol, &mut features, &mut world, &commands[3]).sent,
            []
        );
    }
}

#[test]
fn rejected_predecessor_cannot_alias_a_later_revision_and_a_fresh_sequence_recovers() {
    use eq_network_game::inventory::MoveSequence;
    let protocol = ServerProtocol::Takp;
    let (mut features, mut world) = setup(protocol, vec![stack(22, 12)]);
    let ClientCommand::MoveInventory(mut rejected) = command(&world, 22, 30, MoveQuantity::Whole)
    else {
        unreachable!()
    };
    rejected.sequence = Some(MoveSequence { id: 1, after: None });
    // An intervening authoritative change invalidates the first request.
    observe(
        protocol,
        &mut features,
        &mut world,
        InventoryUpdate::Set(vec![stack(23, 4)]),
    );
    assert_eq!(
        submit(
            protocol,
            &mut features,
            &mut world,
            &ClientCommand::MoveInventory(rejected)
        )
        .sent,
        []
    );
    let ClientCommand::MoveInventory(mut descendant) = command(&world, 23, 30, MoveQuantity::Whole)
    else {
        unreachable!()
    };
    // Its revision is now exactly current, but its predecessor never submitted.
    descendant.sequence = Some(MoveSequence {
        id: 2,
        after: Some(1),
    });
    assert_eq!(
        submit(
            protocol,
            &mut features,
            &mut world,
            &ClientCommand::MoveInventory(descendant)
        )
        .sent,
        []
    );
    let ClientCommand::MoveInventory(mut fresh) = command(&world, 23, 30, MoveQuantity::Whole)
    else {
        unreachable!()
    };
    fresh.sequence = Some(MoveSequence { id: 3, after: None });
    assert_eq!(
        submit(
            protocol,
            &mut features,
            &mut world,
            &ClientCommand::MoveInventory(fresh)
        )
        .sent
        .len(),
        1
    );
}

#[test]
fn transport_failure_reports_no_submitted_prediction_and_leaves_inventory_unchanged() {
    use super::super::feature::{Out, Sink};
    use eq_network_game::{command::EncodedCommand, inventory::MoveSequence};
    #[derive(Default)]
    struct FailingSink(usize);
    impl Sink for FailingSink {
        fn send(&mut self, _packet: &EncodedCommand) -> Result<()> {
            self.0 += 1;
            anyhow::bail!("Synthetic transport failure")
        }
        fn send_unreliable(&mut self, _packet: &EncodedCommand) -> Result<()> {
            panic!("Item moves must use reliable transport")
        }
        fn last_received_seconds(&self) -> u64 {
            0
        }
    }
    for protocol in [
        ServerProtocol::Takp,
        ServerProtocol::Quarm,
        ServerProtocol::EqEmu,
    ] {
        let (mut features, mut world) = setup(protocol, vec![stack(22, 12)]);
        let original = (*world.inventory).clone();
        let ClientCommand::MoveInventory(mut request) =
            command(&world, 22, 30, MoveQuantity::Whole)
        else {
            unreachable!()
        };
        request.sequence = Some(MoveSequence { id: 1, after: None });
        let mut sink = FailingSink::default();
        let outcome = testing::run_on(servers::server_type(protocol).wire(), |out| {
            features.handle(
                &ClientCommand::MoveInventory(request),
                &mut world,
                &mut Out {
                    sink: &mut sink,
                    log: out.log,
                    wire: out.wire,
                    sender: out.sender,
                },
            )
        });
        assert!(outcome.result.is_err());
        assert_eq!(sink.0, 1);
        assert_eq!(*world.inventory, original);
        assert!(!outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::InventoryAction { error: None, .. })
        )));
    }
}

#[test]
fn eqmac_partial_slot_observations_do_not_retire_affecting_equipment_history() {
    use eq_network_game::items::{EquipmentRules, ItemBonuses};
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let mut original = testing::item(24);
        original.details.equipment = Some(EquipmentRules::default());
        original.details.bonuses = Some(ItemBonuses {
            strength: 7,
            ..ItemBonuses::default()
        });
        original.details.slots = u32::MAX;
        original.details.classes = u32::MAX;
        original.details.races = u32::MAX;
        assert_unknown_equipment_actor_refused(protocol, &original);
        let (mut features, mut world) = setup(protocol, vec![original.clone()]);
        // The generic zero-profile fixture cannot equip items. This case needs
        // an explicitly eligible actor, without changing the production guards.
        let mut player = testing::player(7);
        player.class = Some(1);
        player.race = 1;
        player.level = 60;
        world.player.admit(player);
        for (from, to) in [(24, 30), (30, 13), (13, 30), (30, 24)] {
            let revision = world.inventory.revision();
            let request = command(&world, from, to, MoveQuantity::Whole);
            let outcome = submit(protocol, &mut features, &mut world, &request);
            assert!(outcome.result.unwrap());
            let mut moved = original.clone();
            moved.slot = InventorySlot(to);
            assert!(
                matches!(
                    &outcome.events[..],
                    [ClientEvent::World(WorldEvent::InventoryAction {
                        session_id: 5,
                        revision: actual_revision,
                        sequence: None,
                        update: Some(InventoryUpdate::Prediction(items)),
                        error: None,
                    })] if *actual_revision == revision && items.as_slice() == [moved]
                ),
                "{protocol:?} {from}->{to}: {:?}",
                outcome.events
            );
            assert_eq!(outcome.sent.len(), 1, "{protocol:?} {from}->{to}");
            assert_eq!(outcome.sent[0].opcode, 0x2c41);
            let body = [from, to, 0]
                .map(|slot| if slot == 30 { 0 } else { slot })
                .into_iter()
                .flat_map(i32::to_le_bytes)
                .collect::<Vec<_>>();
            assert_eq!(outcome.sent[0].body, body, "{protocol:?} {from}->{to}");
        }
        assert_eq!(world.inventory.unresolved_predictions(), 4);
        assert!(world.inventory.equipment_stats_uncertain());
        for update in [
            InventoryUpdate::Remove(InventorySlot(13)),
            InventoryUpdate::Set(vec![original.clone()]),
            InventoryUpdate::Remove(InventorySlot::CURSOR),
        ] {
            observe(protocol, &mut features, &mut world, update);
        }
        assert!(!world.inventory.stale());
        assert!(!world.inventory.predicted());
        assert!(world.inventory.equipment_stats_uncertain());
        testing::run_on(servers::server_type(protocol).wire(), |out| {
            features.tick(Instant::now() + Duration::from_secs(60), &mut world, out)
        })
        .result
        .unwrap();
        assert!(world.inventory.equipment_stats_uncertain());
        observe(
            protocol,
            &mut features,
            &mut world,
            InventoryUpdate::Snapshot(vec![original]),
        );
        assert!(!world.inventory.equipment_stats_uncertain());
    }
}

#[test]
fn eqmac_cursor_arrivals_recover_a_mass_bound_without_settling_local_moves() {
    use eq_network_game::items::{EquipmentRules, ItemBonuses};
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        for cursor in [false, true] {
            let mut original = testing::item(30);
            original.details.weight_tenths = 37;
            original.details.equipment = Some(EquipmentRules::default());
            original.details.bonuses = Some(ItemBonuses::default());
            original.details.slots = u32::MAX;
            original.details.classes = u32::MAX;
            original.details.races = u32::MAX;
            let mut arrival = original.clone();
            arrival.details.id += 1;
            arrival.details.weight_tenths = 50;
            let (mut features, mut world) = setup(protocol, vec![original]);
            let mut player = testing::player(7);
            player.class = Some(1);
            player.race = 1;
            player.level = 60;
            world.player.admit(player);
            for (from, to) in [(30, 8), (8, 30), (30, 24)] {
                let request = command(&world, from, to, MoveQuantity::Whole);
                let outcome = submit(protocol, &mut features, &mut world, &request);
                assert!(outcome.result.unwrap());
                assert_eq!(outcome.sent.len(), 1, "{protocol:?}: {:?}", outcome.events);
                assert_eq!(outcome.sent[0].opcode, 0x2c41);
                assert!(matches!(
                    outcome.events.as_slice(),
                    [ClientEvent::World(WorldEvent::InventoryAction {
                        update: Some(InventoryUpdate::Prediction(_)),
                        error: None,
                        ..
                    })]
                ));
            }
            observe(
                protocol,
                &mut features,
                &mut world,
                if cursor {
                    InventoryUpdate::Cursor(vec![arrival])
                } else {
                    InventoryUpdate::Set(vec![arrival])
                },
            );
            assert_eq!(
                world
                    .inventory
                    .carried_weight_tenths(eq_network_game::GameDialect::EqMac, 0),
                Ok(87)
            );
            assert!(world.inventory.carried_weight_is_conservative());
            assert!(!world.inventory.stale());
            assert_eq!(world.inventory.unresolved_predictions(), 3);
            testing::run_on(servers::server_type(protocol).wire(), |out| {
                features.tick(Instant::now() + Duration::from_secs(60), &mut world, out)
            })
            .result
            .unwrap();
            assert_eq!(world.inventory.unresolved_predictions(), 3);
            assert!(world.inventory.predicted());
        }
    }
}

/// The original fixture must report its eligibility refusal, without a packet
/// or a prediction for the failed cursor-to-primary move.
fn assert_unknown_equipment_actor_refused(protocol: ServerProtocol, original: &InventoryItem) {
    let (mut features, mut world) = setup(protocol, vec![original.clone()]);
    assert_eq!(world.player.as_ref().unwrap().class, None);
    assert_eq!(world.player.as_ref().unwrap().race, 0);
    let carried = command(&world, 24, 30, MoveQuantity::Whole);
    let outcome = submit(protocol, &mut features, &mut world, &carried);
    assert!(outcome.result.unwrap());
    assert_eq!(outcome.sent.len(), 1, "{protocol:?}: {:?}", outcome.events);
    let before = world.inventory.clone();
    let revision = before.revision();
    let equip = command(&world, 30, 13, MoveQuantity::Whole);
    let outcome = submit(protocol, &mut features, &mut world, &equip);
    assert!(outcome.result.unwrap());
    assert_eq!(outcome.sent, [], "{protocol:?}: {:?}", outcome.events);
    assert!(
        matches!(
            &outcome.events[..],
            [ClientEvent::World(WorldEvent::InventoryAction {
                session_id: 5,
                revision: actual_revision,
                sequence: None,
                update: None,
                error: Some(error),
            })] if *actual_revision == revision && error == "Character class is not known"
        ),
        "{protocol:?}: {:?}",
        outcome.events
    );
    assert_eq!(*world.inventory, before);
}
