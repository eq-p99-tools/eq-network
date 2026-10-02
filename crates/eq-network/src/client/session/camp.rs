//! Camping: the official client's 30-second timer between `OP_Camp` and `OP_Logout`.
//!
//! The server only starts its own camp timer when it receives `OP_Camp`; the client
//! decides when to log out, and standing or moving abandons the attempt.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent, ZoneExit,
};
use anyhow::Result;
use eq_network_game::{
    command::Posture,
    message::Message,
    request::Request,
    world::{CampStatus, WorldEvent},
};
use std::time::{Duration, Instant};

const CAMP_DURATION: Duration = Duration::from_secs(30);
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
enum Phase {
    #[default]
    Idle,
    Preparing(Instant),
    LoggingOut(Instant),
}

/// Local camp progress for one zone admission.
#[derive(Debug, Default)]
pub(super) struct Camp {
    phase: Phase,
}

impl Camp {
    /// Whether a camp or logout is in progress.
    fn active(&self) -> bool {
        !matches!(self.phase, Phase::Idle)
    }

    /// Starts the timer after `OP_Camp` was handed to transport.
    fn start(&mut self, now: Instant) {
        if matches!(self.phase, Phase::Idle) {
            self.phase = Phase::Preparing(now);
        }
    }

    /// Abandons preparation; returns false when nothing was being prepared.
    /// A logout already sent cannot be taken back.
    fn cancel(&mut self) -> bool {
        if matches!(self.phase, Phase::Preparing(_)) {
            self.phase = Phase::Idle;
            true
        } else {
            false
        }
    }

    /// Whether the preparation time has elapsed and `OP_Logout` should be sent.
    fn logout_due(&self, now: Instant) -> bool {
        matches!(self.phase, Phase::Preparing(started)
            if now.saturating_duration_since(started) >= CAMP_DURATION)
    }

    /// Records that `OP_Logout` was sent.
    fn logout_sent(&mut self, now: Instant) {
        self.phase = Phase::LoggingOut(now);
    }

    /// Whether a logout was sent, so the camp can no longer be abandoned.
    #[cfg(test)]
    fn logging_out(&self) -> bool {
        matches!(self.phase, Phase::LoggingOut(_))
    }

    /// Whether a sent logout has waited too long for its reply.
    fn reply_overdue(&self, now: Instant) -> bool {
        matches!(self.phase, Phase::LoggingOut(sent)
            if now.saturating_duration_since(sent) >= REPLY_TIMEOUT)
    }
}

impl Feature for Camp {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![Capability::Camping]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::Camp { .. })
    }

    /// Starts camping.
    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if !matches!(command, ClientCommand::Camp { .. }) || self.active() {
            return Ok(());
        }
        out.request(&Request::Camp)?;
        self.start(Instant::now());
        out.log
            .send(ClientEvent::World(WorldEvent::Camp(CampStatus::Preparing)))
    }

    /// Standing up, ducking or moving abandons a camp still being prepared.
    fn notice(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let abandons = matches!(
            command,
            ClientCommand::SetPosture {
                posture: Posture::Standing | Posture::Ducking,
                ..
            } | ClientCommand::Move(_)
                | ClientCommand::CrossZoneLine { .. }
                | ClientCommand::ClickDoor { .. }
                | ClientCommand::PickUp { .. }
        );
        if abandons && self.cancel() {
            // Only a Standing appearance stops the server's own camp timer (EQEmu
            // client_packet.cpp, OP_SpawnAppearance); without it the server logs the
            // character out of its group and guild 29 seconds after /camp. Moving,
            // ducking, opening a door or picking something up stands the camping
            // character up first.
            let standing = matches!(
                command,
                ClientCommand::SetPosture {
                    posture: Posture::Standing,
                    ..
                }
            );
            let own_spawn = world.player.as_ref().map(|player| player.spawn_id);
            if let (false, Some(spawn_id)) = (standing, own_spawn) {
                world.posture.set(spawn_id, Posture::Standing, out)?;
            }
            out.log
                .send(ClientEvent::World(WorldEvent::Camp(CampStatus::Abandoned)))?;
        }
        Ok(())
    }

    /// Sends the logout once the camp timer completes, and gives up on a
    /// reply that never came.
    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if self.logout_due(now) {
            out.request(&Request::Logout)?;
            self.logout_sent(now);
            out.log
                .send(ClientEvent::World(WorldEvent::Camp(CampStatus::LoggingOut)))?;
        }
        if self.reply_overdue(now) {
            out.log
                .send(ClientEvent::World(WorldEvent::Camp(CampStatus::Camped)))?;
            world.end(ZoneExit::CharacterSelect);
        }
        Ok(())
    }

    /// Dying abandons a camp still being prepared, and the logout's reply
    /// ends the zone connection. A server may log a camping character out
    /// before the timer ends: `EQEmu` camps a GM at once.
    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match message {
            Message::Event(WorldEvent::Death(death))
                if world.is_player(death.spawn_id) && self.cancel() =>
            {
                out.log
                    .send(ClientEvent::World(WorldEvent::Camp(CampStatus::Abandoned)))?;
            }
            Message::LoggedOut if self.active() => {
                out.log
                    .send(ClientEvent::World(WorldEvent::Camp(CampStatus::Camped)))?;
                world.end(ZoneExit::CharacterSelect);
            }
            _ => (),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::command;

    #[test]
    fn camping_sends_the_request_then_the_logout_and_the_reply_ends_the_session() {
        let mut camp = Camp::default();
        let mut world = World::new(5);
        let request = ClientCommand::Camp {
            session_id: 5,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| camp.handle(&request, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [command::titanium_camp()]);
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::Camp(CampStatus::Preparing))]
        ));
        let done = Instant::now() + CAMP_DURATION;
        let outcome = testing::run(|out| camp.tick(done, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [command::titanium_logout()]);
        let outcome = testing::run(|out| camp.observe(&Message::LoggedOut, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(world.exit(), Some(ZoneExit::CharacterSelect)));
    }

    #[test]
    fn a_logout_before_the_timer_ends_the_camp_at_once() {
        let mut camp = Camp::default();
        let mut world = World::new(5);
        // Not camping, a logout is no camp of the player's.
        let outcome = testing::run(|out| camp.observe(&Message::LoggedOut, &mut world, out));
        outcome.result.unwrap();
        assert!(world.exit().is_none() && outcome.events.is_empty());
        // EQEmu camps a GM at once, without waiting for the logout.
        camp.start(Instant::now());
        let outcome = testing::run(|out| camp.observe(&Message::LoggedOut, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::Camp(CampStatus::Camped))]
        ));
        assert!(matches!(world.exit(), Some(ZoneExit::CharacterSelect)));
    }

    #[test]
    fn using_a_door_abandons_the_camp_and_stands_the_player_up_first() {
        let mut camp = Camp::default();
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        camp.start(Instant::now());
        let click = ClientCommand::ClickDoor {
            session_id: 5,
            door_id: 1,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| camp.notice(&click, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [command::titanium_posture(7, Posture::Standing).unwrap()]
        );
        assert!(matches!(
            outcome.events[..],
            [
                ClientEvent::World(WorldEvent::Posture { spawn_id: 7, .. }),
                ClientEvent::World(WorldEvent::Camp(CampStatus::Abandoned)),
            ]
        ));
        assert!(!camp.active());
    }

    #[test]
    fn camp_waits_thirty_seconds_and_standing_cancels_only_before_logout() {
        let now = Instant::now();
        let mut camp = Camp::default();
        assert!(!camp.active() && !camp.cancel());
        camp.start(now);
        assert!(camp.active());
        assert!(!camp.logout_due(now + Duration::from_secs(29)));
        assert!(camp.cancel());
        assert!(!camp.active());
        camp.start(now);
        camp.start(now + Duration::from_secs(20));
        assert!(camp.logout_due(now + CAMP_DURATION));
        camp.logout_sent(now + CAMP_DURATION);
        assert!(camp.logging_out() && !camp.logout_due(now + CAMP_DURATION));
        assert!(!camp.cancel());
        assert!(!camp.reply_overdue(now + CAMP_DURATION + Duration::from_secs(9)));
        assert!(camp.reply_overdue(now + CAMP_DURATION + REPLY_TIMEOUT));
    }
}
