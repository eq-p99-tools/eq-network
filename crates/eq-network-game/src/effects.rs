//! Effects the servers play on a spawn or on the player's screen: a spell's
//! nimbus and a shaking camera. Each comes in one message, which the
//! session's spawn table does not keep; how each looks is the client's
//! business.
//!
//! `EQEmu` sends a nimbus (`OP_SpellEffect`) to the whole zone as a spell
//! with one lands, to a player zoning in for each one on a spawn there, and
//! for quests; Titanium has no message that takes one away. A camera shake
//! (`OP_CameraEffect`) comes from a game master's `#camerashake` and from
//! quests, to one player, a zone or every zone.
//!
//! Layout reference: `EQEmu`'s `SpellEffect_Struct` and `Camera_Struct`
//! (`common/eq_packet_structs.h`), which Titanium's wire carries as built;
//! the senders are `Mob::SendSpellEffect` and `Mob::CameraEffect`
//! (`zone/mob.cpp`) and `EntityList::SendNimbusEffects` and
//! `EntityList::CameraEffect` (`zone/entity.cpp`).
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::time::Duration;

/// `OP_SpellEffect`: a spell's nimbus drawn on a spawn (Titanium).
pub const SPELL_EFFECT_OPCODE: u16 = 0x22c5;
/// `OP_CameraEffect`: the camera shakes (Titanium).
pub const CAMERA_EFFECT_OPCODE: u16 = 0x0937;

/// An effect the server plays, in one message.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Effect {
    /// A spell's effect drawn on a spawn: as a spell with one lands, for
    /// each one a spawn carries as the player zones in, and for quests.
    Nimbus {
        /// The effect's number.
        effect: u32,
        /// The spawn it plays from, which is the one it is drawn on unless
        /// a quest names another.
        caster_id: u16,
        /// The spawn it is drawn on.
        target_id: u16,
        /// How long it plays.
        duration: Duration,
        /// How long its last part waits.
        finish_delay: Duration,
    },
    /// The camera shakes.
    CameraShake {
        /// How long it shakes.
        duration: Duration,
        /// How hard, in the wire's own units.
        intensity: f32,
    },
}

/// What Titanium's effect messages say; None for other messages.
///
/// # Errors
/// Rejects malformed records of those messages.
pub(crate) fn titanium(opcode: u16, body: &[u8]) -> Result<Option<Effect>> {
    Ok(Some(match opcode {
        SPELL_EFFECT_OPCODE => titanium_nimbus(body)?,
        CAMERA_EFFECT_OPCODE => titanium_camera_shake(body)?,
        _ => return Ok(None),
    }))
}

/// Decodes Titanium's `OP_SpellEffect` (28 bytes): the effect, the spawn
/// it plays from and the one it is drawn on, each 32 bits, how long it
/// plays and how long its last part waits, in milliseconds, then a field
/// `EQEmu` sets to 3000 unless a quest gives another, and four bytes it
/// always sets to 1, 1 and 0.
///
/// # Errors
/// Rejects a wrong length and a spawn that is zero or past 16 bits.
fn titanium_nimbus(body: &[u8]) -> Result<Effect> {
    ensure!(body.len() == 28, "invalid spell effect length");
    let word = |at: usize| u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
    let spawn = |at: usize| -> Result<u16> {
        let spawn_id = u16::try_from(word(at)).context("spell effect spawn out of range")?;
        ensure!(spawn_id != 0, "spell effect without a spawn");
        Ok(spawn_id)
    };
    Ok(Effect::Nimbus {
        effect: word(0),
        caster_id: spawn(4)?,
        target_id: spawn(8)?,
        duration: Duration::from_millis(word(12).into()),
        finish_delay: Duration::from_millis(word(16).into()),
    })
}

/// Decodes Titanium's `OP_CameraEffect` (8 bytes): how long the camera
/// shakes, in milliseconds, then how hard, a float.
///
/// # Errors
/// Rejects a wrong length and an intensity that is not a number.
fn titanium_camera_shake(body: &[u8]) -> Result<Effect> {
    let bytes: [u8; 8] = body.try_into().context("invalid camera effect length")?;
    let intensity = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    ensure!(intensity.is_finite(), "invalid camera shake intensity");
    Ok(Effect::CameraShake {
        duration: Duration::from_millis(
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).into(),
        ),
        intensity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{titanium_update, WorldEvent};

    fn words(values: &[u32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    #[test]
    fn nimbuses_name_the_effect_its_spawns_and_its_timing() {
        let mut body = words(&[618, 73, 73, 500, 0, 3000]);
        body.extend_from_slice(&[1, 1, 0, 0]);
        assert_eq!(
            titanium_update(SPELL_EFFECT_OPCODE, &body).unwrap(),
            Some(WorldEvent::Effect(Effect::Nimbus {
                effect: 618,
                caster_id: 73,
                target_id: 73,
                duration: Duration::from_millis(500),
                finish_delay: Duration::ZERO,
            }))
        );
        // A quest's effect played from one spawn onto another.
        let mut quest = words(&[12, 7, 9, 5000, 250, 99]);
        quest.extend_from_slice(&[0; 4]);
        assert_eq!(
            titanium(SPELL_EFFECT_OPCODE, &quest).unwrap(),
            Some(Effect::Nimbus {
                effect: 12,
                caster_id: 7,
                target_id: 9,
                duration: Duration::from_secs(5),
                finish_delay: Duration::from_millis(250),
            })
        );
        assert!(titanium(SPELL_EFFECT_OPCODE, &body[..27]).is_err());
        for spawns in [[0, 73], [73, 0], [70_000, 73], [73, 70_000]] {
            let mut bad = words(&[618, spawns[0], spawns[1], 500, 0, 3000]);
            bad.extend_from_slice(&[1, 1, 0, 0]);
            assert!(titanium(SPELL_EFFECT_OPCODE, &bad).is_err(), "{spawns:?}");
        }
    }

    #[test]
    fn camera_shakes_last_and_shake_as_sent() {
        let mut body = 3000u32.to_le_bytes().to_vec();
        body.extend_from_slice(&5.0f32.to_le_bytes());
        assert_eq!(
            titanium_update(CAMERA_EFFECT_OPCODE, &body).unwrap(),
            Some(WorldEvent::Effect(Effect::CameraShake {
                duration: Duration::from_secs(3),
                intensity: 5.0,
            }))
        );
        assert!(titanium(CAMERA_EFFECT_OPCODE, &body[..7]).is_err());
        body[4..].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(titanium(CAMERA_EFFECT_OPCODE, &body).is_err());
        assert_eq!(titanium(0x1234, &body).unwrap(), None);
    }
}
