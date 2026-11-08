//! The player's target: the spawn the player picks, which must be the player
//! or someone they can see, or the one the server sets itself.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    appearance::AppearanceChange, message::Message, targets::TargetState, world::WorldEvent,
};

/// The spawn the server holds as the player's target: the last one sent, or
/// the one the server set since. Every feature reads it; only the targeting
/// feature changes it.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Target {
    state: TargetState,
    request_id: u64,
    /// Only a never-seen server selection may bind on its first spawn.
    awaiting_spawn: bool,
}

impl Target {
    pub(super) const fn state(&self) -> TargetState {
        self.state
    }

    fn set(&mut self, spawn_id: Option<u16>, available: bool, awaiting_spawn: bool) {
        self.state = TargetState {
            revision: self
                .state
                .revision
                .checked_add(1)
                .expect("target revision exhausted"),
            spawn_id,
            available,
        };
        self.awaiting_spawn = awaiting_spawn;
    }
}

impl std::ops::Deref for Target {
    type Target = Option<u16>;

    fn deref(&self) -> &Option<u16> {
        &self.state.spawn_id
    }
}

/// Lets another feature's tests start with a target.
#[cfg(test)]
impl From<u16> for Target {
    fn from(spawn_id: u16) -> Self {
        Self {
            state: TargetState {
                spawn_id: Some(spawn_id),
                ..TargetState::default()
            },
            ..Self::default()
        }
    }
}

/// Picks and clears the player's target.
pub(super) struct Targeting;

impl Targeting {
    /// Runs after the entity owner has applied this event. Replacing a known
    /// incarnation never silently acquires the replacement under its ID.
    fn apply(event: &WorldEvent, world: &mut World) -> bool {
        if let WorldEvent::Target(id) = event {
            let available = id.is_none_or(|id| world.visible(id));
            let unseen = id.is_some_and(|id| {
                !world.is_player(id) && !world.spawns.all().any(|spawn| spawn.spawn_id == id)
            });
            world.target.set(*id, available, unseen);
            return true;
        }
        if world.owns_death(event) {
            world.target.set(None, true, false);
            return true;
        }
        let Some(id) = *world.target else {
            return false;
        };
        if world.is_player(id) {
            return false;
        }
        let available = match event {
            WorldEvent::Spawns(spawns) if spawns.iter().any(|spawn| spawn.spawn_id == id) => {
                world.target.awaiting_spawn && world.visible(id)
            }
            WorldEvent::Despawn(gone) if *gone == id => false,
            _ => return false,
        };
        world.target.set(Some(id), available, false);
        true
    }

    fn reconcile_sight(world: &mut World) -> bool {
        let Some(sees_invisible) = world.sight.changed() else {
            return false;
        };
        world.happened(Message::Event(WorldEvent::ObserverSight {
            session_id: world.session_id,
            sees_invisible,
        }));
        true
    }

    fn lost_visibility(world: &mut World) -> bool {
        let Some(id) = *world.target else {
            return false;
        };
        if world.target.state().available && !world.visible(id) {
            world.target.set(Some(id), false, false);
            return true;
        }
        false
    }

    fn target_event(world: &World) -> WorldEvent {
        WorldEvent::TargetChanged {
            session_id: world.session_id,
            request_id: world.target.request_id,
            state: world.target.state(),
        }
    }

    fn publish(world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        out.log.send(ClientEvent::World(Self::target_event(world)))
    }
}

impl Feature for Targeting {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![Capability::Targeting]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::SelectTarget { .. })
    }

    /// Targets the player or a spawn they can see, or clears the target; the
    /// host hears the target that went out, or why none did.
    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::SelectTarget {
            session_id,
            request_id,
            spawn_id,
            expected_pet,
        } = *command
        else {
            return Ok(());
        };
        let fresh = session_id == world.session_id && request_id > world.target.request_id;
        if fresh {
            world.target.request_id = request_id;
        }
        let available = spawn_id.is_none_or(|id| world.visible(id));
        let pet_changed = expected_pet
            .is_some_and(|pet| Some(pet) != world.own_pet || Some(pet.spawn_id) != spawn_id);
        if !available || !fresh || pet_changed {
            out.log
                .send(ClientEvent::World(WorldEvent::TargetRejected {
                    session_id,
                    request_id,
                    spawn_id,
                    state: Some(world.target.state()),
                    reason: if !fresh {
                        "Target choice is stale"
                    } else if pet_changed {
                        "Your pet has changed"
                    } else {
                        "Target is invisible or unavailable"
                    }
                    .into(),
                }))?;
            return out.log.diagnostic("Rejected an unavailable target".into());
        }
        if out.command(command)? {
            world.target.set(spawn_id, true, false);
            out.log.send(ClientEvent::World(WorldEvent::TargetSent {
                session_id,
                request_id,
                state: world.target.state(),
            }))?;
        } else {
            out.log
                .send(ClientEvent::World(WorldEvent::TargetRejected {
                    session_id,
                    request_id,
                    spawn_id,
                    state: Some(world.target.state()),
                    reason: "Target request could not be encoded".into(),
                }))?;
        }
        Ok(())
    }

    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        if let Message::Event(event) = message {
            if !Self::apply(event, world) {
                Self::lost_visibility(world);
            }
        }
        Ok(())
    }

    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(sees_invisible) = world.sight.changed() {
            out.log.send(ClientEvent::World(WorldEvent::ObserverSight {
                session_id: world.session_id,
                sees_invisible,
            }))?;
        }
        Self::publish(world, out)
    }

    fn tick(
        &mut self,
        _now: std::time::Instant,
        world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        // P99's spell replay reaches the host provider outside ordinary
        // zone-event delivery. Reevaluate before handling more zone input.
        if !world.ready() {
            return Ok(());
        }
        if Self::reconcile_sight(world) && Self::lost_visibility(world) {
            world.happened(Message::Event(Self::target_event(world)));
        }
        Ok(())
    }

    /// Follows the target the server sets: a spell's or a quest's pick, or
    /// the last target put back as the server refuses one.
    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let Message::Event(event) = message else {
            return Ok(());
        };
        let sight_changed = Self::reconcile_sight(world);
        let target_appearance_changed = match event {
            WorldEvent::Visibility { spawn_id, .. }
            | WorldEvent::Appearance {
                spawn_id,
                change: AppearanceChange::TitaniumGm(_),
            } => *world.target == Some(*spawn_id),
            WorldEvent::Death(death) => world
                .target
                .state()
                .spawn_id
                .is_some_and(|id| u32::from(id) == death.spawn_id),
            _ => false,
        };
        if Self::apply(event, world)
            || ((sight_changed || target_appearance_changed) && Self::lost_visibility(world))
        {
            // The host sees the underlying change first so it can bind the
            // same incarnation of a newly arrived spawn.
            world.happened(Message::Event(Self::target_event(world)));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::world::SpawnKind;

    fn target(request_id: u64, spawn_id: Option<u16>) -> ClientCommand {
        ClientCommand::SelectTarget {
            request_id,
            session_id: 5,
            spawn_id,
            expected_pet: None,
        }
    }

    #[test]
    fn the_target_follows_the_one_the_server_sets() {
        let mut targeting = Targeting;
        let mut world = World::new(5);
        world.target = Target::from(8);
        for spawn_id in [Some(9), None] {
            let set = Message::Event(WorldEvent::Target(spawn_id));
            let outcome = testing::run(|out| targeting.observe(&set, &mut world, out));
            outcome.result.unwrap();
            assert_eq!(*world.target, spawn_id);
            assert_eq!(outcome.sent, []);
        }
    }

    #[test]
    fn only_the_player_or_a_spawn_they_can_see_is_targeted() {
        let mut targeting = Targeting;
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.spawns.insert(testing::spawn(8, SpawnKind::Npc));
        let mut hidden = testing::spawn(9, SpawnKind::Npc);
        hidden.invisible = true;
        world.spawns.insert(hidden);
        for (request_id, spawn_id) in (1..).zip([Some(7), Some(8), None]) {
            let outcome = testing::run(|out| {
                targeting.handle(&target(request_id, spawn_id), &mut world, out)
            });
            outcome.result.unwrap();
            assert_eq!(outcome.sent.len(), 1);
            assert!(matches!(
                outcome.events[..],
                [ClientEvent::World(WorldEvent::TargetSent { state, .. })] if state.spawn_id == spawn_id
            ));
        }
        for (request_id, spawn_id) in (4..).zip([9, 10]) {
            let outcome = testing::run(|out| {
                targeting.handle(&target(request_id, Some(spawn_id)), &mut world, out)
            });
            outcome.result.unwrap();
            assert!(
                outcome.sent.is_empty(),
                "expected nothing sent for spawn {spawn_id:?}, got {:?}",
                outcome.sent
            );
            assert!(matches!(
                outcome.events[..],
                [
                    ClientEvent::World(WorldEvent::TargetRejected { .. }),
                    ClientEvent::Diagnostic(_)
                ]
            ));
        }
    }
}
