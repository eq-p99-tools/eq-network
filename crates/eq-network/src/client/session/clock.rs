//! The time of day: servers send it while the zone admits the player, before
//! the host hears anything of the zone, so the session keeps it until the
//! admission and tells the host then. Weather and zone settings follow the same
//! rule. Later updates reach the host as they come.
use super::{
    feature::{Feature, Out, World},
    ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    clock::{GameTime, Weather},
    message::Message,
    world::WorldEvent,
};

/// The time of day the zone sent before admitting the player.
#[derive(Default)]
pub(super) struct Clock {
    staged: Option<GameTime>,
    weather: Option<Weather>,
    environment: Option<Box<eq_network_game::environment::ZoneEnvironment>>,
}

impl Feature for Clock {
    fn admit(&mut self, message: &Message, _world: &mut World) -> Result<()> {
        match message {
            Message::Event(WorldEvent::TimeOfDay(time)) => self.staged = Some(*time),
            Message::Event(WorldEvent::Weather(weather)) => self.weather = Some(*weather),
            Message::Event(WorldEvent::ZoneEnvironment(environment)) => {
                self.environment = Some(environment.clone());
            }
            _ => (),
        }
        Ok(())
    }

    fn admitted(&mut self, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(environment) = self.environment.take() {
            out.log
                .send(ClientEvent::World(WorldEvent::ZoneEnvironment(environment)))?;
        }
        if let Some(time) = self.staged.take() {
            out.log
                .send(ClientEvent::World(WorldEvent::TimeOfDay(time)))?;
        }
        if let Some(weather) = self.weather.take() {
            out.log
                .send(ClientEvent::World(WorldEvent::Weather(weather)))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;

    #[test]
    fn environment_is_staged_until_admission_and_emitted_once() {
        let zone = Box::new(eq_network_game::environment::titanium(&[0; 700]).unwrap());
        let mut clock = Clock::default();
        let mut world = World::new(5);
        clock
            .admit(
                &Message::Event(WorldEvent::ZoneEnvironment(zone.clone())),
                &mut world,
            )
            .unwrap();
        let outcome = testing::run(|out| clock.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(
            matches!(outcome.events.as_slice(), [ClientEvent::World(WorldEvent::ZoneEnvironment(value))] if value == &zone)
        );
        assert!(testing::run(|out| clock.admitted(&mut world, out))
            .events
            .is_empty());
        assert!(
            testing::run(|out| Clock::default().admitted(&mut world, out))
                .events
                .is_empty()
        );
    }

    #[test]
    fn latest_admission_weather_is_delivered_once_and_not_carried_to_a_new_zone() {
        let mut clock = Clock::default();
        let mut world = World::new(5);
        let first = Weather::Admission {
            marker: 255,
            kind: None,
            mode: 0,
        };
        let latest = Weather::Precipitation {
            kind: eq_network_game::clock::Precipitation::Rain,
            intensity: 3,
        };
        for weather in [first, latest] {
            clock
                .admit(&Message::Event(WorldEvent::Weather(weather)), &mut world)
                .unwrap();
        }
        let outcome = testing::run(|out| clock.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(
            matches!(outcome.events.as_slice(), [ClientEvent::World(WorldEvent::Weather(value))] if *value == latest)
        );
        let outcome = testing::run(|out| clock.admitted(&mut world, out));
        assert!(outcome.events.is_empty());
        let mut next_zone = Clock::default();
        let outcome = testing::run(|out| next_zone.admitted(&mut world, out));
        assert!(outcome.events.is_empty());
    }

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
