//! A change in belongings under way, through each server's real feature
//! assembly: an auto-drink and an auto-eat, and coins arriving (a split's
//! share), keep the grant in force while the host's new inputs are asked
//! for. Nothing is sent but the meal itself, so no stop and no withdrawn
//! grant reaches the server, and the moves and jump that follow go out as
//! before (gap list 3.1, fixed by "keep movement going while the player's
//! speeds are worked out again"; checklist 6.451).
use super::super::feature::testing;
use super::*;
use crate::client::ServerProtocol;
use eq_network_game::{
    food::{AutoEat, Nourishment},
    inventory::{InventoryItem, InventoryUpdate},
    movement::{HeadingUpdate, MovementMode, MovementRequest, SpeedInputState, SpeedInputs},
    world::{Capability, Coins, WorldEvent},
};

/// Every server type that moves and eats on its own.
const SERVERS: [ServerProtocol; 4] = [
    ServerProtocol::EqEmu,
    ServerProtocol::Project1999,
    ServerProtocol::Quarm,
    ServerProtocol::Takp,
];

/// A ration in the first general slot and a drink in the next.
fn meals() -> Vec<InventoryItem> {
    [(22, 14), (23, 15)]
        .into_iter()
        .map(|(slot, item_type)| {
            let mut item = testing::item(slot);
            item.rules.item_type = item_type;
            item
        })
        .collect()
}

fn setup(protocol: ServerProtocol) -> (Features, World) {
    let server = servers::server_type(protocol);
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
    for news in [
        WorldEvent::Inventory(InventoryUpdate::Snapshot(meals())),
        WorldEvent::Nourishment(Nourishment {
            fatigue: Some(0),
            food: 6000,
            water: 6000,
        }),
        WorldEvent::Coins(Coins::default()),
    ] {
        features.admit(&Message::Event(news), &mut world).unwrap();
    }
    testing::run_on(server.wire(), |out| features.admitted(&mut world, out))
        .result
        .unwrap();
    (features, world)
}

/// The revision of the last request for inputs, if one came.
fn asked(world: &mut World) -> Option<u64> {
    world
        .take_news()
        .into_iter()
        .filter_map(|message| match message {
            Message::Event(WorldEvent::SpeedInputsRequired { revision, .. }) => Some(revision),
            _ => None,
        })
        .next_back()
}

fn ready(revision: u64, weight: u16) -> ClientCommand {
    ClientCommand::SpeedInputs {
        session_id: 5,
        revision,
        report_id: revision,
        state: SpeedInputState::Ready(SpeedInputs {
            eqmac_fatigue: Some(eq_network_game::movement::EqMacFatigueInputs {
                strength: 100,
                stamina: Some(100),
            }),
            rate: 0,
            weight,
            capacity: 100,
        }),
    }
}

fn speed(world: &mut World) -> Option<f32> {
    world
        .body
        .motion_mut()
        .unwrap()
        .calibration()
        .map(|speeds| speeds.units_per_second)
}

/// Whether the events grant no speed: movement withdrawn.
fn withdrew(events: &[ClientEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            ClientEvent::World(WorldEvent::MotionState {
                units_per_second: None,
                ..
            })
        )
    })
}

/// Runs one step of the zone loop: what arrived, then the tick after it.
fn arrive(
    features: &mut Features,
    world: &mut World,
    protocol: ServerProtocol,
    news: WorldEvent,
) -> (Vec<u16>, Vec<ClientEvent>) {
    let wire = servers::server_type(protocol).wire();
    let seen = testing::run_on(wire, |out| {
        features.observe(&Message::Event(news), world, out)
    });
    seen.result.unwrap();
    let ticked = testing::run_on(wire, |out| features.tick(Instant::now(), world, out));
    ticked.result.unwrap();
    let mut opcodes: Vec<u16> = seen
        .sent
        .iter()
        .chain(&seen.unreliable)
        .chain(&ticked.sent)
        .chain(&ticked.unreliable)
        .map(|packet| packet.opcode)
        .collect();
    opcodes.sort_unstable();
    let mut events = seen.events;
    events.extend(ticked.events);
    (opcodes, events)
}

/// A forward step and a jump, as the host sends them while autorun runs
/// and a jump is pressed: the step is taken and the jump is sent.
fn move_and_jump(features: &mut Features, world: &mut World, protocol: ServerProtocol, turn: f32) {
    let wire = servers::server_type(protocol).wire();
    // Samples come at least 50 ms apart (eq-network-game's movement rule);
    // the step measures that by the clock, so wait it out.
    std::thread::sleep(Duration::from_millis(60));
    let mut position = world.body.position().unwrap();
    // A turn in place needs no wall-clock allowance for distance.
    position.heading = turn;
    let step = testing::run_on(wire, |out| {
        features.handle(
            &ClientCommand::Move(MovementRequest {
                session_id: 5,
                mode: MovementMode::Forward,
                heading: HeadingUpdate::Rate,
                position,
                created: Instant::now(),
                support: None,
                vertical_speed: None,
            }),
            world,
            out,
        )
    });
    assert!(step.result.unwrap(), "{protocol:?}");
    assert!(
        step.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::MotionSent { refused: None, .. })
        )),
        "{protocol:?}: {:?}",
        step.events
    );
    // Whether this sample goes out now is the generation's pacing; that it
    // is taken is what a withdrawn grant would end.
    let jump = testing::run_on(wire, |out| {
        features.handle(
            &ClientCommand::Jump {
                session_id: 5,
                created: Instant::now(),
            },
            world,
            out,
        )
    });
    assert!(jump.result.unwrap(), "{protocol:?}");
    assert_eq!(jump.sent.len(), 1, "{protocol:?}: {:?}", jump.events);
}

#[test]
fn an_auto_drink_an_auto_eat_or_coins_arriving_keep_autorun_and_a_jump_going() {
    for protocol in SERVERS {
        let (mut features, mut world) = setup(protocol);
        assert!(features.capabilities().contains(&Capability::Moving));
        let first = asked(&mut world).expect("inputs asked at admission");
        let wire = servers::server_type(protocol).wire();
        let granted = testing::run_on(wire, |out| {
            features.handle(&ready(first, 0), &mut world, out)
        });
        assert!(granted.result.unwrap(), "{protocol:?}");
        let full = speed(&mut world).expect("granted");
        // Leave a physics step between calibration and input without
        // sleeping, as the Quarm movement test does.
        let motion = world.body.motion_mut().unwrap();
        let calibration = motion.calibration().unwrap();
        motion.withdraw();
        motion
            .calibrate(
                calibration,
                Instant::now()
                    .checked_sub(Duration::from_millis(200))
                    .unwrap(),
            )
            .unwrap();
        move_and_jump(&mut features, &mut world, protocol, 2.0);
        let consume = match wire.dialect() {
            eq_network_game::GameDialect::EqMac => eq_network_game::food::EQMAC_CONSUME_OPCODE,
            eq_network_game::GameDialect::Titanium | eq_network_game::GameDialect::Rof2 => {
                eq_network_game::food::CONSUME_OPCODE
            }
        };
        let mut revision = first;
        for (index, (news, expected)) in (0u8..).zip([
            // Hungry and thirsty: the ration and the drink are taken on
            // their own.
            (
                WorldEvent::Nourishment(Nourishment {
                    fatigue: Some(0),
                    food: 2000,
                    water: 2000,
                }),
                vec![consume, consume],
            ),
            // A split's share arrives in the purse.
            (
                WorldEvent::Coins(Coins {
                    gold: 3,
                    ..Coins::default()
                }),
                Vec::new(),
            ),
        ]) {
            let (opcodes, events) = arrive(&mut features, &mut world, protocol, news);
            // Only the meal goes out: no stop, no other movement packet.
            assert_eq!(opcodes, expected, "{protocol:?} {index}");
            assert!(!withdrew(&events), "{protocol:?} {index}: {events:?}");
            let next = asked(&mut world).expect("inputs asked again");
            assert!(next > revision, "{protocol:?} {index}");
            revision = next;
            // The grant in force goes on meanwhile.
            assert_eq!(speed(&mut world), Some(full), "{protocol:?} {index}");
            // Autorun's next step and a jump press are taken and sent.
            move_and_jump(&mut features, &mut world, protocol, 3.0 + f32::from(index));
            // The answer replaces the speeds in place.
            let answered = testing::run_on(wire, |out| {
                features.handle(&ready(revision, 0), &mut world, out)
            });
            assert!(answered.result.unwrap(), "{protocol:?} {index}");
            assert!(!withdrew(&answered.events), "{protocol:?} {index}");
            assert_eq!(speed(&mut world), Some(full), "{protocol:?} {index}");
        }
    }
}
