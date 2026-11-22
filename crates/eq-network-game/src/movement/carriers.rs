//! Report-driven carrier poses shared by the session and its frontends.
use super::{carry, Carried};
use crate::{
    world::{Position, SpawnKind, SpawnState, WorldEvent},
    GameDialect,
};
use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

/// Conservative rider reach from a carrier origin, in world units. This is
/// an inferred guard bound, not a ship dimension (recording checklist 6.469).
pub const CARRIER_REACH: f32 = 600.0;
const HISTORY: Duration = Duration::from_secs(2);
const MOST_REPORTS: usize = 64;
const STEP: Duration = Duration::from_millis(50);
const HORIZON: Duration = Duration::from_secs(12);

/// The races Titanium treats as carriers (`eqgame.exe` 0x47a741).
#[must_use]
pub const fn carrier_race(race: u32) -> bool {
    matches!(race, 141 | 72 | 73 | 114 | 404)
}

/// Names the received report whose pose the frontend used. The timestamp is
/// the session's, preserved when the frontend consumes the event later.
/// Obtain this identity from [`CarrierReport::sample`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub struct CarrierSample {
    /// Zone-local carrier identity.
    pub spawn_id: u16,
    /// Unique report number within the admission.
    pub serial: u64,
    /// Report number of the spawn record that began this incarnation.
    pub incarnation: u64,
    /// Receipt of the report, as published in [`CarrierReport`].
    #[serde(skip)]
    pub reported: Instant,
    /// Receipt of the spawn record for this incarnation.
    #[serde(skip)]
    pub spawned: Instant,
}

/// One received carrier report, with no route or extrapolated authority.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub struct CarrierReport {
    sample: CarrierSample,
    dialect: GameDialect,
    race: u32,
    carried: Carried,
}

impl CarrierReport {
    /// Identifies this report for a movement proposal.
    #[must_use]
    pub const fn sample(self) -> CarrierSample {
        self.sample
    }

    /// Carries the received spawn on using the existing spawn step, one call
    /// per 50 ms, held after 12 s. Fixed calls make the result independent of
    /// frontend frame rate and receipt delay. The carrier's between-report
    /// path, the fixed calls and the hold are inferred (6.468). Heading stays
    /// as reported; pitch and continuing yaw are not decoded here.
    #[must_use]
    pub fn pose(self, at: Instant) -> Position {
        let mut carried = self.carried;
        let mut left = at
            .saturating_duration_since(self.sample.reported)
            .min(HORIZON);
        while !left.is_zero() {
            let span = left.min(STEP);
            carry(self.dialect, &mut carried, span);
            left -= span;
        }
        carried.position
    }
}

/// Canonical receipt-time state sent by the session, after the ordinary spawn
/// event. Removal never certifies a boat route or a successful ride.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum CarrierUpdate {
    /// Replaces or advances a carrier's received report.
    Reported(CarrierReport),
    /// Forgets a despawned, replaced or no-longer-carrier spawn.
    Removed(u16),
}

/// A bounded receipt history: late frontends can name the report they saw,
/// without earning the difference between that pose and the session's newer
/// pose. At most 64 reports per carrier and 2 s of superseded reports (6.469).
#[derive(Clone, Debug, Default)]
pub struct Carriers(BTreeMap<u16, VecDeque<CarrierReport>>, u64);

impl Carriers {
    /// Forgets the admission's carriers.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Applies a canonical session observation in the frontend.
    pub fn apply(&mut self, update: CarrierUpdate) {
        match update {
            CarrierUpdate::Removed(id) => {
                self.0.remove(&id);
            }
            CarrierUpdate::Reported(report) => {
                self.1 = self.1.max(report.sample.serial);
                let reports = self.0.entry(report.sample.spawn_id).or_default();
                if reports
                    .back()
                    .is_some_and(|old| old.sample.incarnation != report.sample.incarnation)
                {
                    reports.clear();
                }
                reports.retain(|old| old.sample != report.sample);
                while reports.front().is_some_and(|old| {
                    report
                        .sample
                        .reported
                        .saturating_duration_since(old.sample.reported)
                        > HISTORY
                }) {
                    reports.pop_front();
                }
                reports.push_back(report);
                while reports.len() > MOST_REPORTS {
                    reports.pop_front();
                }
            }
        }
    }

    /// Latest observed report for this spawn.
    #[must_use]
    pub fn latest(&self, id: u16) -> Option<CarrierReport> {
        self.0.get(&id)?.back().copied()
    }

    /// Every current received carrier report.
    pub fn reports(&self) -> impl Iterator<Item = CarrierReport> + '_ {
        self.0
            .values()
            .filter_map(|reports| reports.back().copied())
    }

    /// Resolves only an actual retained report, never a frontend-created pose.
    #[must_use]
    pub fn pose(&self, sample: CarrierSample, at: Instant) -> Option<Position> {
        let reports = self.0.get(&sample.spawn_id)?;
        let index = reports.iter().position(|report| report.sample == sample)?;
        let report = reports.get(index)?;
        if reports.get(index + 1).is_some_and(|successor| {
            at.saturating_duration_since(successor.sample.reported) > HISTORY
        }) {
            return None;
        }
        (at >= sample.reported).then(|| report.pose(at))
    }

    /// Records the ordinary server event at its session receipt time and
    /// returns the canonical observations to forward to the frontend.
    pub fn observe(
        &mut self,
        event: &WorldEvent,
        dialect: GameDialect,
        now: Instant,
    ) -> Vec<CarrierUpdate> {
        let mut updates = Vec::new();
        match event {
            WorldEvent::Spawns(spawns) => {
                // The ordinary spawn table keeps the last record for an ID.
                // Resolve duplicates before publishing carrier authority too.
                let latest: BTreeMap<_, _> =
                    spawns.iter().map(|spawn| (spawn.spawn_id, spawn)).collect();
                for spawn in latest.into_values() {
                    // A replacement is a fresh incarnation, even when its ID and
                    // race happen to match. Old named report stamps disappear.
                    let previous = self.0.remove(&spawn.spawn_id).is_some();
                    if previous {
                        updates.push(CarrierUpdate::Removed(spawn.spawn_id));
                    }
                    let reports = match dialect {
                        GameDialect::Titanium => true,
                        GameDialect::EqMac => false,
                    };
                    if reports
                        && carrier_race(spawn.race)
                        && !matches!(spawn.kind, SpawnKind::NpcCorpse | SpawnKind::PlayerCorpse)
                    {
                        updates.push(CarrierUpdate::Reported(self.report(dialect, spawn, now)));
                    }
                }
            }
            WorldEvent::Position {
                spawn_id,
                position,
                velocity,
                forward,
            } => {
                if let Some(previous) = self.latest(*spawn_id) {
                    self.1 = self.1.wrapping_add(1);
                    updates.push(CarrierUpdate::Reported(CarrierReport {
                        sample: CarrierSample {
                            reported: now,
                            serial: self.1,
                            ..previous.sample
                        },
                        carried: Carried::reported(
                            dialect,
                            previous.carried.kind,
                            previous.race,
                            *position,
                            *velocity,
                            *forward,
                        ),
                        ..previous
                    }));
                }
            }
            WorldEvent::Despawn(id) if self.latest(*id).is_some() => {
                updates.push(CarrierUpdate::Removed(*id));
            }
            WorldEvent::Death(death) => {
                if let Some(id) = u16::try_from(death.spawn_id)
                    .ok()
                    .filter(|id| self.latest(*id).is_some())
                {
                    updates.push(CarrierUpdate::Removed(id));
                }
            }
            WorldEvent::Appearance {
                spawn_id,
                change: crate::appearance::AppearanceChange::Illusion(_),
            } if self.latest(*spawn_id).is_some() => {
                // An appearance can replace the model. Wait for a fresh
                // spawn record before offering its geometry as a carrier.
                updates.push(CarrierUpdate::Removed(*spawn_id));
            }
            _ => (),
        }
        for update in &updates {
            self.apply(*update);
        }
        updates
    }

    fn report(&mut self, dialect: GameDialect, spawn: &SpawnState, now: Instant) -> CarrierReport {
        self.1 = self.1.wrapping_add(1);
        CarrierReport {
            sample: CarrierSample {
                spawn_id: spawn.spawn_id,
                serial: self.1,
                incarnation: self.1,
                reported: now,
                spawned: now,
            },
            dialect,
            race: spawn.race,
            carried: Carried::reported(
                dialect,
                spawn.kind,
                spawn.race,
                spawn.position,
                spawn.velocity,
                spawn.forward,
            ),
        }
    }
}

/// Converts a world pose to the boat's yaw frame, as the read linked-rider step
/// does. Pitch is absent from the decoded pose API.
#[must_use]
pub fn carrier_local(carrier: Position, world: Position) -> Position {
    let (sin, cos) = (carrier.heading / 512.0 * std::f32::consts::TAU).sin_cos();
    let (x, y) = (world.x - carrier.x, world.y - carrier.y);
    Position {
        x: x * cos - y * sin,
        y: x * sin + y * cos,
        z: world.z - carrier.z,
        heading: (world.heading - carrier.heading).rem_euclid(512.0),
    }
}

/// Converts a boat-relative pose back to world space; position reports remain
/// ordinary world-space updates while riding.
#[must_use]
pub fn carrier_world(carrier: Position, local: Position) -> Position {
    let (sin, cos) = (carrier.heading / 512.0 * std::f32::consts::TAU).sin_cos();
    Position {
        x: carrier.x + local.x * cos + local.y * sin,
        y: carrier.y - local.x * sin + local.y * cos,
        z: carrier.z + local.z,
        heading: (local.heading + carrier.heading).rem_euclid(512.0),
    }
}

pub(super) fn within_carrier(position: Position) -> bool {
    position.x.hypot(position.y).hypot(position.z) <= CARRIER_REACH
}

/// Shortest signed difference in EQ heading units.
pub(super) fn heading_arc(from: f32, to: f32) -> f32 {
    (to - from + 256.0).rem_euclid(512.0) - 256.0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn turning_and_translating_a_carrier_preserves_a_riders_local_pose() {
        let local = Position {
            x: 40.0,
            y: -20.0,
            z: 5.0,
            heading: 500.0,
        };
        for heading in [0.0, 128.0, 511.0] {
            let boat = Position {
                x: 200.0,
                y: 500.0,
                z: -8.0,
                heading,
            };
            let restored = carrier_local(boat, carrier_world(boat, local));
            assert!((restored.x - local.x).abs() < 0.001);
            assert!((restored.y - local.y).abs() < 0.001);
            assert!((restored.heading - local.heading).abs() < 0.001);
        }
    }
    #[test]
    fn report_history_preserves_lag_but_retires_old_incarnations_and_despawns() {
        let now = Instant::now();
        let report = CarrierReport {
            sample: CarrierSample {
                spawn_id: 7,
                serial: 1,
                incarnation: 1,
                reported: now,
                spawned: now,
            },
            dialect: GameDialect::Titanium,
            race: 72,
            carried: Carried::reported(
                GameDialect::Titanium,
                SpawnKind::Npc,
                72,
                Position::default(),
                [10.0, 0.0, 0.0],
                0.5,
            ),
        };
        let mut carriers = Carriers::default();
        carriers.apply(CarrierUpdate::Reported(report));
        let later = now + Duration::from_millis(300);
        let mut next = report;
        next.sample.reported = later;
        next.sample.serial += 1;
        next.carried = Carried::reported(
            GameDialect::Titanium,
            SpawnKind::Npc,
            72,
            Position {
                x: 20.0,
                ..Position::default()
            },
            [0.0; 3],
            0.0,
        );
        carriers.apply(CarrierUpdate::Reported(next));
        assert_eq!(
            carriers.pose(report.sample(), later),
            Some(report.pose(later))
        );
        assert_ne!(
            carriers.pose(report.sample(), later),
            carriers.pose(next.sample(), later)
        );
        // Silence after a newer stop cannot retain an old moving report's
        // authority beyond the documented frontend lag window.
        let boundary = later + HISTORY;
        assert!(carriers.pose(report.sample(), boundary).is_some());
        let expired = boundary + Duration::from_nanos(1);
        assert!(carriers.pose(report.sample(), expired).is_none());
        assert_eq!(
            carriers.pose(next.sample(), expired),
            Some(Position {
                x: 20.0,
                ..Position::default()
            })
        );
        carriers.observe(&WorldEvent::Despawn(7), GameDialect::Titanium, later);
        assert!(carriers.pose(report.sample(), later).is_none());
        carriers.clear();
        assert!(carriers.latest(7).is_none());
    }
    fn spawn(id: u16, race: u32, x: f32) -> SpawnState {
        SpawnState {
            spawn_id: id,
            race,
            kind: SpawnKind::Npc,
            position: Position {
                x,
                ..Position::default()
            },
            class: None,
            name: String::new(),
            gender: 0,
            velocity: [0.0; 3],
            forward: 0.0,
            walk_speed: 0.0,
            run_speed: 0.0,
            size: 1.0,
            invisible: false,
            appearance: crate::appearance::Appearance::default(),
            level: 1,
            listing: crate::listing::Listing::default(),
            pet_owner: None,
            hp_percent: None,
            posture: None,
            name_parts: crate::names::NameParts::default(),
            body_type: None,
        }
    }

    #[test]
    fn duplicate_spawn_ids_use_only_the_last_record_and_replacements_have_distinct_incarnations() {
        let now = Instant::now();
        let mut carriers = Carriers::default();
        carriers.observe(
            &WorldEvent::Spawns(vec![spawn(7, 72, 1.0), spawn(7, 73, 2.0)]),
            GameDialect::Titanium,
            now,
        );
        let first = carriers.latest(7).unwrap();
        assert_eq!(first.pose(now).x, 2.0);
        assert_eq!(carriers.pose(first.sample(), now).unwrap().x, 2.0);
        // Even equal receipt instants are not actor identity.
        carriers.observe(
            &WorldEvent::Spawns(vec![spawn(7, 73, 3.0)]),
            GameDialect::Titanium,
            now,
        );
        let replacement = carriers.latest(7).unwrap();
        assert_ne!(first.sample().incarnation, replacement.sample().incarnation);
        assert!(carriers.pose(first.sample(), now).is_none());
        carriers.observe(
            &WorldEvent::Spawns(vec![spawn(7, 72, 4.0), spawn(7, 1, 5.0)]),
            GameDialect::Titanium,
            now,
        );
        assert!(carriers.latest(7).is_none());
    }
}
