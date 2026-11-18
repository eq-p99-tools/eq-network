//! `EQEmu`'s echoed vessel acquire/release contract, separate from steering.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{ensure, Result};
use eq_network_game::{
    boats::{Control, ControlState, Intent, Request as BoatRequest, Vessel},
    message::Message,
    request::Request,
    world::{Capability, SpawnKind, WorldEvent},
};
use std::time::{Duration, Instant};

/// Local unresolved-request deadline, not a measured server response time.
const WAIT: Duration = Duration::from_secs(5);

/// One admission's control owner. Numeric wire IDs are never request nonces.
#[derive(Default)]
pub(super) struct Boats {
    control: Option<Control>,
}

impl Boats {
    fn control(&mut self, session_id: u64) -> &mut Control {
        self.control.get_or_insert_with(|| Control::new(session_id))
    }

    fn publish(&self, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(control) = &self.control {
            out.log
                .send(ClientEvent::World(WorldEvent::BoatControlState {
                    session_id: world.session_id,
                    state: control.state(),
                }))?;
        }
        Ok(())
    }

    /// Sends only the source-backed control request. In particular, acquiring
    /// never sends a position under the boat's spawn ID.
    fn send(
        &mut self,
        vessel: Vessel,
        intent: Intent,
        now: Instant,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if world
            .carriers
            .latest(vessel.spawn_id)
            .is_none_or(|report| report.sample().incarnation != vessel.incarnation)
        {
            self.control(world.session_id).retired(vessel);
            self.publish(world, out)?;
            return out
                .log
                .diagnostic("Boat incarnation ended before the control request".into());
        }
        // Release intent withdraws local authority before attempting transport.
        if intent == Intent::Release {
            self.control(world.session_id).cancel();
        }
        out.request(&Request::ControlBoat {
            spawn_id: vessel.spawn_id,
            intent,
        })?;
        self.control(world.session_id)
            .submitted(world.session_id, vessel, intent, now, now + WAIT);
        self.publish(world, out)
    }

    fn acquire(
        &mut self,
        sample: eq_network_game::movement::CarrierSample,
        position: eq_network_game::world::Position,
        created: Instant,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let check = || -> Result<Vessel> {
            ensure!(
                self.control.as_ref().is_none_or(|control| !matches!(
                    control.state(),
                    ControlState::Acquiring { .. }
                        | ControlState::Acquired(_)
                        | ControlState::Releasing(_)
                )),
                "a boat control request is already active"
            );
            let report = world
                .carriers
                .latest(sample.spawn_id)
                .ok_or_else(|| anyhow::anyhow!("the supporting boat is unavailable"))?;
            ensure!(
                report.sample().incarnation == sample.incarnation
                    && world.carriers.pose(sample, created).is_some(),
                "the supporting boat report is stale"
            );
            ensure!(
                world
                    .body
                    .carrier_support()
                    .is_some_and(|accepted| accepted.spawn_id == sample.spawn_id
                        && accepted.incarnation == sample.incarnation),
                "the boat is not the rider's accepted support"
            );
            let spawn = world
                .spawns
                .get(sample.spawn_id)
                .ok_or_else(|| anyhow::anyhow!("the supporting spawn is unavailable"))?;
            // Native Titanium's interaction path selects NPC race 141.
            // EQEmu also accepts 502, which this client cannot floor-link yet.
            ensure!(
                spawn.kind == SpawnKind::Npc && spawn.race == 141,
                "this vessel's Titanium control path is not supported"
            );
            ensure!(
                world.body.position() == Some(position),
                "the rider position changed before boat acquisition"
            );
            let vessel = Vessel {
                spawn_id: sample.spawn_id,
                incarnation: sample.incarnation,
            };
            ensure!(
                self.control
                    .as_ref()
                    .is_none_or(|control| control.may_acquire(vessel)),
                "the earlier boat acquire is unresolved; release and observe its echo before retrying"
            );
            Ok(vessel)
        };
        match check() {
            Ok(vessel) => self.send(vessel, Intent::Acquire, Instant::now(), world, out),
            Err(error) => out
                .log
                .diagnostic(format!("Rejected boat acquisition: {error}")),
        }
    }

    fn release(&mut self, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(vessel) = self.control.as_ref().and_then(Control::vessel) {
            self.send(vessel, Intent::Release, Instant::now(), world, out)
        } else {
            out.log
                .diagnostic("No boat control request is selected".into())
        }
    }
}

impl Feature for Boats {
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::BoatControl]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::BoatControl { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::BoatControl {
            request, created, ..
        } = command
        else {
            return Ok(());
        };
        match *request {
            BoatRequest::Acquire { support, position } => {
                self.acquire(support, position, *created, world, out)
            }
            BoatRequest::Release => self.release(world, out),
            BoatRequest::Support(sample) => {
                let support = sample.and_then(|sample| {
                    world
                        .carriers
                        .latest(sample.spawn_id)
                        .filter(|report| {
                            report.sample().incarnation == sample.incarnation
                                && world.carriers.pose(sample, *created).is_some()
                        })
                        .map(|_| Vessel {
                            spawn_id: sample.spawn_id,
                            incarnation: sample.incarnation,
                        })
                });
                let release = self
                    .control
                    .as_mut()
                    .and_then(|control| control.support_changed(support));
                if let Some(vessel) = release {
                    self.send(vessel, Intent::Release, Instant::now(), world, out)?;
                }
                Ok(())
            }
        }
    }

    fn notice(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if matches!(
            command,
            ClientCommand::Camp { .. }
                | ClientCommand::ExitAtOnce { .. }
                | ClientCommand::CrossZoneLine { .. }
        ) && self.control.as_ref().is_some_and(Control::needs_release)
        {
            self.release(world, out)?;
        }
        Ok(())
    }

    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let Some(control) = self.control.as_mut() else {
            return Ok(());
        };
        let before = control.state();
        control.expire(now);
        if before != control.state() {
            // A timed-out acquisition might still own the server boat.
            // Release it; a late acquire echo cannot revive this intent.
            if matches!(before, ControlState::Acquiring { .. }) {
                return self.release(world, out);
            }
            self.publish(world, out)?;
        }
        let lost_support = self
            .control
            .as_ref()
            .filter(|control| control.needs_release())
            .and_then(Control::vessel)
            .is_some_and(|vessel| {
                world.body.carrier_support().is_none_or(|sample| {
                    sample.spawn_id != vessel.spawn_id || sample.incarnation != vessel.incarnation
                })
            });
        if (world.lifecycle.blocks_motion() || lost_support)
            && self.control.as_ref().is_some_and(Control::needs_release)
        {
            self.release(world, out)?;
        }
        Ok(())
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let Some(control) = self.control.as_mut() else {
            return Ok(());
        };
        let before = control.state();
        match message {
            Message::ZoneOffer(_) if control.needs_release() => {
                return self.release(world, out);
            }
            Message::Event(WorldEvent::Death(death))
                if world.is_player(death.spawn_id) && control.needs_release() =>
            {
                return self.release(world, out);
            }
            Message::Event(WorldEvent::BoatControl(packet)) => {
                control.received(world.session_id, *packet, Instant::now());
            }
            Message::Event(WorldEvent::Spawns(spawns)) => {
                if let Some(vessel) = control
                    .vessel()
                    .filter(|vessel| spawns.iter().any(|spawn| spawn.spawn_id == vessel.spawn_id))
                {
                    control.retired(vessel);
                }
            }
            Message::Event(WorldEvent::Despawn(id)) => {
                if let Some(vessel) = control.vessel().filter(|vessel| vessel.spawn_id == *id) {
                    control.retired(vessel);
                }
            }
            _ => (),
        }
        if matches!(before, ControlState::Acquiring { .. })
            && matches!(control.state(), ControlState::Unconfirmed(_))
        {
            // Receive can expire the request before the next timer tick.
            // It must perform the same cleanup without accepting the late grant.
            return self.release(world, out);
        }
        if control.state() != before {
            self.publish(world, out)?;
        }
        Ok(())
    }

    fn connection_ended(&mut self, _world: &mut World) {
        if let Some(control) = self.control.as_mut() {
            control.cancel();
        }
    }
}

#[cfg(test)]
mod tests;
