//! `EQMac`'s counted, compressed door table and server actions.
//!
//! TAKP and Quarm use `Door_Struct` and `DoorSpawns_Struct` from
//! `common/eq_packet_structs.h`, filled by `EntityList::SendZoneDoorsBulk`.
//! See `docs/eqmac-world-objects.md` for the independently inspected revisions.

use super::{Door, DoorUpdate, STATIC_ID};
use crate::world::Position;
use anyhow::{anyhow, ensure, Result};
use std::collections::BTreeSet;

/// `OP_SpawnDoor`: a count and zlib-compressed door definitions.
pub const SPAWN_OPCODE: u16 = 0xf741;
/// `OP_DespawnDoor`: every door is removed.
pub const REMOVE_OPCODE: u16 = 0x9b40;
/// `OP_MoveDoor`: the server's action on one door.
pub const MOVE_OPCODE: u16 = 0x8e40;

const RECORD: usize = 44;
const MAX_DOORS: usize = 255;

/// Reads `EQMac` door definitions, removal and actions without changing a table.
/// An entire malformed table is rejected before any definition is exposed.
///
/// # Errors
/// Rejects wrong lengths, invalid compression, counts above 255 or different
/// from the decompressed record count, duplicate nonstatic IDs and nonfinite
/// coordinates. Unknown states and action values remain intact.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<DoorUpdate>> {
    Ok(Some(match opcode {
        REMOVE_OPCODE => {
            ensure!(body.is_empty(), "invalid EQMac door removal length");
            DoorUpdate::RemoveAll
        }
        MOVE_OPCODE => {
            let [id, action] = body else {
                return Err(anyhow!("invalid EQMac door movement length"));
            };
            DoorUpdate::Move {
                id: *id,
                action: *action,
            }
        }
        SPAWN_OPCODE => DoorUpdate::Spawn(table(body)?),
        _ => return Ok(None),
    }))
}

/// The complete bounded table, decoded before the caller applies it.
fn table(body: &[u8]) -> Result<Vec<Door>> {
    ensure!(body.len() > 2, "truncated EQMac door table");
    let count = usize::from(u16::from_le_bytes([body[0], body[1]]));
    ensure!(count <= MAX_DOORS, "too many EQMac doors");
    let data =
        miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&body[2..], MAX_DOORS * RECORD)
            .map_err(|_| anyhow!("invalid or oversized EQMac door table"))?;
    ensure!(data.len() == count * RECORD, "EQMac door count mismatch");
    let mut ids = BTreeSet::new();
    data.as_chunks::<RECORD>()
        .0
        .iter()
        .map(|record| {
            let word = |offset| {
                u32::from_le_bytes([
                    record[offset],
                    record[offset + 1],
                    record[offset + 2],
                    record[offset + 3],
                ])
            };
            let short = |offset| u16::from_le_bytes([record[offset], record[offset + 1]]);
            let position = Position {
                x: f32::from_bits(word(20)),
                y: f32::from_bits(word(16)),
                z: f32::from_bits(word(24)),
                heading: f32::from_bits(word(28)),
            };
            ensure!(
                [position.x, position.y, position.z, position.heading]
                    .iter()
                    .all(|value| value.is_finite()),
                "non-finite EQMac door placement"
            );
            let id = record[38];
            ensure!(id == STATIC_ID || ids.insert(id), "duplicate EQMac door ID");
            let end = record[..16]
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(16);
            Ok(Door {
                id,
                model: String::from_utf8_lossy(&record[..end]).into_owned(),
                position,
                incline: u32::from(short(32)),
                size: short(34),
                open_type: record[39],
                state_at_spawn: record[40],
                invert_state: record[41],
                parameter: u32::from(short(42)),
                action: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doors::Doors;

    fn record(id: u8) -> [u8; RECORD] {
        let mut record = [0; RECORD];
        record[..4].copy_from_slice(b"DOOR");
        for (offset, value) in [(16, -12.0f32), (20, 27.5), (24, 9.0), (28, 128.0)] {
            record[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        record[32..34].copy_from_slice(&0xabcd_u16.to_le_bytes());
        record[34..36].copy_from_slice(&100_u16.to_le_bytes());
        record[38..42].copy_from_slice(&[id, 253, 1, 1]);
        record[42..44].copy_from_slice(&0xfffe_u16.to_le_bytes());
        record
    }

    fn packet(count: u16, records: &[u8]) -> Vec<u8> {
        let mut body = count.to_le_bytes().to_vec();
        body.extend(miniz_oxide::deflate::compress_to_vec_zlib(records, 6));
        body
    }

    #[test]
    fn compressed_definitions_actions_and_removal_use_the_current_table() {
        let body = packet(3, &[record(7), record(255), record(255)].concat());
        let mut state = Doors::default();
        state.apply(&decode(SPAWN_OPCODE, &body).unwrap().unwrap());
        let door = &state.entries()[&7];
        assert_eq!(door.model, "DOOR");
        assert_eq!((door.position.x, door.position.y), (27.5, -12.0));
        assert_eq!(
            (door.incline, door.size, door.parameter),
            (0xabcd, 100, 0xfffe)
        );
        assert_eq!((door.open_type, door.invert_state), (253, 1));
        assert_eq!(door.active_endpoint(), Some(true));
        for (action, endpoint) in [(3, Some(false)), (2, Some(true)), (251, None)] {
            state.apply(&decode(MOVE_OPCODE, &[7, action]).unwrap().unwrap());
            assert_eq!(state.entries()[&7].active_endpoint(), endpoint);
        }
        state.apply(&decode(MOVE_OPCODE, &[8, 2]).unwrap().unwrap());
        assert!(!state.entries().contains_key(&8));
        let mut admitted = Doors::default();
        admitted.apply(&state.admission());
        assert_eq!(admitted.admission(), state.admission());
        let DoorUpdate::Spawn(doors) = admitted.admission() else {
            panic!("definitions");
        };
        assert_eq!(doors.len(), 3);
        state.apply(&decode(REMOVE_OPCODE, &[]).unwrap().unwrap());
        assert!(state.entries().is_empty());
        assert_eq!(decode(0x4c24, &body).unwrap(), None);
    }

    #[test]
    fn a_malformed_compressed_table_never_exposes_partial_definitions() {
        assert!(decode(SPAWN_OPCODE, &packet(2, &record(7))).is_err());
        assert!(decode(SPAWN_OPCODE, &packet(1, &record(7)[..43])).is_err());
        assert!(decode(SPAWN_OPCODE, &packet(256, &record(255).repeat(256))).is_err());
        assert!(decode(SPAWN_OPCODE, &packet(2, &record(7).repeat(2))).is_err());
        let mut invalid = record(8);
        invalid[24..28].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(decode(SPAWN_OPCODE, &packet(2, &[record(7), invalid].concat())).is_err());
        for body in [vec![], vec![1], vec![1, 0], vec![1, 0, 0xff]] {
            assert!(decode(SPAWN_OPCODE, &body).is_err());
        }
        for body in [vec![], vec![7], vec![7, 2, 0]] {
            assert!(decode(MOVE_OPCODE, &body).is_err());
        }
        assert!(decode(REMOVE_OPCODE, &[0]).is_err());
        assert!(decode(SPAWN_OPCODE, &packet(255, &record(255).repeat(255))).is_ok());
        // The limit also applies when the claimed count is small.
        assert!(decode(SPAWN_OPCODE, &packet(1, &record(255).repeat(256))).is_err());
    }
}
