//! Damage the world does to the player. The client works it out, as the
//! official client does (a fall's after the player's Safe Fall skill, which
//! neither server applies; inferred), and reports it; the server takes the
//! amount as it is, lowers it by its own reductions (`EQEmu`'s fall damage
//! reductions from spells, items and AAs; TAKP's Acrobatics AA), and kills
//! a player it brings to nothing (`Client::Handle_OP_EnvDamage` in
//! `EQEmu`, `Handle_OP_Damage` in `EQMacEmu`). The session reports only
//! what the host tells it, and only the hazards its server type takes from
//! the client.
use super::{
    actions,
    bleeding::ItemCount,
    feature::{Feature, Out, World},
    posture, ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    hazards::{self, Hazard, LiquidHazard},
    message::Message,
    request::Request,
    world::{Capability, Position, PostureState, WorldEvent},
};
use std::time::{Duration, Instant};

/// Worker revision for the ordered stream of liquid calculation inputs.
#[derive(Default)]
pub(in crate::client) struct Observations {
    current: Option<(u64, u16, u64)>,
}

impl Observations {
    /// Advances before delivery, returning a stamp to deliver after the input.
    /// This covers both decoded packets and feature-produced inventory news.
    pub(in crate::client) fn observe(&mut self, event: &ClientEvent) -> Option<WorldEvent> {
        let ClientEvent::World(event) = event else {
            return None;
        };
        if let WorldEvent::Entered {
            capabilities,
            session_id,
            player,
            ..
        } = event
        {
            self.current = capabilities
                .iter()
                .any(|capability| matches!(capability, Capability::Drowning | Capability::LavaHeat))
                .then_some((*session_id, player.spawn_id, 0));
            return self.current.map(|(session_id, _, revision)| {
                WorldEvent::LiquidHazardRevision {
                    session_id,
                    revision,
                }
            });
        }
        let (session_id, own, revision) = self.current.as_mut()?;
        let changes_inputs = match event {
            WorldEvent::Position { spawn_id, .. }
            | WorldEvent::HitPoints { spawn_id, .. }
            | WorldEvent::Appearance { spawn_id, .. }
            | WorldEvent::Posture { spawn_id, .. } => *spawn_id == *own,
            WorldEvent::Spawns(spawns) => spawns.iter().any(|spawn| spawn.spawn_id == *own),
            WorldEvent::Buff(buff) => u32::from(*own) == buff.entity_id,
            WorldEvent::SpellEffect(effect) => *own == effect.target_id,
            WorldEvent::Death(death) => death.spawn_id == u32::from(*own),
            WorldEvent::AlternateAdvancement {
                session_id: visit,
                updates,
            } => {
                *visit == *session_id
                    && updates
                        .iter()
                        .any(eq_network_game::alternate_advancement::Update::changes_effects)
            }
            WorldEvent::DeathBind { session_id: visit }
            | WorldEvent::InventoryAction {
                session_id: visit,
                update: Some(_),
                error: None,
                ..
            } => *visit == *session_id,
            WorldEvent::BuffSnapshot(_)
            | WorldEvent::Inventory(_)
            | WorldEvent::Level { .. }
            | WorldEvent::SpellTable(_)
            | WorldEvent::ZoneDescription(_)
            | WorldEvent::ZoneTransfer(_)
            | WorldEvent::ZoneTransferRejected { .. } => true,
            _ => false,
        };
        if !changes_inputs {
            return None;
        }
        *revision = revision.wrapping_add(1);
        Some(WorldEvent::LiquidHazardRevision {
            session_id: *session_id,
            revision: *revision,
        })
    }

    fn matches(&self, session_id: u64, spawn_id: u16, revision: u64) -> bool {
        self.current == Some((session_id, spawn_id, revision))
    }
}

fn same_finite_xyz(current: [f32; 3], observed: Position) -> bool {
    current.into_iter().all(f32::is_finite)
        && current.map(f32::to_bits) == [observed.x, observed.y, observed.z].map(f32::to_bits)
}

/// The HP below which the Titanium program takes the death path instead of
/// sending its damage record: -10, less effect 328's total, which the
/// session doesn't resolve (read R49-3 d, e; inferred, 6.491b).
const DEATH_FLOOR: i64 = -10;

/// The races whose spawn the Titanium program flags as a carrier (spawn
/// +0x1b6, set at 0x47a741), which takes no environmental damage (read
/// R49-3 d).
const CARRIER_RACES: [u32; 5] = [0x8d, 0x48, 0x49, 0x72, 0x194];

/// Reports the world's damage to the server.
pub(super) struct Hazards {
    /// The hazards the server type takes from the client.
    taken: &'static [Hazard],
    /// Whether the client generation sends a notice before each report, as
    /// the `EQMac` program does. A generation without one is Titanium's,
    /// which sends its damage record after each liquid report instead.
    notice: bool,
    last_liquid_sample: [Option<Instant>; 2],
    /// The server's last word on the player's HP, which on both
    /// generations leaves out what the player's items add.
    hit_points: Option<i32>,
    /// How the server type counts what the player's items add to their HP,
    /// where the session can: none yet on Titanium (see [`Self::records`]).
    items: Option<ItemCount>,
}

impl Hazards {
    /// Reports these hazards, each after a notice where `notice` says the
    /// generation sends one, and refuses the rest.
    pub(super) fn new(taken: &'static [Hazard], notice: bool) -> Self {
        Self {
            taken,
            notice,
            last_liquid_sample: [None; 2],
            hit_points: None,
            items: None,
        }
    }

    /// Counts what the player's items add to their HP by this rule, so that
    /// the death path's skip can place the player against the floor.
    #[cfg_attr(not(test), allow(dead_code, reason = "no Titanium count yet"))]
    pub(super) const fn with_item_count(mut self, items: ItemCount) -> Self {
        self.items = Some(items);
        self
    }

    /// Keeps the server's last word on the player's HP.
    fn note(&mut self, message: &Message, world: &World) {
        if let Message::Event(WorldEvent::HitPoints {
            spawn_id, current, ..
        }) = message
        {
            if world.is_player(*spawn_id) {
                self.hit_points = Some(*current);
            }
        }
    }

    /// Why the Titanium program would send nothing for a report now, of the
    /// skips in 0x41c905 the session can see (read R49-3 d; falls share it,
    /// R49-3a): the freeze stance, 102, and a carrier race. A linkdead
    /// player isn't playing, and the mode byte 4 is unread (6.491c). The
    /// `EQMac` program's skips are unread, so they apply to Titanium only.
    fn skipped(&self, world: &World) -> Option<&'static str> {
        if self.notice {
            return None;
        }
        if world.posture.current() == Some(PostureState::Frozen) {
            return Some("The program reports no damage in the freeze stance");
        }
        world
            .player
            .as_ref()
            .is_some_and(|player| CARRIER_RACES.contains(&player.race))
            .then_some("The program reports no damage for a carrier race")
    }

    /// Whether the Titanium damage record follows this report (R49-3 c,
    /// R49-3a): unless the player's HP, less the amount, is below
    /// [`DEATH_FLOOR`], where the program takes the death path and sends
    /// none (read R49-3 e 7). The session drops no HP of its own, so the
    /// server's last report, with what the items add, stands in for the
    /// program's local HP (inferred, 6.491b). That report leaves the items
    /// out, so where the session can't count them it can't place the player
    /// below the floor, and the record goes: `EQEmu` only relays it.
    fn records(&self, amount: u32, world: &World) -> bool {
        if self.notice {
            return false;
        }
        let known = self
            .hit_points
            .zip(self.items.and_then(|count| count(world).ok()))
            .map(|(reported, items)| i64::from(reported) + i64::from(items));
        known.is_none_or(|hp| hp - i64::from(amount) >= DEATH_FLOOR)
    }

    fn send_report(
        &self,
        hazard: Hazard,
        amount: u32,
        safe_fall: bool,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        // Both programs announce Safe Fall first, even when it left nothing.
        if safe_fall && hazard == Hazard::Falling {
            out.request(&Request::SafeFall)?;
            if amount == 0 {
                return out
                    .log
                    .diagnostic("Safe Fall took all of a fall's damage".into());
            }
        }
        let notice = if self.notice {
            Some(out.encode(&Request::EnvironmentalNotice { hazard, amount }))
        } else {
            None
        };
        let record = if self.records(amount, world) {
            Some(hazards::titanium_damage_record(
                out.sender.spawn_id.unwrap_or_default(),
                hazard,
                amount,
            ))
        } else {
            None
        };
        match (
            notice.transpose(),
            out.encode(&Request::EnvironmentalDamage { hazard, amount }),
            record.transpose(),
        ) {
            (Ok(notice), Ok(packet), Ok(record)) => {
                if let Some(notice) = notice {
                    out.send(&notice)?;
                }
                out.send(&packet)?;
                // The program's record goes after its report (R49-3 c).
                if let Some(record) = &record {
                    out.send(record)?;
                }
                // Only an accepted, submitted local report reaches the same
                // posture policy as received damage (read R31 / 6.475).
                // Submission establishes no received damage or HP authority.
                posture::stand_for_own_damage(amount, world, out)?;
                out.log
                    .send(ClientEvent::World(WorldEvent::HazardReported {
                        session_id: world.session_id,
                        hazard,
                        amount,
                        record: record.is_some(),
                    }))?;
                out.log
                    .diagnostic(format!("Reported {amount} points of {hazard:?} damage"))
            }
            // The host works the amount out; one the packet cannot carry is
            // not worth the session.
            (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                out.log.diagnostic(format!("Damage not reported: {error}"))
            }
        }
    }
}

impl Feature for Hazards {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        let mut capabilities = vec![Capability::EnvironmentalDamage];
        if self.taken.contains(&Hazard::Drowning) {
            capabilities.push(Capability::Drowning);
        }
        if self.taken.contains(&Hazard::Lava) {
            capabilities.push(Capability::LavaHeat);
        }
        capabilities
    }

    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        self.note(message, world);
        Ok(())
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.note(message, world);
        Ok(())
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::EnvironmentalDamage { .. } | ClientCommand::LiquidDamage { .. }
        )
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let (hazard, amount, safe_fall) = match *command {
            ClientCommand::EnvironmentalDamage {
                hazard: Hazard::Falling,
                amount,
                safe_fall,
                ..
            } => (Hazard::Falling, amount, safe_fall),
            ClientCommand::EnvironmentalDamage { .. } => {
                return actions::refuse(
                    command,
                    "Only a completed fall is reported here; drowning and lava go through LiquidDamage, and freezing is not reported",
                    out.log,
                );
            }
            ClientCommand::LiquidDamage {
                session_id,
                hazard,
                amount,
                observation,
            } => {
                // The program's liquid routine reports one type per tick,
                // whichever the feet stand in (read R49-3 a).
                let (hazard, index) = match hazard {
                    LiquidHazard::Drowning => (Hazard::Drowning, 0),
                    LiquidHazard::Lava => (Hazard::Lava, 1),
                    LiquidHazard::ZoneHeat => (Hazard::ZoneHeat, 1),
                };
                if !self.taken.contains(&hazard) {
                    return actions::refuse(
                        command,
                        "Liquid reports are unavailable on this session",
                        out.log,
                    );
                }
                let current = world.player_at().is_some_and(|(spawn_id, position)| {
                    let current = [position.x, position.y, position.z];
                    out.log
                        .liquid_inputs
                        .matches(session_id, spawn_id, observation.revision)
                        && same_finite_xyz(current, observation.position)
                });
                if !world.ready() || world.lifecycle.blocks_motion() || !current {
                    return actions::refuse(
                        command,
                        "The liquid observation is no longer current",
                        out.log,
                    );
                }

                if amount == 0
                    || self.last_liquid_sample[index].is_some_and(|last| {
                        observation.sampled_at.saturating_duration_since(last)
                            < Duration::from_secs(1)
                    })
                {
                    return actions::refuse(command, "The liquid tick has no new damage", out.log);
                }
                self.last_liquid_sample[index] = Some(observation.sampled_at);
                (hazard, amount, false)
            }
            _ => return Ok(()),
        };
        if !self.taken.contains(&hazard) {
            return actions::refuse(
                command,
                &format!("{hazard:?} reports are unavailable on this session"),
                out.log,
            );
        }
        if let Some(reason) = self.skipped(world) {
            return actions::refuse(command, reason, out.log);
        }
        self.send_report(hazard, amount, safe_fall, world, out)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;
    use eq_network_game::hazards;

    mod local_posture;

    #[test]
    fn environmental_aa_facts_advance_only_their_current_liquid_revision() {
        use eq_network_game::alternate_advancement::{Timer, Update};
        let mut observations = Observations {
            current: Some((5, 7, 10)),
        };
        let event = |session_id| {
            ClientEvent::World(WorldEvent::AlternateAdvancement {
                session_id,
                updates: vec![
                    eq_network_game::alternate_advancement::Update::Selections(None),
                    eq_network_game::alternate_advancement::Update::DefinitionsUnknown,
                ],
            })
        };
        assert!(observations.observe(&event(4)).is_none());
        assert_eq!(
            observations.observe(&event(5)),
            Some(WorldEvent::LiquidHazardRevision {
                session_id: 5,
                revision: 11
            })
        );
        // Points, experience and timers change no effect, so no revision.
        let unchanged = ClientEvent::World(WorldEvent::AlternateAdvancement {
            session_id: 5,
            updates: vec![
                Update::Points(None),
                Update::Progress(None),
                Update::Timer(Timer {
                    begin: 0,
                    ability: 9,
                    end: 1,
                }),
            ],
        });
        assert!(observations.observe(&unchanged).is_none());
    }

    fn report(hazard: Hazard, amount: u32) -> ClientCommand {
        ClientCommand::EnvironmentalDamage {
            session_id: 5,
            hazard,
            amount,
            safe_fall: false,
        }
    }

    #[test]
    fn pending_ground_drop_still_reports_fall_and_safe_fall() {
        use eq_network_game::inventory::{Inventory, InventorySlot, InventoryUpdate};
        use std::num::NonZeroU32;

        let mut inventory = Inventory::default();
        let serial = NonZeroU32::new(17).unwrap();
        let mut item = testing::item(InventorySlot::CURSOR.0);
        item.rules.ground_drop_serial = Some(serial);
        inventory.apply(InventoryUpdate::Snapshot(vec![item]));
        inventory.apply(InventoryUpdate::GroundDropPending(serial));
        let mut world = World::new(5);
        world.inventory = inventory.into();
        let mut feature = Hazards::new(&[Hazard::Falling], false);
        let command = ClientCommand::EnvironmentalDamage {
            session_id: 5,
            hazard: Hazard::Falling,
            amount: 160,
            safe_fall: true,
        };
        let reported = testing::run(|out| feature.handle(&command, &mut world, out));
        reported.result.unwrap();
        assert_eq!(
            reported.sent,
            [
                hazards::safe_fall(true),
                hazards::titanium_damage(7, Hazard::Falling, 160).unwrap(),
                hazards::titanium_damage_record(7, Hazard::Falling, 160).unwrap(),
            ]
        );
        assert!(world.inventory.ground_drop_pending());
    }

    #[test]
    fn a_hazard_the_server_takes_is_reported_as_the_host_worked_it_out() {
        let mut feature = Hazards::new(&[Hazard::Falling], false);
        let mut world = World::new(5);
        let outcome =
            testing::run(|out| feature.handle(&report(Hazard::Falling, 160), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [
                hazards::titanium_damage(7, Hazard::Falling, 160).unwrap(),
                hazards::titanium_damage_record(7, Hazard::Falling, 160).unwrap(),
            ]
        );
        assert_eq!(outcome.unreliable, []);
        assert!(matches!(
            &outcome.events[..],
            [
                ClientEvent::World(WorldEvent::HazardReported {
                    session_id: 5,
                    hazard: Hazard::Falling,
                    amount: 160,
                    record: true,
                }),
                ClientEvent::Diagnostic(line),
            ] if line.contains("160")
        ));
    }

    #[test]
    fn legacy_damage_command_cannot_bypass_liquid_observations() {
        let mut feature = Hazards::new(&[Hazard::Falling, Hazard::Drowning, Hazard::Lava], false);
        let mut world = World::new(5);
        for hazard in [Hazard::Drowning, Hazard::Lava] {
            let outcome = testing::run(|out| feature.handle(&report(hazard, 13), &mut world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent, []);
            assert_eq!(outcome.unreliable, []);
        }
    }

    #[test]
    fn the_eqmac_program_sends_a_notice_before_the_report() {
        let mut feature = Hazards::new(&[Hazard::Falling], true);
        let mut world = World::new(5);
        let outcome = testing::run(|out| {
            out.wire = &super::super::wire::EqMac;
            feature.handle(&report(Hazard::Falling, 160), &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [
                hazards::eqmac_notice(7, Hazard::Falling, 160).unwrap(),
                hazards::eqmac_damage(7, Hazard::Falling, 160).unwrap(),
            ]
        );
    }

    #[test]
    fn safe_fall_is_announced_first_even_when_it_leaves_nothing() {
        let mut feature = Hazards::new(&[Hazard::Falling], false);
        let mut world = World::new(5);
        let lessened = |amount| ClientCommand::EnvironmentalDamage {
            session_id: 5,
            hazard: Hazard::Falling,
            amount,
            safe_fall: true,
        };
        let outcome = testing::run(|out| feature.handle(&lessened(40), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [
                hazards::safe_fall(true),
                hazards::titanium_damage(7, Hazard::Falling, 40).unwrap(),
                hazards::titanium_damage_record(7, Hazard::Falling, 40).unwrap(),
            ]
        );
        let outcome = testing::run(|out| feature.handle(&lessened(0), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [hazards::safe_fall(true)]);
    }

    #[test]
    fn other_hazards_and_amounts_the_packet_cannot_carry_are_only_noted() {
        let mut feature = Hazards::new(&[Hazard::Falling], false);
        let mut world = World::new(5);
        for command in [
            // Not taken from the client on this server type.
            report(Hazard::Drowning, 20),
            // Nothing to report.
            report(Hazard::Falling, 0),
            // More than the servers read as it is.
            report(Hazard::Falling, u32::MAX),
        ] {
            let outcome = testing::run(|out| feature.handle(&command, &mut world, out));
            outcome.result.unwrap();
            assert!(outcome.sent.is_empty(), "{command:?}");
            assert!(
                matches!(&outcome.events[..], [ClientEvent::Diagnostic(_)]),
                "{command:?}"
            );
        }
    }
}
