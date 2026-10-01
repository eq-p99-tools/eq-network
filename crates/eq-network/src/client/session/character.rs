//! The player's character: what the zone says about them before it admits
//! them, and their level, skills and what they wear as these change.
use super::{
    feature::{Feature, Out, World},
    ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    appearance::WearChange,
    buffs::Buff,
    message::Message,
    world::{Coins, PlayerState, WorldEvent},
};
use std::collections::BTreeMap;

/// Keeps the admitted player's level, skills and appearance current, and
/// holds what arrives about them before the admission until it can be told.
#[derive(Default)]
pub(super) struct Character {
    staged: Staged,
}

/// What the zone said about the player before admitting them.
#[derive(Default)]
struct Staged {
    level: Option<u8>,
    experience: Option<u32>,
    skills: BTreeMap<u32, u32>,
    /// The player's own wear changes, which arrive before their state exists.
    wear: Vec<WearChange>,
    buffs: Option<Vec<Option<Buff>>>,
    coins: Option<Coins>,
}

impl Feature for Character {
    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        let Message::Event(event) = message else {
            return Ok(());
        };
        let staged = &mut self.staged;
        match event {
            WorldEvent::Skill { skill_id, value } if *skill_id < 100 => {
                staged.skills.insert(*skill_id, *value);
            }
            WorldEvent::Level {
                current,
                experience,
                ..
            } => {
                staged.level = Some(*current);
                staged.experience = Some(*experience);
            }
            WorldEvent::Experience(value) => staged.experience = Some(*value),
            WorldEvent::WearChange(change) if world.is_player(change.spawn_id) => {
                staged.wear.push(*change);
            }
            WorldEvent::BuffSnapshot(buffs) => staged.buffs = Some(buffs.clone()),
            WorldEvent::Coins(coins) => staged.coins = Some(*coins),
            _ => (),
        }
        Ok(())
    }

    fn shape(&mut self, player: &mut PlayerState) {
        let staged = &mut self.staged;
        if let Some(level) = staged.level.take() {
            player.level = level;
        }
        for change in std::mem::take(&mut staged.wear) {
            player.appearance.apply(&change);
        }
        for (skill, value) in std::mem::take(&mut staged.skills) {
            player.apply_skill(skill, value);
        }
    }

    fn admitted(&mut self, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let staged = std::mem::take(&mut self.staged);
        let news = [
            staged.buffs.map(WorldEvent::BuffSnapshot),
            staged.coins.map(WorldEvent::Coins),
            staged.experience.map(WorldEvent::Experience),
        ];
        for event in news.into_iter().flatten() {
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
        let (Message::Event(event), Some(player)) = (message, world.player.as_mut()) else {
            return Ok(());
        };
        match event {
            WorldEvent::WearChange(change) if change.spawn_id == player.spawn_id => {
                player.appearance.apply(change);
            }
            WorldEvent::Level { current, .. } => player.level = *current,
            WorldEvent::Skill { skill_id, value } => player.apply_skill(*skill_id, *value),
            _ => (),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;

    #[test]
    fn the_player_levels_and_learns() {
        let mut character = Character::default();
        let mut world = World::new(5);
        world.player = Some(testing::player(7));
        for event in [
            WorldEvent::Level {
                current: 12,
                previous: 11,
                experience: 0,
            },
            WorldEvent::Skill {
                skill_id: 22,
                value: 40,
            },
        ] {
            testing::run(|out| character.observe(&Message::Event(event.clone()), &mut world, out))
                .result
                .unwrap();
        }
        let player = world.player.as_ref().unwrap();
        assert_eq!(player.level, 12);
        assert_eq!(
            player
                .skills
                .as_ref()
                .and_then(|skills| skills.get(22))
                .copied(),
            Some(40)
        );
    }

    #[test]
    fn what_arrives_before_the_admission_shapes_the_player_then_is_told() {
        let mut character = Character::default();
        let mut world = World::new(5);
        for event in [
            WorldEvent::Level {
                current: 12,
                previous: 11,
                experience: 120,
            },
            WorldEvent::Skill {
                skill_id: 22,
                value: 40,
            },
            WorldEvent::Coins(Coins {
                platinum: 1,
                gold: 2,
                silver: 3,
                copper: 4,
            }),
            WorldEvent::BuffSnapshot(Vec::new()),
        ] {
            character.admit(&Message::Event(event), &mut world).unwrap();
        }
        let mut player = testing::player(7);
        character.shape(&mut player);
        assert_eq!(player.level, 12);
        assert_eq!(
            player
                .skills
                .as_ref()
                .and_then(|skills| skills.get(22))
                .copied(),
            Some(40)
        );
        world.player = Some(player);
        let outcome = testing::run(|out| character.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            outcome.events[..],
            [
                ClientEvent::World(WorldEvent::BuffSnapshot(_)),
                ClientEvent::World(WorldEvent::Coins(_)),
                ClientEvent::World(WorldEvent::Experience(120))
            ]
        ));
        // Told once, the staging is empty.
        let again = testing::run(|out| character.admitted(&mut world, out));
        assert!(again.events.is_empty());
    }
}

#[cfg(test)]
mod profile_tests {
    use super::super::feature::testing;
    use super::*;

    #[test]
    fn the_profile_read_before_admission_is_told_after_it() {
        let mut character = Character::default();
        let mut world = World::new(5);
        let mut profile = vec![0; 19592];
        profile[4428..4432].copy_from_slice(&3u32.to_le_bytes());
        for message in eq_network_game::message::titanium(0x75df, &profile) {
            character.admit(&message, &mut world).unwrap();
        }
        world.player = Some(testing::player(7));
        let outcome = testing::run(|out| character.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Coins(Coins { platinum: 3, .. }))
        )));
    }
}
