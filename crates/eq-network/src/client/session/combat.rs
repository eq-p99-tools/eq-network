//! Combat: sizing up a spawn the player can see, and attacking.
use super::{
    feature::{Encoder, Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;

/// Considers spawns and turns auto attack on and off.
pub(super) struct Combat {
    encoder: Encoder,
}

impl Combat {
    pub(super) fn new(encoder: Encoder) -> Self {
        Self { encoder }
    }
}

impl Feature for Combat {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![Capability::Combat]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::Consider { .. } | ClientCommand::AutoAttack { .. }
        )
    }

    /// The player considers only a spawn they can see; attacking needs
    /// nothing from the session.
    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let ClientCommand::Consider {
            own_id, target_id, ..
        } = *command
        {
            if !world.is_player(own_id) || world.spawns.visible(target_id).is_none() {
                return out
                    .log
                    .diagnostic("Rejected considering an unavailable spawn".into());
            }
        }
        self.encoder.send(command, out).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::world::SpawnKind;
    use std::time::Instant;

    #[test]
    fn the_player_considers_only_a_spawn_they_can_see() {
        let mut combat = Combat::new(Encoder::new("Tester"));
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.spawns.insert(testing::spawn(8, SpawnKind::Npc));
        let consider = |own_id, target_id| ClientCommand::Consider {
            session_id: 5,
            own_id,
            target_id,
            created: Instant::now(),
        };
        for (command, sent) in [
            (consider(7, 8), 1),
            (consider(7, 9), 0),
            (consider(6, 8), 0),
        ] {
            let outcome = testing::run(|out| combat.handle(&command, &mut world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent.len(), sent, "{command:?}");
        }
        let attack = ClientCommand::AutoAttack {
            session_id: 5,
            enabled: true,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| combat.handle(&attack, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
    }
}
