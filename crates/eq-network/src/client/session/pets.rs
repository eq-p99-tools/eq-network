//! Ordinary pet orders use the session's current ownership and target
//! identities. Sending a request never predicts a pet's actions.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    message::Message,
    pets::{PetCommand, PetIdentity},
    request::Request,
    world::{SpawnKind, WorldEvent},
};

/// The set of pet orders a server provider exposes.
pub(super) enum PetOrderSet {
    Classic,
    Expanded,
}

/// Projects the entity table's commandable pet and sends its orders.
pub(super) struct Pets {
    orders: PetOrderSet,
    revision: u64,
}

impl Pets {
    pub(super) const fn new(orders: PetOrderSet) -> Self {
        Self {
            orders,
            revision: 0,
        }
    }

    fn project(&mut self, world: &mut World, event: Option<&WorldEvent>) -> Option<WorldEvent> {
        let id = if world.lifecycle.is_dead() || event.is_some_and(|event| world.owns_death(event))
        {
            None
        } else {
            world.own_spawn.and_then(|owner| world.spawns.pet(owner))
        };
        let replaced = matches!(event, Some(WorldEvent::Spawns(spawns))
            if spawns.iter().any(|spawn| id == Some(spawn.spawn_id)));
        if event.is_some() && !replaced && id == world.own_pet.map(|pet| pet.spawn_id) {
            return None;
        }
        world.own_pet = id.map(|spawn_id| {
            self.revision = self
                .revision
                .checked_add(1)
                .expect("pet revision exhausted");
            PetIdentity {
                spawn_id,
                revision: self.revision,
            }
        });
        Some(WorldEvent::OwnPet {
            session_id: world.session_id,
            pet: world.own_pet,
        })
    }
}

impl Feature for Pets {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Pets]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::Pet { .. })
    }

    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(event) = self.project(world, None) {
            out.log.send(ClientEvent::World(event))?;
        }
        Ok(())
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        // Entities has already applied the event. Publish after the original
        // event so the host sees the spawn/owner before the order identity.
        let Message::Event(
            event @ (WorldEvent::Spawns(_)
            | WorldEvent::Despawn(_)
            | WorldEvent::Death(_)
            | WorldEvent::DeathBind { .. }
            | WorldEvent::PetOwner { .. }
            | WorldEvent::Charm(_)),
        ) = message
        else {
            return Ok(());
        };
        if let Some(update) = self.project(world, Some(event)) {
            world.happened(Message::Event(update));
        }
        Ok(())
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::Pet {
            command: order,
            pet,
            target,
            ..
        } = *command
        else {
            return Ok(());
        };
        if matches!(self.orders, PetOrderSet::Classic) && !order.in_classic_order_set() {
            return actions::refuse(
                command,
                "That pet command is unavailable on this server",
                out.log,
            );
        }
        let target = if order.names_target() {
            let Some(target) =
                target.filter(|target| target.available && *target == world.target.state())
            else {
                return actions::refuse(
                    command,
                    "The target choice is unavailable or stale",
                    out.log,
                );
            };
            target.spawn_id
        } else {
            None
        };
        // A targeted Leader query asks another spawn. With no target, the
        // server asks our own pet, so that case still needs its identity.
        if order != PetCommand::Leader || target.is_none() {
            if pet != world.own_pet {
                return actions::refuse(command, "Your pet has changed", out.log);
            }
            if world.own_pet.is_none() {
                return actions::refuse_officially(
                    command,
                    ("You have no pet", Some(13091)),
                    out.log,
                );
            }
        }
        if order == PetCommand::Attack {
            let Some(id) = target else {
                return actions::refuse(
                    command,
                    "You must first select a target for your pet.",
                    out.log,
                );
            };
            if pet.is_some_and(|pet| pet.spawn_id == id)
                || world.is_player(id)
                || world
                    .spawns
                    .visible(id)
                    .is_none_or(|spawn| !matches!(spawn.kind, SpawnKind::Npc | SpawnKind::Player))
            {
                return actions::refuse(
                    command,
                    "That target is unavailable for pet attack",
                    out.log,
                );
            }
        }
        out.request(&Request::Pet {
            command: order,
            target,
        })
    }
}
