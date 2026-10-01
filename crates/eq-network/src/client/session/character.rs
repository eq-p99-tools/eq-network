//! The admitted player's character as it changes: level, skills and what
//! they wear.
use super::feature::{Feature, Out, World};
use anyhow::Result;
use eq_network_game::{message::Message, world::WorldEvent};

/// Keeps the admitted player's level, skills and appearance current.
#[derive(Default)]
pub(super) struct Character;

impl Feature for Character {
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
        let mut character = Character;
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
}
