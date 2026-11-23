//! Using the potion belt (6.568) through the real casting owner: the slot
//! it chooses, what it refuses, and the slot its cast answers on.
use super::{super::casting::Casting, super::feature::testing, *};
use eq_network_game::{
    bandolier::{BeltItem, BeltUse, BELT_SLOTS},
    inventory::{ClickEffect, ClickKind, Inventory, InventoryItem, InventoryUpdate, CAST_OPCODE},
};

const SPELL: u32 = 2706;
const OWN: u16 = 7;
const POTION: u32 = 42;
const NOWHERE: &str = "You carry that item nowhere it can be used from the belt.";

struct Belt {
    features: Features,
    world: World,
}

fn potion(slot: i32) -> InventoryItem {
    let mut item = testing::item(slot);
    item.details.id = POTION;
    item.activation.effect = Some(ClickEffect {
        spell_id: SPELL,
        kind: ClickKind::Expendable,
        required_level: 0,
        effect_level: 1,
        cast_time_ms: 3000,
        recast_delay_seconds: 0,
        recast_type: 0,
    });
    item.charges = 5;
    item
}

impl Belt {
    /// A stock `EQEmu` caster taking belt uses, with a potion on belt slot 0
    /// and these items carried.
    fn new(items: Vec<InventoryItem>) -> Self {
        let features = Features {
            provided: vec![Box::new(
                Casting::with_titanium_combat(true, Duration::from_secs(4300)).with_belt_use(),
            )],
            choices: Vec::new(),
            spell_landing_flag: None,
            answers_list_polls: false,
            dialect: eq_network_game::GameDialect::Titanium,
            server_filters_at_once: false,
            gameplay_departures: false,
        };
        let mut world = World::new(5);
        world.admitted = Some(Instant::now());
        world.own_spawn = Some(OWN);
        world.player.admit(testing::player(OWN));
        let mut belt: [Option<BeltItem>; BELT_SLOTS] = Default::default();
        belt[0] = Some(BeltItem {
            item_id: POTION,
            icon: 1,
            name: "Potion".into(),
        });
        world.belt = Some(Box::new(belt));
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(items));
        world.inventory = inventory.into();
        Self { features, world }
    }

    fn use_slot(belt_slot: u8) -> ClientCommand {
        ClientCommand::UseBeltItem(BeltUse {
            request_id: 1,
            session_id: 5,
            belt_slot,
            target_id: OWN,
            created: Instant::now(),
            installed_class_spell: None,
        })
    }

    fn run(&mut self, command: ClientCommand) -> testing::Outcome<Result<()>> {
        testing::run(|out| {
            apply_commands(&[command], &mut self.features, &mut self.world, out, true)
                .map(|exit| assert!(exit.is_none()))
        })
    }

    fn held(&self) -> bool {
        self.features
            .refusal(
                &ClientCommand::CastSpell {
                    session_id: 5,
                    gem: 0,
                    spell_id: SPELL,
                    target_id: OWN,
                    created: Instant::now(),
                },
                &self.world,
                Instant::now(),
            )
            .is_some()
    }
}

/// The `OP_CastSpell` words sent, or `None` when nothing was.
fn cast(outcome: &testing::Outcome<Result<()>>) -> Option<Vec<u32>> {
    outcome.result.as_ref().unwrap();
    let packet = outcome
        .sent
        .iter()
        .find(|packet| packet.opcode == CAST_OPCODE)?;
    Some(
        packet
            .body
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| u32::from_le_bytes(*bytes))
            .collect(),
    )
}

/// The item-use refusal's words, where nothing was sent.
fn refusal(outcome: &testing::Outcome<Result<()>>) -> Option<String> {
    assert!(cast(outcome).is_none());
    outcome.events.iter().find_map(|event| match event {
        ClientEvent::World(WorldEvent::ItemUseAction {
            request_id: 1,
            error: Some(error),
            ..
        }) => Some(error.clone()),
        _ => None,
    })
}

#[test]
fn a_belt_use_casts_from_the_first_worn_then_packed_slot_on_slot_11() {
    // Worn 0 to 21 come before packs 22 to 29 (inferred until R49-45 c).
    let mut belt = Belt::new(vec![potion(23), potion(5)]);
    let sent = cast(&belt.run(Belt::use_slot(0))).expect("the belt use is sent");
    assert_eq!(sent, [11, SPELL, 5, u32::from(OWN), 0]);

    let mut belt = Belt::new(vec![potion(29), potion(23)]);
    let sent = cast(&belt.run(Belt::use_slot(0))).expect("the belt use is sent");
    assert_eq!(sent[..3], [11, SPELL, 23]);
}

#[test]
fn a_belt_use_never_casts_from_a_bag_a_bank_or_the_cursor() {
    // A bag is not a click slot for an item use (`check_item_cast`), and
    // the cursor and both banks never count (6.568 B4).
    let mut bag = testing::item(24);
    bag.bag_slots = 10;
    bag.details.id = 17;
    for slot in [271, 30, 2000, 2500] {
        let mut belt = Belt::new(vec![bag.clone(), potion(slot)]);
        assert_eq!(
            refusal(&belt.run(Belt::use_slot(0))).as_deref(),
            Some(NOWHERE),
            "{slot}"
        );
    }
}

#[test]
fn a_belt_use_refuses_an_empty_or_unknown_belt_and_a_potion_that_left() {
    let mut belt = Belt::new(vec![potion(22)]);
    assert_eq!(
        refusal(&belt.run(Belt::use_slot(1))).as_deref(),
        Some("That belt slot is empty.")
    );
    assert_eq!(
        refusal(&belt.run(Belt::use_slot(4))).as_deref(),
        Some("Choose a belt slot from 1 to 4.")
    );
    belt.world.belt = None;
    assert_eq!(
        refusal(&belt.run(Belt::use_slot(0))).as_deref(),
        Some("Your potion belt is not known; it is read again when you zone.")
    );

    let mut belt = Belt::new(Vec::new());
    assert_eq!(
        refusal(&belt.run(Belt::use_slot(0))).as_deref(),
        Some(NOWHERE)
    );
}

#[test]
fn a_belt_use_waits_for_known_contents_and_custody() {
    let mut belt = Belt::new(vec![potion(22)]);
    belt.world.inventory = Inventory::default().into();
    assert_eq!(
        refusal(&belt.run(Belt::use_slot(0))).as_deref(),
        Some("Wait for current inventory contents")
    );

    // A move the server never confirmed leaves the slot's custody
    // projected; the use does not pass it over for a later slot.
    let mut belt = Belt::new(vec![potion(22), potion(25)]);
    let mut inventory = Inventory::default();
    inventory.apply(InventoryUpdate::Snapshot(vec![potion(23), potion(25)]));
    inventory.apply(InventoryUpdate::Prediction(vec![potion(22), potion(25)]));
    inventory.apply(InventoryUpdate::Settled);
    belt.world.inventory = inventory.into();
    assert_eq!(
        refusal(&belt.run(Belt::use_slot(0))).as_deref(),
        Some(NOWHERE)
    );
}

#[test]
fn a_belt_cast_answers_only_to_the_belt_slot_refresh() {
    // 6.568 B5: a refresh on the item slot (10) is not this cast's.
    let mut belt = Belt::new(vec![potion(22)]);
    assert!(cast(&belt.run(Belt::use_slot(0))).is_some());
    assert!(belt.held());
    let refresh = |slot: u32| {
        let mut body = [0; 16];
        body[..4].copy_from_slice(&slot.to_le_bytes());
        body[4..8].copy_from_slice(&SPELL.to_le_bytes());
        body[8..12].copy_from_slice(&3u32.to_le_bytes());
        body
    };
    let server = servers::server_type(crate::client::ServerProtocol::EqEmu);
    for (slot, held) in [(10, true), (11, false)] {
        let decoded = read_messages(server.wire(), 0x308e, &refresh(slot), server.item_rules());
        assert!(!decoded.is_empty());
        testing::run(|out| hear(decoded, &mut belt.features, &mut belt.world, out, true))
            .result
            .unwrap();
        assert_eq!(belt.held(), held, "{slot}");
    }
}

#[test]
fn an_item_use_the_programs_item_path_refuses_names_its_line() {
    // Read R49-34 e 8: the level (0x88), then a worn-only effect not worn
    // (0xb3), then no charges left (0xb6).
    let line = |outcome: &testing::Outcome<Result<()>>| {
        assert!(cast(outcome).is_none());
        outcome.events.iter().find_map(|event| match event {
            ClientEvent::World(WorldEvent::ItemUseAction {
                request_id: 1,
                error: Some(_),
                string_id,
                ..
            }) => Some(*string_id),
            _ => None,
        })
    };
    let mut high = potion(23);
    high.activation.effect.as_mut().unwrap().required_level = 60;
    high.activation.effect.as_mut().unwrap().kind = ClickKind::Equipped;
    let mut belt = Belt::new(vec![high.clone()]);
    assert_eq!(
        line(&belt.run(Belt::use_slot(0))),
        Some(Some(eq_network_game::inventory::ITEM_LEVEL_LINE))
    );
    high.activation.effect.as_mut().unwrap().required_level = 0;
    let mut belt = Belt::new(vec![high]);
    assert_eq!(
        line(&belt.run(Belt::use_slot(0))),
        Some(Some(eq_network_game::inventory::ITEM_NOT_WORN_LINE))
    );
    let mut spent = potion(23);
    spent.activation.maximum_charges = 5;
    spent.charges = 0;
    spent.stack_count = None;
    let mut belt = Belt::new(vec![spent]);
    assert_eq!(
        line(&belt.run(Belt::use_slot(0))),
        Some(Some(eq_network_game::inventory::ITEM_NO_CHARGES_LINE))
    );
    // This library's own refusals name no line.
    let mut belt = Belt::new(Vec::new());
    assert_eq!(line(&belt.run(Belt::use_slot(0))), Some(None));
}

#[test]
fn an_item_used_before_its_recast_names_the_programs_line() {
    // Read R49-34 k: a send stamps the item and its type with now plus the
    // delay, in wall-clock seconds; a use before the later stamp is 0xec.
    let line = |outcome: &testing::Outcome<Result<()>>| {
        outcome.events.iter().find_map(|event| match event {
            ClientEvent::World(WorldEvent::ItemUseAction {
                request_id: 1,
                error: Some(_),
                string_id,
                ..
            }) => Some(*string_id),
            _ => None,
        })
    };
    let mut slow = potion(23);
    slow.activation
        .effect
        .as_mut()
        .unwrap()
        .recast_delay_seconds = 30;
    slow.activation.effect.as_mut().unwrap().recast_type = 4;
    let mut belt = Belt::new(vec![slow.clone()]);
    assert!(cast(&belt.run(Belt::use_slot(0))).is_some());
    let again = belt.run(Belt::use_slot(0));
    assert!(cast(&again).is_none());
    assert_eq!(line(&again), Some(Some(0xec)));
    // The server's own stamp for the instance holds it too.
    let mut stamped = slow;
    stamped.activation.recast_timestamp = u32::MAX;
    let mut belt = Belt::new(vec![stamped]);
    assert_eq!(line(&belt.run(Belt::use_slot(0))), Some(Some(0xec)));
}
