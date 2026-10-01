//! Titanium ground objects: items lying on the ground and world containers.
//!
//! Layouts follow `EQEmu`'s Titanium `Object_Struct` (`OP_GroundSpawn`),
//! `ClickObject_Struct` (`OP_ClickObject`) and `ClickObjectAck_Struct`
//! (`OP_ClickObjectAction`); `zone/object.cpp` decides what a click does.
use crate::{command::EncodedCommand, world::Position};
use anyhow::{anyhow, ensure, Result};
use serde::Serialize;
use std::collections::BTreeMap;

/// `OP_GroundSpawn`: an object appears.
pub const SPAWN_OPCODE: u16 = 0x0f47;
/// `OP_ClickObject`: a pickup request, and the server's notice that an object is gone.
pub const CLICK_OPCODE: u16 = 0x3bc2;
/// `OP_ClickObjectAction`: a world container opened for a click, or the client closing it.
pub const CONTAINER_OPCODE: u16 = 0x6937;

const SPAWN_LENGTH: usize = 92;
const CONTAINER_LENGTH: usize = 92;

/// What an object is, judged from its model, since servers do not say
/// reliably: P99 sends object type 0 for everything, containers included.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ObjectKind {
    /// An item model (`IT` and a number): a dropped item or ground spawn to pick up.
    Item,
    /// Any other model, such as a forge: a world container or fixture.
    Fixture,
}

/// An object the server placed in the zone.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GroundObject {
    /// Zone-scoped identifier.
    pub drop_id: u32,
    /// Actor model, such as `IT63_ACTORDEF`; a name, never a path.
    pub model: String,
    /// EQ coordinates of the object's base, and its 0..512 heading.
    pub position: Position,
    /// `EQEmu`'s object type: 0 or 1 for items and tradeskill kinds from 10 up.
    /// Dropped items and ground spawns carry 0, as does everything on P99.
    pub object_type: u32,
}

impl GroundObject {
    /// Whether this is an item to pick up or a fixture.
    #[must_use]
    pub fn kind(&self) -> ObjectKind {
        if self.object_type <= 1 && is_item_model(&self.model) {
            ObjectKind::Item
        } else {
            ObjectKind::Fixture
        }
    }
}

/// Item models are named `IT` and a number, with or without `_ACTORDEF`.
fn is_item_model(model: &str) -> bool {
    let name = model.trim().to_ascii_uppercase();
    let name = name.strip_suffix("_ACTORDEF").unwrap_or(&name);
    name.strip_prefix("IT")
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
}

/// Validated server object updates.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum ObjectUpdate {
    /// Every object in the zone at entry; replaces any earlier table.
    Snapshot(Vec<GroundObject>),
    /// One object appeared; a known ID is placed again (servers resend an
    /// object whose position or model changed).
    Spawn(GroundObject),
    /// The object is gone: picked up, decayed or removed.
    Remove {
        /// Object that left.
        drop_id: u32,
        /// Spawn ID of whoever picked it up; None when it decayed or was removed.
        taken_by: Option<u32>,
    },
    /// A pickup the server refused, so the object stays: `EQEmu` echoes the
    /// click with drop ID 0 when a quest keeps the item.
    Kept {
        /// Spawn ID of the player who clicked.
        player_id: u32,
    },
    /// A world container answered a click. Containers are not supported yet,
    /// so sessions close one that opens for them at once.
    Container(ContainerView),
}

/// A world container's answer to a click (`ClickObjectAck_Struct`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ContainerView {
    /// Spawn ID of the player who clicked.
    pub player_id: u32,
    /// Container object.
    pub drop_id: u32,
    /// False when someone else is using it.
    pub open: bool,
    /// Tradeskill kind, as in [`GroundObject::object_type`].
    pub object_type: u32,
    /// Icon the container window shows.
    pub icon: u32,
    /// Name the container window shows.
    pub name: String,
}

impl ContainerView {
    /// `OP_ClickObjectAction` closing this container: its own record with `open`
    /// cleared, which `EQEmu` answers by closing it (`Object::Close`).
    #[must_use]
    pub fn close_packet(&self) -> EncodedCommand {
        let mut body = vec![0; CONTAINER_LENGTH];
        for (offset, value) in [
            (0, self.player_id),
            (4, self.drop_id),
            (12, self.object_type),
            (16, 0x0a),
            (20, self.icon),
        ] {
            body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        let name = self.name.as_bytes();
        let length = name.len().min(63);
        body[28..28 + length].copy_from_slice(&name[..length]);
        EncodedCommand {
            opcode: CONTAINER_OPCODE,
            body,
        }
    }
}

/// Zone-local objects. Unknown IDs never create synthetic objects.
#[derive(Clone, Debug, Default)]
pub struct Objects(BTreeMap<u32, GroundObject>);

impl Objects {
    /// Local reach for picking items up, the same as for doors; not a claim
    /// about server rules (`EQEmu` checks no distance).
    pub const USE_DISTANCE: f32 = crate::doors::Doors::USE_DISTANCE;

    /// Current objects in stable ID order.
    #[must_use]
    pub fn entries(&self) -> &BTreeMap<u32, GroundObject> {
        &self.0
    }

    /// Applies an already validated update.
    pub fn apply(&mut self, update: &ObjectUpdate) {
        match update {
            ObjectUpdate::Snapshot(objects) => {
                self.0 = objects
                    .iter()
                    .map(|object| (object.drop_id, object.clone()))
                    .collect();
            }
            ObjectUpdate::Spawn(object) => {
                self.0.insert(object.drop_id, object.clone());
            }
            ObjectUpdate::Remove { drop_id, .. } => {
                self.0.remove(drop_id);
            }
            ObjectUpdate::Kept { .. } | ObjectUpdate::Container(_) => (),
        }
    }

    /// The whole table, for zone admission.
    #[must_use]
    pub fn admission(&self) -> ObjectUpdate {
        ObjectUpdate::Snapshot(self.0.values().cloned().collect())
    }

    /// Encodes picking an item up after checking the table and the player's reach.
    ///
    /// # Errors
    /// Rejects unknown objects, fixtures, invalid own IDs and non-finite or
    /// distant positions.
    pub fn pickup_packet(
        &self,
        drop_id: u32,
        player_id: u16,
        position: Position,
    ) -> Result<EncodedCommand> {
        let object = self
            .0
            .get(&drop_id)
            .ok_or_else(|| anyhow!("that is no longer there"))?;
        ensure!(
            object.kind() == ObjectKind::Item,
            "tradeskill containers are not supported yet"
        );
        ensure!(player_id != 0, "player is unavailable");
        let distance = (position.x - object.position.x)
            .hypot(position.y - object.position.y)
            .hypot(position.z - object.position.z);
        ensure!(
            distance.is_finite() && distance <= Self::USE_DISTANCE,
            "too far away to pick that up"
        );
        let mut body = vec![0; 8];
        body[..4].copy_from_slice(&drop_id.to_le_bytes());
        body[4..].copy_from_slice(&u32::from(player_id).to_le_bytes());
        Ok(EncodedCommand {
            opcode: CLICK_OPCODE,
            body,
        })
    }
}

/// Decodes Titanium object packets.
///
/// # Errors
/// Rejects wrong lengths, zero object IDs, non-finite placements and model
/// names that are empty or not plain ASCII.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<ObjectUpdate>> {
    let word = |offset: usize| {
        u32::from_le_bytes([
            body[offset],
            body[offset + 1],
            body[offset + 2],
            body[offset + 3],
        ])
    };
    match opcode {
        SPAWN_OPCODE => {
            // EQEmu sizes the packet by the model name's length; P99 sends 92.
            ensure!(body.len() >= SPAWN_LENGTH, "truncated ground object");
            let drop_id = word(12);
            ensure!(drop_id != 0, "ground object without an ID");
            let position = Position {
                x: f32::from_bits(word(36)),
                y: f32::from_bits(word(40)),
                z: f32::from_bits(word(32)),
                heading: f32::from_bits(word(28)),
            };
            ensure!(
                [position.x, position.y, position.z, position.heading]
                    .iter()
                    .all(|value| value.is_finite()),
                "non-finite ground object placement"
            );
            let model =
                text(&body[44..76]).ok_or_else(|| anyhow!("invalid ground object model"))?;
            Ok(Some(ObjectUpdate::Spawn(GroundObject {
                drop_id,
                model,
                position,
                object_type: word(80),
            })))
        }
        CLICK_OPCODE => {
            ensure!(body.len() == 8, "invalid object click length");
            let (drop_id, player_id) = (word(0), word(4));
            Ok(Some(if drop_id == 0 {
                ObjectUpdate::Kept { player_id }
            } else {
                ObjectUpdate::Remove {
                    drop_id,
                    taken_by: (player_id != 0).then_some(player_id),
                }
            }))
        }
        CONTAINER_OPCODE => {
            ensure!(
                body.len() == CONTAINER_LENGTH,
                "invalid container answer length"
            );
            Ok(Some(ObjectUpdate::Container(ContainerView {
                player_id: word(0),
                drop_id: word(4),
                open: word(8) != 0,
                object_type: word(12),
                icon: word(20),
                // A display name; an unreadable one is shown as nothing.
                name: text(&body[28..92]).unwrap_or_default(),
            })))
        }
        _ => Ok(None),
    }
}

/// A NUL-terminated name of printable ASCII, not blank. P99 names some
/// fixtures with spaces.
fn text(field: &[u8]) -> Option<String> {
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    let name = &field[..end];
    (name.iter().any(u8::is_ascii_graphic)
        && name
            .iter()
            .all(|byte| byte.is_ascii_graphic() || *byte == b' '))
    .then(|| String::from_utf8_lossy(name).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_body(drop_id: u32, model: &str, [x, y, z, heading]: [f32; 4]) -> Vec<u8> {
        let mut body = vec![0u8; SPAWN_LENGTH];
        body[12..16].copy_from_slice(&drop_id.to_le_bytes());
        for (offset, value) in [(28, heading), (32, z), (36, x), (40, y)] {
            body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        body[44..44 + model.len()].copy_from_slice(model.as_bytes());
        body[84..88].copy_from_slice(&u32::MAX.to_le_bytes());
        body
    }

    fn spawned(body: &[u8]) -> GroundObject {
        let Some(ObjectUpdate::Spawn(object)) = decode(SPAWN_OPCODE, body).unwrap() else {
            panic!("spawn")
        };
        object
    }

    #[test]
    fn ground_objects_decode_the_titanium_layout_at_either_length() {
        let body = spawn_body(71, "IT63_ACTORDEF", [12.5, -40.0, 3.25, 128.0]);
        let object = spawned(&body);
        assert_eq!(object.drop_id, 71);
        assert_eq!(object.model, "IT63_ACTORDEF");
        // The coordinates pass through unchanged, bit for bit.
        assert_eq!(
            [
                object.position.x,
                object.position.y,
                object.position.z,
                object.position.heading
            ]
            .map(f32::to_bits),
            [12.5_f32, -40.0, 3.25, 128.0].map(f32::to_bits)
        );
        assert_eq!(object.kind(), ObjectKind::Item);
        // EQEmu adds the name's length less one to the record.
        let mut longer = body.clone();
        longer.extend_from_slice(&[0; 12]);
        assert_eq!(spawned(&longer), object);
        for length in [0, 44, 91] {
            assert!(decode(SPAWN_OPCODE, &body[..length]).is_err());
        }
    }

    #[test]
    fn malformed_ground_objects_are_rejected() {
        assert!(decode(SPAWN_OPCODE, &spawn_body(0, "IT63_ACTORDEF", [0.0; 4])).is_err());
        assert!(decode(SPAWN_OPCODE, &spawn_body(1, "", [0.0; 4])).is_err());
        assert!(decode(SPAWN_OPCODE, &spawn_body(1, "   ", [0.0; 4])).is_err());
        assert!(decode(SPAWN_OPCODE, &spawn_body(1, "IT\t63", [0.0; 4])).is_err());
        // P99 names some fixtures with spaces; they are fixtures, not items.
        let named = spawned(&spawn_body(1, "A FIXTURE 1", [0.0; 4]));
        assert_eq!(named.kind(), ObjectKind::Fixture);
        let not_finite = spawn_body(1, "IT63_ACTORDEF", [f32::NAN, 0.0, 0.0, 0.0]);
        assert!(decode(SPAWN_OPCODE, &not_finite).is_err());
        assert!(decode(0x1234, &[]).unwrap().is_none());
    }

    #[test]
    fn only_item_models_of_item_types_are_items() {
        let mut object = spawned(&spawn_body(1, "IT10742_ACTORDEF", [0.0; 4]));
        assert_eq!(object.kind(), ObjectKind::Item);
        object.model = "it5".into();
        assert_eq!(object.kind(), ObjectKind::Item);
        object.object_type = 1;
        assert_eq!(object.kind(), ObjectKind::Item);
        // A tradeskill type is a container whatever its model.
        object.object_type = 17;
        assert_eq!(object.kind(), ObjectKind::Fixture);
        object.object_type = 0;
        for model in ["FORGE_ACTORDEF", "IT_ACTORDEF", "ITEM", "IT12B"] {
            object.model = model.into();
            assert_eq!(object.kind(), ObjectKind::Fixture, "{model}");
        }
    }

    #[test]
    fn clicks_remove_objects_or_report_a_kept_item() {
        let mut click = [0u8; 8];
        click[..4].copy_from_slice(&71u32.to_le_bytes());
        click[4..].copy_from_slice(&9u32.to_le_bytes());
        assert_eq!(
            decode(CLICK_OPCODE, &click).unwrap(),
            Some(ObjectUpdate::Remove {
                drop_id: 71,
                taken_by: Some(9)
            })
        );
        click[4..].fill(0);
        assert_eq!(
            decode(CLICK_OPCODE, &click).unwrap(),
            Some(ObjectUpdate::Remove {
                drop_id: 71,
                taken_by: None
            })
        );
        click[..4].fill(0);
        click[4..].copy_from_slice(&9u32.to_le_bytes());
        assert_eq!(
            decode(CLICK_OPCODE, &click).unwrap(),
            Some(ObjectUpdate::Kept { player_id: 9 })
        );
        assert!(decode(CLICK_OPCODE, &click[..7]).is_err());
    }

    #[test]
    fn the_table_follows_spawns_removals_and_admission() {
        let mut objects = Objects::default();
        let bag = spawned(&spawn_body(71, "IT63_ACTORDEF", [1.0, 2.0, 3.0, 0.0]));
        let forge = spawned(&spawn_body(72, "FORGE", [5.0, 2.0, 3.0, 0.0]));
        objects.apply(&ObjectUpdate::Spawn(bag.clone()));
        objects.apply(&ObjectUpdate::Spawn(forge.clone()));
        objects.apply(&ObjectUpdate::Kept { player_id: 9 });
        assert_eq!(objects.entries().len(), 2);
        let mut moved = bag.clone();
        moved.position.x = 4.0;
        objects.apply(&ObjectUpdate::Spawn(moved.clone()));
        assert_eq!(objects.entries()[&71], moved);
        let mut admitted = Objects::default();
        admitted.apply(&ObjectUpdate::Spawn(bag.clone()));
        admitted.apply(&objects.admission());
        assert_eq!(admitted.entries(), objects.entries());
        objects.apply(&ObjectUpdate::Remove {
            drop_id: 71,
            taken_by: Some(9),
        });
        assert_eq!(objects.entries().keys().copied().collect::<Vec<_>>(), [72]);
        objects.apply(&ObjectUpdate::Snapshot(Vec::new()));
        assert!(objects.entries().is_empty());
    }

    #[test]
    fn pickups_need_a_nearby_item_and_an_own_id() {
        let mut objects = Objects::default();
        objects.apply(&ObjectUpdate::Spawn(spawned(&spawn_body(
            71,
            "IT63_ACTORDEF",
            [10.0, 20.0, 5.0, 0.0],
        ))));
        objects.apply(&ObjectUpdate::Spawn(spawned(&spawn_body(
            72,
            "FORGE",
            [10.0, 20.0, 5.0, 0.0],
        ))));
        let near = Position {
            x: 12.0,
            y: 20.0,
            z: 8.0,
            heading: 0.0,
        };
        let packet = objects.pickup_packet(71, 0x1234, near).unwrap();
        assert_eq!(packet.opcode, CLICK_OPCODE);
        assert_eq!(packet.body, [71, 0, 0, 0, 0x34, 0x12, 0, 0]);
        assert!(objects.pickup_packet(72, 0x1234, near).is_err());
        assert!(objects.pickup_packet(73, 0x1234, near).is_err());
        assert!(objects.pickup_packet(71, 0, near).is_err());
        let mut far = near;
        far.x += Objects::USE_DISTANCE;
        assert!(objects.pickup_packet(71, 0x1234, far).is_err());
        far.x = f32::NAN;
        assert!(objects.pickup_packet(71, 0x1234, far).is_err());
    }

    #[test]
    fn a_container_that_opens_is_closed_with_its_own_record() {
        let mut body = [0u8; CONTAINER_LENGTH];
        for (offset, value) in [(0, 9u32), (4, 72), (8, 1), (12, 17), (16, 0x0a), (20, 1234)] {
            body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        body[28..33].copy_from_slice(b"Forge");
        let Some(ObjectUpdate::Container(view)) = decode(CONTAINER_OPCODE, &body).unwrap() else {
            panic!("container")
        };
        assert!(view.open);
        assert_eq!(
            (view.drop_id, view.icon, view.name.as_str()),
            (72, 1234, "Forge")
        );
        let close = view.close_packet();
        body[8..12].fill(0);
        assert_eq!(close.opcode, CONTAINER_OPCODE);
        assert_eq!(close.body, body);
        assert!(decode(CONTAINER_OPCODE, &body[..91]).is_err());
    }
}
