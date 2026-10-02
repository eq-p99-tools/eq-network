//! The time of day: servers send it while the zone admits the player, before
//! the host hears anything of the zone, so the session keeps it until the
//! admission and tells the host then. Later times reach the host as they
//! come.
use super::{
    feature::{Feature, Out, World},
    ClientEvent,
};
use anyhow::Result;
use eq_network_game::{clock::GameTime, message::Message, world::WorldEvent};

/// The time of day the zone sent before admitting the player.
#[derive(Default)]
pub(super) struct Clock {
    staged: Option<GameTime>,
}

impl Feature for Clock {
    fn admit(&mut self, message: &Message, _world: &mut World) -> Result<()> {
        if let Message::Event(WorldEvent::TimeOfDay(time)) = message {
            self.staged = Some(*time);
        }
        Ok(())
    }

    fn admitted(&mut self, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        match self.staged.take() {
            Some(time) => out
                .log
                .send(ClientEvent::World(WorldEvent::TimeOfDay(time))),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;

    #[test]
    fn the_time_sent_before_the_admission_is_told_after_it() {
        let time = GameTime {
            hour: 6,
            minute: 0,
            day: 1,
            month: 1,
            year: 3100,
        };
        let mut clock = Clock::default();
        let mut world = World::new(5);
        clock
            .admit(&Message::Event(WorldEvent::TimeOfDay(time)), &mut world)
            .unwrap();
        let outcome = testing::run(|out| clock.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::TimeOfDay(told))] if told == time
        ));
        // Told once.
        let outcome = testing::run(|out| clock.admitted(&mut world, out));
        assert!(outcome.events.is_empty());
    }
}
