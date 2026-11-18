//! Titanium's riding wire, as the program speaks it (reads R36, R36b):
//!
//! - Once a second it names the boat underfoot: the first linked tick sends
//!   `OP_BoardBoat` with the boat's raw name, two unlinked ticks in a row
//!   forget the name and send `OP_LeaveBoat` (0x422198). Whether the last
//!   word was a boarding is kept across zones, as the program's global is.
//! - The server's 0x0b71 naming the saved vehicle zones the player with the
//!   boat, with reason 2 (0x45c277). The save before leaving then carries
//!   the player's place in the boat's frame and its name (0x4976bf, in
//!   [`super::transfers`]).
//! - At arrival the received profile names the vehicle. When a spawn of
//!   that name was already in at the profile, the host stands the player
//!   on it and the session checks the placement against the profile's
//!   position in the boat's frame. Otherwise the session sends 0x3710 and
//!   holds the player until the server's 0x0b71 carries them on, or 60 s
//!   pass and they go to the zone's safe point (0x4aba14, 0x4a8afa,
//!   0x43d7c5).
//!
//! No rule here tests a race or a server: every carrier the floor sweep
//! links counts (R36 E), and the wire is the Titanium program's.
use super::{
    feature::{Feature, Out, World},
    transfers::Transfers,
    ClientCommand, ClientEvent,
};
use anyhow::{Context, Result};
use eq_network_game::{
    boats::riders::{self, Arrival},
    message::Message,
    movement::{carrier_world, CarrierSample},
    request::Request,
    world::{Capability, Position, WorldEvent},
    zoning::ZoneOffer,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

/// The character tick that names the boat underfoot (0x422198).
const TICK: Duration = Duration::from_secs(1);
/// Unlinked ticks in a row before the boat is forgotten.
const UNLINKED_TICKS: u8 = 2;
/// How long the program waits for the server to carry the player on
/// (0x4a8afa); the session holds a placement aboard no longer.
const WAIT: Duration = Duration::from_secs(60);
/// The program's second try, in the boat's frame (0x4aba14).
const RETRY: [f32; 3] = [0.0, 0.0, 25.0];
/// The program sweeps for the floor from one unit above the restored point.
const SWEEP_LIFT: f32 = 1.0;
/// How far below the restored point the host's floor sweep may land
/// (inferred, recording checklist 6.478): the program's reach is unread.
const SWEEP_DROP: f32 = 30.0;
/// How far across the placement may differ from the restored point, and its
/// heading from the restored heading: float rounding only.
const SAME_PLACE: f32 = 0.05;
const SAME_HEADING: f32 = 0.5;

/// Where the program restores a rider in the boat's frame from a received
/// profile (0x4ba21e-0x4ba25d; read R36c). Its loader stores the profile's
/// three coordinates into the record's first, second and first fields
/// again, and the heading into the fourth, so the first field keeps the
/// profile's third coordinate and the third field keeps the departure's z
/// (zero when this client saved none). Which of this client's axes the
/// record's first two fields are follows the zone request's order
/// (inferred, 6.478).
fn restored(profile: Position, departure_z: Option<f32>) -> Position {
    Position {
        x: profile.y,
        y: profile.z,
        z: departure_z.unwrap_or(0.0),
        heading: profile.heading,
    }
}

/// An arrival aboard that holds the player's movement.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Pending {
    /// The vehicle is here; the host's placement on it is awaited.
    Aboard {
        /// The report the name found.
        sample: CarrierSample,
        /// The profile's position, in the boat's frame.
        local: Position,
        /// When the session stops waiting for the host and uses the safe
        /// point. The program places at once; this bounds a host that
        /// never answers (inferred, 6.478).
        until: Instant,
    },
    /// The vehicle is missing; the server's transfer is awaited.
    Waiting {
        /// When the program gives up and uses the safe point.
        until: Instant,
    },
}

/// Where a transfer under way began, for the save before leaving.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum CarriedFrom {
    /// Anywhere but the arrival wait: the save is made.
    #[default]
    Elsewhere,
    /// The arrival wait: the program sends no save and keeps the vehicle
    /// (0x4976bf skips while [0x924676] is set; read R36 §2).
    Wait,
}

/// What the riding wire keeps across a character's zones, as the
/// program's globals do.
#[derive(Debug, Default)]
pub(in crate::client) struct Riding {
    /// Whether the last boat word sent was `OP_BoardBoat`.
    aboard: AtomicBool,
    /// The boat-frame z the last boat save carried. The program's profile
    /// loader never writes the record's third coordinate (0x4ba21e; read
    /// R36c), so its restore keeps the one the departure wrote.
    departure_z: Mutex<Option<f32>>,
}

impl Riding {
    /// Notes the boat-frame z a boat save carried, or forgets it.
    pub(super) fn depart_at(&self, z: Option<f32>) {
        if let Ok(mut departure) = self.departure_z.lock() {
            *departure = z;
        }
    }

    /// The boat-frame z the last boat save carried, if it is kept.
    pub(super) fn departure_z(&self) -> Option<f32> {
        self.departure_z
            .lock()
            .ok()
            .and_then(|departure| *departure)
    }
}

/// The riding wire's state in one zone session.
#[derive(Default)]
pub(super) struct Riders {
    /// When the next character tick runs.
    next_tick: Option<Instant>,
    /// Unlinked ticks in a row.
    unlinked: u8,
    /// The vehicle the profile named, until the admission.
    profiled: Option<String>,
    /// The spawn the profile's vehicle found among the spawns in at the
    /// profile, as the program's one lookup does (R36b §6).
    found: Option<u16>,
}

impl Riders {
    fn publish(arrival: Arrival, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        out.log.send(ClientEvent::World(WorldEvent::Arrival {
            session_id: world.session_id,
            arrival,
        }))
    }

    /// Stands the player where the session decided and tells the host.
    fn place(position: Position, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        world.correct_own(position, Instant::now())?;
        out.log.send(ClientEvent::World(WorldEvent::Position {
            spawn_id: world.player.as_ref().map_or(0, |player| player.spawn_id),
            position,
            velocity: [0.0; 3],
            forward: 0.0,
        }))
    }

    /// The program's safe point: the vehicle forgotten, the player at the
    /// zone's safe point, nothing sent (0x43d7c5).
    pub(super) fn safe_point(world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        world.arrival = None;
        world.forget_vehicle();
        let position = world
            .safe_point
            .or_else(|| world.body.position())
            .context("arrival without a safe point or a position")?;
        Self::place(position, world, out)?;
        Self::publish(Arrival::SafePoint { position }, world, out)
    }

    /// Whether `position` is where the host's floor sweep could stand the
    /// player from the restored point `from`: the same place across and
    /// heading, and at most one above or [`SWEEP_DROP`] below it.
    fn swept_from(from: Position, position: Position) -> bool {
        let turn = (position.heading - from.heading).rem_euclid(512.0);
        (position.x - from.x).abs() <= SAME_PLACE
            && (position.y - from.y).abs() <= SAME_PLACE
            && position.z <= from.z + SWEEP_LIFT
            && position.z >= from.z - SWEEP_DROP
            && turn.min(512.0 - turn) <= SAME_HEADING
    }

    fn placed(
        position: Option<Position>,
        created: Instant,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let Some(Pending::Aboard { sample, local, .. }) = world.arrival.clone() else {
            return out
                .log
                .diagnostic("Refused an arrival placement: no boat is awaiting one".into());
        };
        let Some(position) = position else {
            // No floor or no link at either try: the safe point.
            return Self::safe_point(world, out);
        };
        let restored = world.carriers.pose(sample, created).map(|pose| {
            let retry = Position {
                x: RETRY[0],
                y: RETRY[1],
                z: RETRY[2],
                heading: local.heading,
            };
            (carrier_world(pose, local), carrier_world(pose, retry))
        });
        if restored.is_some_and(|(first, retry)| {
            Self::swept_from(first, position) || Self::swept_from(retry, position)
        }) {
            world.arrival = None;
            Self::place(position, world, out)?;
            return Self::publish(Arrival::Placed { position }, world, out);
        }
        out.log.diagnostic(
            "Refused an arrival placement away from the boat's restored point; \
             using the safe point"
                .into(),
        )?;
        Self::safe_point(world, out)
    }

    /// At admission: the profile's vehicle found, or asked for.
    fn arrive(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        world.vehicle = self.profiled.take();
        let Some(name) = world.vehicle.clone() else {
            return Ok(());
        };
        let profile = world
            .body
            .position()
            .context("arrival aboard without a profile position")?;
        let local = restored(profile, world.riding.departure_z());
        let found = self
            .found
            .take()
            .and_then(|id| world.carriers.latest(id))
            .map(eq_network_game::movement::CarrierReport::sample);
        if let Some(sample) = found {
            world.arrival = Some(Pending::Aboard {
                sample,
                local,
                until: Instant::now() + WAIT,
            });
            return Self::publish(
                Arrival::Aboard {
                    name,
                    sample,
                    profile,
                    local,
                },
                world,
                out,
            );
        }
        out.request(&Request::VehicleMissing(name.clone()))?;
        world.arrival = Some(Pending::Waiting {
            until: Instant::now() + WAIT,
        });
        Self::publish(Arrival::Waiting { name }, world, out)
    }

    /// The server's word that a boat carries its riders on.
    fn carried(
        transfer: &riders::Transfer,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let names = world
            .vehicle
            .as_deref()
            .is_some_and(|saved| transfer.names(saved));
        if !names || !world.ready() || world.lifecycle.blocks_motion() {
            // Another boat, or no boat saved: the program does nothing.
            return Ok(());
        }
        let Ok(zone_id) = u16::try_from(transfer.zone_id) else {
            return out.log.diagnostic(format!(
                "Ignored a vehicle transfer to zone {}, which no zone id can name",
                transfer.zone_id
            ));
        };
        let from_wait = matches!(world.arrival, Some(Pending::Waiting { .. }));
        // The program passes the profile's last saved position; this
        // session passes the accepted one (inferred, 6.478). The save after
        // the approval carries the place in the boat's frame.
        let position = world
            .body
            .position()
            .context("vehicle transfer without a position")?;
        world.arrival = None;
        Transfers::start(
            ZoneOffer {
                zone_id,
                instance_id: 0,
                position,
                reason: riders::ZONE_REASON,
                to_bind: false,
                solicited: true,
            },
            world,
            out,
        )?;
        // Carried on from the wait, the program sends no save at the
        // approval and keeps the name (0x4976bf; R36 §2).
        if from_wait {
            world.carried_from = CarriedFrom::Wait;
        }
        Ok(())
    }

    /// The character tick: name the boat underfoot, or forget it.
    fn name_the_boat(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let boat = world
            .body
            .carrier_support()
            .and_then(|sample| world.spawns.get(sample.spawn_id))
            .map(|spawn| spawn.name.clone())
            .filter(|name| !name.is_empty());
        if let Some(name) = boat {
            self.unlinked = 0;
            world.vehicle = Some(name.clone());
            if !world.riding.aboard.swap(true, Ordering::Relaxed) {
                out.request(&Request::BoardBoat(name))?;
            }
            return Ok(());
        }
        self.unlinked = self.unlinked.saturating_add(1);
        if self.unlinked >= UNLINKED_TICKS {
            world.forget_vehicle();
            if world.riding.aboard.swap(false, Ordering::Relaxed) {
                out.request(&Request::LeaveBoat)?;
            }
        }
        Ok(())
    }
}

impl Feature for Riders {
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::BoatRiding]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::ArrivalPlacement { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::ArrivalPlacement {
            position, created, ..
        } = command
        else {
            return Ok(());
        };
        Self::placed(*position, *created, world, out)
    }

    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        if let Message::VehicleName(name) = message {
            self.found = name.as_deref().and_then(|name| {
                riders::find(
                    name,
                    world
                        .spawns
                        .all()
                        .map(|spawn| (spawn.spawn_id, spawn.name.as_str())),
                )
            });
            self.profiled.clone_from(name);
        }
        Ok(())
    }

    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        self.next_tick = Some(Instant::now() + TICK);
        self.arrive(world, out)
    }

    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if matches!(
            world.arrival,
            Some(Pending::Waiting { until } | Pending::Aboard { until, .. }) if now >= until
        ) {
            return Self::safe_point(world, out);
        }
        // No frames run while the arrival is held, nor in a transfer.
        if world.arrival.is_some() || !world.ready() || world.lifecycle.blocks_motion() {
            return Ok(());
        }
        match self.next_tick {
            Some(next) if now < next => return Ok(()),
            _ => self.next_tick = Some(now + TICK),
        }
        self.name_the_boat(world, out)
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match message {
            Message::VehicleTransfer(transfer) => Self::carried(transfer, world, out),
            Message::Event(WorldEvent::Death(death)) if world.is_player(death.spawn_id) => {
                // Death forgets the boat and any arrival (0x4564ba).
                world.arrival = None;
                world.forget_vehicle();
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
