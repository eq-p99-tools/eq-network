//! Zone transfers: the zone lines the player crosses, the server's offers to
//! move the player, its answer and the handoff to the next zone, and the
//! player's death, which waits for the offer home or, where the client asks
//! its own way home, asks for the bind point once the death pause is over.
//! Every death of the player comes here, the one the server names and the
//! one the client reports itself, so this feature alone marks the player
//! dead.
//!
//! Once a zone approves a transfer, the player departs as the official client
//! does: it asks the zone to save the player and takes its own spawn out
//! (`OP_SaveOnZoneReq`, then `OP_DeleteSpawn`), and goes on to the world
//! server when the zone answers with a logout, the connection ends or a
//! moment passes, whichever comes first. `EQEmu` saves, drops the spawn and
//! closes the connection on them (`zone/client_packet.cpp` `Handle_OP_SaveOnZoneReq`,
//! `Handle_OP_DeleteSpawn`); TAKP holds the move until the spawn leaves
//! (`zone/zoning.cpp` `HandleZoneTransferResponse`).
use super::{
    feature::{Feature, Out, World},
    motion, zoning, ClientCommand, ClientEvent, ConnectionStage, ConnectionState, ZoneExit,
};
use anyhow::{bail, ensure, Context, Result};
use eq_network_game::{
    message::{Message, Part},
    request::Request,
    world::{Position, WorldEvent},
};
use std::time::{Duration, Instant};

/// How long the player's departure waits for the zone's answer (inferred).
/// In Adam's P99 recording the zone answered 23 and 30 ms after the
/// departure, and the official client reached the world server 0.5 and
/// 0.9 s after it; whether it waits for the answer is unrecorded, and
/// `EQMacEmu` says the official `EQMac` client ignores it.
const DEPARTURE: Duration = Duration::from_secs(2);

/// How long a dead player who asks their way home waits before asking for
/// their bind point: none, as Adam chose for eq-network#90 (2026-10-05).
/// The official client's wait is unrecorded (inferred).
const DEATH_PAUSE: Duration = Duration::ZERO;

/// How the player goes home once they die.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Home {
    /// The server offers the move to the bind point
    /// (`OP_ZonePlayerToBind`), as Titanium's servers do.
    Offered,
    /// The client asks for its bind point itself, as `EQMac`'s does: TAKP
    /// holds the move home until the client asks (`zone/zoning.cpp`
    /// `GoToDeath`, `Handle_OP_ZoneChange`), and removes a dead client that
    /// never does.
    Asked,
}

/// The zone's zone points, and the transfers they and the server start.
pub(super) struct Transfers {
    /// The destinations the server numbered for the zone's zone lines.
    points: zoning::ZonePoints,
    /// The player's name, which the server's answer to a transfer names.
    character: String,
    /// How the player goes home once they die.
    home: Home,
    /// Where home is, once the zone says.
    bind: Option<zoning::BindPoint>,
    /// When a dead player who asks their way home asks for their bind
    /// point, from their death on.
    going_home: Option<Instant>,
}

impl Transfers {
    pub(super) fn new(character: &str, home: Home) -> Self {
        Self {
            points: zoning::ZonePoints::default(),
            character: character.into(),
            home,
            bind: None,
            going_home: None,
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
    fn start(offer: zoning::ZoneOffer, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        noted(
            out,
            &Request::AnswerZoneOffer {
                zone_id: offer.zone_id,
                instance_id: offer.instance_id,
                position: offer.position,
                reason: offer.reason,
            },
            "the zone change",
        )?;
        world.transfer_offered(offer.clone(), Instant::now())?;
        out.log
            .send(ClientEvent::World(WorldEvent::ZoneTransfer(offer)))?;
        out.log
            .send(ClientEvent::World(motion::withdrawn(world.session_id)))?;
        out.status(ConnectionState::Zoning, world)
    }

    /// Keeps the zone's zone points, an unreadable table leaving none, and
    /// the player's bind point. True when the message was about them.
    fn note_points(&mut self, message: &Message) -> bool {
        match message {
            Message::ZonePoints(points) => self.points = points.clone(),
            Message::Bind(bind) => self.bind = Some(*bind),
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
    fn offered(offer: &zoning::ZoneOffer, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let offer = offer.clone();
        if let Some(position) = offer.local_position(world.zone) {
            return relocate(position, world, out);
        }
        if let Some(pending) = world.lifecycle.pending() {
            ensure!(pending == &offer, "conflicting zone transfer offer");
            return Ok(());
        }
        Self::start(offer, world, out)
    }

    /// Takes the server's answer to a transfer request: the player departs
    /// for the world server, or stays, perhaps back where they were.
    fn answered(
        &self,
        answer: &zoning::ZoneAnswer,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let pending = world
            .lifecycle
            .pending()
            .context("zone approval without a pending server offer")?;
        let heading = world
            .body
            .position()
            .map_or(0.0, |position| position.heading);
        let reply = zoning::reply(answer, &self.character, pending, world.zone, heading)?;
        if reply == zoning::ZoneReply::Approved {
            world.lifecycle.finish(true, Instant::now())?;
            noted(out, &Request::SaveOnZone, "the save before leaving")?;
            noted(out, &Request::Depart, "the departure")?;
            out.log
                .send(ClientEvent::Progress(ConnectionStage::ConnectingWorld))?;
            return Ok(());
        }
        // The player stays, so movement resumes before anyone hears of the
        // rewind or the refusal: a calibration sent in answer postdates it.
        world.transfer_refused(Instant::now())?;
        let reason = match &reply {
            zoning::ZoneReply::Denied(reason) => *reason,
            zoning::ZoneReply::Rewind(_) => zoning::ZoneRejection::Cancelled,
            zoning::ZoneReply::Approved => unreachable!("approved transfer returned above"),
        };
        let alive = !world.lifecycle.is_dead();
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

/// Sends a request and says so: zoning's few packets are worth reading back
/// in the session's diagnostics.
fn noted(out: &mut Out<'_, '_>, request: &Request, what: &str) -> Result<()> {
    let packet = out.encode(request)?;
    out.send(&packet)?;
    out.log.diagnostic(format!(
        "Zoning: sent {what} (0x{:04x}, {} bytes)",
        packet.opcode,
        packet.body.len()
    ))
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
            Ok(offer) => Self::start(offer, world, out)?,
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
            Message::ZoneOffer(offer) => Self::offered(offer, world, out),
            Message::Unreadable {
                part: Part::ZoneOffer | Part::ZoneAnswer,
                error,
            } => bail!("{error}"),
            // EQEmu answers a move to another zone twice, as the world's
            // reply reaches the zone twice (`zone/worldserver.cpp`), and
            // TAKP answers any repeat of the request again (`zone/zoning.cpp`
            // `Handle_OP_ZoneChange`); the player has already departed.
            Message::ZoneAnswer(_) if world.lifecycle.departing() => out
                .log
                .diagnostic("Zoning: the zone answered again while the player departs".into()),
            Message::ZoneAnswer(answer) => self.answered(answer, world, out),
            // The zone's answer to the player's departure.
            Message::LoggedOut if world.lifecycle.departing() => {
                world.end(ZoneExit::World);
                out.log
                    .diagnostic("Zoning: the zone answered the departure".into())
            }
            Message::Event(WorldEvent::Death(death)) if world.is_player(death.spawn_id) => {
                let already = world.lifecycle.is_dead();
                world.died();
                match self.home {
                    // A second word of the same death, such as the server's
                    // after the client's own report, changes nothing.
                    Home::Asked if already => Ok(()),
                    Home::Asked => {
                        self.going_home = Some(Instant::now() + DEATH_PAUSE);
                        out.log.diagnostic(
                            "Own character died; asking for the bind point after the death pause"
                                .into(),
                        )
                    }
                    Home::Offered => out
                        .log
                        .diagnostic("Own character died; waiting for the server bind offer".into()),
                }
            }
            Message::Handoff(address) => {
                ensure!(
                    world.lifecycle.pending().is_some(),
                    "zone handoff without a pending transfer"
                );
                out.log
                    .send(ClientEvent::Progress(ConnectionStage::ConnectingZone))?;
                world.end(ZoneExit::Direct(address.clone()));
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Gives up on a transfer the server never answered, goes on to the
    /// world server once a departure has waited long enough, and asks a dead
    /// player's way home once the death pause is over and no transfer is
    /// under way.
    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        ensure!(
            !world.lifecycle.expired(now),
            "zone transfer approval timed out"
        );
        if world.lifecycle.departed(now, DEPARTURE) {
            world.end(ZoneExit::World);
            return out
                .log
                .diagnostic("Zoning: the zone did not answer the departure".into());
        }
        let due = self.going_home.is_some_and(|at| now >= at);
        if due && world.lifecycle.is_dead() && world.lifecycle.pending().is_none() {
            self.going_home = None;
            return match self.bind {
                Some(bind) => Self::start(bind.offer(), world, out),
                None => out.log.diagnostic(
                    "Own character died, and the zone never said where its bind point is".into(),
                ),
            };
        }
        Ok(())
    }

    /// A zone closing the connection on the departing player lets them go.
    fn connection_ended(&mut self, world: &mut World) {
        if world.lifecycle.departing() {
            world.end(ZoneExit::World);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::movement::MotionSession;

    #[test]
    fn movement_resumes_before_a_refused_transfer_is_reported() {
        let mut transfers = Transfers::new("Tester", Home::Offered);
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
        world.player.admit(testing::player(7));
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
        let answer = zoning::ZoneAnswer::titanium(&answer).unwrap();
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
        let mut transfers = Transfers::new("Tester", Home::Offered);
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
                ClientEvent::Diagnostic(_),
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
        assert!(matches!(world.exit(), Some(ZoneExit::Direct(address)) if address == &[1, 2, 3]));
    }

    /// A world whose zone approved a crossing to zone 4: the player's spawn
    /// 7 is departing.
    fn approved(transfers: &mut Transfers) -> (World, testing::Outcome<Result<()>>) {
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
        world.player.admit(testing::player(7));
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
        answer[84..88].copy_from_slice(&1i32.to_le_bytes());
        let answer = zoning::ZoneAnswer::titanium(&answer).unwrap();
        let outcome =
            testing::run(|out| transfers.observe(&Message::ZoneAnswer(answer), &mut world, out));
        (world, outcome)
    }

    #[test]
    fn an_approved_transfer_saves_and_departs_before_the_world() {
        let mut transfers = Transfers::new("Tester", Home::Offered);
        let (mut world, outcome) = approved(&mut transfers);
        outcome.result.unwrap();
        // The save and the departure go out, in the official client's order,
        // and the player waits for the zone.
        assert_eq!(
            outcome.sent,
            [zoning::titanium_save_on_zone(), zoning::titanium_depart(7)]
        );
        assert!(world.exit().is_none() && world.lifecycle.departing());
        // EQEmu's second answer changes nothing.
        let mut again = zoning::titanium_answer("Tester", (4, 0), Position::default(), 0)
            .unwrap()
            .body;
        again[84..88].copy_from_slice(&1i32.to_le_bytes());
        let again = zoning::ZoneAnswer::titanium(&again).unwrap();
        let outcome =
            testing::run(|out| transfers.observe(&Message::ZoneAnswer(again), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 0);
        assert!(world.exit().is_none() && world.lifecycle.departing());
        // The zone's logout lets the player go.
        let outcome = testing::run(|out| transfers.observe(&Message::LoggedOut, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(world.exit(), Some(ZoneExit::World)));
    }

    #[test]
    fn a_departure_goes_on_once_the_wait_runs_out() {
        let mut transfers = Transfers::new("Tester", Home::Offered);
        let (mut world, _) = approved(&mut transfers);
        let now = Instant::now();
        testing::run(|out| transfers.tick(now, &mut world, out))
            .result
            .unwrap();
        assert!(world.exit().is_none());
        testing::run(|out| transfers.tick(now + DEPARTURE, &mut world, out))
            .result
            .unwrap();
        assert!(matches!(world.exit(), Some(ZoneExit::World)));
    }

    #[test]
    fn a_departure_goes_on_when_the_zone_closes_the_connection() {
        let mut transfers = Transfers::new("Tester", Home::Offered);
        let (mut world, _) = approved(&mut transfers);
        transfers.connection_ended(&mut world);
        assert!(matches!(world.exit(), Some(ZoneExit::World)));
        // A connection ending at any other time is not the player's to take.
        let mut transfers = Transfers::new("Tester", Home::Offered);
        let mut quiet = World::new(5);
        transfers.connection_ended(&mut quiet);
        assert!(quiet.exit().is_none());
        // Nor is a logout without a departure.
        let outcome = testing::run(|out| transfers.observe(&Message::LoggedOut, &mut quiet, out));
        outcome.result.unwrap();
        assert!(quiet.exit().is_none());
    }

    #[test]
    fn zone_lines_are_crossed_from_where_the_player_stands() {
        let transfers = Transfers::new("Tester", Home::Offered);
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

    /// An admitted player, spawn 7, in zone 2 with their bind point there.
    fn bound(transfers: &mut Transfers) -> (World, zoning::BindPoint) {
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        world.own_spawn = Some(7);
        world.zone = (2, 0);
        world.admitted = Some(Instant::now());
        let bind = zoning::BindPoint {
            zone_id: 2,
            position: Position {
                x: -74.0,
                y: 428.0,
                z: 3.75,
                heading: 0.0,
            },
        };
        testing::run(|out| transfers.observe(&Message::Bind(bind), &mut world, out))
            .result
            .unwrap();
        (world, bind)
    }

    /// The player's death, as the server or the client's own report names it.
    fn died() -> Message {
        Message::Event(WorldEvent::Death(zoning::Death {
            spawn_id: 7,
            killer_id: 0,
            corpse_id: 7,
            bind_zone_id: 0,
            corpse_name: None,
        }))
    }

    #[test]
    fn a_client_that_asks_its_way_home_asks_for_its_bind_point_after_the_death_pause() {
        let mut transfers = Transfers::new("Tester", Home::Asked);
        let (mut world, bind) = bound(&mut transfers);
        let before = Instant::now()
            .checked_sub(Duration::from_millis(1))
            .unwrap();
        let outcome = testing::run(|out| transfers.observe(&died(), &mut world, out));
        outcome.result.unwrap();
        // The player is dead at once, and asks for nothing yet.
        assert_eq!(outcome.sent, []);
        assert!(world.lifecycle.is_dead() && world.lifecycle.pending().is_none());
        testing::run(|out| transfers.tick(before, &mut world, out))
            .result
            .unwrap();
        assert!(world.lifecycle.pending().is_none());
        // A second word of the same death changes nothing.
        let outcome = testing::run(|out| transfers.observe(&died(), &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.events.is_empty());
        // Once the pause is over, the request goes out once, for the bind
        // point's zone.
        let after = Instant::now() + DEATH_PAUSE;
        let outcome = testing::run(|out| transfers.tick(after, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, zoning::CHANGE_OPCODE);
        assert_eq!(world.lifecycle.pending(), Some(&bind.offer()));
        assert!(world.lifecycle.is_dead());
        let outcome = testing::run(|out| transfers.tick(after, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        // Without a bind point there is nothing to ask for.
        let mut lost = Transfers::new("Tester", Home::Asked);
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        world.own_spawn = Some(7);
        testing::run(|out| lost.observe(&died(), &mut world, out))
            .result
            .unwrap();
        let outcome = testing::run(|out| lost.tick(after, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 0);
        assert!(world.lifecycle.pending().is_none() && world.lifecycle.is_dead());
    }

    #[test]
    fn a_death_while_zoning_asks_its_way_home_once_the_transfer_is_refused() {
        let mut transfers = Transfers::new("Tester", Home::Asked);
        let (mut world, bind) = bound(&mut transfers);
        let here = Position {
            x: 10.0,
            y: 20.0,
            z: 3.0,
            heading: 0.0,
        };
        world
            .body
            .admit(MotionSession::new(5, 7, here, Instant::now()).unwrap());
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
        let crossing = world.lifecycle.pending().cloned().unwrap();
        testing::run(|out| transfers.observe(&died(), &mut world, out))
            .result
            .unwrap();
        // The crossing is still under way, so the player waits for its answer.
        let after = Instant::now() + DEATH_PAUSE;
        let outcome = testing::run(|out| transfers.tick(after, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert_eq!(world.lifecycle.pending(), Some(&crossing));
        let mut answer = crossing.response("Tester").unwrap().body;
        answer[84..88].copy_from_slice(&(-1i32).to_le_bytes());
        let answer = zoning::ZoneAnswer::titanium(&answer).unwrap();
        testing::run(|out| transfers.observe(&Message::ZoneAnswer(answer), &mut world, out))
            .result
            .unwrap();
        assert!(world.lifecycle.is_dead() && world.lifecycle.pending().is_none());
        // Refused, the player asks their way home.
        let outcome = testing::run(|out| transfers.tick(after, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(world.lifecycle.pending(), Some(&bind.offer()));
    }

    #[test]
    fn the_player_dying_waits_for_the_offer_home() {
        let mut transfers = Transfers::new("Tester", Home::Offered);
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let death = |spawn_id| {
            Message::Event(WorldEvent::Death(zoning::Death {
                spawn_id,
                killer_id: 0,
                corpse_id: 9,
                bind_zone_id: 2,
                corpse_name: None,
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
