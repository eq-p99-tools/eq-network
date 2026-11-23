//! `RoF2`'s doors, ground objects and weather, as stock `EQEmu` sends them,
//! inferred for `RoF2` (6.539).
//!
//! Each reader gives Titanium's event for the same server values, with
//! Titanium's refusals. The door table and the ground object have `RoF2`
//! encoders (`common/patches/rof2.cpp`); a door's move, an object's click
//! and the weather go out unencoded, the bytes Titanium gets under its own
//! numbers.
//!
//! What the `RoF2` program does with the door record's constants, with the
//! ground object's tilts and size, and whether it reads the zone-in weather,
//! is not read (6.539).
use crate::{
    doors::DoorUpdate,
    objects::{GroundObject, ObjectUpdate},
    world::{Position, WorldEvent},
};
use anyhow::{anyhow, ensure, Result};

/// `OP_SpawnDoor` (`utils/patches/patch_RoF2.conf`; inferred, 6.539).
pub const SPAWN_DOOR_OPCODE: u16 = 0x7291;
/// `OP_MoveDoor` (`utils/patches/patch_RoF2.conf`; inferred, 6.539).
pub const MOVE_DOOR_OPCODE: u16 = 0x08e8;
/// `OP_GroundSpawn` (`utils/patches/patch_RoF2.conf`; inferred, 6.539).
pub const GROUND_SPAWN_OPCODE: u16 = 0x6fca;
/// `OP_ClickObject` (`utils/patches/patch_RoF2.conf`; inferred, 6.539).
pub const CLICK_OBJECT_OPCODE: u16 = 0x4aa1;
/// `OP_Weather` (`utils/patches/patch_RoF2.conf`; inferred, 6.539).
pub const WEATHER_OPCODE: u16 = 0x661e;

/// The length of a `RoF2` door record, `rof2_structs.h`'s `Door_Struct`.
const DOOR_RECORD: usize = 100;
/// The longest ground object model name, the server's 32-byte field less
/// its terminator.
const LONGEST_MODEL: usize = 31;
/// What a ground object's body holds past its model name: the encoder
/// sizes it as the name's length plus `sizeof(Object_Struct) - 1`.
const GROUND_SPAWN_PAST_NAME: usize = 101;
/// What the encoder writes past the model name's terminator: the zone,
/// the instance, the drop id again, a zero word, seven floats and the
/// object type. The rest of the body is never written.
const GROUND_SPAWN_WRITTEN: usize = 44;
/// The length of the weather `EQEmu` sends at zone-in, a `Weather_Struct`
/// (`Client::Handle_Connect_OP_ZoneEntry`, `zone/client_packet.cpp`).
const ZONE_IN_WEATHER_LENGTH: usize = 12;

/// What one of this module's messages says; None for another opcode, and
/// for the 12-byte zone-in weather, as on Titanium.
///
/// # Errors
/// Refuses a body its reader refuses.
pub fn read(opcode: u16, body: &[u8]) -> Result<Option<WorldEvent>> {
    Ok(Some(match opcode {
        SPAWN_DOOR_OPCODE => WorldEvent::Doors(doors(body)?),
        MOVE_DOOR_OPCODE => WorldEvent::Doors(crate::doors::door_move(body)?),
        GROUND_SPAWN_OPCODE => WorldEvent::Objects(ground_object(body)?),
        CLICK_OBJECT_OPCODE => WorldEvent::Objects(crate::objects::click(body)?),
        WEATHER_OPCODE => match weather(body)? {
            Some(weather) => WorldEvent::Weather(weather),
            None => return Ok(None),
        },
        _ => return Ok(None),
    }))
}

/// A door table: `ENCODE(OP_SpawnDoor)`'s 100-byte records
/// (`EntityList::MakeDoorSpawnPacket`, `zone/entity.cpp`), with every field
/// up to the parameter at 64 where Titanium's is. The size is a u32 at 52,
/// the server's u16 widened, and reads as the server's value.
///
/// The four bytes at 56 and everything from 68 are not read: the encoder
/// writes bytes 80 to 86 only, as 0, 1, 0, 1, 0, 0 and 0, and carries
/// nothing from the server there; byte 87 and the 12 after it are not
/// written.
///
/// # Errors
/// Refuses what Titanium's door table refuses (a partial record, more than
/// 500 doors, a repeated id other than the static object's, a placement
/// that is not finite), and a size past 16 bits.
pub fn doors(body: &[u8]) -> Result<DoorUpdate> {
    crate::doors::door_table(body, DOOR_RECORD, |record| {
        let size = u32::from_le_bytes([record[52], record[53], record[54], record[55]]);
        u16::try_from(size).map_err(|_| anyhow!("invalid RoF2 door size"))
    })
}

/// A ground object: `ENCODE(OP_GroundSpawn)`'s variable record. The drop
/// id, the model name with its terminator, the zone and instance, the drop
/// id again, a zero word, the heading, two tilts, the size, y, x and z as
/// floats, and the object type. Gives Titanium's spawn for the same server
/// values.
///
/// The encoder sizes the body at the name's length plus 101 and writes 52
/// bytes fewer than that; the tail is never read. The zone, the instance,
/// the tilts and the size are not read either: Titanium's [`GroundObject`]
/// has no field for them.
///
/// # Errors
/// Refuses a model name that does not end inside the body, is longer than
/// 31 characters or breaks Titanium's text rule; a length other than the
/// name's plus 101; drop ids that disagree or are 0; and a placement that
/// is not finite.
pub fn ground_object(body: &[u8]) -> Result<ObjectUpdate> {
    let name_end = body
        .get(4..)
        .and_then(|rest| rest.iter().position(|byte| *byte == 0))
        .map(|end| 4 + end)
        .ok_or_else(|| anyhow!("invalid RoF2 ground object model"))?;
    let name = &body[4..name_end];
    ensure!(
        name.len() <= LONGEST_MODEL,
        "invalid RoF2 ground object model"
    );
    let model =
        crate::objects::text(name).ok_or_else(|| anyhow!("invalid RoF2 ground object model"))?;
    ensure!(
        body.len() == name.len() + GROUND_SPAWN_PAST_NAME,
        "invalid RoF2 ground object length"
    );
    let written = &body[name_end + 1..name_end + 1 + GROUND_SPAWN_WRITTEN];
    let word = |at: usize| u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
    let field = |at: usize| {
        u32::from_le_bytes([
            written[at],
            written[at + 1],
            written[at + 2],
            written[at + 3],
        ])
    };
    let drop_id = word(0);
    ensure!(
        drop_id != 0 && field(4) == drop_id,
        "invalid RoF2 ground object ID"
    );
    let position = Position {
        x: f32::from_bits(field(32)),
        y: f32::from_bits(field(28)),
        z: f32::from_bits(field(36)),
        heading: f32::from_bits(field(12)),
    };
    ensure!(
        [position.x, position.y, position.z, position.heading]
            .iter()
            .all(|value| value.is_finite()),
        "non-finite RoF2 ground object placement"
    );
    Ok(ObjectUpdate::Spawn(GroundObject {
        drop_id,
        model,
        position,
        object_type: field(40),
    }))
}

/// The weather: the server's 8 bytes, which `Zone::weatherSend`
/// (`zone/zone.cpp`) sends `RoF2` and Titanium alike, read as Titanium's.
/// The 12-byte form `EQEmu` sends at zone-in gives nothing, as on
/// Titanium; `RoF2`'s entry does not read it yet (6.539).
///
/// # Errors
/// Refuses any other length.
pub fn weather(body: &[u8]) -> Result<Option<crate::clock::Weather>> {
    if body.len() == ZONE_IN_WEATHER_LENGTH {
        return Ok(None);
    }
    crate::clock::weather(body).map(Some)
}

#[cfg(test)]
mod tests;
