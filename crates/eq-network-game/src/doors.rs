//! Titanium door definitions and server movement instructions.
use crate::{command::EncodedCommand, world::Position};
use anyhow::{ensure, Result};
use serde::Serialize;
use std::collections::BTreeMap;

/// `OP_ClickDoor`: the player uses a door.
pub const CLICK_OPCODE: u16 = 0x043b;

/// The door ID of a zone's static objects. `EQEmu` loads each object of
/// type 0 as a door with no ID of its own and lists it in the door table with
/// this one, so it repeats there.
const STATIC_ID: u8 = 255;

/// The most doors `EQEmu` lists in one table.
const MOST_DOORS: usize = 500;

/// A server-defined door or interactive zone object; open types are not all hinged doors.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Door {
    /// Zone-local identifier, independent of spawn IDs. 255 is a static
    /// object rather than a door: every static object in the zone comes with
    /// it, and nothing uses one.
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
pub struct Doors {
    /// Definitions by ID; of the static objects, the last one listed.
    doors: BTreeMap<u8, Door>,
    /// Every static object listed, in order.
    statics: Vec<Door>,
}

impl Doors {
    /// Local proximity policy for ordinary use, not a claim about server lock/key rules.
    pub const USE_DISTANCE: f32 = 20.0;

    /// Encodes ordinary use after checking the current door table and player position.
    ///
    /// # Errors
    /// Rejects static objects, unknown doors, invalid own IDs and non-finite or
    /// distant positions.
    pub fn click_packet(
        &self,
        id: u8,
        player_id: u16,
        position: Position,
    ) -> Result<EncodedCommand> {
        self.check_click(id, player_id, position)?;
        Ok(titanium_click(id, player_id))
    }

    /// Checks a click on a door against the current door table and the
    /// player's position: the door must still be there and within reach.
    /// Nothing uses a static object, so a click on one never goes out.
    ///
    /// # Errors
    /// Rejects static objects, unknown doors, invalid own IDs and non-finite or
    /// distant positions.
    pub fn check_click(&self, id: u8, player_id: u16, position: Position) -> Result<()> {
        ensure!(id != STATIC_ID, "a static object cannot be used");
        let door = self
            .doors
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("door is no longer available"))?;
        let distance = (position.x - door.position.x)
            .hypot(position.y - door.position.y)
            .hypot(position.z - door.position.z);
        ensure!(
            player_id != 0 && distance.is_finite() && distance <= Self::USE_DISTANCE,
            "door is out of reach or player position is unavailable"
        );
        Ok(())
    }
    /// Current server-provided definitions in stable door-ID order. Static
    /// objects share one ID, so only the last one listed is here;
    /// [`Doors::admission`] carries them all.
    #[must_use]
    pub fn entries(&self) -> &BTreeMap<u8, Door> {
        &self.doors
    }

    /// Applies an already validated packet. A new definition resets previous animation state.
    pub fn apply(&mut self, update: &DoorUpdate) {
        match update {
            DoorUpdate::RemoveAll => {
                self.doors.clear();
                self.statics.clear();
            }
            DoorUpdate::Spawn(doors) => {
                for door in doors {
                    if door.id == STATIC_ID {
                        self.statics.push(door.clone());
                    }
                    self.doors.insert(door.id, door.clone());
                }
            }
            DoorUpdate::Move { id, action } => {
                if let Some(door) = self.doors.get_mut(id) {
                    door.action = Some(*action);
                }
            }
        }
    }

    /// Reconstructs admission state without losing actions received during
    /// loading, with every static object.
    #[must_use]
    pub fn admission(&self) -> DoorUpdate {
        DoorUpdate::Spawn(
            self.doors
                .values()
                .filter(|door| door.id != STATIC_ID)
                .chain(&self.statics)
                .cloned()
                .collect(),
        )
    }
}

/// Titanium's `OP_ClickDoor` for the player using a door, already checked.
#[must_use]
pub fn titanium_click(id: u8, player_id: u16) -> EncodedCommand {
    let mut body = vec![0; 16];
    body[0] = id;
    body[12..14].copy_from_slice(&player_id.to_le_bytes());
    EncodedCommand {
        opcode: CLICK_OPCODE,
        body,
    }
}

/// Decodes exact Titanium door records, rejecting a malformed batch atomically.
///
/// # Errors
/// Rejects partial records, tables of more than 500 doors, a repeated ID other
/// than a static object's, and non-finite placement coordinates.
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
                body.len().is_multiple_of(80) && body.len() <= MOST_DOORS * 80,
                "invalid door table length"
            );
            let mut ids = std::collections::BTreeSet::new();
            let mut doors = Vec::with_capacity(body.len() / 80);
            for record in body.as_chunks::<80>().0 {
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
                ensure!(id == STATIC_ID || ids.insert(id), "duplicate door ID");
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
        assert_eq!(packet.opcode, CLICK_OPCODE);
        assert_eq!(packet.body[0], 7);
        assert_eq!(&packet.body[1..12], &[0; 11]);
        assert_eq!(&packet.body[12..], &[0x34, 0x12, 0, 0]);
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

    /// One door record with this ID and model, at this x.
    fn record(id: u8, model: &[u8], x: f32) -> [u8; 80] {
        let mut body = [0u8; 80];
        body[..model.len()].copy_from_slice(model);
        body[36..40].copy_from_slice(&x.to_le_bytes());
        body[60] = id;
        body
    }

    fn models(update: &DoorUpdate) -> Vec<&str> {
        let DoorUpdate::Spawn(doors) = update else {
            panic!("spawn")
        };
        doors.iter().map(|door| door.model.as_str()).collect()
    }

    #[test]
    fn static_objects_repeat_id_255_and_are_all_kept_but_never_used() {
        let body = [
            record(255, b"ROCKA", 1.0),
            record(7, b"DOOR", 2.0),
            record(255, b"ROCKB", 3.0),
        ]
        .concat();
        let update = decode(0x4c24, &body).unwrap().unwrap();
        assert_eq!(models(&update), ["ROCKA", "DOOR", "ROCKB"]);
        let mut state = Doors::default();
        state.apply(&update);
        assert_eq!(
            state.entries().keys().copied().collect::<Vec<_>>(),
            [7, 255]
        );
        assert_eq!(state.entries()[&255].model, "ROCKB");
        let admission = state.admission();
        assert_eq!(models(&admission), ["DOOR", "ROCKA", "ROCKB"]);
        let mut admitted = Doors::default();
        admitted.apply(&admission);
        assert_eq!(admitted.admission(), admission);
        let at = |x| Position {
            x,
            ..Position::default()
        };
        assert!(state.click_packet(7, 1, at(2.0)).is_ok());
        for x in [1.0, 3.0] {
            let error = state.click_packet(255, 1, at(x)).unwrap_err();
            assert_eq!(error.to_string(), "a static object cannot be used");
        }
        state.apply(&DoorUpdate::RemoveAll);
        assert!(state.entries().is_empty());
        assert_eq!(state.admission(), DoorUpdate::Spawn(Vec::new()));
    }

    #[test]
    fn a_repeated_door_id_other_than_a_static_objects_refuses_the_table() {
        let body = [record(7, b"DOOR", 0.0), record(7, b"GATE", 1.0)].concat();
        let error = decode(0x4c24, &body).unwrap_err();
        assert_eq!(error.to_string(), "duplicate door ID");
    }

    #[test]
    fn a_table_holds_up_to_500_doors() {
        let table = |count: usize| {
            (0..count)
                .flat_map(|index| record(u8::try_from(index).unwrap_or(255), b"DOOR", 0.0))
                .collect::<Vec<_>>()
        };
        let update = decode(0x4c24, &table(300)).unwrap().unwrap();
        let mut state = Doors::default();
        state.apply(&update);
        assert_eq!(state.entries().len(), 256);
        assert_eq!(models(&state.admission()).len(), 300);
        assert!(decode(0x4c24, &table(500)).is_ok());
        let error = decode(0x4c24, &table(501)).unwrap_err();
        assert_eq!(error.to_string(), "invalid door table length");
    }
}
