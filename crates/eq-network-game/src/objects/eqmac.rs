//! `EQMac` ground-object placement, disappearance and pickup refusal.
//!
//! TAKP and Quarm use the packed `Object_Struct` and `ClickObject_Struct`
//! directly; the Mac patch does not translate them. The placement's model
//! and type do not prove that a player can obtain an item. Only the inventory
//! owner consumes the separate item packets; no item is synthesized here.

use super::{GroundObject, ObjectUpdate};
use crate::{command::EncodedCommand, world::Position};
use anyhow::{ensure, Result};

/// `OP_GroundSpawn`: one ground object appears.
pub const SPAWN_OPCODE: u16 = 0x2c40;
/// `OP_ClickObject`: a request, disappearance or refusal.
pub const CLICK_OPCODE: u16 = 0x2b40;

/// Encodes an already checked pickup using the `EQMac` click layout.
/// The response never substitutes for authoritative cursor item packets.
#[must_use]
pub fn pickup(drop_id: u32, player_id: u16) -> EncodedCommand {
    let mut packet = super::titanium_pickup(drop_id, player_id);
    packet.opcode = CLICK_OPCODE;
    packet
}

/// Reads `EQMac` ground placements and click results.
///
/// # Errors
/// Rejects wrong lengths, reserved object IDs, nonfinite placement and empty
/// or non-ASCII model names. A refusal's `u32::MAX` is never an object ID.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<ObjectUpdate>> {
    let word = |offset: usize| {
        u32::from_le_bytes([
            body[offset],
            body[offset + 1],
            body[offset + 2],
            body[offset + 3],
        ])
    };
    Ok(Some(match opcode {
        SPAWN_OPCODE => {
            ensure!(body.len() == 222, "invalid EQMac ground object length");
            let drop_id = word(12);
            ensure!(
                drop_id != 0 && drop_id != u32::MAX,
                "EQMac ground object without an ID"
            );
            let position = Position {
                heading: f32::from_bits(word(140)),
                z: f32::from_bits(word(144)),
                x: f32::from_bits(word(148)),
                y: f32::from_bits(word(152)),
            };
            ensure!(
                [position.x, position.y, position.z, position.heading]
                    .iter()
                    .all(|value| value.is_finite()),
                "non-finite EQMac ground object placement"
            );
            let model = super::text(&body[156..172])
                .ok_or_else(|| anyhow::anyhow!("invalid EQMac ground object model"))?;
            ObjectUpdate::Spawn(GroundObject {
                drop_id,
                model,
                position,
                object_type: word(212),
            })
        }
        CLICK_OPCODE => {
            ensure!(body.len() == 8, "invalid EQMac object click length");
            let drop_id = word(0);
            let player_id = word(4);
            if drop_id == u32::MAX {
                ObjectUpdate::Kept { player_id }
            } else {
                ensure!(drop_id != 0, "EQMac object removal without an ID");
                ObjectUpdate::Remove {
                    drop_id,
                    taken_by: (player_id != 0).then_some(player_id),
                }
            }
        }
        _ => return Ok(None),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::{ObjectKind, Objects};

    fn record(id: u32, model: &[u8]) -> [u8; 222] {
        let mut body = [0; 222];
        body[12..16].copy_from_slice(&id.to_le_bytes());
        body[156..156 + model.len()].copy_from_slice(model);
        for (offset, value) in [(140, 128.0f32), (144, 9.0), (148, 27.5), (152, -12.0)] {
            body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        body
    }

    #[test]
    fn server_placement_is_identity_and_visibility_not_an_inventory_grant() {
        let mut objects = Objects::default();
        let spawn = decode(SPAWN_OPCODE, &record(71, b"IT63_ACTORDEF"))
            .unwrap()
            .unwrap();
        objects.apply(&spawn);
        let object = &objects.entries()[&71];
        assert_eq!(object.kind(), ObjectKind::Item);
        assert_eq!(
            (object.position.x, object.position.y, object.position.z),
            (27.5, -12.0, 9.0)
        );
        let mut fixture = record(72, b"FORGE_ACTORDEF");
        fixture[212..216].copy_from_slice(&u32::MAX.to_le_bytes());
        objects.apply(&decode(SPAWN_OPCODE, &fixture).unwrap().unwrap());
        assert_eq!(objects.entries()[&72].object_type, u32::MAX);
        assert_eq!(objects.entries()[&72].kind(), ObjectKind::Fixture);
        let denied = [u32::MAX.to_le_bytes(), 7_u32.to_le_bytes()].concat();
        let kept = decode(CLICK_OPCODE, &denied).unwrap().unwrap();
        assert_eq!(kept, ObjectUpdate::Kept { player_id: 7 });
        objects.apply(&kept);
        assert_eq!(objects.entries().len(), 2);
        for player_id in [0_u32, 9] {
            let removed = decode(
                CLICK_OPCODE,
                &[71_u32.to_le_bytes(), player_id.to_le_bytes()].concat(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                removed,
                ObjectUpdate::Remove {
                    drop_id: 71,
                    taken_by: (player_id != 0).then_some(player_id)
                }
            );
            objects.apply(&removed);
            assert!(!objects.entries().contains_key(&71));
            objects.apply(&spawn);
        }
        assert_eq!(decode(super::super::SPAWN_OPCODE, &fixture).unwrap(), None);
        assert_eq!(pickup(71, 7).body, [71, 0, 0, 0, 7, 0, 0, 0]);
        assert_eq!(pickup(71, 7).opcode, CLICK_OPCODE);
    }

    #[test]
    fn malformed_placements_and_clicks_are_rejected() {
        let body = record(71, b"IT63_ACTORDEF");
        for length in 0..222 {
            assert!(decode(SPAWN_OPCODE, &body[..length]).is_err());
        }
        for padding in [1, 2] {
            // The client's opposite-direction drop has 224 bytes; that does
            // not make it a server placement. IDs/model remain valid here.
            assert!(decode(SPAWN_OPCODE, &[body.as_slice(), &vec![0; padding]].concat()).is_err());
        }
        for id in [0, u32::MAX] {
            assert!(decode(SPAWN_OPCODE, &record(id, b"IT63_ACTORDEF")).is_err());
        }
        for offset in [140, 144, 148, 152] {
            let mut invalid = body;
            invalid[offset..offset + 4].copy_from_slice(&f32::INFINITY.to_le_bytes());
            assert!(decode(SPAWN_OPCODE, &invalid).is_err());
        }
        assert!(decode(SPAWN_OPCODE, &record(71, b"")).is_err());
        assert!(decode(SPAWN_OPCODE, &record(71, &[0xff])).is_err());
        for length in [0, 1, 7, 9] {
            assert!(decode(CLICK_OPCODE, &vec![0; length]).is_err());
        }
        assert!(decode(CLICK_OPCODE, &[0; 8]).is_err());
    }
}
