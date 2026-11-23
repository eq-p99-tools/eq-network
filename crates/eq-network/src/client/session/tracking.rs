//! Tracking's window and following (read R14 section 2, R15a): the server's
//! list loses the spawns the session does not know, Track names one and for
//! six minutes the session tells the host where it lies every two seconds,
//! and stopping names no one. A press of the Tracking ability itself goes
//! out with the other abilities.
//!
//! Following ends silently when six minutes pass, when the spawn is removed
//! and when the player dies, as the program's does (read R49-30 c); a zone
//! change ends it with the zone session. The program's 0x30d3 is for a
//! tracked spawn it no longer holds while still tracking, a state removal
//! never leaves here, so this session doesn't say it (inferred, 6.636).
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    message::Message,
    request::Request,
    tracking::{self, lines, TrackAction, TrackingUpdate},
    world::WorldEvent,
};
use std::time::Instant;

/// The Tracking skill's place in the player's skills.
const TRACKING_SKILL: usize = 53;

/// The spawn followed, if any.
#[derive(Default)]
pub(super) struct Tracking {
    following: Option<Following>,
}

/// Following a spawn: when it ends, and when the last bearing was told.
struct Following {
    spawn_id: u16,
    ends: Instant,
    reported: Option<Instant>,
}

impl Tracking {
    fn end(&mut self, string_id: Option<u32>, out: &mut Out<'_, '_>) -> Result<()> {
        self.following = None;
        out.log.send(ClientEvent::World(WorldEvent::Tracking(
            TrackingUpdate::Ended { string_id },
        )))
    }

    fn follow(
        &mut self,
        spawn_id: u16,
        now: Instant,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        // The window's rows are spawns the session knew when it listed them.
        let Some(spawn) = world.spawns.get(spawn_id) else {
            return out.log.diagnostic(format!(
                "Track not sent: spawn {spawn_id} is not in the zone"
            ));
        };
        let name = spawn.name.clone();
        out.request(&Request::TrackSpawn(spawn_id))?;
        self.following = Some(Following {
            spawn_id,
            ends: now + tracking::FOLLOWED_FOR,
            reported: None,
        });
        out.log.send(ClientEvent::World(WorldEvent::Tracking(
            TrackingUpdate::Following {
                spawn_id,
                string_id: lines::FOLLOWING,
                arguments: vec![name],
            },
        )))
    }
}

impl Feature for Tracking {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Tracking]
    }

    fn explain(&mut self, message: &mut Message, world: &World) {
        if let Message::Event(WorldEvent::Tracking(TrackingUpdate::Listed(entries))) = message {
            entries.retain(|entry| world.spawns.get(entry.spawn_id).is_some());
        }
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::Track { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::Track { action, .. } = command else {
            return Ok(());
        };
        match *action {
            TrackAction::Refresh => out.request(&Request::TrackingList),
            TrackAction::Follow(spawn_id) => self.follow(spawn_id, Instant::now(), world, out),
            // Stopping says something only while following.
            TrackAction::Stop if self.following.is_some() => {
                out.request(&Request::TrackSpawn(0))?;
                self.end(None, out)
            }
            TrackAction::Stop => Ok(()),
        }
    }

    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let Some(following) = &mut self.following else {
            return Ok(());
        };
        // The program's removal of the tracked spawn ends it silently
        // (0x4581b7, 0x4849b2).
        let Some(spawn) = world.spawns.get(following.spawn_id) else {
            return self.end(None, out);
        };
        // Its report runs only while the Tracking skill (53) reads 0 or more
        // as the program's signed value (+0x1044, read R49-30 b).
        let skilled = world
            .player
            .as_ref()
            .and_then(|player| player.skills.as_ref()?.get(TRACKING_SKILL).copied())
            .is_none_or(|value| i32::from_ne_bytes(value.to_ne_bytes()) >= 0);
        if skilled
            && following
                .reported
                .is_none_or(|told| now.duration_since(told) >= tracking::REPORT_EVERY)
        {
            if let Some((_, at)) = world.player_at() {
                following.reported = Some(now);
                let (string_id, arguments) =
                    tracking::titanium_bearing(at, spawn.position, &spawn.name);
                out.log.send(ClientEvent::World(WorldEvent::Tracking(
                    TrackingUpdate::Bearing {
                        string_id,
                        arguments,
                    },
                )))?;
            }
        }
        // Six minutes end it silently too (0x422198).
        if now > following.ends {
            return self.end(None, out);
        }
        Ok(())
    }

    /// The player's death ends following silently (`OP_Death`, 0x4a8cd5).
    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if self.following.is_some()
            && matches!(message, Message::Event(WorldEvent::Death(death)) if world.is_player(death.spawn_id))
        {
            return self.end(None, out);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::{
        tracking::Tracked,
        world::{SpawnKind, SpawnState},
    };
    use std::time::Duration;

    fn spawn(id: u16, x: f32, y: f32) -> SpawnState {
        let mut spawn = testing::spawn(id, SpawnKind::Npc);
        spawn.name = format!("spawn{id}");
        (spawn.position.x, spawn.position.y) = (x, y);
        spawn
    }

    fn track(action: TrackAction) -> ClientCommand {
        ClientCommand::Track {
            session_id: 5,
            action,
        }
    }

    fn updates(events: &[ClientEvent]) -> Vec<TrackingUpdate> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::Tracking(update)) => Some(update.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_list_keeps_only_spawns_the_session_knows_in_the_servers_order() {
        let mut world = World::new(5);
        world.spawns.insert(spawn(7, 0.0, 0.0));
        world.spawns.insert(spawn(42, 0.0, 0.0));
        let entry = |spawn_id| Tracked {
            spawn_id,
            distance: 1.0,
        };
        let mut message = Message::Event(WorldEvent::Tracking(TrackingUpdate::Listed(vec![
            entry(42),
            entry(9),
            entry(7),
        ])));
        Tracking::default().explain(&mut message, &world);
        let Message::Event(WorldEvent::Tracking(TrackingUpdate::Listed(kept))) = message else {
            panic!("the list stays a list");
        };
        assert_eq!(kept, [entry(42), entry(7)]);
    }

    #[test]
    fn refresh_asks_for_the_list_and_stop_says_nothing_unless_following() {
        let mut world = World::new(5);
        let mut feature = Tracking::default();
        let outcome =
            testing::run(|out| feature.handle(&track(TrackAction::Refresh), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [tracking::titanium_refresh()]);
        let outcome =
            testing::run(|out| feature.handle(&track(TrackAction::Stop), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert_eq!(updates(&outcome.events), []);
    }

    #[test]
    fn following_names_the_spawn_tells_bearings_every_two_seconds_and_stops() {
        let mut world = World::new(5);
        world.spawns.insert(spawn(42, 0.0, 10.0));
        let mut feature = Tracking::default();
        let outcome =
            testing::run(|out| feature.handle(&track(TrackAction::Follow(42)), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [tracking::titanium_follow(42)]);
        assert_eq!(
            updates(&outcome.events),
            [TrackingUpdate::Following {
                spawn_id: 42,
                string_id: lines::FOLLOWING,
                arguments: vec!["spawn42".into()],
            }]
        );
        let unknown = testing::run(|out| {
            Tracking::default().handle(&track(TrackAction::Follow(9)), &mut world, out)
        });
        unknown.result.unwrap();
        assert_eq!(unknown.sent, []);

        // Without a player there is no heading to tell it against.
        let start = Instant::now();
        let outcome = testing::run(|out| feature.tick(start, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(updates(&outcome.events), []);

        world.player.admit(testing::player(7));
        let mut told = 0;
        for step in 0..=4 {
            let now = start + Duration::from_millis(step * 1000);
            let outcome = testing::run(|out| feature.tick(now, &mut world, out));
            outcome.result.unwrap();
            for update in updates(&outcome.events) {
                assert_eq!(
                    update,
                    TrackingUpdate::Bearing {
                        string_id: lines::BEARINGS[0],
                        arguments: vec!["spawn42".into()],
                    }
                );
                told += 1;
            }
            assert_eq!(outcome.sent, []);
        }
        assert_eq!(told, 3);

        let outcome =
            testing::run(|out| feature.handle(&track(TrackAction::Stop), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [tracking::titanium_follow(0)]);
        assert_eq!(
            updates(&outcome.events),
            [TrackingUpdate::Ended { string_id: None }]
        );
    }

    #[test]
    fn following_ends_when_the_spawn_goes_or_six_minutes_pass() {
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        world.spawns.insert(spawn(42, 0.0, 10.0));
        let mut feature = Tracking::default();
        testing::run(|out| feature.handle(&track(TrackAction::Follow(42)), &mut world, out))
            .result
            .unwrap();
        let late = Instant::now() + tracking::FOLLOWED_FOR + Duration::from_secs(1);
        let outcome = testing::run(|out| feature.tick(late, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            updates(&outcome.events).last(),
            Some(&TrackingUpdate::Ended { string_id: None })
        );

        testing::run(|out| feature.handle(&track(TrackAction::Follow(42)), &mut world, out))
            .result
            .unwrap();
        world.spawns = super::super::entities::Spawns::default();
        let outcome = testing::run(|out| feature.tick(Instant::now(), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            updates(&outcome.events),
            [TrackingUpdate::Ended { string_id: None }]
        );
        let outcome = testing::run(|out| feature.tick(Instant::now(), &mut world, out));
        assert_eq!(updates(&outcome.events), []);
    }

    #[test]
    fn the_players_death_ends_following_silently_and_anothers_does_not() {
        use eq_network_game::zoning::Death;
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        world.own_spawn = Some(7);
        world.spawns.insert(spawn(42, 0.0, 10.0));
        let mut feature = Tracking::default();
        testing::run(|out| feature.handle(&track(TrackAction::Follow(42)), &mut world, out))
            .result
            .unwrap();
        let death = |spawn_id| {
            Message::Event(WorldEvent::Death(Death {
                spawn_id,
                killer_id: 0,
                corpse_id: spawn_id,
                bind_zone_id: 0,
                corpse_name: None,
            }))
        };
        let outcome = testing::run(|out| feature.observe(&death(42), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(updates(&outcome.events), []);
        let outcome = testing::run(|out| feature.observe(&death(7), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            updates(&outcome.events),
            [TrackingUpdate::Ended { string_id: None }]
        );
        assert_eq!(outcome.sent, []);
    }

    #[test]
    fn the_report_waits_while_the_tracking_skill_reads_below_zero() {
        let mut world = World::new(5);
        let mut player = testing::player(7);
        let mut skills = vec![0; 100];
        skills[TRACKING_SKILL] = u32::MAX;
        player.skills = Some(skills);
        world.player.admit(player);
        world.spawns.insert(spawn(42, 0.0, 10.0));
        let mut feature = Tracking::default();
        testing::run(|out| feature.handle(&track(TrackAction::Follow(42)), &mut world, out))
            .result
            .unwrap();
        let outcome = testing::run(|out| feature.tick(Instant::now(), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(updates(&outcome.events), []);
    }
}
