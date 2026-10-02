//! Commands to the player's pet. The session knows the pet as the spawn the
//! player owns, and refuses a command without one in the official client's
//! words; an attack names the player's target, which it needs.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::{pets, request::Request};

/// Sends the player's commands to their pet.
pub(super) struct Pets;

impl Feature for Pets {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Pets]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::Pet { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::Pet {
            command: order,
            target,
            ..
        } = command
        else {
            return Ok(());
        };
        let has_pet = world
            .own_spawn
            .is_some_and(|own| world.spawns.all().any(|spawn| spawn.pet_owner == Some(own)));
        // Asking whose pet the target is needs no pet of the player's own.
        if !has_pet && *order != pets::PetCommand::Leader {
            // The official client says so in its own words (eqstr 13091).
            return actions::refuse_officially(command, ("You have no pet", Some(13091)), out.log);
        }
        if *order == pets::PetCommand::Attack && target.is_none() {
            return actions::refuse(
                command,
                "You must first select a target for your pet.",
                out.log,
            );
        }
        let target = target.filter(|_| order.names_target());
        out.request(&Request::Pet {
            command: *order,
            target,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::{
        pets::PetCommand,
        world::{SpawnKind, WorldEvent},
    };

    fn order(command: PetCommand, target: Option<u16>) -> ClientCommand {
        ClientCommand::Pet {
            session_id: 5,
            command,
            target,
        }
    }

    fn refusals(events: &[super::super::ClientEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                super::super::ClientEvent::World(WorldEvent::PetRefused { reason, .. }) => {
                    Some(reason.clone())
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_pet_takes_commands_and_an_attack_names_the_target() {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        // No pet yet: refused, though anyone may ask whose pet a target is.
        let outcome =
            testing::run(|out| Pets.handle(&order(PetCommand::Follow, None), &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert_eq!(refusals(&outcome.events), ["You have no pet"]);
        let outcome =
            testing::run(|out| Pets.handle(&order(PetCommand::Leader, Some(9)), &mut world, out));
        assert_eq!(outcome.sent, [pets::command(PetCommand::Leader, Some(9))]);
        let mut pet = testing::spawn(8, SpawnKind::Npc);
        pet.pet_owner = Some(7);
        world.spawns.insert(pet);
        let outcome =
            testing::run(|out| Pets.handle(&order(PetCommand::Attack, Some(9)), &mut world, out));
        assert_eq!(outcome.sent, [pets::command(PetCommand::Attack, Some(9))]);
        let outcome =
            testing::run(|out| Pets.handle(&order(PetCommand::Attack, None), &mut world, out));
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert_eq!(
            refusals(&outcome.events),
            ["You must first select a target for your pet."]
        );
        // Commands that need no target send none.
        let outcome =
            testing::run(|out| Pets.handle(&order(PetCommand::Sit, Some(9)), &mut world, out));
        assert_eq!(outcome.sent, [pets::command(PetCommand::Sit, None)]);
    }
}
