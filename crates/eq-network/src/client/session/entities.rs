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
    world::{PostureState, SpawnKind, SpawnState, WorldEvent},
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
            WorldEvent::Appearance { spawn_id, change } => {
                if let Some(spawn) = self.0.get_mut(spawn_id) {
                    spawn.apply_appearance(change);
                }
            }
            WorldEvent::FaceChanged {
                spawn_id: Some(spawn_id),
                features,
                ..
            } => {
                if let Some(spawn) = self.0.get_mut(spawn_id) {
                    spawn.appearance.set_features(*features);
                }
            }
            WorldEvent::Listing { spawn_id, change } => {
                if let Some(spawn) = self.0.get_mut(spawn_id) {
                    spawn.apply_listing(*change);
                }
            }
            WorldEvent::PetOwner { spawn_id, owner } => {
                if let Some(spawn) = self.0.get_mut(spawn_id) {
                    spawn.pet_owner = *owner;
                }
            }
            WorldEvent::Renamed {
                spawn_id: Some(spawn_id),
                new_name,
                ..
            } => {
                if let Some(spawn) = self.0.get_mut(spawn_id) {
                    spawn.name.clone_from(new_name);
                }
            }
            WorldEvent::CorpseMoved { spawn_id, position } => {
                if let Some(spawn) = self.0.get_mut(spawn_id) {
                    let [x, y, z] = *position;
                    (spawn.position.x, spawn.position.y, spawn.position.z) = (x, y, z);
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

    /// The spawn a rename is about, which the record does not name: the one
    /// with the old name, or the NPC corpse whose name by the generation's
    /// rule is the new one, as the server renames a corpse from the name the
    /// client gives it to its own.
    fn renamed(&self, world: &World, old_name: &str, new_name: &str) -> Option<u16> {
        let spawns = &world.spawns;
        spawns
            .all()
            .find(|spawn| spawn.name == old_name)
            .or_else(|| {
                spawns.all().find(|spawn| {
                    spawn.kind == SpawnKind::NpcCorpse
                        && eq_network_game::world::corpse_name(
                            self.dialect,
                            &spawn.name,
                            spawn.kind,
                            spawn.spawn_id,
                        )
                        .is_some_and(|name| name == new_name)
                })
            })
            .map(|spawn| spawn.spawn_id)
    }
}

impl Feature for Entities {
    /// Names the corpse a death leaves, from the spawn that died, the spawn
    /// a rename is about, and the player in the notices that leave them
    /// unnamed, by the client generation's rules, before any feature or the
    /// host hears it.
    fn explain(&mut self, message: &mut Message, world: &World) {
        let Message::Event(event) = message else {
            return;
        };
        self.name_the_player(event, world);
        if let WorldEvent::Renamed {
            spawn_id: spawn_id @ None,
            old_name,
            new_name,
        } = event
        {
            *spawn_id = self.renamed(world, old_name, new_name);
            return;
        }
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
    fn the_table_keeps_what_appearance_listing_and_pet_updates_change() {
        use eq_network_game::{
            appearance::{AppearanceChange, FlyMode},
            listing::{GuildRank, ListingChange},
        };
        let mut entities = Entities::default();
        let mut world = World::new(5);
        let spawns = vec![spawn(7, SpawnKind::Player), spawn(8, SpawnKind::Npc)];
        entities
            .admit(&event(WorldEvent::Spawns(spawns)), &mut world)
            .unwrap();
        // Before the admission and after it alike.
        entities
            .admit(
                &event(WorldEvent::Appearance {
                    spawn_id: 8,
                    change: AppearanceChange::Size(9.0),
                }),
                &mut world,
            )
            .unwrap();
        let updates = [
            WorldEvent::Appearance {
                spawn_id: 8,
                change: AppearanceChange::FlyMode(FlyMode::Levitating),
            },
            WorldEvent::Appearance {
                spawn_id: 8,
                change: AppearanceChange::Light(4),
            },
            // Once-only kinds change nothing.
            WorldEvent::Appearance {
                spawn_id: 8,
                change: AppearanceChange::Other { kind: 2, value: 1 },
            },
            WorldEvent::Listing {
                spawn_id: 7,
                change: ListingChange::Guild(Some(3)),
            },
            WorldEvent::Listing {
                spawn_id: 7,
                change: ListingChange::GuildRank(GuildRank::Officer),
            },
            WorldEvent::Listing {
                spawn_id: 7,
                change: ListingChange::Level(12),
            },
            WorldEvent::PetOwner {
                spawn_id: 8,
                owner: Some(7),
            },
            // News of a spawn the table has not seen is left alone.
            WorldEvent::Appearance {
                spawn_id: 9,
                change: AppearanceChange::Pvp(true),
            },
        ];
        for update in updates {
            testing::run(|out| entities.observe(&event(update), &mut world, out))
                .result
                .unwrap();
        }
        let npc = world.spawns.visible(8).unwrap();
        assert_eq!(npc.size, 9.0);
        assert_eq!(
            (npc.appearance.fly_mode, npc.appearance.light, npc.pet_owner),
            (FlyMode::Levitating, 4, Some(7))
        );
        let player = world.spawns.visible(7).unwrap();
        assert_eq!(
            (
                player.level,
                player.listing.guild,
                player.listing.guild_rank
            ),
            (12, Some(3), Some(GuildRank::Officer))
        );
        assert!(world.spawns.visible(9).is_none());
    }

    #[test]
    fn illusions_and_named_face_changes_change_the_table() {
        use eq_network_game::appearance::{AppearanceChange, Features, Illusion};
        let mut entities = Entities::default();
        let mut world = World::new(5);
        let spawns = vec![spawn(7, SpawnKind::Player), spawn(8, SpawnKind::Player)];
        entities
            .admit(&event(WorldEvent::Spawns(spawns)), &mut world)
            .unwrap();
        let features = |face| Features {
            face,
            ..Features::default()
        };
        let updates = [
            WorldEvent::Appearance {
                spawn_id: 7,
                change: AppearanceChange::Illusion(Illusion {
                    race: 60,
                    gender: 2,
                    texture: None,
                    helm_texture: None,
                    size: 6.0,
                    features: features(1),
                }),
            },
            WorldEvent::FaceChanged {
                spawn_id: Some(8),
                features: features(3),
                eye_colors: None,
            },
            // A face change that names no one changes no one.
            WorldEvent::FaceChanged {
                spawn_id: None,
                features: features(5),
                eye_colors: Some([1, 1]),
            },
        ];
        for update in updates {
            testing::run(|out| entities.observe(&event(update), &mut world, out))
                .result
                .unwrap();
        }
        let illusioned = world.spawns.visible(7).unwrap();
        assert_eq!(
            (
                illusioned.race,
                illusioned.gender,
                illusioned.size,
                illusioned.appearance.face
            ),
            (60, 2, 6.0, 1)
        );
        assert_eq!(world.spawns.visible(8).unwrap().appearance.face, 3);
    }

    #[test]
    fn a_rename_names_its_spawn_and_renames_it_in_the_table() {
        let mut entities = Entities::new(GameDialect::Titanium);
        let mut world = World::new(5);
        let mut npc = spawn(8, SpawnKind::NpcCorpse);
        npc.name = "a_rat008".into();
        let mut guard = spawn(9, SpawnKind::Npc);
        guard.name = "Guard_Synth000".into();
        entities
            .admit(&event(WorldEvent::Spawns(vec![npc, guard])), &mut world)
            .unwrap();
        let named = |entities: &mut Entities, world: &World, old: &str, new: &str| {
            let mut rename = event(WorldEvent::Renamed {
                spawn_id: None,
                old_name: old.into(),
                new_name: new.into(),
            });
            entities.explain(&mut rename, world);
            rename
        };
        // An NPC's corpse, from the name the client gave it to the server's.
        let corpse = named(&mut entities, &world, "a_rat's_corpse8", "a_rat`s_corpse8");
        assert!(matches!(
            corpse,
            Message::Event(WorldEvent::Renamed {
                spawn_id: Some(8),
                ..
            })
        ));
        // A temporary name, by the name the spawn has.
        let guard = named(&mut entities, &world, "Guard_Synth000", "Gate_Keeper");
        assert!(matches!(
            guard,
            Message::Event(WorldEvent::Renamed {
                spawn_id: Some(9),
                ..
            })
        ));
        // No one in the zone.
        let nobody = named(&mut entities, &world, "Absent", "Elsewhere");
        assert!(matches!(
            nobody,
            Message::Event(WorldEvent::Renamed { spawn_id: None, .. })
        ));
        for message in [corpse, guard, nobody] {
            testing::run(|out| entities.observe(&message, &mut world, out))
                .result
                .unwrap();
        }
        assert_eq!(world.spawns.visible(8).unwrap().name, "a_rat`s_corpse8");
        assert_eq!(world.spawns.visible(9).unwrap().name, "Gate_Keeper");
    }

    #[test]
    fn an_eqmac_corpse_moves_and_keeps_its_heading() {
        let mut entities = Entities::new(GameDialect::EqMac);
        let mut world = World::new(5);
        let mut corpse = spawn(8, SpawnKind::PlayerCorpse);
        corpse.position.heading = 128.0;
        entities
            .admit(&event(WorldEvent::Spawns(vec![corpse])), &mut world)
            .unwrap();
        let moved = event(WorldEvent::CorpseMoved {
            spawn_id: 8,
            position: [1.0, 2.0, 3.0],
        });
        testing::run(|out| entities.observe(&moved, &mut world, out))
            .result
            .unwrap();
        let position = world.spawns.visible(8).unwrap().position;
        assert_eq!(
            [position.x, position.y, position.z, position.heading],
            [1.0, 2.0, 3.0, 128.0]
        );
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
