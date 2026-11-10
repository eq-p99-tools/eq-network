//! Synthetic actions through Quarm's real feature assembly and wire decoder.
use super::super::feature::testing;
use super::*;
use crate::client::ServerProtocol;
use eq_network_game::GameDialect;
use eq_network_game::{
    food::AutoEat,
    inventory::{InventoryItem, InventoryMove, InventorySlot, InventoryUpdate, MoveQuantity},
    money::{Coin, CoinPlace, CoinTransfer},
    world::{Capability, Coins, SpawnKind, WorldEvent},
};
use std::num::NonZeroU32;

fn setup(items: Vec<InventoryItem>) -> (Features, World) {
    let server = servers::server_type(ServerProtocol::Quarm);
    let mut features = Features::new(server, "Tester", AutoEat::Anything);
    let mut world = World::new(5);
    let mut player = testing::player(7);
    player.class = Some(1);
    player.race = 1;
    player.level = 10;
    player.run_speed = 0.7;
    player.walk_speed = 0.46;
    world.player.admit(player);
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

fn request(world: &World, from: i32, to: i32, count: Option<u32>) -> ClientCommand {
    ClientCommand::MoveInventory(InventoryMove {
        sequence: None,
        session_id: world.session_id,
        revision: world.inventory.revision(),
        from: InventorySlot(from),
        to: InventorySlot(to),
        quantity: count.map_or(MoveQuantity::Whole, |n| {
            MoveQuantity::Count(NonZeroU32::new(n).unwrap())
        }),
        created: Instant::now(),
    })
}

/// Runs the zone loop's actual gate and feature dispatch with a recording sink.
fn submit(
    features: &mut Features,
    world: &mut World,
    command: &ClientCommand,
) -> testing::Outcome<Result<bool>> {
    testing::run_on(servers::server_type(ServerProtocol::Quarm).wire(), |out| {
        let reason = features.refusal(command, world, Instant::now());
        if let Some(reason) = reason {
            actions::refuse(command, reason, out.log)?;
            Ok(true)
        } else {
            features.handle(command, world, out)
        }
    })
}

fn observe(features: &mut Features, world: &mut World, update: InventoryUpdate) {
    testing::run_on(servers::server_type(ServerProtocol::Quarm).wire(), |out| {
        features.observe(&Message::Event(WorldEvent::Inventory(update)), world, out)
    })
    .result
    .unwrap();
}

fn paced(features: &mut Features, world: &mut World) {
    let output = testing::run_on(servers::server_type(ServerProtocol::Quarm).wire(), |out| {
        features.tick(Instant::now() + Duration::from_secs(3), world, out)
    });
    output.result.unwrap();
    assert!(!output.events.iter().any(|e| matches!(
        e,
        ClientEvent::World(WorldEvent::Inventory(InventoryUpdate::Settled))
    )));
}

fn stack(slot: i32, count: u32) -> InventoryItem {
    let mut item = testing::item(slot);
    item.stack_count = Some(count);
    item.rules.stack_size = 20;
    item
}

fn bag(slot: i32) -> InventoryItem {
    let mut item = testing::item(slot);
    item.details.id = 50;
    item.bag_slots = 2;
    item.rules.bag_size = 4;
    item
}

fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn input_revision(world: &mut World) -> u64 {
    world
        .take_news()
        .into_iter()
        .filter_map(|message| match message {
            Message::Event(WorldEvent::SpeedInputsRequired { revision, .. }) => Some(revision),
            _ => None,
        })
        .next_back()
        .expect("new input revision")
}

/// The run speed granted now, if any.
fn speed(world: &mut World) -> Option<f32> {
    world
        .body
        .motion_mut()
        .unwrap()
        .calibration()
        .map(|speeds| speeds.units_per_second)
}

#[test]
fn quarm_prediction_pacing_slows_but_never_stops_an_unproven_weight_grant() {
    use eq_network_game::{
        inventory::WeightUnavailable,
        movement::{SpeedInputState, SpeedInputUnavailable, SpeedInputs},
    };
    let mut plain = testing::item(22);
    plain.details.weight_tenths = 40;
    let mut container = bag(23);
    container.details.weight_tenths = 0;
    container.rules.bag_weight_reduction = 50;
    let snapshot = vec![plain, container];
    let (mut features, mut world) = setup(snapshot.clone());
    for capability in [
        Capability::Inventory,
        Capability::Abilities,
        Capability::Casting,
        Capability::Spellbook,
    ] {
        assert!(features.capabilities().contains(&capability));
    }
    let ready = |revision| ClientCommand::SpeedInputs {
        session_id: 5,
        revision,
        report_id: 1,
        state: SpeedInputState::Ready(SpeedInputs {
            eqmac_fatigue: None,
            rate: 0,
            weight: 4,
            capacity: 100,
        }),
    };
    let initial = input_revision(&mut world);
    submit(&mut features, &mut world, &ready(initial))
        .result
        .unwrap();
    assert!(world.body.motion_mut().unwrap().calibration().is_some());
    // Ordinary cursor pickup preserves weight without confirming the prediction.
    let pickup = request(&world, 22, 30, None);
    assert_eq!(submit(&mut features, &mut world, &pickup).sent.len(), 1);
    paced(&mut features, &mut world);
    let cursor_revision = input_revision(&mut world);
    assert!(world.inventory.predicted());
    assert_eq!(
        world.inventory.carried_weight_tenths(GameDialect::EqMac, 0),
        Ok(40)
    );
    submit(&mut features, &mut world, &ready(cursor_revision))
        .result
        .unwrap();
    let full = speed(&mut world).unwrap();
    // Into a reduction bag: weight changes, pacing proves nothing.
    let reduce = request(&world, 30, 261, None);
    assert_eq!(submit(&mut features, &mut world, &reduce).sent.len(), 1);
    paced(&mut features, &mut world);
    let changed = input_revision(&mut world);
    assert_eq!(
        world.inventory.carried_weight_tenths(GameDialect::EqMac, 0),
        Err(WeightUnavailable::Prediction)
    );
    submit(&mut features, &mut world, &ready(cursor_revision))
        .result
        .unwrap();
    // The old report answers nothing; the grant goes on (6.452).
    assert!(speed(&mut world).is_some_and(|speed| speed <= full));
    submit(
        &mut features,
        &mut world,
        &ClientCommand::SpeedInputs {
            session_id: 5,
            revision: changed,
            report_id: 1,
            state: SpeedInputState::Unavailable(SpeedInputUnavailable::Prediction),
        },
    )
    .result
    .unwrap();
    paced(&mut features, &mut world);
    // Unbounded: held at the walk rate (6.452), not stopped.
    assert!(speed(&mut world).is_some_and(|walking| walking < full));
    assert!(world.inventory.predicted());
    // A contradiction stays unusable after pacing; only the full snapshot recovers.
    observe(
        &mut features,
        &mut world,
        InventoryUpdate::Set(vec![testing::item(22)]),
    );
    paced(&mut features, &mut world);
    assert_eq!(
        world.inventory.carried_weight_tenths(GameDialect::EqMac, 0),
        Err(WeightUnavailable::Inventory)
    );
    observe(
        &mut features,
        &mut world,
        InventoryUpdate::Snapshot(snapshot),
    );
    paced(&mut features, &mut world);
    let recovered = input_revision(&mut world);
    assert_eq!(
        world.inventory.carried_weight_tenths(GameDialect::EqMac, 0),
        Ok(40)
    );
    submit(&mut features, &mut world, &ready(recovered))
        .result
        .unwrap();
    assert_eq!(speed(&mut world), Some(full));
}

#[test]
fn quarm_root_bag_cursor_splits_merges_and_equipment_use_exact_wire() {
    let mut worn = testing::item(24);
    worn.details.id = 60;
    worn.details.classes = 1;
    worn.details.races = 1;
    worn.details.slots = 1 << 17;
    let (mut features, mut world) = setup(vec![stack(22, 12), bag(23), worn]);
    assert!(features.capabilities().contains(&Capability::Inventory));
    for (from, to, count, raw_from, raw_to) in [
        (22, 30, Some(5), 22, 0),
        (30, 261, Some(3), 0, 260),
        (30, 22, Some(2), 0, 22),
        (261, 30, None, 260, 0),
        (30, 22, Some(3), 0, 22),
        (24, 30, None, 24, 0),
        (30, 17, None, 0, 17),
        (23, 30, None, 23, 0),
        (30, 25, None, 0, 25),
    ] {
        let command = request(&world, from, to, count);
        let output = submit(&mut features, &mut world, &command);
        assert!(output.result.unwrap());
        assert_eq!(output.sent.len(), 1, "{from} -> {to}: {:?}", output.events);
        assert_eq!(output.sent[0].opcode, 0x2c41);
        assert_eq!(
            output.sent[0].body,
            words(&[raw_from, raw_to, count.unwrap_or(0)])
        );
        assert!(world.inventory.predicted());
        let before = world.inventory.revision();
        paced(&mut features, &mut world);
        assert_eq!(world.inventory.revision(), before); // No fabricated confirmation.
        assert!(world.inventory.predicted());
    }
    assert_eq!(
        world.inventory.items()[&InventorySlot(22)].stack_count,
        Some(12)
    );
    assert_eq!(world.inventory.items()[&InventorySlot(17)].details.id, 60);
    assert_eq!(world.inventory.items()[&InventorySlot(25)].bag_slots, 2);
}

#[test]
fn quarm_rejects_stale_unavailable_bad_slots_and_quantities_before_sending() {
    let (mut features, mut world) = setup(vec![stack(22, 4), bag(23)]);
    for (from, to, count) in [
        (22, 0, None),
        (22, -1, None),
        (22, 331, None),
        (22, 263, None),
        (22, 3000, None),
        (22, 4000, None),
        (22, 2500, None),
        (22, 30, Some(5)),
    ] {
        let command = request(&world, from, to, count);
        assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 0);
    }
    let current = request(&world, 22, 30, None);
    observe(
        &mut features,
        &mut world,
        InventoryUpdate::Set(vec![stack(22, 3)]),
    );
    assert_eq!(submit(&mut features, &mut world, &current).sent.len(), 0);
    for old_session in [true, false] {
        let ClientCommand::MoveInventory(mut stale) = request(&world, 22, 30, None) else {
            unreachable!()
        };
        if old_session {
            stale.session_id = 4;
        } else {
            stale.created = stale.created.checked_sub(Duration::from_secs(2)).unwrap();
        }
        assert_eq!(
            submit(
                &mut features,
                &mut world,
                &ClientCommand::MoveInventory(stale)
            )
            .sent
            .len(),
            0
        );
    }
    world.inventory = super::super::inventory::Carried::default();
    let command = request(&world, 22, 30, None);
    assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 0);
    observe(
        &mut features,
        &mut world,
        InventoryUpdate::Snapshot(vec![stack(22, 4)]),
    );
    observe(&mut features, &mut world, InventoryUpdate::Invalidated);
    let command = request(&world, 22, 30, None);
    assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 0);
}

#[test]
fn quarm_refusal_and_incomplete_bag_corrections_require_a_fresh_snapshot() {
    for original in [vec![testing::item(22)], vec![bag(22), stack(251, 3)]] {
        let (mut features, mut world) = setup(original.clone());
        let command = request(&world, 22, 30, None);
        assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 1);
        // The real resync first sends a temporary Copper Coin, then a root
        // replacement, and may omit empty cursor/bank slots and bag contents.
        paced(&mut features, &mut world); // Late corrections still matter.
        let mut token = testing::item(22);
        token.details.id = 22292;
        observe(&mut features, &mut world, InventoryUpdate::Set(vec![token]));
        observe(
            &mut features,
            &mut world,
            InventoryUpdate::Set(vec![original[0].clone()]),
        );
        observe(
            &mut features,
            &mut world,
            InventoryUpdate::Remove(InventorySlot::CURSOR),
        );
        paced(&mut features, &mut world);
        assert!(world.inventory.stale());
        let command = request(&world, 22, 30, None);
        assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 0);
        // Re-admission's complete snapshot is the supported recovery.
        observe(
            &mut features,
            &mut world,
            InventoryUpdate::Snapshot(original),
        );
        assert!(!world.inventory.stale());
        assert!(!world.inventory.predicted());
        let command = request(&world, 22, 30, None);
        assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 1);
    }
}

#[test]
fn quarm_cursor_refill_is_observed_without_confirming_the_move() {
    let (mut features, mut world) = setup(vec![testing::item(30)]);
    let command = request(&world, 30, 22, None);
    assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 1);
    let mut next = testing::item(30);
    next.details.id = 99;
    observe(
        &mut features,
        &mut world,
        InventoryUpdate::Cursor(vec![next]),
    );
    paced(&mut features, &mut world);
    assert_eq!(
        world.inventory.items()[&InventorySlot::CURSOR].details.id,
        99
    );
    assert!(world.inventory.predicted());
    let command = request(&world, 30, 23, None);
    assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 1);
}

#[test]
fn quarm_full_bags_keep_contents_through_cursor_and_bank_roots() {
    let (mut features, mut world) = setup(vec![bag(22), stack(251, 3)]);
    let mut banker = testing::spawn(9, SpawnKind::Npc);
    banker.class = Some(40);
    world.spawns.insert(banker);
    for (from, to, child, raw_from, raw_to) in [
        (22, 30, 331, 22, 0),
        (30, 2007, 2101, 0, 2007),
        (2007, 30, 331, 2007, 0),
        (30, 23, 261, 0, 23),
    ] {
        let command = request(&world, from, to, None);
        let output = submit(&mut features, &mut world, &command);
        assert_eq!(output.sent.len(), 1, "{:?}", output.events);
        assert_eq!(output.sent[0].body, words(&[raw_from, raw_to, 0]));
        assert_eq!(
            world.inventory.items()[&InventorySlot(child)].stack_count,
            Some(3)
        );
        assert_eq!(world.inventory.items().len(), 2);
        paced(&mut features, &mut world);
    }
    // Quarm resends carried bag children after putting the bag down.
    let child = world.inventory.items()[&InventorySlot(261)].clone();
    observe(&mut features, &mut world, InventoryUpdate::Set(vec![child]));
    assert!(!world.inventory.stale());
    assert!(world.inventory.predicted()); // The child update is not a whole-move ACK.
}

#[test]
fn quarm_matching_slot_updates_only_confirm_what_the_server_reports() {
    let (mut features, mut world) = setup(vec![testing::item(22)]);
    let command = request(&world, 22, 30, None);
    assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 1);
    let cursor = world.inventory.items()[&InventorySlot::CURSOR].clone();
    observe(
        &mut features,
        &mut world,
        InventoryUpdate::Set(vec![cursor]),
    );
    assert!(world.inventory.predicted());
    observe(
        &mut features,
        &mut world,
        InventoryUpdate::Remove(InventorySlot(22)),
    );
    assert!(!world.inventory.predicted());
    assert!(!world.inventory.stale());
}

fn coin(from: CoinPlace, to: CoinPlace, kind: Coin, into: Coin, amount: u32) -> ClientCommand {
    ClientCommand::MoveCoins {
        session_id: 5,
        transfer: CoinTransfer {
            from,
            to,
            coin: kind,
            into,
            amount,
        },
        created: Instant::now(),
    }
}

fn fund(features: &mut Features, world: &mut World) {
    for event in [
        WorldEvent::Coins(Coins {
            platinum: 20,
            gold: 20,
            silver: 20,
            copper: 20,
        }),
        WorldEvent::CoinsElsewhere {
            cursor: Coins::default(),
            bank: Coins::default(),
            given: Coins::default(),
            offered: Coins::default(),
        },
    ] {
        testing::run_on(servers::server_type(ServerProtocol::Quarm).wire(), |out| {
            features.observe(&Message::Event(event), world, out)
        })
        .result
        .unwrap();
    }
}

#[test]
fn quarm_coins_use_all_denominations_and_authoritative_updates_replace_the_ledger() {
    let (mut features, mut world) = setup(vec![]);
    fund(&mut features, &mut world);
    for (kind, wire) in [
        (Coin::Copper, 0),
        (Coin::Silver, 1),
        (Coin::Gold, 2),
        (Coin::Platinum, 3),
    ] {
        let command = coin(CoinPlace::Purse, CoinPlace::Cursor, kind, kind, 11);
        let output = submit(&mut features, &mut world, &command);
        assert_eq!(output.sent.len(), 1);
        assert_eq!(output.sent[0].opcode, 0x2d41);
        assert_eq!(output.sent[0].body, words(&[1, 0, wire, wire, 11]));
        assert_eq!(world.coins.cursor.of(kind), 11);
    }
    let command = coin(
        CoinPlace::Cursor,
        CoinPlace::Purse,
        Coin::Gold,
        Coin::Platinum,
        11,
    );
    let output = submit(&mut features, &mut world, &command);
    assert_eq!(output.sent[0].body, words(&[0, 1, 2, 3, 11]));
    assert_eq!(world.coins.cursor.gold, 1);
    assert_eq!(world.coins.purse.unwrap().platinum, 10);
    for command in [
        coin(
            CoinPlace::Purse,
            CoinPlace::Cursor,
            Coin::Gold,
            Coin::Gold,
            100,
        ),
        coin(
            CoinPlace::Cursor,
            CoinPlace::Purse,
            Coin::Gold,
            Coin::Platinum,
            1,
        ),
        coin(
            CoinPlace::Cursor,
            CoinPlace::Trade,
            Coin::Gold,
            Coin::Gold,
            1,
        ),
        coin(
            CoinPlace::Purse,
            CoinPlace::Cursor,
            Coin::Gold,
            Coin::Gold,
            0,
        ),
    ] {
        assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 0);
    }
    fund(&mut features, &mut world);
    assert_eq!(world.coins.purse.unwrap().gold, 20);
    assert!(world.coins.cursor.is_empty());
}

#[test]
fn quarm_coin_moves_need_known_balances_fresh_intents_and_signed_capacity() {
    let (mut features, mut world) = setup(vec![]);
    let command = coin(
        CoinPlace::Purse,
        CoinPlace::Cursor,
        Coin::Copper,
        Coin::Copper,
        1,
    );
    assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 0);
    fund(&mut features, &mut world);
    for old_session in [true, false] {
        let ClientCommand::MoveCoins { transfer, .. } = command else {
            unreachable!()
        };
        let stale = ClientCommand::MoveCoins {
            session_id: if old_session { 4 } else { 5 },
            transfer,
            created: Instant::now()
                .checked_sub(if old_session {
                    Duration::ZERO
                } else {
                    Duration::from_secs(2)
                })
                .unwrap(),
        };
        assert_eq!(submit(&mut features, &mut world, &stale).sent.len(), 0);
    }
    let update = Message::Event(WorldEvent::CoinsElsewhere {
        cursor: Coins {
            copper: i32::MAX.unsigned_abs(),
            ..Coins::default()
        },
        bank: Coins::default(),
        given: Coins::default(),
        offered: Coins::default(),
    });
    testing::run_on(servers::server_type(ServerProtocol::Quarm).wire(), |out| {
        features.observe(&update, &mut world, out)
    })
    .result
    .unwrap();
    assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 0);
    let before = *world.coins;
    let too_large = coin(
        CoinPlace::Cursor,
        CoinPlace::Purse,
        Coin::Copper,
        Coin::Copper,
        u32::MAX,
    );
    assert_eq!(submit(&mut features, &mut world, &too_large).sent.len(), 0);
    assert_eq!(*world.coins, before);
}

#[test]
fn quarm_bank_items_and_coins_recheck_banker_identity_visibility_and_range() {
    let (mut features, mut world) = setup(vec![testing::item(22), bag(2000), stack(2031, 3)]);
    fund(&mut features, &mut world);
    let deposit = coin(CoinPlace::Purse, CoinPlace::Bank, Coin::Gold, Coin::Gold, 2);
    assert_eq!(submit(&mut features, &mut world, &deposit).sent.len(), 0);
    for (class, hidden, distance) in [
        (None, false, 0.0),
        (Some(40), true, 0.0),
        (Some(40), false, 20.1),
        (Some(40), false, f32::NAN),
    ] {
        let mut banker = testing::spawn(9, SpawnKind::Npc);
        banker.class = class;
        banker.invisible = hidden;
        banker.position.x = distance;
        world.spawns.insert(banker);
        for (from, to) in [(22, 2001), (2000, 30), (2031, 30)] {
            let command = request(&world, from, to, None);
            assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 0);
        }
        assert_eq!(submit(&mut features, &mut world, &deposit).sent.len(), 0);
    }
    let mut banker = testing::spawn(9, SpawnKind::Npc);
    banker.class = Some(40);
    banker.position.x = 20.0;
    world.spawns.insert(banker);
    assert_eq!(
        submit(&mut features, &mut world, &deposit).sent[0].body,
        words(&[1, 2, 2, 2, 2])
    );
    for (from, to, raw_from, raw_to) in [(2031, 30, 2030, 0), (30, 24, 0, 24)] {
        let command = request(&world, from, to, None);
        let output = submit(&mut features, &mut world, &command);
        assert_eq!(output.sent[0].body, words(&[raw_from, raw_to, 0]));
        paced(&mut features, &mut world);
    }
    // A banker despawning or a new zone's empty spawn table revokes access.
    world.spawns = super::super::entities::Spawns::default();
    assert_eq!(submit(&mut features, &mut world, &deposit).sent.len(), 0);
    let command = request(&world, 2000, 30, None);
    assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 0);
}

#[test]
fn quarm_banking_requires_a_live_admitted_session_even_beside_a_banker() {
    use eq_network_game::{
        world::Position,
        zoning::{Death, ZoneOffer},
    };
    for condition in 0..3 {
        let (mut features, mut world) = setup(vec![testing::item(22)]);
        fund(&mut features, &mut world);
        let mut banker = testing::spawn(9, SpawnKind::Npc);
        banker.class = Some(40);
        world.spawns.insert(banker);
        match condition {
            0 => world.admitted = None,
            1 => {
                testing::run_on(servers::server_type(ServerProtocol::Quarm).wire(), |out| {
                    features.observe(
                        &Message::Event(WorldEvent::Death(Death {
                            spawn_id: 7,
                            killer_id: 9,
                            corpse_id: 7,
                            bind_zone_id: 0,
                            corpse_name: None,
                        })),
                        &mut world,
                        out,
                    )
                })
                .result
                .unwrap();
            }
            _ => world
                .transfer_offered(
                    ZoneOffer {
                        zone_id: 2,
                        instance_id: 0,
                        position: Position::default(),
                        reason: 0,
                        to_bind: false,
                        solicited: true,
                    },
                    Instant::now(),
                )
                .unwrap(),
        }
        let command = request(&world, 22, 2000, None);
        assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 0);
        let command = coin(CoinPlace::Purse, CoinPlace::Bank, Coin::Gold, Coin::Gold, 2);
        assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 0);
    }
}

#[test]
fn quarm_inventory_composes_with_meals_and_keeps_exchanges_disabled() {
    use eq_network_game::food::Nourishment;
    let mut food = stack(22, 10);
    food.rules.item_type = 14;
    let (mut features, mut world) = setup(vec![food]);
    for capability in [
        Capability::Casting,
        Capability::Spellbook,
        Capability::Abilities,
        Capability::Looting,
        Capability::Trading,
        Capability::Meals,
    ] {
        assert!(features.capabilities().contains(&capability));
    }
    assert!(!features.capabilities().contains(&Capability::Giving));
    let output = submit(
        &mut features,
        &mut world,
        &ClientCommand::AutoEat {
            session_id: 5,
            auto_eat: AutoEat::Anything,
        },
    );
    assert!(output.result.unwrap());
    assert_eq!(output.sent, []);
    let output = submit(
        &mut features,
        &mut world,
        &ClientCommand::OfferTrade {
            session_id: 5,
            with_id: 9,
            created: Instant::now(),
        },
    );
    assert!(!output.result.unwrap());
    assert_eq!(output.sent, []);
    let output = testing::run_on(servers::server_type(ServerProtocol::Quarm).wire(), |out| {
        features.observe(
            &Message::Event(WorldEvent::Nourishment(Nourishment {
                fatigue: Some(0),
                food: 6000,
                water: 6000,
            })),
            &mut world,
            out,
        )
    });
    output.result.unwrap();
    assert_eq!(output.sent.len(), 0);
    assert_eq!(
        world.inventory.items()[&InventorySlot(22)].stack_count,
        Some(10)
    );
}

#[test]
fn quarm_assembly_decodes_stack_flags_before_admission_and_later_replacements() {
    // Same record: Quarm says a non-stack charge item while TAKP derives a stack.
    let mut record = vec![0; 360];
    record[..14].copy_from_slice(b"Synthetic item");
    record[175] = 1;
    record[176] = 1;
    record[180..182].copy_from_slice(&42u16.to_le_bytes());
    record[184..186].copy_from_slice(&22u16.to_le_bytes());
    record[246] = 20;
    record[253] = 14;
    record[276] = 3;
    record[278] = 5;
    let (mut features, mut world) = setup(vec![]);
    for flag in [3, 1] {
        record[276] = flag;
        let messages = servers::server_type(ServerProtocol::Quarm).wire().messages(
            0x3140,
            &record,
            servers::server_type(ServerProtocol::Quarm).item_rules(),
        );
        assert_eq!(messages.len(), 1);
        if flag == 3 {
            features.admit(&messages[0], &mut world).unwrap();
        } else {
            testing::run_on(servers::server_type(ServerProtocol::Quarm).wire(), |out| {
                features.observe(&messages[0], &mut world, out)
            })
            .result
            .unwrap();
        }
        assert_eq!(
            world.inventory.items()[&InventorySlot(22)].stack_count,
            (flag == 1).then_some(5)
        );
    }
    record[276] = 2;
    let messages = servers::server_type(ServerProtocol::Quarm).wire().messages(
        0x3140,
        &record,
        servers::server_type(ServerProtocol::Quarm).item_rules(),
    );
    testing::run_on(servers::server_type(ServerProtocol::Quarm).wire(), |out| {
        features.observe(&messages[0], &mut world, out)
    })
    .result
    .unwrap();
    assert!(!world.inventory.stale()); // Still available to readers such as movement.
    let command = request(&world, 22, 30, None);
    assert_eq!(submit(&mut features, &mut world, &command).sent.len(), 0);
}
