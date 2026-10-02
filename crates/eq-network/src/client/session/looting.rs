//! Looting: opening a corpse the player can see, taking its items and
//! closing it. Taken items arrive as item updates for the inventory.
use super::{
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::world::SpawnKind;

/// Loots corpses.
pub(super) struct Looting;

impl Feature for Looting {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![Capability::Looting]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::Loot { .. }
                | ClientCommand::LootItem { .. }
                | ClientCommand::EndLoot { .. }
        )
    }

    /// Opens only a corpse the player can see; the server judges the rest.
    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let ClientCommand::Loot { corpse_id, .. } = *command {
            let corpse = world.spawns.visible(corpse_id).is_some_and(|spawn| {
                matches!(spawn.kind, SpawnKind::NpcCorpse | SpawnKind::PlayerCorpse)
            });
            if !corpse {
                return out
                    .log
                    .diagnostic("Rejected looting an unavailable corpse".into());
            }
        }
        out.command(command).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use std::time::Instant;

    #[test]
    fn only_a_corpse_the_player_can_see_is_opened() {
        let mut looting = Looting;
        let mut world = World::new(5);
        world.spawns.insert(testing::spawn(8, SpawnKind::NpcCorpse));
        world.spawns.insert(testing::spawn(9, SpawnKind::Npc));
        let loot = |corpse_id| ClientCommand::Loot {
            session_id: 5,
            corpse_id,
            created: Instant::now(),
        };
        for (corpse_id, sent) in [(8, 1), (9, 0), (10, 0)] {
            let outcome = testing::run(|out| looting.handle(&loot(corpse_id), &mut world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent.len(), sent, "corpse {corpse_id}");
        }
        let close = ClientCommand::EndLoot {
            session_id: 5,
            corpse_id: 8,
        };
        let outcome = testing::run(|out| looting.handle(&close, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
    }
}
