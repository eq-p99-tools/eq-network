//! Everyone in the zone: the spawn table, which only this feature changes,
//! and the read view other features check their targets against.
use super::{
    feature::{Feature, Out, World},
    posture::OwnPosture,
    ClientEvent,
};
use anyhow::{ensure, Result};
use eq_network_game::{
    message::Message,
    world::{PostureState, SpawnState, WorldEvent},
};
use std::collections::BTreeMap;

/// The most spawns a Titanium zone can name.
const MOST_SPAWNS: usize = 65_535;

/// The zone's spawns, by spawn ID. Other features read it; only the entities
/// feature changes it.
#[derive(Debug, Default)]
pub(super) struct Spawns(BTreeMap<u16, SpawnState>);

impl Spawns {
    /// A spawn the player can see.
    pub(super) fn visible(&self, id: u16) -> Option<&SpawnState> {
        self.0.get(&id).filter(|spawn| !spawn.invisible)
    }

    /// Every spawn, seen or not.
    pub(super) fn all(&self) -> impl Iterator<Item = &SpawnState> {
        self.0.values()
    }

    /// Records what an event says about the zone's spawns.
    fn apply(&mut self, event: &WorldEvent) {
        match event {
            WorldEvent::Spawns(spawns) => {
                for spawn in spawns {
                    self.0.insert(spawn.spawn_id, spawn.clone());
                }
            }
            WorldEvent::Despawn(id) => {
                self.0.remove(id);
            }
            WorldEvent::Visibility {
                spawn_id,
                invisible,
            } => {
                if let Some(spawn) = self.0.get_mut(spawn_id) {
                    spawn.invisible = *invisible;
                }
            }
            WorldEvent::Position {
                spawn_id,
                position,
                velocity,
            } => {
                if let Some(spawn) = self.0.get_mut(spawn_id) {
                    spawn.position = *position;
                    spawn.velocity = *velocity;
                }
            }
            WorldEvent::WearChange(change) => {
                if let Some(spawn) = self.0.get_mut(&change.spawn_id) {
                    spawn.appearance.apply(change);
                }
            }
            _ => (),
        }
    }
}

/// Keeps the spawn table, and the postures reported before admission until
/// the admission report.
#[derive(Debug, Default)]
pub(super) struct Entities {
    postures: BTreeMap<u16, PostureState>,
}

impl Feature for Entities {
    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        let Message::Event(event) = message else {
            return Ok(());
        };
        world.spawns.apply(event);
        match event {
            WorldEvent::Spawns(_) => {
                ensure!(
                    world.spawns.0.len() <= MOST_SPAWNS,
                    "zone entity limit exceeded"
                );
            }
            WorldEvent::Despawn(id) => {
                self.postures.remove(id);
            }
            WorldEvent::Posture { spawn_id, posture } => {
                self.postures.insert(*spawn_id, *posture);
            }
            _ => (),
        }
        Ok(())
    }

    /// Reports the spawns and their postures; the player's own posture is
    /// noted, since the server never repeats it.
    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        out.log.send(ClientEvent::World(WorldEvent::Spawns(
            world.spawns.all().cloned().collect(),
        )))?;
        world.posture = OwnPosture::default();
        for (spawn_id, posture) in std::mem::take(&mut self.postures) {
            if world.is_player(spawn_id) {
                world.posture.observed(posture);
            }
            out.log.send(ClientEvent::World(WorldEvent::Posture {
                spawn_id,
                posture,
            }))?;
        }
        Ok(())
    }

    /// Keeps the table current; a corpse keeps its spawn ID.
    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let Message::Event(event) = message else {
            return Ok(());
        };
        world.spawns.apply(event);
        if let WorldEvent::Death(death) = event {
            if let Some(spawn) = u16::try_from(death.spawn_id)
                .ok()
                .filter(|id| !world.is_player(*id))
                .and_then(|id| world.spawns.0.get_mut(&id))
            {
                spawn.kind = spawn.kind.corpse();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::world::{Position, SpawnKind};

    fn spawn(spawn_id: u16, kind: SpawnKind) -> SpawnState {
        SpawnState {
            class: None,
            spawn_id,
            name: format!("Spawn {spawn_id}"),
            kind,
            race: 1,
            gender: 0,
            position: Position::default(),
            velocity: [0.0; 3],
            size: 6.0,
            invisible: false,
            appearance: eq_network_game::appearance::Appearance::default(),
        }
    }

    fn event(event: WorldEvent) -> Message {
        Message::Event(event)
    }

    #[test]
    fn spawns_staged_before_admission_are_reported_with_their_postures() {
        let mut entities = Entities::default();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let spawns = vec![spawn(7, SpawnKind::Player), spawn(8, SpawnKind::Npc)];
        entities
            .admit(&event(WorldEvent::Spawns(spawns)), &mut world)
            .unwrap();
        for (spawn_id, posture) in [(7, PostureState::Sitting), (8, PostureState::Lying)] {
            entities
                .admit(
                    &event(WorldEvent::Posture { spawn_id, posture }),
                    &mut world,
                )
                .unwrap();
        }
        let hidden = WorldEvent::Visibility {
            spawn_id: 8,
            invisible: true,
        };
        entities.admit(&event(hidden), &mut world).unwrap();
        assert!(world.spawns.visible(8).is_none());
        let outcome = testing::run(|out| entities.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            &outcome.events[..],
            [
                ClientEvent::World(WorldEvent::Spawns(spawns)),
                ClientEvent::World(WorldEvent::Posture { spawn_id: 7, .. }),
                ClientEvent::World(WorldEvent::Posture { spawn_id: 8, .. }),
            ] if spawns.len() == 2
        ));
    }

    #[test]
    fn another_spawn_dying_leaves_a_corpse_under_the_same_id() {
        let mut entities = Entities::default();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let spawns = vec![spawn(7, SpawnKind::Player), spawn(8, SpawnKind::Npc)];
        entities
            .admit(&event(WorldEvent::Spawns(spawns)), &mut world)
            .unwrap();
        let death = |spawn_id| {
            event(WorldEvent::Death(eq_network_game::zoning::Death {
                spawn_id,
                killer_id: 0,
                corpse_id: 0,
                bind_zone_id: 0,
            }))
        };
        for spawn_id in [7, 8] {
            testing::run(|out| entities.observe(&death(spawn_id), &mut world, out))
                .result
                .unwrap();
        }
        assert_eq!(world.spawns.visible(8).unwrap().kind, SpawnKind::NpcCorpse);
        assert_eq!(world.spawns.visible(7).unwrap().kind, SpawnKind::Player);
    }
}
