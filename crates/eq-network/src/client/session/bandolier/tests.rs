use super::super::feature::testing;
use super::*;
use eq_network_game::{
    inventory::{Inventory, InventoryUpdate},
    world::Capability,
};

const SESSION: u64 = 5;
const ALL: Offered = Offered {
    sets: true,
    swap: true,
    belt: true,
};

fn worn(slot: i32, id: u32, name: &str) -> InventoryItem {
    let mut item = testing::item(slot);
    item.details.id = id;
    item.details.icon = Some(id + 1000);
    item.details.name = name.into();
    item
}

fn potion(slot: i32, id: u32) -> InventoryItem {
    let mut item = worn(slot, id, "Potion");
    item.rules.item_type = bandolier::POTION;
    item
}

/// A world holding a sword in the primary, a shield in the secondary, a
/// potion in pack 22, a potion in a bag (251), and a potion in the bank.
fn world() -> World {
    let mut world = World::new(SESSION);
    let mut inventory = Inventory::default();
    inventory.apply(InventoryUpdate::Snapshot(vec![
        worn(13, 100, "Sword"),
        worn(14, 101, "Shield"),
        potion(22, 200),
        potion(251, 201),
        potion(2000, 202),
        worn(23, 300, "Rock"),
    ]));
    world.inventory = inventory.into();
    world
}

fn sets() -> Sets {
    let mut sets: Sets = Default::default();
    sets[1] = Some(BandolierSet {
        name: "Old".into(),
        items: [None, None, None, None],
    });
    sets
}

fn belt() -> Belt {
    let mut belt: Belt = Default::default();
    belt[2] = Some(BeltItem {
        item_id: 200,
        icon: 1200,
        name: "Potion".into(),
    });
    belt
}

/// A feature taking both kinds of command, admitted with `sets()` and
/// `belt()`.
fn admitted() -> Bandolier {
    let mut feature = Bandolier::sending(ALL);
    let mut world = World::new(SESSION);
    for message in [
        Message::Event(WorldEvent::Bandoliers(Box::new(sets()))),
        Message::Event(WorldEvent::PotionBelt(Box::new(belt()))),
    ] {
        feature.admit(&message, &mut world).unwrap();
    }
    feature
}

fn save(set: u8, name: &str) -> ClientCommand {
    ClientCommand::SaveBandolier {
        session_id: SESSION,
        set,
        name: name.into(),
    }
}

fn clear(set: u8) -> ClientCommand {
    ClientCommand::ClearBandolier {
        session_id: SESSION,
        set,
    }
}

fn set_belt(slot: u8, item_id: u32) -> ClientCommand {
    ClientCommand::SetPotionBelt {
        session_id: SESSION,
        slot,
        item_id,
    }
}

fn swap(set: u8) -> ClientCommand {
    ClientCommand::SwapBandolier {
        session_id: SESSION,
        set,
    }
}

fn clear_belt(slot: u8) -> ClientCommand {
    ClientCommand::ClearPotionBelt {
        session_id: SESSION,
        slot,
    }
}

fn run(
    feature: &mut Bandolier,
    world: &mut World,
    command: &ClientCommand,
) -> testing::Outcome<Result<()>> {
    // Each step comes after the floor, as a player's clicks would.
    feature.sent = None;
    let outcome = testing::run(|out| feature.handle(command, world, out));
    outcome.result.as_ref().unwrap();
    outcome
}

/// The refusal's words, where the step was refused and sent nothing.
fn refusal(outcome: &testing::Outcome<Result<()>>) -> Option<String> {
    if !outcome.sent.is_empty() {
        return None;
    }
    outcome.events.iter().find_map(|event| match event {
        ClientEvent::World(WorldEvent::BandolierRefused { session_id, reason })
            if *session_id == SESSION =>
        {
            Some(reason.clone())
        }
        _ => None,
    })
}

fn told_sets(outcome: &testing::Outcome<Result<()>>) -> &Sets {
    outcome
        .events
        .iter()
        .find_map(|event| match event {
            ClientEvent::World(WorldEvent::Bandoliers(sets)) => Some(&**sets),
            _ => None,
        })
        .expect("the sets are told")
}

fn told_belt(outcome: &testing::Outcome<Result<()>>) -> &Belt {
    outcome
        .events
        .iter()
        .find_map(|event| match event {
            ClientEvent::World(WorldEvent::PotionBelt(belt)) => Some(&**belt),
            _ => None,
        })
        .expect("the belt is told")
}

#[test]
fn the_profiles_copy_is_told_at_admission_on_every_titanium_server() {
    let mut feature = Bandolier::reading();
    let mut world = World::new(SESSION);
    for message in [
        Message::Event(WorldEvent::Bandoliers(Box::new(sets()))),
        Message::Event(WorldEvent::PotionBelt(Box::new(belt()))),
    ] {
        feature.admit(&message, &mut world).unwrap();
    }
    let told = testing::run(|out| feature.admitted(&mut world, out));
    assert!(matches!(
        &told.events[..],
        [
            ClientEvent::World(WorldEvent::Bandoliers(sets)),
            ClientEvent::World(WorldEvent::PotionBelt(belt)),
        ] if **sets == super::tests::sets() && **belt == super::tests::belt()
    ));
    assert_eq!(feature.capabilities(), Vec::new());
    for command in [
        save(0, "A"),
        clear(1),
        swap(1),
        set_belt(0, 200),
        clear_belt(2),
    ] {
        assert!(!feature.owns(&command), "{command:?}");
    }
}

#[test]
fn an_unreadable_profile_is_told_and_refuses_all_four() {
    let mut feature = Bandolier::sending(ALL);
    let mut world = world();
    feature
        .admit(&Message::Event(WorldEvent::BandoliersUnknown), &mut world)
        .unwrap();
    let told = testing::run(|out| feature.admitted(&mut world, out));
    assert!(matches!(
        &told.events[..],
        [ClientEvent::World(WorldEvent::BandoliersUnknown)]
    ));
    for command in [
        save(0, "A"),
        clear(1),
        swap(1),
        set_belt(0, 200),
        clear_belt(2),
    ] {
        let outcome = run(&mut feature, &mut world, &command);
        assert_eq!(refusal(&outcome).as_deref(), Some(UNKNOWN), "{command:?}");
    }
    // Before any profile, too.
    let mut unread = Bandolier::sending(ALL);
    let outcome = run(&mut unread, &mut world, &clear(1));
    assert_eq!(refusal(&outcome).as_deref(), Some(UNKNOWN));
}

#[test]
fn each_kind_is_offered_and_taken_only_once_its_row_passes() {
    let only = |sets, swap, belt| Bandolier::sending(Offered { sets, swap, belt });
    let sets = only(true, false, false);
    assert_eq!(sets.capabilities(), [Capability::Bandoliers]);
    assert!(sets.owns(&save(0, "A")) && sets.owns(&clear(0)));
    assert!(!sets.owns(&set_belt(0, 1)) && !sets.owns(&clear_belt(0)));
    assert!(!sets.owns(&swap(0)));
    let swaps = only(false, true, false);
    assert_eq!(swaps.capabilities(), [Capability::BandolierSwap]);
    assert!(swaps.owns(&swap(0)) && !swaps.owns(&save(0, "A")));
    let belt = only(false, false, true);
    assert_eq!(belt.capabilities(), [Capability::PotionBelt]);
    assert!(belt.owns(&set_belt(0, 1)) && belt.owns(&clear_belt(0)));
    assert!(!belt.owns(&save(0, "A")));
}

#[test]
fn a_save_copies_the_worn_weapons_into_the_set_and_tells_it() {
    let mut feature = admitted();
    let mut world = world();
    let outcome = run(&mut feature, &mut world, &save(0, "Tank"));
    assert_eq!(outcome.sent.len(), 1);
    assert_eq!(outcome.sent[0].opcode, bandolier::OPCODE);
    assert_eq!(outcome.sent[0].body, bandolier::save(0, "Tank").unwrap());
    let sets = told_sets(&outcome);
    let item = |id: u32, name: &str| {
        Some(BeltItem {
            item_id: id,
            icon: id + 1000,
            name: name.into(),
        })
    };
    assert_eq!(
        sets[0],
        Some(BandolierSet {
            name: "Tank".into(),
            // The range and ammo slots are empty.
            items: [item(100, "Sword"), item(101, "Shield"), None, None],
        })
    );
    // The other sets are as they were.
    assert_eq!(sets[1], super::tests::sets()[1]);
    // A save over a set replaces it.
    let outcome = run(&mut feature, &mut world, &save(1, "New"));
    assert_eq!(told_sets(&outcome)[1].as_ref().unwrap().name, "New");
}

#[test]
fn a_clear_empties_the_set_and_an_empty_set_is_not_cleared() {
    let mut feature = admitted();
    let mut world = world();
    let outcome = run(&mut feature, &mut world, &clear(1));
    assert_eq!(outcome.sent.len(), 1);
    assert_eq!(outcome.sent[0].body, bandolier::clear(1).unwrap());
    assert_eq!(told_sets(&outcome)[1], None);
    for set in [1, 0] {
        let outcome = run(&mut feature, &mut world, &clear(set));
        assert_eq!(
            refusal(&outcome).as_deref(),
            Some("That set is already empty.")
        );
    }
}

#[test]
fn a_bad_set_or_name_is_refused_in_our_words() {
    let mut feature = admitted();
    let mut world = world();
    for command in [
        save(4, "A"),
        save(0, ""),
        save(0, &"a".repeat(32)),
        save(0, "Bad\u{7}"),
        save(0, "Caf\u{e9}"),
        clear(4),
        swap(4),
        set_belt(4, 200),
        set_belt(0, 0),
        clear_belt(4),
    ] {
        let outcome = run(&mut feature, &mut world, &command);
        assert!(
            refusal(&outcome).is_some_and(|reason| reason.starts_with("Choose")),
            "{command:?}"
        );
    }
    // A name of 31 goes.
    let outcome = run(&mut feature, &mut world, &save(3, &"a".repeat(31)));
    assert_eq!(outcome.sent.len(), 1);
}

#[test]
fn a_save_waits_for_a_known_inventory() {
    let mut feature = admitted();
    let mut unknown = World::new(SESSION);
    let outcome = run(&mut feature, &mut unknown, &save(0, "A"));
    assert_eq!(
        refusal(&outcome).as_deref(),
        Some("Wait until your inventory is known to save a set.")
    );
    let mut moving = world();
    let mut inventory = Inventory::default();
    inventory.apply(InventoryUpdate::Snapshot(vec![worn(13, 100, "Sword")]));
    inventory.apply(InventoryUpdate::Prediction(vec![worn(30, 100, "Sword")]));
    moving.inventory = inventory.into();
    let outcome = run(&mut feature, &mut moving, &save(0, "A"));
    assert!(refusal(&outcome).is_some());
    // A cast or a move in flight holds the save before it reaches here.
    assert_eq!(
        actions::needs(&save(0, "A")),
        [actions::Resource::Casting, actions::Resource::Inventory]
    );
    for command in [clear(0), set_belt(0, 1), clear_belt(0)] {
        assert_eq!(actions::needs(&command), [], "{command:?}");
    }
}

#[test]
fn the_belt_takes_a_carried_potion_and_tells_the_belt() {
    let mut feature = admitted();
    let mut world = world();
    let outcome = run(&mut feature, &mut world, &set_belt(0, 201));
    assert_eq!(outcome.sent.len(), 1);
    assert_eq!(outcome.sent[0].opcode, bandolier::BELT_OPCODE);
    assert_eq!(outcome.sent[0].body, bandolier::set_belt(0, 201).unwrap());
    let belt = told_belt(&outcome);
    assert_eq!(
        belt[0],
        Some(BeltItem {
            item_id: 201,
            icon: 1201,
            name: "Potion".into(),
        })
    );
    assert_eq!(belt[2], super::tests::belt()[2]);
}

#[test]
fn an_item_made_for_the_belt_goes_on_it_too() {
    let mut feature = admitted();
    let mut world = world();
    let mut inventory = Inventory::default();
    let mut flagged = worn(22, 400, "Elixir");
    flagged.rules.potion_belt = true;
    inventory.apply(InventoryUpdate::Snapshot(vec![flagged]));
    world.inventory = inventory.into();
    let outcome = run(&mut feature, &mut world, &set_belt(1, 400));
    assert_eq!(outcome.sent.len(), 1);
    assert_eq!(told_belt(&outcome)[1].as_ref().unwrap().item_id, 400);
}

#[test]
fn the_belt_refuses_what_is_not_carried_or_not_a_potion() {
    let mut feature = admitted();
    let mut world = world();
    // In the bank, and not held at all.
    for item_id in [202, 999] {
        let outcome = run(&mut feature, &mut world, &set_belt(0, item_id));
        assert_eq!(
            refusal(&outcome).as_deref(),
            Some("You do not carry that item."),
            "{item_id}"
        );
    }
    // On the cursor.
    let mut cursor = World::new(SESSION);
    let mut inventory = Inventory::default();
    inventory.apply(InventoryUpdate::Snapshot(vec![potion(30, 203)]));
    cursor.inventory = inventory.into();
    let outcome = run(&mut feature, &mut cursor, &set_belt(0, 203));
    assert_eq!(
        refusal(&outcome).as_deref(),
        Some("You do not carry that item.")
    );
    // Carried, but a rock.
    let outcome = run(&mut feature, &mut world, &set_belt(0, 300));
    assert_eq!(
        refusal(&outcome).as_deref(),
        Some("Only a potion goes on the belt.")
    );
    // No inventory yet.
    let outcome = run(&mut feature, &mut World::new(SESSION), &set_belt(0, 200));
    assert_eq!(
        refusal(&outcome).as_deref(),
        Some("Wait until your inventory is known to fill the belt.")
    );
}

#[test]
fn a_belt_clear_names_the_item_and_an_empty_slot_is_not_cleared() {
    let mut feature = admitted();
    let mut world = world();
    let outcome = run(&mut feature, &mut world, &clear_belt(2));
    assert_eq!(outcome.sent.len(), 1);
    assert_eq!(outcome.sent[0].body, bandolier::clear_belt(2, 200).unwrap());
    assert_eq!(told_belt(&outcome)[2], None);
    for slot in [2, 0] {
        let outcome = run(&mut feature, &mut world, &clear_belt(slot));
        assert_eq!(
            refusal(&outcome).as_deref(),
            Some("That belt slot is already empty.")
        );
    }
}

#[test]
fn sends_are_a_floor_apart() {
    let mut feature = admitted();
    let mut world = world();
    let first = testing::run(|out| feature.handle(&save(0, "A"), &mut world, out));
    assert_eq!(first.sent.len(), 1);
    let second = testing::run(|out| feature.handle(&set_belt(0, 200), &mut world, out));
    assert_eq!(
        refusal(&second).as_deref(),
        Some("You just changed your bandolier.")
    );
    feature.sent = Instant::now().checked_sub(FLOOR);
    let third = testing::run(|out| feature.handle(&set_belt(0, 200), &mut world, out));
    assert_eq!(third.sent.len(), 1);
    // A refusal sets no floor.
    let mut feature = admitted();
    let refused = testing::run(|out| feature.handle(&clear(0), &mut world, out));
    assert!(refusal(&refused).is_some());
    let next = testing::run(|out| feature.handle(&clear(1), &mut world, out));
    assert_eq!(next.sent.len(), 1);
}

#[test]
fn a_swap_sends_and_predicts_every_move_the_server_makes() {
    let mut feature = admitted();
    let mut world = world();
    // Save set 0 with the sword and the shield worn.
    let saved = run(&mut feature, &mut world, &save(0, "Tank"));
    assert_eq!(saved.sent.len(), 1);
    let mut world = World::new(SESSION);
    let mut inventory = Inventory::default();
    let mut sword = worn(13, 100, "Sword");
    sword.rules.titanium_item_class = Some(0);
    let mut shield = worn(14, 101, "Shield");
    shield.rules.titanium_item_class = Some(0);
    inventory.apply(InventoryUpdate::Snapshot(vec![sword, shield]));
    world.inventory = inventory.into();
    // Swap to the set just saved: both are found where they are.
    let outcome = run(&mut feature, &mut world, &swap(0));
    assert_eq!(outcome.sent.len(), 1);
    assert_eq!(outcome.sent[0].opcode, bandolier::OPCODE);
    assert_eq!(outcome.sent[0].body, bandolier::swap_to(0).unwrap());
    assert!(outcome.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::Inventory(InventoryUpdate::Prediction(_)))
    )));
    // Nothing moved, so nothing is left to settle.
    assert!(!world.inventory.predicted());
    // Swap to set 1, which names nothing: both go back to the packs.
    let outcome = run(&mut feature, &mut world, &swap(1));
    assert_eq!(outcome.sent.len(), 1);
    let at = |slot: i32| {
        world
            .inventory
            .items()
            .get(&eq_network_game::inventory::InventorySlot(slot))
            .map(|item| item.details.id)
    };
    assert_eq!(
        (at(13), at(14), at(22), at(23)),
        (None, None, Some(100), Some(101))
    );
    // Until those moves settle, the next swap waits.
    let outcome = run(&mut feature, &mut world, &swap(0));
    assert_eq!(
        refusal(&outcome).as_deref(),
        Some("Wait until your inventory is known to swap sets.")
    );
}

#[test]
fn a_swap_to_an_empty_set_returns_the_weapons() {
    let mut feature = admitted();
    let mut world = World::new(SESSION);
    let mut inventory = Inventory::default();
    let mut sword = worn(13, 100, "Sword");
    sword.rules.titanium_item_class = Some(0);
    inventory.apply(InventoryUpdate::Snapshot(vec![sword]));
    world.inventory = inventory.into();
    // Set 1 holds a name and no item.
    let outcome = run(&mut feature, &mut world, &swap(1));
    assert_eq!(outcome.sent.len(), 1);
    let items = world.inventory.items();
    assert_eq!(items.keys().map(|slot| slot.0).collect::<Vec<_>>(), [22]);
    // Set 2 is empty: nothing is sent.
    let outcome = run(&mut feature, &mut world, &swap(2));
    assert_eq!(refusal(&outcome).as_deref(), Some("That set is empty."));
}

#[test]
fn a_swap_waits_for_a_merchant_trade_or_container_and_the_mirrors_refusals() {
    let mut feature = admitted();
    let mut world = World::new(SESSION);
    let mut inventory = Inventory::default();
    let mut sword = worn(13, 100, "Sword");
    sword.rules.titanium_item_class = Some(0);
    let cursor = potion(30, 200);
    inventory.apply(InventoryUpdate::Snapshot(vec![sword, cursor]));
    world.inventory = inventory.into();
    // The mirror's own words for the cursor.
    let outcome = run(&mut feature, &mut world, &swap(1));
    assert_eq!(
        refusal(&outcome).as_deref(),
        Some("Put down what you hold before swapping sets.")
    );
    let opened = Message::Event(WorldEvent::Merchant(MerchantUpdate::Opened {
        merchant_id: 9,
        accepted: true,
        rate: 1.0,
    }));
    testing::run(|out| feature.observe(&opened, &mut world, out))
        .result
        .unwrap();
    let outcome = run(&mut feature, &mut world, &swap(1));
    assert_eq!(
        refusal(&outcome).as_deref(),
        Some("Close the merchant's window before swapping sets.")
    );
    let gone = Message::Event(WorldEvent::Despawn(9));
    testing::run(|out| feature.observe(&gone, &mut world, out))
        .result
        .unwrap();
    let outcome = run(&mut feature, &mut world, &swap(1));
    assert_eq!(
        refusal(&outcome).as_deref(),
        Some("Put down what you hold before swapping sets.")
    );
    // A cast, a move, a station and a trader's window hold it first.
    assert_eq!(
        actions::needs(&swap(0)),
        [
            actions::Resource::Casting,
            actions::Resource::Inventory,
            actions::Resource::CursorAndStation,
            actions::Resource::Shopping,
        ]
    );
}
