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
    spells::SpellUpdate,
    world::{PostureState, SpawnState, WorldEvent},
    GameDialect,
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

    /// Puts a spawn in the zone for a test.
    #[cfg(test)]
    pub(super) fn insert(&mut self, spawn: SpawnState) {
        self.0.insert(spawn.spawn_id, spawn);
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
    /// The client generation, whose rules name a corpse and say whom a
    /// notice is about.
    dialect: GameDialect,
}

impl Entities {
    pub(super) fn new(dialect: GameDialect) -> Self {
        Self {
            dialect,
            ..Self::default()
        }
    }

    /// Names the player in the notices `EQMac`'s server sends them alone
    /// without naming anyone: a cast interruption (TAKP
    /// `Mob::InterruptSpell`) and a buff's fade
    /// (`Client::MakeBuffFadePacket`).
    fn name_the_player(&self, event: &mut WorldEvent, world: &World) {
        let (GameDialect::EqMac, Some(own)) = (self.dialect, world.own_spawn) else {
            return;
        };
        match event {
            WorldEvent::Spell(SpellUpdate::Interrupted { caster_id, .. }) if *caster_id == 0 => {
                *caster_id = u32::from(own);
            }
            WorldEvent::Buff(update) if update.entity_id == 0 => {
                update.entity_id = u32::from(own);
            }
            _ => (),
        }
    }
}

impl Feature for Entities {
    /// Names the corpse a death leaves, from the spawn that died, and the
    /// player in the notices that leave them unnamed, by the client
    /// generation's rules, before any feature or the host hears it.
    fn explain(&mut self, message: &mut Message, world: &World) {
        let Message::Event(event) = message else {
            return;
        };
        self.name_the_player(event, world);
        let WorldEvent::Death(death) = event else {
            return;
        };
        let spawn = u16::try_from(death.spawn_id)
            .ok()
            .and_then(|id| world.spawns.0.get(&id));
        death.corpse_name = spawn.and_then(|spawn| {
            eq_network_game::world::corpse_name(
                self.dialect,
                &spawn.name,
                spawn.kind,
                spawn.spawn_id,
            )
        });
    }

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
    use eq_network_game::world::SpawnKind;
    use testing::spawn;

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
    fn a_death_names_its_corpse_by_the_generations_rule() {
        let mut world = World::new(5);
        world.spawns.0.insert(8, spawn(8, SpawnKind::Npc));
        world.spawns.0.get_mut(&8).unwrap().name = "a_rat001".into();
        world.spawns.0.insert(9, spawn(9, SpawnKind::Player));
        world.spawns.0.get_mut(&9).unwrap().name = "Synthetic".into();
        let named = |dialect, spawn_id| {
            let mut death = event(WorldEvent::Death(eq_network_game::zoning::Death {
                spawn_id,
                killer_id: 0,
                corpse_id: 0,
                bind_zone_id: 0,
                corpse_name: None,
            }));
            Entities::new(dialect).explain(&mut death, &world);
            match death {
                Message::Event(WorldEvent::Death(death)) => death.corpse_name,
                _ => unreachable!("a death stays a death"),
            }
        };
        assert_eq!(
            named(GameDialect::Titanium, 8).as_deref(),
            Some("a_rat`s_corpse8")
        );
        assert_eq!(
            named(GameDialect::Titanium, 9).as_deref(),
            Some("Synthetic's corpse9")
        );
        // An unseen spawn, and a generation whose rule is not checked, name
        // no corpse.
        assert_eq!(named(GameDialect::Titanium, 10), None);
        assert_eq!(named(GameDialect::EqMac, 8), None);
    }

    #[test]
    fn eqmacs_unnamed_interruptions_and_fades_are_the_players_own() {
        use eq_network_game::buffs::{BuffUpdate, UNKNOWN_SLOT};
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let explained = |dialect, world: &World, event: WorldEvent| {
            let mut message = Message::Event(event);
            Entities::new(dialect).explain(&mut message, world);
            match message {
                Message::Event(event) => event,
                _ => unreachable!("an event stays an event"),
            }
        };
        let interrupted = |caster_id| {
            WorldEvent::Spell(SpellUpdate::Interrupted {
                caster_id,
                message_id: 173,
                caster_name: None,
            })
        };
        let fade = |entity_id| {
            WorldEvent::Buff(BuffUpdate {
                spell_id: 42,
                entity_id,
                slot: UNKNOWN_SLOT,
                buff: None,
            })
        };
        assert_eq!(
            explained(GameDialect::EqMac, &world, interrupted(0)),
            interrupted(7)
        );
        assert_eq!(explained(GameDialect::EqMac, &world, fade(0)), fade(7));
        // A notice that names someone keeps them, Titanium's keep theirs, and
        // before the zone names the player there is no one to name.
        assert_eq!(
            explained(GameDialect::EqMac, &world, interrupted(9)),
            interrupted(9)
        );
        assert_eq!(explained(GameDialect::Titanium, &world, fade(0)), fade(0));
        world.own_spawn = None;
        assert_eq!(
            explained(GameDialect::EqMac, &world, interrupted(0)),
            interrupted(0)
        );
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
                corpse_name: None,
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
