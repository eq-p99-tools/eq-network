//! Zone transfers: the zone lines the player crosses, the server's offers to
//! move the player, its answer and the handoff to the next zone.
use super::{
    feature::{Feature, Out, World},
    motion, spellbook, zoning, ClientCommand, ClientEvent, ConnectionStage, ConnectionState,
    ZoneExit,
};
use anyhow::{ensure, Context, Result};
use eq_network_game::world::{Position, WorldEvent};
use std::time::{Duration, Instant};

/// `OP_SendZonepoints`, the zone's numbered destinations.
const ZONE_POINTS_OPCODE: u16 = 0x3eba;
/// `OP_ZonePlayerToBind`, the server returning the player to their bind point.
const TO_BIND_OPCODE: u16 = 0x385e;
/// `OP_RequestClientZoneChange`, the server moving the player.
const MOVE_OPCODE: u16 = 0x7834;
/// `OP_ZoneChange`, the player's transfer request and the server's answer.
const ZONE_CHANGE_OPCODE: u16 = 0x5dd8;
/// `OP_ZoneServerInfo`, the next zone's address.
const HANDOFF_OPCODE: u16 = 0x61b6;
/// How long a request to cross a zone line stays fresh.
const ZONE_LINE_FRESHNESS: Duration = Duration::from_millis(250);

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

    /// The transfer for a fresh request to cross a zone line where the player
    /// stands.
    fn zone_line(
        &self,
        command: &ClientCommand,
        world: &World,
        now: Instant,
    ) -> Result<zoning::ZoneOffer> {
        let ClientCommand::CrossZoneLine {
            session_id,
            destination,
            position,
            created,
        } = command
        else {
            unreachable!("only zone-line crossings cross zone lines");
        };
        ensure!(
            *session_id == world.session_id
                && *created <= now
                && now.duration_since(*created) < ZONE_LINE_FRESHNESS,
            "stale zone-line request"
        );
        ensure!(
            world
                .motion
                .as_ref()
                .is_some_and(|motion| motion.position() == *position),
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
        out.session
            .send(ZONE_CHANGE_OPCODE, &offer.response(&self.character)?)?;
        world.lifecycle.offer(offer.clone(), Instant::now())?;
        spellbook::cancel_pending(&mut world.book_action, "Zone transfer started", out.log)?;
        if let Some(motion) = world.motion.as_mut() {
            motion.suspend();
        }
        out.log
            .send(ClientEvent::World(WorldEvent::ZoneTransfer(offer)))?;
        out.log
            .send(ClientEvent::World(motion::withdrawn(world.session_id)))?;
        out.log.status(
            ConnectionState::Zoning,
            world.packets,
            Some(out.session.last_received_seconds()),
        )?;
        Ok(())
    }

    /// Takes the server's offer to move the player, within the zone or out
    /// of it.
    fn offered(
        &self,
        opcode: u16,
        body: &[u8],
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let offer = zoning::offer(opcode, body)?;
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
            .motion
            .as_ref()
            .map_or(0.0, |motion| motion.position().heading);
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
        if let zoning::ZoneReply::Rewind(position) = reply {
            let player = world
                .player
                .as_mut()
                .context("rewind without admitted player")?;
            motion::correct_own(
                player,
                world.motion.as_mut(),
                &mut world.stationary,
                world.sequence,
                position,
                Instant::now(),
            )?;
            out.log.send(ClientEvent::World(WorldEvent::Position {
                spawn_id: player.spawn_id,
                position,
                velocity: [0.0; 3],
            }))?;
        }
        out.log
            .send(ClientEvent::World(WorldEvent::ZoneTransferRejected {
                session_id: world.session_id,
                reason,
            }))?;
        if !world.lifecycle.is_dead() {
            if let Some(motion) = world.motion.as_mut() {
                motion.resume_stationary(Instant::now());
            }
            out.log.status(
                ConnectionState::Connected,
                world.packets,
                Some(out.session.last_received_seconds()),
            )?;
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
    spellbook::cancel_pending(
        &mut world.book_action,
        "Server relocated character",
        out.log,
    )?;
    let player = world
        .player
        .as_mut()
        .context("relocation without admitted player")?;
    motion::correct_own(
        player,
        world.motion.as_mut(),
        &mut world.stationary,
        world.sequence,
        position,
        Instant::now(),
    )?;
    out.log
        .send(ClientEvent::World(motion::withdrawn(world.session_id)))?;
    out.log.send(ClientEvent::World(WorldEvent::Position {
        spawn_id: player.spawn_id,
        position,
        velocity: [0.0; 3],
    }))?;
    Ok(())
}

impl Feature for Transfers {
    /// Crosses the zone line the client found the player on.
    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<bool> {
        let ClientCommand::CrossZoneLine { session_id, .. } = command else {
            return Ok(false);
        };
        match self.zone_line(command, world, Instant::now()) {
            Ok(offer) => self.start(offer, world, out)?,
            Err(error) => out
                .log
                .send(ClientEvent::World(WorldEvent::ZoneLineRejected {
                    session_id: *session_id,
                    reason: error.to_string(),
                }))?,
        }
        Ok(true)
    }

    /// The zone's zone points, the server's offers and answer, and the next
    /// zone's address. Offers and answers count only once the zone has
    /// admitted the player.
    fn receive(
        &mut self,
        opcode: u16,
        body: &[u8],
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<bool> {
        match opcode {
            ZONE_POINTS_OPCODE => match zoning::ZonePoints::decode(body) {
                Ok(points) => self.points = points,
                Err(error) => {
                    self.points = zoning::ZonePoints::default();
                    out.log
                        .diagnostic(format!("Zone-point table rejected: {error}"))?;
                }
            },
            TO_BIND_OPCODE | MOVE_OPCODE if world.ready => {
                self.offered(opcode, body, world, out)?;
            }
            ZONE_CHANGE_OPCODE if world.ready => self.answered(body, world, out)?,
            HANDOFF_OPCODE => {
                ensure!(
                    world.ready && world.lifecycle.pending().is_some(),
                    "zone handoff without a pending transfer"
                );
                out.log
                    .send(ClientEvent::Progress(ConnectionStage::ConnectingZone))?;
                world.exit = Some(ZoneExit::Direct(body.to_vec()));
            }
            _ => return Ok(false),
        }
        Ok(true)
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
    use super::*;
    use eq_network_game::movement::MotionSession;

    #[test]
    fn zone_lines_need_a_fresh_request_from_where_the_player_stands() {
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
                .zone_line(&command, world, now)
                .unwrap_err()
                .to_string()
        };
        let old = now.checked_sub(ZONE_LINE_FRESHNESS).unwrap();
        assert_eq!(
            error(&world, cross(5, here, old)),
            "stale zone-line request"
        );
        assert_eq!(
            error(&world, cross(4, here, now)),
            "stale zone-line request"
        );
        let moved = "zone-line position is no longer current";
        assert_eq!(error(&world, cross(5, here, now)), moved);
        world.motion = Some(MotionSession::new(5, 7, here, now).unwrap());
        let elsewhere = Position { x: 11.0, ..here };
        assert_eq!(error(&world, cross(5, elsewhere, now)), moved);
        let unknown = "zone identity is unavailable";
        assert_eq!(error(&world, cross(5, here, now)), unknown);
        world.zone = (2, 0);
        let offer = transfers
            .zone_line(&cross(5, here, now), &world, now)
            .unwrap();
        assert_eq!((offer.zone_id, offer.solicited), (4, false));
    }
}
