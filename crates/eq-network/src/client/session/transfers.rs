//! Zone transfers: the zone lines the player crosses, the server's offers to
//! move the player, its answer and the handoff to the next zone, and the
//! player's death, which waits for the offer home.
use super::{
    feature::{Feature, Out, World},
    motion, zoning, ClientCommand, ClientEvent, ConnectionStage, ConnectionState, ZoneExit,
};
use anyhow::{bail, ensure, Context, Result};
use eq_network_game::{
    message::{Message, Part},
    world::{Position, WorldEvent},
};
use std::time::Instant;

/// The zone's zone points, and the transfers they and the server start.
pub(super) struct Transfers {
    /// The destinations the server numbered for the zone's zone lines.
    points: zoning::ZonePoints,
    /// The player's name, which every transfer request carries.
    character: String,
}

impl Transfers {
    pub(super) fn new(character: &str) -> Self {
        Self {
            points: zoning::ZonePoints::default(),
            character: character.into(),
        }
    }

    /// The transfer for crossing a zone line where the player stands.
    fn zone_line(&self, command: &ClientCommand, world: &World) -> Result<zoning::ZoneOffer> {
        let ClientCommand::CrossZoneLine {
            destination,
            position,
            ..
        } = command
        else {
            unreachable!("only zone-line crossings cross zone lines");
        };
        ensure!(
            world.body.position() == Some(*position),
            "zone-line position is no longer current"
        );
        ensure!(world.zone.0 != 0, "zone identity is unavailable");
        self.points
            .request(destination, *position, world.zone.0, world.zone.1)
    }

    /// Asks for a transfer: the player stops, and every later command waits
    /// for the server's answer.
    fn start(
        &self,
        offer: zoning::ZoneOffer,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        out.send(&offer.response(&self.character)?)?;
        world.lifecycle.offer(offer.clone(), Instant::now())?;
        world.body.suspend();
        out.log
            .send(ClientEvent::World(WorldEvent::ZoneTransfer(offer)))?;
        out.log
            .send(ClientEvent::World(motion::withdrawn(world.session_id)))?;
        out.status(ConnectionState::Zoning, world)
    }

    /// Keeps the zone's zone points; an unreadable table leaves none. True
    /// when the message was about them.
    fn note_points(&mut self, message: &Message) -> bool {
        match message {
            Message::ZonePoints(points) => self.points = points.clone(),
            Message::Unreadable {
                part: Part::ZonePoints,
                ..
            } => self.points = zoning::ZonePoints::default(),
            _ => return false,
        }
        true
    }

    /// Takes the server's offer to move the player, within the zone or out
    /// of it.
    fn offered(
        &self,
        offer: &zoning::ZoneOffer,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let offer = offer.clone();
        if let Some(position) = offer.local_position(world.zone) {
            return relocate(position, world, out);
        }
        if let Some(pending) = world.lifecycle.pending() {
            ensure!(pending == &offer, "conflicting zone transfer offer");
            return Ok(());
        }
        self.start(offer, world, out)
    }

    /// Takes the server's answer to a transfer request: the player leaves
    /// through the world server, or stays, perhaps back where they were.
    fn answered(&self, body: &[u8], world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let pending = world
            .lifecycle
            .pending()
            .context("zone approval without a pending server offer")?;
        let heading = world
            .body
            .position()
            .map_or(0.0, |position| position.heading);
        let reply = zoning::reply(body, &self.character, pending, world.zone, heading)?;
        if reply == zoning::ZoneReply::Approved {
            world.lifecycle.finish(true)?;
            out.log
                .send(ClientEvent::Progress(ConnectionStage::ConnectingWorld))?;
            world.exit = Some(ZoneExit::World);
            return Ok(());
        }
        world.lifecycle.finish(false)?;
        let reason = match &reply {
            zoning::ZoneReply::Denied(reason) => *reason,
            zoning::ZoneReply::Rewind(_) => zoning::ZoneRejection::Cancelled,
            zoning::ZoneReply::Approved => unreachable!("approved transfer returned above"),
        };
        // The player stays, so movement resumes before anyone hears of the
        // rewind or the refusal: a calibration sent in answer postdates it.
        let alive = !world.lifecycle.is_dead();
        if alive {
            world.body.resume(Instant::now());
        }
        if let zoning::ZoneReply::Rewind(position) = reply {
            world.correct_own(position, Instant::now())?;
            out.log.send(ClientEvent::World(WorldEvent::Position {
                spawn_id: world.player.as_ref().map_or(0, |player| player.spawn_id),
                position,
                velocity: [0.0; 3],
            }))?;
        }
        out.log
            .send(ClientEvent::World(WorldEvent::ZoneTransferRejected {
                session_id: world.session_id,
                reason,
            }))?;
        if alive {
            out.status(ConnectionState::Connected, world)?;
        }
        Ok(())
    }
}

/// Moves the player within the zone, as the server asked.
fn relocate(position: Position, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
    ensure!(
        !world.lifecycle.blocks_motion(),
        "same-zone relocation conflicts with death or transfer"
    );
    world.correct_own(position, Instant::now())?;
    out.log
        .send(ClientEvent::World(motion::withdrawn(world.session_id)))?;
    out.log.send(ClientEvent::World(WorldEvent::Position {
        spawn_id: world.player.as_ref().map_or(0, |player| player.spawn_id),
        position,
        velocity: [0.0; 3],
    }))?;
    Ok(())
}

impl Feature for Transfers {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![Capability::Zoning]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::CrossZoneLine { .. })
    }

    /// Crosses the zone line the client found the player on.
    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::CrossZoneLine { session_id, .. } = command else {
            return Ok(());
        };
        match self.zone_line(command, world) {
            Ok(offer) => self.start(offer, world, out)?,
            Err(error) => out
                .log
                .send(ClientEvent::World(WorldEvent::ZoneLineRejected {
                    session_id: *session_id,
                    reason: error.to_string(),
                }))?,
        }
        Ok(())
    }

    /// The zone points arrive while the zone admits the player; the server
    /// moves no one before then.
    fn admit(&mut self, message: &Message, _world: &mut World) -> Result<()> {
        if !self.note_points(message) && matches!(message, Message::Handoff(_)) {
            bail!("zone handoff without a pending transfer");
        }
        Ok(())
    }

    /// The zone's zone points, the server's offers and answer, and the next
    /// zone's address.
    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if self.note_points(message) {
            return Ok(());
        }
        match message {
            Message::ZoneOffer(offer) => self.offered(offer, world, out),
            Message::Unreadable {
                part: Part::ZoneOffer,
                error,
            } => bail!("{error}"),
            Message::ZoneAnswer(body) => self.answered(body, world, out),
            Message::Event(WorldEvent::Death(death)) if world.is_player(death.spawn_id) => {
                world.lifecycle.mark_dead();
                out.log
                    .diagnostic("Own character died; waiting for the server bind offer".into())
            }
            Message::Handoff(address) => {
                ensure!(
                    world.lifecycle.pending().is_some(),
                    "zone handoff without a pending transfer"
                );
                out.log
                    .send(ClientEvent::Progress(ConnectionStage::ConnectingZone))?;
                world.exit = Some(ZoneExit::Direct(address.clone()));
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Gives up on a transfer the server never answered.
    fn tick(&mut self, now: Instant, world: &mut World, _out: &mut Out<'_, '_>) -> Result<()> {
        ensure!(
            !world.lifecycle.expired(now),
            "zone transfer approval timed out"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::movement::MotionSession;

    #[test]
    fn movement_resumes_before_a_refused_transfer_is_reported() {
        let mut transfers = Transfers::new("Tester");
        let mut world = World::new(5);
        let here = Position {
            x: 10.0,
            y: 20.0,
            z: 3.0,
            heading: 0.0,
        };
        world
            .body
            .admit(MotionSession::new(5, 7, here, Instant::now()).unwrap());
        world.player = Some(testing::player(7));
        world.zone = (2, 0);
        world.admitted = Some(Instant::now());
        let cross = ClientCommand::CrossZoneLine {
            session_id: 5,
            destination: zoning::ZoneLineDestination::Absolute {
                zone_id: 4,
                position: here,
            },
            position: here,
            created: Instant::now(),
        };
        testing::run(|out| transfers.handle(&cross, &mut world, out))
            .result
            .unwrap();
        let mut answer = world
            .lifecycle
            .pending()
            .unwrap()
            .response("Tester")
            .unwrap()
            .body;
        answer[84..88].copy_from_slice(&(-1i32).to_le_bytes());
        let outcome =
            testing::run(|out| transfers.observe(&Message::ZoneAnswer(answer), &mut world, out));
        outcome.result.unwrap();
        // The host calibrates movement again when it hears of the refusal.
        let heard = outcome
            .events
            .iter()
            .zip(&outcome.heard)
            .find(|(event, _)| {
                matches!(
                    event,
                    ClientEvent::World(WorldEvent::ZoneTransferRejected { .. })
                )
            })
            .map(|(_, at)| *at)
            .unwrap();
        let calibration = eq_network_game::movement::MotionCalibration {
            units_per_second: 6.0,
            velocity_scale: 0.05,
            animation: 12,
            backward: None,
            walk: None,
            strafe: None,
        };
        world
            .body
            .motion_mut()
            .unwrap()
            .calibrate_fresh(calibration, heard, Instant::now())
            .unwrap();
    }

    #[test]
    fn a_zone_line_asks_for_the_transfer_and_the_handoff_ends_the_session() {
        let mut transfers = Transfers::new("Tester");
        let mut world = World::new(5);
        let here = Position {
            x: 10.0,
            y: 20.0,
            z: 3.0,
            heading: 0.0,
        };
        world
            .body
            .admit(MotionSession::new(5, 7, here, Instant::now()).unwrap());
        world.zone = (2, 0);
        world.admitted = Some(Instant::now());
        let cross = ClientCommand::CrossZoneLine {
            session_id: 5,
            destination: zoning::ZoneLineDestination::Absolute {
                zone_id: 4,
                position: here,
            },
            position: here,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| transfers.handle(&cross, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, zoning::CHANGE_OPCODE);
        assert!(world
            .lifecycle
            .pending()
            .is_some_and(|offer| offer.zone_id == 4));
        assert!(matches!(
            outcome.events[..],
            [
                ClientEvent::World(WorldEvent::ZoneTransfer(_)),
                ClientEvent::World(WorldEvent::MotionState {
                    units_per_second: None,
                    ..
                }),
                ClientEvent::Status(_),
            ]
        ));
        let handoff = Message::Handoff(vec![1, 2, 3]);
        let outcome = testing::run(|out| transfers.observe(&handoff, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(&world.exit, Some(ZoneExit::Direct(address)) if address == &[1, 2, 3]));
    }

    #[test]
    fn zone_lines_are_crossed_from_where_the_player_stands() {
        let transfers = Transfers::new("Tester");
        let mut world = World::new(5);
        let now = Instant::now();
        let here = Position {
            x: 10.0,
            y: 20.0,
            z: 3.0,
            heading: 0.0,
        };
        let cross = |session_id, position, created| ClientCommand::CrossZoneLine {
            session_id,
            destination: zoning::ZoneLineDestination::Absolute {
                zone_id: 4,
                position: Position {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                    heading: 0.0,
                },
            },
            position,
            created,
        };
        let error = |world: &World, command| {
            transfers
                .zone_line(&command, world)
                .unwrap_err()
                .to_string()
        };
        let moved = "zone-line position is no longer current";
        assert_eq!(error(&world, cross(5, here, now)), moved);
        world
            .body
            .admit(MotionSession::new(5, 7, here, now).unwrap());
        let elsewhere = Position { x: 11.0, ..here };
        assert_eq!(error(&world, cross(5, elsewhere, now)), moved);
        let unknown = "zone identity is unavailable";
        assert_eq!(error(&world, cross(5, here, now)), unknown);
        world.zone = (2, 0);
        let offer = transfers.zone_line(&cross(5, here, now), &world).unwrap();
        assert_eq!((offer.zone_id, offer.solicited), (4, false));
    }

    #[test]
    fn the_player_dying_waits_for_the_offer_home() {
        let mut transfers = Transfers::new("Tester");
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let death = |spawn_id| {
            Message::Event(WorldEvent::Death(zoning::Death {
                spawn_id,
                killer_id: 0,
                corpse_id: 9,
                bind_zone_id: 2,
            }))
        };
        testing::run(|out| transfers.observe(&death(8), &mut world, out))
            .result
            .unwrap();
        assert!(!world.lifecycle.is_dead());
        let outcome = testing::run(|out| transfers.observe(&death(7), &mut world, out));
        outcome.result.unwrap();
        assert!(world.lifecycle.is_dead());
        assert!(matches!(outcome.events[..], [ClientEvent::Diagnostic(_)]));
    }
}
