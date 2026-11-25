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
    boats::riders,
    message::{Message, Part},
    movement::carrier_local,
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
    /// A profile-specific stand-in used only in approval of a bind transfer
    /// back to this exact current zone. Never applies to ordinary transfers.
    same_zone_bind_reply: Option<u16>,
}

impl Transfers {
    pub(super) fn new(character: &str, home: Home) -> Self {
        Self {
            points: zoning::ZonePoints::default(),
            character: character.into(),
            home,
            bind: None,
            going_home: None,
            same_zone_bind_reply: None,
        }
    }

    /// Quarm's worldserver.cpp answers a same-zone move with zone 185.
    pub(super) fn with_same_zone_bind_reply(mut self, alias: u16) -> Self {
        self.same_zone_bind_reply = Some(alias);
        self
    }

    /// The transfer for crossing a zone line where the player stands.
    fn zone_line(&self, command: &ClientCommand, world: &World) -> Result<zoning::BoundaryRoute> {
        let ClientCommand::CrossZoneLine {
            destination,
            position,
            created,
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
        let crossing = self.points.crossing(destination, *position, world.zone)?;
        if matches!(crossing, zoning::BoundaryRoute::SameZone(_)) {
            let boundaries = world
                .pad_boundaries
                .as_ref()
                .context("same-zone boundary needs a server relocation")?;
            ensure!(
                world.ready() && !world.lifecycle.blocks_motion() && world.player.is_some(),
                "pad entry outside active admission"
            );
            if let Some(reason) = world.control.input_refusal(*created) {
                bail!("{reason}");
            }
            let approach = world
                .body
                .pad_approach(*created)
                .context("stale pad entry")?;
            ensure!(
                boundaries
                    .entered(&world.zone_name, approach, *position)
                    .as_ref()
                    == Some(destination),
                "no matching pad entrance in the installed zone"
            );
            if let Some(reason) = super::actions::stale(command, world.session_id, Instant::now()) {
                bail!("{reason}");
            }
        }
        Ok(crossing)
    }

    /// Asks for a transfer: the player stops, and every later command waits
    /// for the server's answer.
    pub(super) fn start(
        offer: zoning::ZoneOffer,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        // The program flushes the movement history before its zone-change
        // request, whatever the records' age (0x4ab056).
        let allowed = !world.validated;
        world
            .body
            .report_history(Instant::now(), allowed, true, out)?;
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
        // A server zone command forgets the boat before anything else
        // (Titanium 0x456110; read R36 B), and so does the move home after
        // death (0x4564ba). The server places the player now, so an arrival
        // aboard is no longer awaited either.
        world.forget_vehicle();
        world.arrival = None;
        world.carried_from = super::riders::CarriedFrom::Elsewhere;
        let offer = offer.clone();
        if let Some(position) = offer.local_position(world.zone) {
            // The explicit movement instruction supplies its own absolute
            // landing. Preserve-value sentinels belong to zone-point routes.
            return relocate(position, world);
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
        let mut answer = answer.clone();
        if pending.to_bind
            && pending.solicited
            && answer.success == 1
            && (pending.zone_id, pending.instance_id) == world.zone
            && self.same_zone_bind_reply == Some(answer.zone_id)
            && answer.instance_id == pending.instance_id
        {
            // Only normalize the documented profile's stand-in. reply still
            // checks identity, success and instance against the real request.
            answer.zone_id = pending.zone_id;
        }
        let reply = zoning::reply(&answer, &self.character, pending, world.zone, heading)?;
        if reply == zoning::ZoneReply::Approved {
            let save = save_on_leaving(&pending.clone(), world);
            world.lifecycle.finish(true, Instant::now())?;
            if let Some(save) = save {
                noted(out, &Request::SaveOnZone(save), "the save before leaving")?;
            }
            noted(out, &Request::Depart, "the departure")?;
            out.log
                .send(ClientEvent::Progress(ConnectionStage::ConnectingWorld))?;
            return Ok(());
        }
        // The player stays, so movement resumes before anyone hears of the
        // rewind or the refusal: a calibration sent in answer postdates it.
        let from_wait = world.carried_from == super::riders::CarriedFrom::Wait;
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
                forward: 0.0,
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
        if from_wait {
            // A refused zone request clears the link and the name and
            // stands the player at the safe point (0x496d00; read R36b §7):
            // the rider carried on from the arrival wait has no boat here.
            super::riders::Riders::safe_point(world, out)?;
        }
        Ok(())
    }
}

/// The save before leaving, as Titanium makes it at the accept (0x4ab056,
/// 0x4976bf; reads R36 §2, R36b §1), or None when the program sends none.
///
/// - A transfer that began from the arrival wait sends no save and keeps
///   the vehicle: the boat was never found here.
/// - A boat transfer (reason 2), while the session still finds the boat by
///   its name, saves the player's place in the boat's frame and the name.
/// - Any other transfer forgets the vehicle and saves where the accepted
///   request goes, its heading resolved as the request resolved it (999
///   keeping the player's): the program fills every save from the profile
///   (R49-36b), and that the profile then holds the request's place and
///   heading is capture-derived (every recorded official save holds them).
fn save_on_leaving(pending: &zoning::ZoneOffer, world: &mut World) -> Option<zoning::Save> {
    let reason = pending.reason;
    if std::mem::take(&mut world.carried_from) == super::riders::CarriedFrom::Wait {
        return None;
    }
    let position = world.body.position();
    let aboard = (reason == riders::ZONE_REASON)
        .then(|| world.vehicle.clone())
        .flatten()
        .and_then(|name| {
            let id = riders::find(
                &name,
                world
                    .spawns
                    .all()
                    .map(|spawn| (spawn.spawn_id, spawn.name.as_str())),
            )?;
            let report = world.carriers.latest(id)?;
            let pose = world.carriers.pose(report.sample(), Instant::now())?;
            Some((name, carrier_local(pose, position?)))
        });
    let save = if let Some((name, local)) = aboard {
        zoning::Save {
            position: Some(local),
            vehicle: Some(name),
        }
    } else {
        world.forget_vehicle();
        zoning::Save {
            position: Some(pending.position),
            vehicle: None,
        }
    };
    // The record's third field keeps the z this save carries (read
    // R49-22f, 6.613).
    world
        .riding
        .saved(save.position.map_or(0.0, |position| position.z));
    Some(save)
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
fn relocate(position: Position, world: &mut World) -> Result<()> {
    ensure!(
        !world.lifecycle.blocks_motion(),
        "same-zone relocation conflicts with death or transfer"
    );
    let (spawn_id, _) = world.player_at().context("relocation before admission")?;
    world.correct_own(position, Instant::now())?;
    // The motion feature hears it next and keeps a grant in force (read
    // R18, the same-zone warp 0x48aaa6); the host hears it after.
    world.happened(Message::Event(WorldEvent::SameZoneRelocated(
        zoning::SameZoneRelocation {
            session_id: world.session_id,
            spawn_id,
            position,
        },
    )));
    Ok(())
}

impl Feature for Transfers {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![Capability::Zoning]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::CrossZoneLine { .. } | ClientCommand::FinishLocalPad { .. }
        )
    }

    /// Crosses the zone line the client found the player on.
    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let ClientCommand::FinishLocalPad {
            session_id,
            revision,
            step,
            created,
        } = command
        {
            let completion = (|| {
                ensure!(
                    world.ready() && !world.lifecycle.blocks_motion(),
                    "pad completion outside active admission"
                );
                let player = world
                    .player
                    .corrected()
                    .context("pad completion before admission")?;
                world
                    .body
                    .finish_pad(player, *session_id, *revision, *step, *created)
            })();
            match completion {
                Ok(completed) => {
                    world.happened(Message::Event(WorldEvent::LocalPadCompleted(completed)));
                }
                Err(error) => super::actions::refuse(command, &error.to_string(), out.log)?,
            }
            return Ok(());
        }
        let ClientCommand::CrossZoneLine {
            session_id,
            created,
            ..
        } = command
        else {
            return Ok(());
        };
        match self.zone_line(command, world) {
            Ok(zoning::BoundaryRoute::Transfer(offer)) => Self::start(offer, world, out)?,
            Ok(zoning::BoundaryRoute::SameZone(pad)) => {
                let player = world
                    .player
                    .corrected()
                    .context("pad entry before admission")?;
                let revision = world.body.cross_pad(player, pad, *created, out)?;
                let crossing = zoning::LocalPadCrossing {
                    revision,
                    session_id: world.session_id,
                    spawn_id: player.spawn_id,
                    entry: pad.entry(),
                    landing: pad.landing(),
                };
                // As for a relocation: the motion feature hears it next.
                world.happened(Message::Event(WorldEvent::LocalPadCrossed(crossing)));
            }
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
            Message::ZoneOffer(_) if world.lifecycle.departing() => out
                .log
                .diagnostic("Zoning: ignored another offer while the player departs".into()),
            Message::ZoneOffer(offer)
                if self.same_zone_bind_reply.is_some()
                    && offer.to_bind
                    && !world.lifecycle.is_dead() =>
            {
                // Quarm's only outgoing GMGoto is ZoneToBindPoint (Death
                // Only); forced non-death binds use RequestClientZoneChange.
                // Let every owner and the host hear this knowledge before the
                // transfer, without manufacturing a Death/corpse packet.
                world.happened(Message::Event(WorldEvent::DeathBind {
                    session_id: world.session_id,
                }));
                world.happened(Message::ZoneOffer(offer.clone()));
                Ok(())
            }
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
            Message::Event(WorldEvent::DeathBind { session_id })
                if *session_id == world.session_id =>
            {
                world.died();
                // The queued bind offer owns the destination; do not also
                // start a cached-profile bind request on the next tick.
                self.going_home = None;
                Ok(())
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
    use eq_network_game::movement::{DiagonalRates, MotionSession};

    #[test]
    fn a_server_move_within_the_zone_keeps_its_wire_values() {
        use crate::client::{ClientConfig, ServerProtocol};
        let mut config = ClientConfig::new("Example", "Example", "Example", "Tester");
        config.local_only = true;
        config.protocol = ServerProtocol::EqEmu;
        let here = Position {
            x: 12.0,
            y: -4.0,
            z: 3.0,
            heading: 64.0,
        };
        let mut world = World::for_zone(5, &config);
        world
            .body
            .admit(MotionSession::new(5, 7, here, Instant::now()).unwrap());
        world.player.admit(testing::player(7));
        world.zone = (2, 0);
        world.admitted = Some(Instant::now());
        let mut transfers = Transfers::new("Tester", Home::Offered);

        // These finite values happen to equal the separate zone-point
        // sentinels. The explicit move must preserve their wire meaning.
        let mut packet = [0; 24];
        packet[..2].copy_from_slice(&2u16.to_le_bytes());
        for (offset, value) in [(4, 72.0f32), (8, 999_999.0), (12, 8.0), (16, 999.0)] {
            packet[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        packet[20..24].copy_from_slice(&1u32.to_le_bytes());
        let offer = zoning::offer(zoning::MOVE_OPCODE, &packet).unwrap();
        let arrival =
            testing::run(|out| transfers.observe(&Message::ZoneOffer(offer), &mut world, out));
        arrival.result.unwrap();
        assert_eq!(arrival.sent, []);
        let landing = Position {
            x: 999_999.0,
            y: 72.0,
            z: 8.0,
            heading: 999.0,
        };
        assert_eq!(world.body.position(), Some(landing));
        // No withdrawal here: the motion feature hears the relocation next
        // and keeps a grant in force.
        assert!(arrival.events.is_empty());
        assert!(
            matches!(world.take_news().as_slice(), [Message::Event(WorldEvent::SameZoneRelocated(arrival))]
            if arrival.session_id == 5 && arrival.spawn_id == 7 && arrival.position == landing)
        );
        assert!(world.lifecycle.pending().is_none());
    }

    /// The Mac programs' and `RoF2`'s same-zone lines are unread: they stay
    /// refused, as before 6.640.
    #[test]
    fn mac_and_rof2_same_zone_lines_stay_refused() {
        use crate::client::{ClientConfig, ServerProtocol};
        for protocol in [
            ServerProtocol::Quarm,
            ServerProtocol::Takp,
            ServerProtocol::EqEmuRof2,
        ] {
            let mut config = ClientConfig::new("Example", "Example", "Example", "Tester");
            config.protocol = protocol;
            let here = Position {
                x: 12.0,
                y: -4.0,
                z: 3.0,
                heading: 64.0,
            };
            let mut world = World::for_zone(5, &config);
            world
                .body
                .admit(MotionSession::new(5, 7, here, Instant::now()).unwrap());
            world.player.admit(testing::player(7));
            world.zone = (2, 0);
            world.admitted = Some(Instant::now());
            let mut transfers = Transfers::new("Tester", Home::Offered);
            let cross = ClientCommand::CrossZoneLine {
                session_id: 5,
                destination: zoning::ZoneLineDestination::Absolute {
                    zone_id: 2,
                    position: Position::default(),
                },
                position: here,
                created: Instant::now(),
            };
            let trigger = testing::run(|out| transfers.handle(&cross, &mut world, out));
            trigger.result.unwrap();
            assert_eq!(trigger.sent, [], "{protocol:?}");
            assert!(
                matches!(
                    trigger.events.as_slice(),
                    [ClientEvent::World(WorldEvent::ZoneLineRejected {
                        session_id: 5,
                        ..
                    })]
                ),
                "{protocol:?}"
            );
            assert_eq!(world.body.position(), Some(here));
        }
    }

    #[test]
    fn p99_pad_checks_installed_entry_and_submits_only_ordinary_positions() {
        same_zone_line_checks_installed_entry_and_lands_locally(
            crate::client::ServerProtocol::Project1999,
            false,
        );
    }

    /// Titanium's 0x48aaa6 places the player itself on any server (read
    /// same-zone zone line, 6.640), local `EQEmu` included.
    #[test]
    fn a_local_eqemu_same_zone_line_lands_locally_as_the_program_does() {
        same_zone_line_checks_installed_entry_and_lands_locally(
            crate::client::ServerProtocol::EqEmu,
            true,
        );
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Keeps the pad route, gate and packet assertions together"
    )]
    fn same_zone_line_checks_installed_entry_and_lands_locally(
        protocol: crate::client::ServerProtocol,
        local_only: bool,
    ) {
        use crate::client::{ClientConfig, InstalledZoneBoundaries};
        use eq_network_game::movement::{
            MotionCalibration, MovementMode, MovementRequest, UpdateGate, POSITION_OPCODE,
            TITANIUM_FLAGS,
        };

        #[derive(Debug)]
        struct Boundaries {
            from: Position,
            to: Position,
        }

        impl InstalledZoneBoundaries for Boundaries {
            fn entered(
                &self,
                zone: &str,
                from: Position,
                to: Position,
            ) -> Option<zoning::ZoneLineDestination> {
                (zone == "example" && from == self.from && to == self.to)
                    .then_some(zoning::ZoneLineDestination::Reference(7))
            }
        }

        let start = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        let from = Position {
            y: 3.0,
            heading: 64.0,
            ..Position::default()
        };
        let entry = Position { x: 0.5, ..from };
        let landing = Position {
            x: -20.0,
            z: 6.0,
            ..entry
        };
        let mut config = ClientConfig::new("Example", "Example", "Example", "Tester");
        config.protocol = protocol;
        config.local_only = local_only;
        config.installed_zone_boundaries =
            Some(std::sync::Arc::new(Boundaries { from, to: entry }));
        let mut world = World::for_zone(5, &config);
        world.zone = (42, 3);
        world.zone_name = "example".to_owned();
        world.admitted = Some(start);
        world.player.admit(testing::player(7));
        let mut motion = MotionSession::new(5, 7, from, start)
            .unwrap()
            .with_gate(Some(UpdateGate::Flagged {
                pack: |sample| sample.packet(),
                rule: TITANIUM_FLAGS,
            }));
        motion
            .calibrate(
                MotionCalibration {
                    swim_backward_units_per_second: None,
                    swim: None,
                    units_per_second: 6.0,
                    velocity_scale: 0.05,
                    animation: 12,
                    walk: None,
                    strafe: None,
                    backward: None,
                    diagonal: DiagonalRates::default(),
                },
                start,
            )
            .unwrap();
        let moved = start + Duration::from_millis(300);
        motion
            .send_move_sample(
                &MovementRequest::new(MovementMode::Forward, 5, entry, moved),
                moved,
                |_| Ok(()),
            )
            .unwrap();
        world.body.admit(motion);

        let mut transfers = Transfers::new("Tester", Home::Offered);
        let mut points = [0; 28];
        points[..4].copy_from_slice(&1u32.to_le_bytes());
        points[4..8].copy_from_slice(&7u32.to_le_bytes());
        for (offset, value) in [(8, 1_000_000.0_f32), (12, -20.0), (16, 6.0), (20, 513.0)] {
            points[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        points[24..26].copy_from_slice(&42u16.to_le_bytes());
        points[26..28].copy_from_slice(&3u16.to_le_bytes());
        transfers.points = zoning::ZonePoints::decode(&points).unwrap();
        let crossing = ClientCommand::CrossZoneLine {
            session_id: 5,
            destination: zoning::ZoneLineDestination::Reference(7),
            position: entry,
            created: Instant::now(),
        };
        world.zone_name = "different".to_owned();
        let refused = testing::run(|out| transfers.handle(&crossing, &mut world, out));
        refused.result.unwrap();
        assert_eq!(refused.sent, []);
        assert!(matches!(
            refused.events.as_slice(),
            [ClientEvent::World(WorldEvent::ZoneLineRejected { .. })]
        ));
        world.zone_name = "example".to_owned();
        let crossed = testing::run(|out| transfers.handle(&crossing, &mut world, out));
        crossed.result.unwrap();
        let [packet] = crossed.sent.as_slice() else {
            panic!("expected one entry report");
        };
        assert_eq!(packet.opcode, POSITION_OPCODE);
        assert_eq!(packet.body.len(), 36);
        assert_eq!(&packet.body[..4], &[7, 0, 1, 0]);
        assert_eq!(&packet.body[24..28], &entry.x.to_le_bytes());
        assert!(crossed.events.is_empty());
        assert!(matches!(
            world.take_news().as_slice(),
            [Message::Event(WorldEvent::LocalPadCrossed(pad))]
                if pad.entry == entry && pad.landing == landing && pad.session_id == 5
        ));
        assert_eq!(world.body.position(), Some(landing));
        assert_eq!(world.player.as_ref().unwrap().position, landing);
        let repeated = testing::run(|out| transfers.handle(&crossing, &mut world, out));
        repeated.result.unwrap();
        assert_eq!(repeated.sent, []);
        let held = testing::run(|out| world.body.heartbeat(Instant::now(), out));
        held.result.unwrap();
        assert!(held.sent.is_empty() && held.unreliable.is_empty());
        let finish = ClientCommand::FinishLocalPad {
            session_id: 5,
            revision: 1,
            step: zoning::PadArrivalStep {
                floor: Some(6.0),
                stand_height: 3.75,
                vertical_speed: -12.0,
            },
            created: Instant::now(),
        };
        let invalid = ClientCommand::FinishLocalPad {
            session_id: 5,
            revision: 1,
            step: zoning::PadArrivalStep {
                floor: Some(100.0),
                stand_height: 3.75,
                vertical_speed: -12.0,
            },
            created: Instant::now(),
        };
        let refused = testing::run(|out| transfers.handle(&invalid, &mut world, out));
        refused.result.unwrap();
        assert!(refused.sent.is_empty() && refused.unreliable.is_empty());
        assert!(refused.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::LocalPadRefused {
                session_id: 5,
                revision: 1,
                ..
            })
        )));
        assert_eq!(world.body.position(), Some(landing));
        let finished = testing::run(|out| transfers.handle(&finish, &mut world, out));
        finished.result.unwrap();
        assert!(finished.sent.is_empty() && finished.unreliable.is_empty());
        let following = Position { z: 9.75, ..landing };
        assert_eq!(world.body.position(), Some(following));
        assert_eq!(world.player.as_ref().unwrap().position, following);
        assert!(
            matches!(world.take_news().as_slice(), [Message::Event(WorldEvent::LocalPadCompleted(pad))] if pad.position == following)
        );
        let duplicate = testing::run(|out| transfers.handle(&finish, &mut world, out));
        duplicate.result.unwrap();
        assert!(duplicate.sent.is_empty() && duplicate.unreliable.is_empty());
        assert!(matches!(
            duplicate
                .events
                .iter()
                .find(|event| matches!(event, ClientEvent::World(_))),
            Some(ClientEvent::World(WorldEvent::LocalPadRefused {
                session_id: 5,
                revision: 1,
                ..
            }))
        ));
        assert_eq!(world.body.position(), Some(following));
        let due = testing::run(|out| {
            world
                .body
                .heartbeat(start + Duration::from_millis(1400), out)
        });
        due.result.unwrap();
        assert_eq!(due.sent, []);
        let [packet] = due.unreliable.as_slice() else {
            panic!("expected one landing report");
        };
        assert_eq!(packet.opcode, POSITION_OPCODE);
        assert_eq!(&packet.body[..4], &[7, 0, 2, 0]);
        assert_eq!(&packet.body[24..28], &following.x.to_le_bytes());
        assert_eq!(&packet.body[28..32], &following.z.to_le_bytes());
    }

    #[test]
    fn explicit_offers_to_another_zone_or_instance_still_start_a_transfer() {
        for (zone_id, instance_id) in [(3, 0), (2, 1)] {
            let mut world = World::new(5);
            world.zone = (2, 0);
            world.player.admit(testing::player(7));
            world.admitted = Some(Instant::now());
            let mut transfers = Transfers::new("Tester", Home::Offered);
            let offer = zoning::ZoneOffer {
                zone_id,
                instance_id,
                position: Position {
                    x: 30.0,
                    y: 42.0,
                    z: 3.0,
                    heading: 6.0,
                },
                reason: 1,
                to_bind: false,
                solicited: true,
            };
            let answer = offer.response("Tester").unwrap();
            let result = testing::run(|out| {
                transfers.observe(&Message::ZoneOffer(offer.clone()), &mut world, out)
            });
            result.result.unwrap();
            assert_eq!(result.sent, [answer]);
            assert_eq!(world.lifecycle.pending(), Some(&offer));
            assert!(!result.events.iter().any(|event| matches!(
                event,
                ClientEvent::World(WorldEvent::SameZoneRelocated(_))
            )));
        }
    }

    #[test]
    fn quarm_same_zone_bind_alias_is_narrow_and_duplicate_approval_is_quiet() {
        use super::super::wire::EqMac;
        for invalid in 0..10 {
            let mut world = World::new(5);
            world.zone = (2, 0);
            world.player.admit(testing::player(7));
            world.admitted = Some(Instant::now());
            let mut transfers = Transfers::new("Tester", Home::Asked);
            if invalid != 1 {
                transfers = transfers.with_same_zone_bind_reply(185);
            }
            let mut offer = zoning::BindPoint {
                zone_id: 2,
                position: Position::default(),
            }
            .offer();
            if invalid == 2 {
                offer.to_bind = false;
            }
            if invalid == 3 {
                offer.zone_id = 4;
            }
            if invalid == 6 {
                offer.solicited = false;
            }
            world.died();
            if invalid != 8 {
                world.transfer_offered(offer, Instant::now()).unwrap();
            }
            let answer = zoning::ZoneAnswer {
                character: if invalid == 4 {
                    "SomeoneElse"
                } else {
                    "Tester"
                }
                .into(),
                zone_id: if invalid == 7 { 186 } else { 185 },
                instance_id: u16::from(invalid == 5),
                position: None,
                success: if invalid == 9 { -1 } else { 1 },
            };
            let output =
                testing::run_on(&EqMac, |out| transfers.answered(&answer, &mut world, out));
            if invalid == 9 {
                output.result.unwrap();
                assert_eq!(output.sent, Vec::new());
                assert!(!world.lifecycle.departing());
                assert!(world.lifecycle.is_dead());
                continue;
            }
            if invalid != 0 {
                assert!(output.result.is_err());
                assert_eq!(output.sent, Vec::new());
                assert!(!world.lifecycle.departing());
                continue;
            }
            output.result.unwrap();
            assert!(world.lifecycle.departing());
            assert!(world.lifecycle.is_dead());
            assert_eq!(
                output.sent.len(),
                2,
                "save and departure follow validated approval"
            );
            let duplicate = testing::run_on(&EqMac, |out| {
                transfers.observe(&Message::ZoneAnswer(answer), &mut world, out)
            });
            duplicate.result.unwrap();
            assert_eq!(duplicate.sent, Vec::new());
        }
    }

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
            swim_backward_units_per_second: None,
            swim: None,
            diagonal: DiagonalRates::default(),
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
    fn a_zone_change_flushes_the_movement_history_first_unless_validated() {
        use eq_network_game::movement::{history, TITANIUM_UPDATE_COUNTER};
        for validated in [false, true] {
            let mut transfers = Transfers::new("Tester", Home::Offered);
            let mut world = World::new(5);
            world.validated = validated;
            let here = Position {
                x: 10.0,
                y: 20.0,
                z: 3.0,
                heading: 0.0,
            };
            // Admitted a moment ago: the spawn's first frame is queued, far
            // younger than a report waits for.
            let admitted = Instant::now()
                .checked_sub(Duration::from_millis(200))
                .unwrap();
            world.body.admit(
                MotionSession::new(5, 7, here, admitted)
                    .unwrap()
                    .with_counter(TITANIUM_UPDATE_COUNTER)
                    .with_history(true),
            );
            world.zone = (2, 0);
            world.admitted = Some(admitted);
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
            let opcodes: Vec<u16> = outcome.sent.iter().map(|packet| packet.opcode).collect();
            if validated {
                assert_eq!(opcodes, [zoning::CHANGE_OPCODE]);
            } else {
                assert_eq!(opcodes, [history::OPCODE, zoning::CHANGE_OPCODE]);
                let report = &outcome.sent[0].body;
                assert_eq!(report.len(), 17 + 1);
                let record = history::Record::decode(&report[..17]).unwrap();
                assert_eq!(
                    (record.cause, record.sequence),
                    (history::Cause::Collision, 0)
                );
            }
        }
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
            [
                zoning::titanium_save_on_zone(&zoning::Save {
                    position: Some(Position {
                        x: 10.0,
                        y: 20.0,
                        z: 3.0,
                        heading: 0.0,
                    }),
                    vehicle: None,
                }),
                zoning::titanium_depart(7)
            ]
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
        let zoning::BoundaryRoute::Transfer(offer) =
            transfers.zone_line(&cross(5, here, now), &world).unwrap()
        else {
            panic!("a boundary to another zone must start a transfer");
        };
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

    /// A transfer the zone accepted, going to `position` for `reason`.
    fn going(reason: u32, position: Position) -> zoning::ZoneOffer {
        zoning::ZoneOffer {
            zone_id: 4,
            instance_id: 0,
            position,
            reason,
            to_bind: false,
            solicited: false,
        }
    }

    #[test]
    fn an_ordinary_save_holds_where_the_request_goes() {
        let there = Position {
            x: 10.0,
            y: 20.0,
            z: 3.0,
            heading: 64.0,
        };
        let mut world = World::new(5);
        world.body.admit(
            MotionSession::new(5, 7, Position { x: 99.0, ..there }, Instant::now()).unwrap(),
        );
        assert_eq!(
            save_on_leaving(&going(0, there), &mut world),
            Some(zoning::Save {
                position: Some(there),
                vehicle: None,
            })
        );
        // Carried on from the arrival wait, still no save.
        world.carried_from = super::super::riders::CarriedFrom::Wait;
        assert_eq!(save_on_leaving(&going(0, there), &mut world), None);
    }

    #[test]
    fn a_zone_point_that_keeps_the_heading_saves_the_players_cut_to_a_whole_number() {
        let point = zoning::ZonePoint {
            number: 1,
            zone_id: 4,
            instance_id: 0,
            destination: Position {
                x: 10.0,
                y: 20.0,
                z: 3.0,
                heading: 999.0,
            },
        };
        let standing = Position {
            x: -5.0,
            y: 6.0,
            z: 1.0,
            heading: 479.5,
        };
        let offer = point.request(standing).unwrap();
        let mut world = World::new(5);
        let save = save_on_leaving(&offer, &mut world).unwrap();
        assert_eq!(save.position.map(|position| position.heading), Some(479.5));
        // As recorded: the save holds 479, not 480.
        let body = zoning::titanium_save_on_zone(&save).body;
        assert_eq!(&body[17..21], &479u32.to_le_bytes());
        assert_eq!(&body[5..9], &20.0f32.to_le_bytes());
    }

    #[test]
    fn only_a_boat_transfer_saves_the_place_aboard_and_the_boat() {
        use eq_network_game::{movement::carrier_world, world::SpawnKind, GameDialect};
        let boat_at = Position {
            x: 100.0,
            y: 50.0,
            z: 0.0,
            heading: 128.0,
        };
        let deck = Position {
            x: 4.0,
            y: -2.0,
            z: 3.0,
            heading: 10.0,
        };
        let aboard = || {
            let mut world = World::new(5);
            let mut player = testing::player(7);
            player.position = carrier_world(boat_at, deck);
            world
                .body
                .admit(MotionSession::new(5, 7, player.position, Instant::now()).unwrap());
            world.player.admit(player);
            let mut boat = testing::spawn(8, SpawnKind::Npc);
            boat.race = 72;
            boat.name = "SirensBane02".into();
            boat.position = boat_at;
            world.spawns.insert(boat.clone());
            world.carriers.observe(
                &WorldEvent::Spawns(vec![boat]),
                GameDialect::Titanium,
                Instant::now(),
            );
            world.vehicle = Some("SirensBane00".into());
            world
        };
        let there = Position {
            x: -40.0,
            ..Position::default()
        };
        let mut world = aboard();
        let save = save_on_leaving(&going(riders::ZONE_REASON, there), &mut world).unwrap();
        assert_eq!(save.vehicle.as_deref(), Some("SirensBane00"));
        let local = save.position.unwrap();
        // The restore in the next zone keeps this z (read R36c).
        assert_eq!(world.riding.saved_z(), Some(local.z));
        for (got, want) in [
            (local.x, deck.x),
            (local.y, deck.y),
            (local.z, deck.z),
            (local.heading, deck.heading),
        ] {
            assert!((got - want).abs() < 0.01, "{local:?}");
        }
        // Any other transfer forgets the boat and saves where it goes.
        let ordinary = Some(zoning::Save {
            position: Some(there),
            vehicle: None,
        });
        let mut world = aboard();
        assert_eq!(save_on_leaving(&going(0, there), &mut world), ordinary);
        assert_eq!(world.vehicle, None);
        // The save's z, the destination's, is the record's third field now.
        assert_eq!(world.riding.saved_z(), Some(there.z));
        // So does a boat transfer once the boat is no longer found.
        let mut world = aboard();
        world.vehicle = Some("Stormbreaker".into());
        assert_eq!(
            save_on_leaving(&going(riders::ZONE_REASON, there), &mut world),
            ordinary
        );
        assert_eq!(world.vehicle, None);
        // Carried on from the arrival wait: no save, the name kept, and
        // the last save's z unchanged.
        let mut world = aboard();
        world.vehicle = Some("Stormbreaker".into());
        world.riding.saved(6.0);
        world.carried_from = super::super::riders::CarriedFrom::Wait;
        assert_eq!(
            save_on_leaving(&going(riders::ZONE_REASON, there), &mut world),
            None
        );
        assert_eq!(world.vehicle.as_deref(), Some("Stormbreaker"));
        assert_eq!(world.riding.saved_z(), Some(6.0));
        assert_eq!(
            world.carried_from,
            super::super::riders::CarriedFrom::Elsewhere
        );
    }

    #[test]
    fn a_server_zone_command_ends_a_held_arrival_and_forgets_the_boat() {
        let mut world = World::new(5);
        world.zone = (2, 0);
        world.player.admit(testing::player(7));
        world.admitted = Some(Instant::now());
        world
            .body
            .admit(MotionSession::new(5, 7, Position::default(), Instant::now()).unwrap());
        world.vehicle = Some("SirensBane00".into());
        world.arrival = Some(super::super::riders::Pending::Waiting {
            until: Instant::now() + Duration::from_secs(60),
        });
        world.carried_from = super::super::riders::CarriedFrom::Wait;
        let mut transfers = Transfers::new("Tester", Home::Offered);
        // The server's move within the zone, as a ferry script's would be.
        let offer = zoning::ZoneOffer {
            zone_id: 2,
            instance_id: 0,
            position: Position {
                x: 30.0,
                y: 42.0,
                z: 3.0,
                heading: 6.0,
            },
            reason: 0,
            to_bind: false,
            solicited: false,
        };
        let outcome = testing::run(|out| {
            transfers.observe(&Message::ZoneOffer(offer.clone()), &mut world, out)
        });
        outcome.result.unwrap();
        assert!(world.arrival.is_none() && world.vehicle.is_none());
        assert!(!world.motion_held());
        assert_eq!(
            world.carried_from,
            super::super::riders::CarriedFrom::Elsewhere
        );
    }
}
