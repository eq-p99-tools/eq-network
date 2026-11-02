//! Inspection of Titanium movement-history records.
//!
//! This module does not send packets or schedule reports. See
//! `docs/protocol/movement-history.md` for the evidence and remaining gaps.

use anyhow::{ensure, Result};

/// Titanium's `OP_FloatListThing`, handled as movement history by `EQEmu`.
pub const OPCODE: u16 = 0x6a1b;

/// Size of one packed record, excluding any packet trailer.
pub const RECORD_LEN: usize = 17;

/// Cause codes named by `EQEmu`'s movement-history handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Cause {
    /// A position recorded by the client's collision update.
    Collision,
    /// The server's `TeleportB` code.
    TeleportB,
    /// The server's `TeleportA` code.
    TeleportA,
    /// A zone-line record.
    ZoneLine,
    /// An unclassified cause, retained exactly as received.
    Unknown(u8),
}

impl From<u8> for Cause {
    fn from(value: u8) -> Self {
        match value {
            1 => Self::Collision,
            2 => Self::TeleportB,
            3 => Self::TeleportA,
            4 => Self::ZoneLine,
            other => Self::Unknown(other),
        }
    }
}

/// One record from the Titanium client's movement-history serializer.
///
/// Coordinates remain in wire order and retain non-finite values for diagnostics.
/// The record has no heading. This is decoded evidence, not a movement request.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Record {
    /// Y coordinate, the first float on the wire.
    pub y: f32,
    /// X coordinate, the second float on the wire.
    pub x: f32,
    /// Z coordinate, the third float on the wire.
    pub z: f32,
    /// Why the client recorded this value.
    pub cause: Cause,
    /// Low 16 bits of the client's millisecond clock; wraps every 65.536 seconds.
    pub timestamp_ms: u16,
    /// The client's wrapping 16-bit record sequence.
    pub sequence: u16,
}

impl Record {
    /// Decode exactly one record without interpreting an enclosing packet.
    ///
    /// # Errors
    /// Returns an error unless `bytes` contains exactly [`RECORD_LEN`] bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() == RECORD_LEN,
            "movement history record must be 17 bytes"
        );
        Ok(Self {
            y: f32::from_le_bytes(bytes[0..4].try_into()?),
            x: f32::from_le_bytes(bytes[4..8].try_into()?),
            z: f32::from_le_bytes(bytes[8..12].try_into()?),
            cause: bytes[12].into(),
            timestamp_ms: u16::from_le_bytes(bytes[13..15].try_into()?),
            sequence: u16::from_le_bytes(bytes[15..17].try_into()?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_axes_clock_and_sequence() {
        let record = Record::decode(&[
            0x00, 0x00, 0x00, 0x40, // Y = 2
            0x00, 0x00, 0x80, 0xbf, // X = -1
            0x00, 0x00, 0x40, 0x40, // Z = 3
            4, 0xfe, 0xff, 0x34, 0x12,
        ])
        .unwrap();
        assert_eq!([record.y, record.x, record.z], [2.0, -1.0, 3.0]);
        assert_eq!(record.cause, Cause::ZoneLine);
        assert_eq!(record.timestamp_ms, 65534);
        assert_eq!(record.sequence, 0x1234);
    }

    #[test]
    fn rejects_partial_records_and_packet_trailers() {
        for length in 0..=35 {
            assert_eq!(
                Record::decode(&vec![0; length]).is_ok(),
                length == RECORD_LEN
            );
        }
    }

    #[test]
    fn preserves_unknown_causes_and_non_finite_coordinates() {
        let mut bytes = [0; RECORD_LEN];
        bytes[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
        for cause in 0..=255 {
            bytes[12] = cause;
            let record = Record::decode(&bytes).unwrap();
            assert!(record.y.is_nan());
            let expected = match cause {
                1 => Cause::Collision,
                2 => Cause::TeleportB,
                3 => Cause::TeleportA,
                4 => Cause::ZoneLine,
                other => Cause::Unknown(other),
            };
            assert_eq!(record.cause, expected);
        }
    }
}
