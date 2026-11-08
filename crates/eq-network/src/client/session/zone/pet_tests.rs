//! Pet orders through zone admission, entity updates and the command gate.
use super::super::feature::testing;
use super::*;
use crate::client::ServerProtocol;
use eq_network_game::{
    control::Charm,
    food::AutoEat,
    pets::{self, PetCommand, PetIdentity},
    world::{SpawnKind, WorldEvent},
    zoning::Death,
};

struct Session {
    features: Features,
    world: World,
}

#[derive(Debug, PartialEq, Eq)]
enum PetObservation {
    Unreported,
    Cleared,
    Owned(PetIdentity),
}

impl PetObservation {
    fn identity(self) -> Option<PetIdentity> {
        match self {
            Self::Owned(pet) => Some(pet),
            Self::Cleared | Self::Unreported => None,
        }
    }
}

impl Session {
    fn new(protocol: ServerProtocol, session_id: u64) -> Self {
        let mut features =
            Features::new(servers::server_type(protocol), "Tester", AutoEat::default());
        let mut world = World::new(session_id);
        world.own_spawn = Some(7);
        testing::run(|out| {
            admit(
                testing::player(7),
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
        })
        .result
        .unwrap();
        Self { features, world }
    }

    fn hear(&mut self, event: WorldEvent) -> PetObservation {
        let outcome = testing::run(|out| {
            hear(
                vec![Message::Event(event)],
                &mut self.features,
                &mut self.world,
                out,
                true,
            )
        });
        assert!(outcome.result.unwrap().is_none());
        outcome
            .events
            .iter()
            .find_map(|event| match event {
                ClientEvent::World(WorldEvent::OwnPet { session_id, pet })
                    if *session_id == self.world.session_id =>
                {
                    Some(pet.map_or(PetObservation::Cleared, PetObservation::Owned))
                }
                _ => None,
            })
            .unwrap_or(PetObservation::Unreported)
    }

    fn order(&self, command: PetCommand, pet: Option<PetIdentity>) -> ClientCommand {
        ClientCommand::Pet {
            session_id: self.world.session_id,
            command,
            pet,
            target: command.names_target().then(|| self.world.target.state()),
            created: Instant::now(),
        }
    }

    fn send(&mut self, command: ClientCommand) -> testing::Outcome<Result<Option<ZoneExit>>> {
        testing::run(|out| {
            apply_commands(&[command], &mut self.features, &mut self.world, out, true)
        })
    }
}

fn npc(id: u16, owner: Option<u16>) -> eq_network_game::world::SpawnState {
    let mut spawn = testing::spawn(id, SpawnKind::Npc);
    spawn.pet_owner = owner;
    spawn
}

fn charm(owner_id: Option<u16>, pet_id: u16, charmed: bool) -> WorldEvent {
    WorldEvent::Charm(Charm {
        owner_id,
        pet_id,
        charmed,
    })
}

fn death(id: u16) -> WorldEvent {
    WorldEvent::Death(Death {
        spawn_id: u32::from(id),
        killer_id: 0,
        corpse_id: u32::from(id),
        bind_zone_id: 0,
        corpse_name: None,
    })
}

fn refused(outcome: testing::Outcome<Result<Option<ZoneExit>>>, expected: &str) {
    assert!(outcome.result.unwrap().is_none());
    assert_eq!(
        outcome.sent,
        [] as [eq_network_game::command::EncodedCommand; 0]
    );
    assert!(outcome.events.iter().any(|event| {
        matches!(event, ClientEvent::World(WorldEvent::PetRefused { reason, .. }) if reason == expected)
    }));
}

fn sent(
    outcome: testing::Outcome<Result<Option<ZoneExit>>>,
    command: PetCommand,
    target: Option<u16>,
) {
    assert!(outcome.result.unwrap().is_none());
    assert_eq!(outcome.sent, [pets::command(command, target)]);
    assert!(outcome.events.is_empty());
}

#[test]
fn pet_ownership_changes_invalidate_orders_even_when_the_id_returns() {
    let mut zone = Session::new(ServerProtocol::EqEmu, 5);
    refused(
        zone.send(zone.order(PetCommand::Follow, None)),
        "You have no pet",
    );

    // A charm of an unseen ID does not attach itself to a later spawn.
    assert_eq!(
        zone.hear(charm(Some(7), 8, true)),
        PetObservation::Unreported
    );
    assert_eq!(
        zone.hear(WorldEvent::Spawns(vec![npc(8, None)])),
        PetObservation::Unreported
    );
    let pet = zone.hear(charm(Some(7), 8, true)).identity().unwrap();
    assert_eq!(pet.spawn_id, 8);
    let follow = zone.order(PetCommand::Follow, Some(pet));
    assert_eq!(zone.hear(charm(None, 8, false)), PetObservation::Unreported);
    assert_eq!(
        zone.hear(charm(Some(9), 8, false)),
        PetObservation::Unreported
    );
    sent(zone.send(follow.clone()), PetCommand::Follow, None);

    // Another player becomes the owner. Our late release cannot remove it.
    assert_eq!(zone.hear(charm(Some(9), 8, true)), PetObservation::Cleared);
    assert_eq!(
        zone.hear(charm(Some(7), 8, false)),
        PetObservation::Unreported
    );
    assert_eq!(zone.world.spawns.pet(9), Some(8));
    refused(zone.send(follow.clone()), "Your pet has changed");
    let pet = zone.hear(charm(Some(7), 8, true)).identity().unwrap();
    assert_eq!(pet.spawn_id, 8);
    refused(zone.send(follow), "Your pet has changed");

    // A duplicate appearance report is normal. An actual death drops the
    // identity even while the corpse still uses that ID.
    assert_eq!(
        zone.hear(WorldEvent::PetOwner {
            spawn_id: 8,
            owner: Some(7)
        }),
        PetObservation::Unreported
    );
    let back = zone.order(PetCommand::BackOff, Some(pet));
    sent(zone.send(back.clone()), PetCommand::BackOff, None);
    assert_eq!(zone.hear(death(8)), PetObservation::Cleared);
    assert_eq!(
        zone.world.spawns.visible(8).unwrap().kind,
        SpawnKind::NpcCorpse
    );
    refused(zone.send(back.clone()), "Your pet has changed");
    let pet = zone
        .hear(WorldEvent::Spawns(vec![npc(8, Some(7))]))
        .identity()
        .unwrap();
    assert_eq!(pet.spawn_id, 8);
    refused(zone.send(back), "Your pet has changed");

    // An owned replacement can arrive without a preceding Despawn.
    let replaced = zone
        .hear(WorldEvent::Spawns(vec![npc(8, Some(7))]))
        .identity()
        .unwrap();
    assert_ne!(pet, replaced);
    let previous_admission = zone.order(PetCommand::Follow, Some(replaced));
    let mut next = Session::new(ServerProtocol::EqEmu, 6);
    let _ = next.hear(WorldEvent::Spawns(vec![npc(8, Some(7))]));
    refused(
        next.send(previous_admission),
        "That was meant for an earlier zone visit",
    );
}

#[test]
fn pet_attack_uses_the_current_living_target_and_never_predicts_an_outcome() {
    let mut zone = Session::new(ServerProtocol::EqEmu, 5);
    let mut distant = npc(9, None);
    distant.position.x = 2_000.0;
    let pet = zone
        .hear(WorldEvent::Spawns(vec![npc(8, Some(7)), distant]))
        .identity();
    refused(
        zone.send(zone.order(PetCommand::Attack, pet)),
        "You must first select a target for your pet.",
    );

    let _ = zone.hear(WorldEvent::Target(Some(9)));
    let attack = zone.order(PetCommand::Attack, pet);
    sent(zone.send(attack.clone()), PetCommand::Attack, Some(9));
    let _ = zone.hear(WorldEvent::Despawn(9));
    let _ = zone.hear(WorldEvent::Spawns(vec![npc(9, None)]));
    refused(
        zone.send(attack.clone()),
        "The target choice is unavailable or stale",
    );

    let _ = zone.hear(WorldEvent::Target(Some(9)));
    refused(
        zone.send(attack),
        "The target choice is unavailable or stale",
    );
    sent(
        zone.send(zone.order(PetCommand::Attack, pet)),
        PetCommand::Attack,
        Some(9),
    );

    let _ = zone.hear(death(9));
    refused(
        zone.send(zone.order(PetCommand::Attack, pet)),
        "That target is unavailable for pet attack",
    );
    let _ = zone.hear(WorldEvent::Target(Some(8)));
    refused(
        zone.send(zone.order(PetCommand::Attack, pet)),
        "That target is unavailable for pet attack",
    );
}

#[test]
fn pet_target_selection_refuses_an_owned_replacement_with_the_same_id() {
    let mut zone = Session::new(ServerProtocol::EqEmu, 5);
    let old_pet = zone
        .hear(WorldEvent::Spawns(vec![npc(8, Some(7))]))
        .identity();
    let replacement = zone
        .hear(WorldEvent::Spawns(vec![npc(8, Some(7))]))
        .identity();
    let select = |request_id, expected_pet| ClientCommand::SelectTarget {
        session_id: 5,
        request_id,
        spawn_id: Some(8),
        expected_pet,
    };
    let stale = zone.send(select(1, old_pet));
    stale.result.unwrap();
    assert_eq!(
        stale.sent,
        [] as [eq_network_game::command::EncodedCommand; 0]
    );
    assert!(stale.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::TargetRejected { reason, .. }) if reason == "Your pet has changed"
    )));
    assert!(zone.world.target.is_none());

    let fresh = zone.send(select(2, replacement));
    fresh.result.unwrap();
    assert_eq!(fresh.sent.len(), 1);
    assert_eq!(*zone.world.target, Some(8));
}

#[test]
fn p99_refuses_later_commands_and_leader_can_ask_a_visible_pet_without_owning_one() {
    let mut zone = Session::new(ServerProtocol::Project1999, 5);
    let _ = zone.hear(WorldEvent::Spawns(vec![npc(9, Some(10))]));
    let _ = zone.hear(WorldEvent::Target(Some(9)));
    sent(
        zone.send(zone.order(PetCommand::Leader, None)),
        PetCommand::Leader,
        Some(9),
    );
    let _ = zone.hear(WorldEvent::Visibility {
        spawn_id: 9,
        state: eq_network_game::vision::Invisibility::Titanium(1),
    });
    refused(
        zone.send(zone.order(PetCommand::Leader, None)),
        "The target choice is unavailable or stale",
    );

    let pet = zone
        .hear(WorldEvent::Spawns(vec![npc(8, Some(7))]))
        .identity();
    let mut expanded = Session::new(ServerProtocol::EqEmu, 5);
    let expanded_pet = expanded
        .hear(WorldEvent::Spawns(vec![npc(8, Some(7))]))
        .identity();
    for order in [
        PetCommand::Hold,
        PetCommand::Feign,
        PetCommand::NoCast,
        PetCommand::Focus,
    ] {
        refused(
            zone.send(zone.order(order, pet)),
            "That pet command is unavailable on this server",
        );
        sent(
            expanded.send(expanded.order(order, expanded_pet)),
            order,
            None,
        );
    }
    sent(
        zone.send(zone.order(PetCommand::GuardMe, pet)),
        PetCommand::GuardMe,
        None,
    );
}
