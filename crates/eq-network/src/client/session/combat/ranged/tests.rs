use super::*;
use crate::client::session::feature::{testing, Sink};
use eq_network_game::{
    combat::ranged::RangedKind,
    command::EncodedCommand,
    inventory::{InventoryItem, InventorySlot, InventoryUpdate},
};
use std::time::Duration;

fn setup(kind: RangedKind) -> World {
    let mut world = World::new(5);
    world.own_spawn = Some(7);
    let mut player = testing::player(7);
    player.skills.as_mut().unwrap()[usize::from(kind.skill())] = 20;
    world.player.admit(player);
    world.admitted = Some(Instant::now());
    let mut target = testing::spawn(8, SpawnKind::Npc);
    target.position.x = 60.0;
    world.spawns.insert(target);
    world.target = 8.into();
    world.inventory = super::super::test_ranged_inventory(kind).into();
    world
}

fn quiver(slot: i32, weight_reduction: u8) -> InventoryItem {
    let mut item = testing::item(slot);
    item.bag_slots = 6;
    item.rules.titanium_item_class = Some(1);
    item.rules.bag_type = 2;
    item.rules.bag_weight_reduction = weight_reduction;
    item
}

fn attack(world: &World, now: Instant) -> RangedAttack {
    RangedAttack {
        session_id: world.session_id,
        target: world.target.state(),
        loadout: RangedLoadout::current(&world.inventory).unwrap(),
        created: now,
    }
}

fn attempt(shots: &mut Shots, world: &World, attack: RangedAttack, now: Instant, expected: bool) {
    let command = ClientCommand::RangedAttack(attack);
    let output = testing::run(|out| shots.send(attack, &command, world, out, now));
    assert_eq!(output.result.unwrap(), expected);
    assert_eq!(output.sent.len(), usize::from(expected));
    if expected {
        assert_eq!(output.sent[0].opcode, 0x5ee8);
        assert_eq!(output.sent[0].body[8], attack.loadout.kind().skill());
    }
}

#[test]
fn first_eligible_quiver_shortens_only_bow_recovery_at_the_native_boundary() {
    for (kind, expected_ms) in [(RangedKind::Archery, 2125), (RangedKind::Throwing, 2500)] {
        let mut world = setup(kind);
        let mut inventory = (*world.inventory).clone();
        let mut fake = quiver(22, 100);
        fake.rules.titanium_item_class = Some(0);
        let mut other_bag = quiver(23, 100);
        other_bag.rules.bag_type = 1;
        for item in [
            quiver(2000, 100),
            quiver(29, 100),
            quiver(25, 60),
            quiver(24, 0),
            other_bag,
            fake,
        ] {
            inventory.apply(InventoryUpdate::Set(vec![item]));
        }
        world.inventory = inventory.into();
        let mut shots = Shots::new(SkillAdmission::ReportedRank);
        let now = Instant::now();
        let attack = attack(&world, now);
        let before = (*world.inventory).clone();
        attempt(&mut shots, &world, attack, now, true);
        attempt(
            &mut shots,
            &world,
            attack,
            now + Duration::from_millis(expected_ms - 1),
            false,
        );
        attempt(
            &mut shots,
            &world,
            attack,
            now + Duration::from_millis(expected_ms),
            true,
        );
        assert_eq!(*world.inventory, before);
    }
}

#[test]
fn a_new_loadout_uses_the_last_submission_time_and_rejects_the_old_intent() {
    let mut world = setup(RangedKind::Archery);
    let mut shots = Shots::new(SkillAdmission::ServerCap);
    let first = Instant::now();
    let old = attack(&world, first);
    attempt(&mut shots, &world, old, first, true);

    let mut inventory = (*world.inventory).clone();
    inventory.apply(InventoryUpdate::Set(vec![quiver(22, 60)]));
    world.inventory = inventory.into();
    let at_reuse = first + Duration::from_millis(2125);
    attempt(&mut shots, &world, old, at_reuse, false);
    attempt(&mut shots, &world, attack(&world, at_reuse), at_reuse, true);

    world.inventory = super::super::test_ranged_inventory(RangedKind::Archery).into();
    let next = attack(&world, at_reuse);
    attempt(
        &mut shots,
        &world,
        next,
        at_reuse + Duration::from_millis(2499),
        false,
    );
    attempt(
        &mut shots,
        &world,
        next,
        at_reuse + Duration::from_millis(2500),
        true,
    );
}

#[test]
fn unconfirmed_or_unreadable_earlier_slots_do_not_authorize_a_shorter_reuse() {
    let mut world = setup(RangedKind::Archery);
    let mut inventory = (*world.inventory).clone();
    inventory.apply(InventoryUpdate::Set(vec![quiver(24, 60)]));
    let mut unreadable = quiver(22, 100);
    unreadable.rules.titanium_item_class = None;
    inventory.apply(InventoryUpdate::Set(vec![unreadable]));
    let now = Instant::now();
    world.inventory = inventory.clone().into();
    assert_eq!(
        attack(&world, now).loadout.local_reuse_delay(),
        Duration::from_millis(2500)
    );

    inventory.apply(InventoryUpdate::Set(vec![quiver(22, 50)]));
    world.inventory = inventory.clone().into();
    assert_eq!(
        attack(&world, now).loadout.local_reuse_delay(),
        Duration::from_millis(2187)
    );
    let mut items: Vec<_> = inventory.items().values().cloned().collect();
    items
        .iter_mut()
        .find(|item| item.slot == InventorySlot(22))
        .unwrap()
        .slot = InventorySlot::CURSOR;
    inventory.apply(InventoryUpdate::Prediction(items));
    inventory.apply(InventoryUpdate::Settled);
    assert!(!inventory.custody_known(InventorySlot(22)));
    world.inventory = inventory.clone().into();
    let mut shots = Shots::new(SkillAdmission::ServerCap);
    let uncertain = attack(&world, now);
    let retry = now + Duration::from_millis(2125);
    attempt(&mut shots, &world, uncertain, now, true);
    attempt(&mut shots, &world, uncertain, retry, false);

    let items = inventory.items().values().cloned().collect();
    inventory.apply(InventoryUpdate::Snapshot(items));
    world.inventory = inventory.into();
    attempt(&mut shots, &world, attack(&world, retry), retry, true);
}

#[test]
fn quiver_reduction_is_discarded_at_or_below_one_second() {
    let world = setup(RangedKind::Archery);
    let mut inventory = (*world.inventory).clone();
    let mut weapon = inventory.items()[&InventorySlot(11)].clone();
    weapon
        .details
        .stats
        .iter_mut()
        .find(|stat| stat.label == "Delay")
        .unwrap()
        .value = 16;
    inventory.apply(InventoryUpdate::Set(vec![weapon]));
    // A positive weight-reduction byte above 100 is useful for hitting the
    // exact boundary with an integer base weapon delay.
    for (weight, expected_ms) in [(149, 1004), (150, 1600), (151, 1600)] {
        inventory.apply(InventoryUpdate::Set(vec![quiver(22, weight)]));
        assert_eq!(
            RangedLoadout::current(&inventory)
                .unwrap()
                .local_reuse_delay(),
            Duration::from_millis(expected_ms)
        );
    }
}

#[test]
fn a_failed_submission_does_not_start_the_ranged_wait() {
    struct Fails;
    impl Sink for Fails {
        fn send(&mut self, _packet: &EncodedCommand) -> Result<()> {
            anyhow::bail!("synthetic send failure")
        }
        fn send_unreliable(&mut self, packet: &EncodedCommand) -> Result<()> {
            self.send(packet)
        }
        fn last_received_seconds(&self) -> u64 {
            0
        }
    }

    let world = setup(RangedKind::Archery);
    let mut shots = Shots::new(SkillAdmission::ServerCap);
    let now = Instant::now();
    let attack = attack(&world, now);
    let command = ClientCommand::RangedAttack(attack);
    let result = testing::run(|out| {
        let mut failing = Out {
            sink: &mut Fails,
            log: out.log,
            wire: out.wire,
            sender: out.sender,
        };
        shots.send(attack, &command, &world, &mut failing, now)
    });
    assert!(result.result.is_err());
    assert!(result.events.is_empty());
    attempt(&mut shots, &world, attack, now, true);
}
