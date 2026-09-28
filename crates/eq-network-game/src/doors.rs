//! Titanium door definitions and server movement instructions.
use crate::world::Position;
use anyhow::{ensure, Result};
use serde::Serialize;
use std::collections::BTreeMap;

/// A server-defined door or interactive zone object; open types are not all hinged doors.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Door {
    /// Zone-local identifier, independent of spawn IDs.
    pub id: u8,
    /// Local asset model name, not a filesystem path.
    pub model: String,
    /// EQ coordinates and the raw door heading.
    pub position: Position,
    /// Raw incline; interpretation depends on object type.
    pub incline: u32,
    /// Model scale as a percentage, with 100 meaning normal size.
    pub size: u16,
    /// Client behavior selector; unknown types remain intact.
    pub open_type: u8,
    /// Initial state byte supplied by the server.
    pub state_at_spawn: u8,
    /// Raw inversion flag.
    pub invert_state: u8,
    /// Open-type-specific parameter.
    pub parameter: u32,
    /// Latest server action; not interpreted as a universal open/closed boolean.
    pub action: Option<u8>,
}

impl Door {
    /// Visual endpoint; the server already applies inversion to spawn state and actions.
    /// Unknown actions and initial states do not imply closure.
    #[must_use]
    pub fn active_endpoint(&self) -> Option<bool> {
        match self.action {
            Some(2) => Some(true),
            Some(3) => Some(false),
            Some(_) => None,
            None => match self.state_at_spawn {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            },
        }
    }
}

/// Validated server door updates.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum DoorUpdate {
    /// All definitions were removed; later spawn packets populate a new table.
    RemoveAll,
    /// Definitions in one spawn packet; later packets may add more doors.
    Spawn(Vec<Door>),
    /// Server-selected action, including unknown values.
    Move {
        /// Door identifier.
        id: u8,
        /// Raw animation/action code.
        action: u8,
    },
}

/// Zone-local door state. Unknown IDs never create synthetic doors.
#[derive(Clone, Debug, Default)]
pub struct Doors(BTreeMap<u8, Door>);

impl Doors {
    /// Local proximity policy for ordinary use, not a claim about server lock/key rules.
    pub const USE_DISTANCE: f32 = 20.0;

    /// Encodes ordinary use after checking the current door table and player position.
    ///
    /// # Errors
    /// Rejects unknown doors, invalid own IDs and non-finite or distant positions.
    pub fn click_packet(&self, id: u8, player_id: u16, position: Position) -> Result<[u8; 16]> {
        let door = self
            .0
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("door is no longer available"))?;
        let distance = (position.x - door.position.x)
            .hypot(position.y - door.position.y)
            .hypot(position.z - door.position.z);
        ensure!(
            player_id != 0 && distance.is_finite() && distance <= Self::USE_DISTANCE,
            "door is out of reach or player position is unavailable"
        );
        let mut body = [0; 16];
        body[0] = id;
        body[12..14].copy_from_slice(&player_id.to_le_bytes());
        Ok(body)
    }
    /// Current server-provided definitions in stable door-ID order.
    #[must_use]
    pub fn entries(&self) -> &BTreeMap<u8, Door> {
        &self.0
    }

    /// Applies an already validated packet. A new definition resets previous animation state.
    pub fn apply(&mut self, update: &DoorUpdate) {
        match update {
            DoorUpdate::RemoveAll => self.0.clear(),
            DoorUpdate::Spawn(doors) => {
                for door in doors {
                    self.0.insert(door.id, door.clone());
                }
            }
            DoorUpdate::Move { id, action } => {
                if let Some(door) = self.0.get_mut(id) {
                    door.action = Some(*action);
                }
            }
        }
    }

    /// Reconstructs admission state without losing actions received during loading.
    #[must_use]
    pub fn admission(&self) -> DoorUpdate {
        DoorUpdate::Spawn(self.0.values().cloned().collect())
    }
}

/// Decodes exact Titanium door records, rejecting a malformed batch atomically.
///
/// # Errors
/// Rejects partial records, duplicate IDs and non-finite placement coordinates.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<DoorUpdate>> {
    match opcode {
        0x77d0 => {
            ensure!(body.is_empty(), "invalid door removal length");
            Ok(Some(DoorUpdate::RemoveAll))
        }
        0x700d => {
            ensure!(body.len() == 2, "invalid door movement length");
            Ok(Some(DoorUpdate::Move {
                id: body[0],
                action: body[1],
            }))
        }
        0x4c24 => {
            ensure!(
                body.len().is_multiple_of(80) && body.len() <= 256 * 80,
                "invalid door table length"
            );
            let mut ids = std::collections::BTreeSet::new();
            let mut doors = Vec::with_capacity(body.len() / 80);
            for record in body.chunks_exact(80) {
                let word = |offset| {
                    u32::from_le_bytes([
                        record[offset],
                        record[offset + 1],
                        record[offset + 2],
                        record[offset + 3],
                    ])
                };
                let position = Position {
                    x: f32::from_bits(word(36)),
                    y: f32::from_bits(word(32)),
                    z: f32::from_bits(word(40)),
                    heading: f32::from_bits(word(44)),
                };
                ensure!(
                    [position.x, position.y, position.z, position.heading]
                        .iter()
                        .all(|v| v.is_finite()),
                    "non-finite door placement"
                );
                let id = record[60];
                ensure!(ids.insert(id), "duplicate door ID");
                let end = record[..32].iter().position(|b| *b == 0).unwrap_or(32);
                doors.push(Door {
                    id,
                    model: String::from_utf8_lossy(&record[..end]).into_owned(),
                    position,
                    incline: word(48),
                    size: u16::from_le_bytes([record[52], record[53]]),
                    open_type: record[61],
                    state_at_spawn: record[62],
                    invert_state: record[63],
                    parameter: word(64),
                    action: None,
                });
            }
            Ok(Some(DoorUpdate::Spawn(doors)))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_does_not_double_invert_server_state() {
        let mut body = [0u8; 80];
        body[62] = 1;
        body[63] = 1;
        let DoorUpdate::Spawn(mut doors) = decode(0x4c24, &body).unwrap().unwrap() else {
            panic!("spawn")
        };
        let door = &mut doors[0];
        assert_eq!(door.active_endpoint(), Some(true));
        door.action = Some(3);
        assert_eq!(door.active_endpoint(), Some(false));
        door.action = Some(2);
        assert_eq!(door.active_endpoint(), Some(true));
        door.action = Some(255);
        assert_eq!(door.active_endpoint(), None);
        door.action = None;
        door.state_at_spawn = 255;
        assert_eq!(door.active_endpoint(), None);
    }
    #[test]
    fn removal_discards_definitions_actions_and_click_eligibility() {
        let mut body = [0u8; 80];
        body[60] = 7;
        let spawn = decode(0x4c24, &body).unwrap().unwrap();
        let mut state = Doors::default();
        state.apply(&spawn);
        state.apply(&DoorUpdate::Move { id: 7, action: 2 });
        assert!(state.click_packet(7, 1, Position::default()).is_ok());
        assert!(decode(0x77d0, &[0]).is_err());
        state.apply(&decode(0x77d0, &[]).unwrap().unwrap());
        assert!(state.entries().is_empty());
        assert!(state.click_packet(7, 1, Position::default()).is_err());
        assert_eq!(state.admission(), DoorUpdate::Spawn(Vec::new()));
        state.apply(&DoorUpdate::Move { id: 7, action: 2 });
        assert!(state.entries().is_empty());
        state.apply(&spawn);
        assert_eq!(state.entries()[&7].action, None);
    }
    #[test]
    fn placement_actions_and_admission_preserve_server_fields() {
        let mut body = [0u8; 80];
        body[..4].copy_from_slice(b"TEST");
        for (offset, value) in [(32, -12f32), (36, 27.5), (40, 9.0), (44, 128.0)] {
            body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        body[52] = 100;
        body[60..64].copy_from_slice(&[7, 255, 1, 1]);
        let update = decode(0x4c24, &body).unwrap().unwrap();
        let mut state = Doors::default();
        state.apply(&update);
        let door = &state.entries()[&7];
        assert_eq!(
            (door.position.x, door.position.y, door.model.as_str()),
            (27.5, -12.0, "TEST")
        );
        assert_eq!(
            (door.open_type, door.state_at_spawn, door.invert_state),
            (255, 1, 1)
        );
        state.apply(&decode(0x700d, &[7, 253]).unwrap().unwrap());
        state.apply(&DoorUpdate::Move { id: 8, action: 1 });
        let mut admitted = Doors::default();
        admitted.apply(&state.admission());
        assert_eq!(admitted.entries().len(), 1);
        assert_eq!(admitted.entries()[&7].action, Some(253));
        let position = admitted.entries()[&7].position;
        let packet = admitted.click_packet(7, 0x1234, position).unwrap();
        assert_eq!(packet[0], 7);
        assert_eq!(&packet[1..12], &[0; 11]);
        assert_eq!(&packet[12..], &[0x34, 0x12, 0, 0]);
        assert!(admitted.click_packet(7, 0, position).is_err());
        assert!(admitted.click_packet(8, 1, position).is_err());
        let mut distant = position;
        distant.z += Doors::USE_DISTANCE + 1.0;
        assert!(admitted.click_packet(7, 1, distant).is_err());
        distant.z = f32::NAN;
        assert!(admitted.click_packet(7, 1, distant).is_err());
        assert_eq!(admitted.entries()[&7].action, Some(253));
        admitted.apply(&update);
        assert_eq!(admitted.entries()[&7].action, None);
        for length in 1..80 {
            assert!(decode(0x4c24, &body[..length]).is_err());
        }
        assert!(decode(0x4c24, &body.repeat(2)).is_err());
        body[40..44].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(decode(0x4c24, &body).is_err());
        for bytes in [&[][..], &[7], &[7, 1, 0]] {
            assert!(decode(0x700d, bytes).is_err());
        }
    }
}
