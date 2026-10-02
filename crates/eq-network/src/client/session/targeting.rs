//! The player's target: the spawn the player picks, which must be the player
//! or someone they can see.
use super::{
    feature::{Encoder, Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::world::WorldEvent;

/// The spawn the server holds as the player's target: the last one sent.
/// Every feature reads it; only the targeting feature changes it.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Target(Option<u16>);

impl std::ops::Deref for Target {
    type Target = Option<u16>;

    fn deref(&self) -> &Option<u16> {
        &self.0
    }
}

/// Lets another feature's tests start with a target.
#[cfg(test)]
impl From<u16> for Target {
    fn from(spawn_id: u16) -> Self {
        Self(Some(spawn_id))
    }
}

/// Picks and clears the player's target.
pub(super) struct Targeting {
    encoder: Encoder,
}

impl Targeting {
    pub(super) fn new(encoder: Encoder) -> Self {
        Self { encoder }
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
            spawn_id,
        } = *command
        else {
            return Ok(());
        };
        let available = spawn_id.is_none_or(|id| world.visible(id));
        if !available {
            out.log
                .send(ClientEvent::World(WorldEvent::TargetRejected {
                    session_id,
                    spawn_id,
                    reason: "Target is invisible or unavailable".into(),
                }))?;
            return out.log.diagnostic("Rejected an unavailable target".into());
        }
        if self.encoder.send(command, out)? {
            world.target.0 = spawn_id;
            out.log
                .send(ClientEvent::World(WorldEvent::TargetSent(spawn_id)))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::world::SpawnKind;

    fn target(spawn_id: Option<u16>) -> ClientCommand {
        ClientCommand::SelectTarget {
            session_id: 5,
            spawn_id,
        }
    }

    #[test]
    fn only_the_player_or_a_spawn_they_can_see_is_targeted() {
        let mut targeting = Targeting::new(Encoder::new("Tester"));
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.spawns.insert(testing::spawn(8, SpawnKind::Npc));
        let mut hidden = testing::spawn(9, SpawnKind::Npc);
        hidden.invisible = true;
        world.spawns.insert(hidden);
        for spawn_id in [Some(7), Some(8), None] {
            let outcome = testing::run(|out| targeting.handle(&target(spawn_id), &mut world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent.len(), 1);
            assert!(matches!(
                outcome.events[..],
                [ClientEvent::World(WorldEvent::TargetSent(sent))] if sent == spawn_id
            ));
        }
        for spawn_id in [9, 10] {
            let outcome =
                testing::run(|out| targeting.handle(&target(Some(spawn_id)), &mut world, out));
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
