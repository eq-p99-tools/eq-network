//! Numbered Titanium destination routes; these are not source boundary volumes.
use super::{finite, float, half, word, ZoneOffer};
use crate::world::Position;
use anyhow::{ensure, Result};
use serde::Serialize;
use std::collections::BTreeMap;

/// A destination advertised in `OP_SendZonepoints`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ZonePoint {
    /// Number referenced by the local WLD zone-line tag, independent of list order.
    pub number: u32,
    /// Destination zone identifier.
    pub zone_id: u16,
    /// Destination instance identifier.
    pub instance_id: u16,
    /// Destination coordinates; 999999 preserves an axis and heading 999 preserves facing.
    pub destination: Position,
}

/// Validated destination table for one zone admission.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ZonePoints(BTreeMap<u32, ZonePoint>);

/// Destination selected by crossing a boundary in the locally installed zone.
#[derive(Clone, Debug, PartialEq)]
pub enum ZoneLineDestination {
    /// Number in the current server-advertised table.
    Reference(u32),
    /// Destination embedded directly in an older WLD region tag.
    Absolute {
        /// Destination zone ID.
        zone_id: u16,
        /// Destination in server axis order, including preserve-value sentinels.
        position: Position,
    },
}

/// What crossing a zone boundary can ask of the session.
#[derive(Clone, Debug, PartialEq)]
pub enum BoundaryRoute {
    /// A normal zone or instance transfer.
    Transfer(ZoneOffer),
    /// A WLD boundary resolving to the current zone and instance. The session
    /// must still verify the actual region entry and the server's pad policy.
    SameZone(LocalPad),
}

/// Entry and landing resolved from an installed WLD boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalPad {
    entry: Position,
    landing: Position,
}

impl LocalPad {
    #[cfg(test)]
    pub(crate) const fn new(entry: Position, landing: Position) -> Self {
        Self { entry, landing }
    }

    /// Position at the checked entrance.
    #[must_use]
    pub const fn entry(self) -> Position {
        self.entry
    }

    /// Landing after applying Titanium's preserve thresholds.
    #[must_use]
    pub const fn landing(self) -> Position {
        self.landing
    }
}

impl ZonePoints {
    /// Decodes the counted Titanium table, accepting its optional unused final record.
    ///
    /// # Errors
    /// Rejects truncation, excessive counts, duplicate IDs and non-finite destinations.
    pub fn decode(body: &[u8]) -> Result<Self> {
        Self::read(body, true)
    }

    /// Decodes `EQMac`'s table, which TAKP lays out as Titanium's but for an
    /// unused field where Titanium has the destination's instance
    /// (`common/eq_packet_structs.h` `ZonePoint_Entry`).
    ///
    /// # Errors
    /// Rejects what [`ZonePoints::decode`] rejects.
    pub fn decode_eqmac(body: &[u8]) -> Result<Self> {
        Self::read(body, false)
    }

    /// The counted table, with or without instances.
    fn read(body: &[u8], instances: bool) -> Result<Self> {
        ensure!(body.len() >= 4, "truncated zone-point header");
        let count = usize::try_from(word(body, 0))?;
        ensure!(count <= 4096, "excessive zone-point count");
        let expected = 4 + count * 24;
        ensure!(
            body.len() == expected || body.len() == expected + 24,
            "invalid zone-point length"
        );
        let mut entries = BTreeMap::new();
        for record in body[4..expected].as_chunks::<24>().0 {
            let point = ZonePoint {
                number: word(record, 0),
                zone_id: half(record, 20),
                instance_id: if instances { half(record, 22) } else { 0 },
                destination: Position {
                    x: float(record, 8),
                    y: float(record, 4),
                    z: float(record, 12),
                    heading: float(record, 16),
                },
            };
            ensure!(finite(point.destination), "invalid zone-point destination");
            ensure!(point.zone_id != 0, "zone point has no destination");
            ensure!(
                entries.insert(point.number, point).is_none(),
                "duplicate zone-point number"
            );
        }
        Ok(Self(entries))
    }

    /// Resolves a local boundary reference without treating it as a destination ID.
    #[must_use]
    pub fn get(&self, number: u32) -> Option<&ZonePoint> {
        self.0.get(&number)
    }

    /// Resolves a boundary against current admission data; same-zone teleports are separate.
    ///
    /// # Errors
    /// Rejects unknown references, invalid coordinates and same-zone destinations.
    pub fn request(
        &self,
        destination: &ZoneLineDestination,
        current: Position,
        current_zone: u16,
        current_instance: u16,
    ) -> Result<ZoneOffer> {
        match self.crossing(destination, current, (current_zone, current_instance))? {
            BoundaryRoute::Transfer(offer) => Ok(offer),
            BoundaryRoute::SameZone(_) => {
                anyhow::bail!("same-zone boundary needs a server relocation")
            }
        }
    }

    /// Resolve an asset boundary. Current zone and instance must both match
    /// for a local pad.
    ///
    /// # Errors
    /// Rejects unknown references and invalid coordinates.
    pub fn crossing(
        &self,
        destination: &ZoneLineDestination,
        current: Position,
        zone: (u16, u16),
    ) -> Result<BoundaryRoute> {
        let point = match destination {
            ZoneLineDestination::Reference(number) => self
                .get(*number)
                .ok_or_else(|| anyhow::anyhow!("unknown zone-line reference"))?
                .clone(),
            ZoneLineDestination::Absolute { zone_id, position } => {
                ensure!(*zone_id != 0, "zone line has no destination");
                ZonePoint {
                    number: 0,
                    zone_id: *zone_id,
                    instance_id: 0,
                    destination: *position,
                }
            }
        };
        let request = point.request(current)?;
        Ok(if (point.zone_id, point.instance_id) == zone {
            BoundaryRoute::SameZone(LocalPad {
                entry: current,
                landing: pad_landing(point.destination, current),
            })
        } else {
            BoundaryRoute::Transfer(request)
        })
    }
}

/// Titanium's local branch at 0x48aaa6 uses upper thresholds for all four
/// fields. Transfer offers have a separate exact-sentinel contract.
fn pad_landing(target: Position, entry: Position) -> Position {
    let axis = |to, from| if to < 999_999.0 { to } else { from };
    Position {
        x: axis(target.x, entry.x),
        y: axis(target.y, entry.y),
        z: axis(target.z, entry.z),
        heading: if target.heading < 513.0 {
            target.heading
        } else {
            entry.heading
        },
    }
}

impl ZonePoint {
    /// Resolves protocol sentinels against the current position for a zone-line request.
    ///
    /// # Errors
    /// Rejects a non-finite current position.
    #[allow(clippy::float_cmp)] // Exact wire sentinels, not computed coordinates.
    pub fn request(&self, current: Position) -> Result<ZoneOffer> {
        ensure!(
            finite(current) && finite(self.destination),
            "invalid zone-line position"
        );
        Ok(ZoneOffer {
            solicited: false,
            zone_id: self.zone_id,
            instance_id: self.instance_id,
            position: preserve_axes(self.destination, current),
            reason: 0,
            to_bind: false,
        })
    }
}

/// Resolve the preserve sentinels of a zone-point destination.
#[allow(clippy::float_cmp)] // Exact protocol sentinels.
fn preserve_axes(destination: Position, current: Position) -> Position {
    let preserve = |target, source| if target == 999_999.0 { source } else { target };
    Position {
        x: preserve(destination.x, current.x),
        y: preserve(destination.y, current.y),
        z: preserve(destination.z, current.z),
        heading: if destination.heading == 999.0 {
            current.heading
        } else {
            destination.heading
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet() -> Vec<u8> {
        let mut body = vec![0; 52]; // One route and an ignored terminal entry.
        body[..4].copy_from_slice(&1u32.to_le_bytes());
        body[4..8].copy_from_slice(&7u32.to_le_bytes());
        for (offset, value) in [(8, 12.5f32), (12, 999_999.0), (16, -8.0), (20, 999.0)] {
            body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        body[24..26].copy_from_slice(&42u16.to_le_bytes());
        body[26..28].copy_from_slice(&3u16.to_le_bytes());
        body
    }
    #[test]
    fn numbered_destination_preserves_axes_and_resolves_sentinels() {
        let body = packet();
        let table = ZonePoints::decode(&body).unwrap();
        assert!(table.get(0).is_none());
        let request = table
            .get(7)
            .unwrap()
            .request(Position {
                x: -3.0,
                y: 99.0,
                z: 4.0,
                heading: 48.0,
            })
            .unwrap();
        assert_eq!((request.zone_id, request.instance_id), (42, 3));
        assert_eq!(
            request.position,
            Position {
                x: -3.0,
                y: 12.5,
                z: -8.0,
                heading: 48.0
            }
        );
        assert_eq!(ZonePoints::decode(&body[..28]).unwrap(), table);
    }
    #[test]
    fn partial_duplicate_and_invalid_tables_are_rejected() {
        let body = packet();
        for len in 0..body.len() {
            if len != 28 {
                assert!(ZonePoints::decode(&body[..len]).is_err());
            }
        }
        let mut duplicate = body.clone();
        duplicate[..4].copy_from_slice(&2u32.to_le_bytes());
        duplicate[28..52].copy_from_slice(&body[4..28]);
        assert!(ZonePoints::decode(&duplicate).is_err());
        let mut invalid = body;
        invalid[8..12].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(ZonePoints::decode(&invalid).is_err());
        assert!(ZonePoints::decode(&u32::MAX.to_le_bytes()).is_err());
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "Exact equality verifies unchanged or explicitly assigned state"
    )]
    fn route_resolution_rejects_unknown_and_same_zone_but_preserves_asset_destinations() {
        let table = ZonePoints::decode(&packet()).unwrap();
        let current = Position::default();
        assert!(table
            .request(&ZoneLineDestination::Reference(1), current, 9, 0)
            .is_err());
        assert!(table
            .request(&ZoneLineDestination::Reference(7), current, 42, 3)
            .is_err());
        assert!(matches!(
            table
                .crossing(&ZoneLineDestination::Reference(7), current, (42, 3))
                .unwrap(),
            BoundaryRoute::SameZone(_)
        ));
        assert!(matches!(
            table
                .crossing(&ZoneLineDestination::Reference(7), current, (42, 0))
                .unwrap(),
            BoundaryRoute::Transfer(_)
        ));
        let destination = ZoneLineDestination::Absolute {
            zone_id: 42,
            position: Position {
                x: 5.0,
                y: 6.0,
                z: 7.0,
                heading: 8.0,
            },
        };
        let request = table.request(&destination, current, 9, 0).unwrap();
        assert_eq!(request.position.x, 5.0);
        assert_eq!(request.position.y, 6.0);
        assert_eq!(request.instance_id, 0);
        assert_eq!(request.reason, 0);
        assert!(table.request(&destination, current, 42, 0).is_err());
    }
}
