//! Effects the servers play on a spawn or on the player's screen: a spell's
//! nimbus, particles on a spawn, a shaking camera and sounds. Each comes in
//! one message, which the session's spawn table does not keep; how each
//! looks or sounds is the client's business.
//!
//! `EQEmu` sends a nimbus (`OP_SpellEffect`) to the whole zone as a spell
//! with one lands, to a player zoning in for each one on a spawn there, and
//! for quests; Titanium has no message that takes one away. Particles
//! (`OP_LevelAppearance`) play as an NPC rises in level and for the effects
//! `EQEmu` keeps on a spawn, which a game master's `#appearanceeffects` and
//! quests set and which it plays again after an illusion and to a player
//! zoning in. A camera shake (`OP_CameraEffect`) comes from a game master's
//! `#camerashake` and from quests, to one player, a zone or every zone. A
//! sound comes to one player: on Titanium a sound file, which only quests
//! send (`OP_PlayMP3`), and on `EQMac` a sound by number, from a game
//! master's `#playsound` and quests (`OP_PlaySound`).
//!
//! Layout reference: `EQEmu`'s `SpellEffect_Struct`, `LevelAppearance_Struct`,
//! `Camera_Struct` and `PlayMP3_Struct` (`common/eq_packet_structs.h`), which
//! Titanium's wire carries as built, and TAKP's and Quarm's `Sound_Struct`
//! (`common/eq_packet_structs.h`). The senders are `EQEmu`'s
//! `Mob::SendSpellEffect`, `Mob::SendLevelAppearance`,
//! `Mob::SendAppearanceEffect` and `Mob::CameraEffect` (`zone/mob.cpp`),
//! `EntityList::SendNimbusEffects` and `EntityList::CameraEffect`
//! (`zone/entity.cpp`) and `Client::PlayMP3` (`zone/client.cpp`), and TAKP's
//! and Quarm's `Client::SendSound` (`zone/client.cpp`).
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::time::Duration;

/// `OP_SpellEffect`: a spell's nimbus drawn on a spawn (Titanium).
pub const SPELL_EFFECT_OPCODE: u16 = 0x22c5;
/// `OP_LevelAppearance`: particles played on a spawn (Titanium).
pub const LEVEL_APPEARANCE_OPCODE: u16 = 0x358e;
/// `OP_CameraEffect`: the camera shakes (Titanium).
pub const CAMERA_EFFECT_OPCODE: u16 = 0x0937;
/// `OP_PlayMP3`: a sound file to play (Titanium).
pub const PLAY_MP3_OPCODE: u16 = 0x26ab;
/// `EQMac`'s `OP_PlaySound`: a sound to play, by number.
pub const EQMAC_PLAY_SOUND_OPCODE: u16 = 0x4840;

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
    /// Particles played on a spawn: as an NPC rises in level, and the
    /// effects `EQEmu` keeps on a spawn, which it plays again after an
    /// illusion and to a player zoning in.
    Particles {
        /// The spawn they play on.
        spawn_id: u16,
        /// Up to five, in the order sent; none when the record names no
        /// effect.
        effects: Vec<Particle>,
    },
    /// The camera shakes.
    CameraShake {
        /// How long it shakes.
        duration: Duration,
        /// How hard, in the wire's own units.
        intensity: f32,
    },
    /// A sound for the player alone.
    Sound(Cue),
}

/// One of a spawn's particles, with the two values sent with it, kept as
/// sent: `EQEmu`'s comments disagree on what they do. One says the first
/// places the effect on the spawn (0 to 9, such as 2 for the head) and a
/// second of 1 leaves it on the ground for good; the other says the first
/// does nothing and a second of 0 turns the effect off
/// (`Mob::SendAppearanceEffect`, `zone/mob.cpp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Particle {
    /// The effect's number; never 0, which names none.
    pub effect: u32,
    /// The first value: where on the spawn, by the first comment.
    pub slot: u32,
    /// The second value: on the ground for good when 1, by the first
    /// comment.
    pub ground: u32,
}

/// A sound the server asks the player's client to play.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Cue {
    /// A sound file, by name (Titanium's `OP_PlayMP3`).
    File(String),
    /// A sound by its number (`EQMac`'s `OP_PlaySound`).
    Number(u16),
}

/// What Titanium's effect messages say; None for other messages.
///
/// # Errors
/// Rejects malformed records of those messages.
pub(crate) fn titanium(opcode: u16, body: &[u8]) -> Result<Option<Effect>> {
    Ok(Some(match opcode {
        SPELL_EFFECT_OPCODE => titanium_nimbus(body)?,
        LEVEL_APPEARANCE_OPCODE => titanium_particles(body)?,
        CAMERA_EFFECT_OPCODE => titanium_camera_shake(body)?,
        PLAY_MP3_OPCODE => titanium_sound_file(body)?,
        _ => return Ok(None),
    }))
}

/// A little-endian word of a fixed-length record.
fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

/// A spawn named by a 32-bit word: never zero, and within 16 bits.
fn spawn(body: &[u8], at: usize, what: &str) -> Result<u16> {
    let spawn_id =
        u16::try_from(word(body, at)).with_context(|| format!("{what} spawn out of range"))?;
    ensure!(spawn_id != 0, "{what} without a spawn");
    Ok(spawn_id)
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
    Ok(Effect::Nimbus {
        effect: word(body, 0),
        caster_id: spawn(body, 4, "spell effect")?,
        target_id: spawn(body, 8, "spell effect")?,
        duration: Duration::from_millis(word(body, 12).into()),
        finish_delay: Duration::from_millis(word(body, 16).into()),
    })
}

/// Decodes Titanium's `OP_LevelAppearance` (64 bytes): the spawn, then five
/// particles, each an effect and its two values, all 32 bits. An effect of
/// 0 names none.
///
/// # Errors
/// Rejects a wrong length and a spawn that is zero or past 16 bits.
fn titanium_particles(body: &[u8]) -> Result<Effect> {
    ensure!(body.len() == 64, "invalid level appearance length");
    Ok(Effect::Particles {
        spawn_id: spawn(body, 0, "level appearance")?,
        effects: (0..5)
            .map(|index| 4 + index * 12)
            .map(|at| Particle {
                effect: word(body, at),
                slot: word(body, at + 4),
                ground: word(body, at + 8),
            })
            .filter(|particle| particle.effect != 0)
            .collect(),
    })
}

/// Decodes Titanium's `OP_CameraEffect` (8 bytes): how long the camera
/// shakes, in milliseconds, then how hard, a float.
///
/// # Errors
/// Rejects a wrong length and an intensity that is not a number.
fn titanium_camera_shake(body: &[u8]) -> Result<Effect> {
    ensure!(body.len() == 8, "invalid camera effect length");
    let intensity = f32::from_bits(word(body, 4));
    ensure!(intensity.is_finite(), "invalid camera shake intensity");
    Ok(Effect::CameraShake {
        duration: Duration::from_millis(word(body, 0).into()),
        intensity,
    })
}

/// Decodes Titanium's `OP_PlayMP3`: the sound file's name and its NUL,
/// which `EQEmu` sizes the record to fit.
///
/// # Errors
/// Rejects a record that does not end the name.
fn titanium_sound_file(body: &[u8]) -> Result<Effect> {
    ensure!(body.last() == Some(&0), "sound file name without its end");
    Ok(Effect::Sound(Cue::File(
        String::from_utf8_lossy(crate::world::until_nul(body)).into_owned(),
    )))
}

/// Decodes `EQMac`'s `OP_PlaySound` (4 bytes): the spawn of the player it is
/// sent to, which the session knows, then the sound's number in 16 bits.
/// `Sound_Struct` declares the number signed, but `Client::SendSound` fills
/// it from an unsigned number, so it is read unsigned, as the server means
/// it.
///
/// # Errors
/// Rejects a wrong length.
pub(crate) fn eqmac_sound(body: &[u8]) -> Result<Effect> {
    let bytes: [u8; 4] = body.try_into().context("invalid play sound length")?;
    Ok(Effect::Sound(Cue::Number(u16::from_le_bytes([
        bytes[2], bytes[3],
    ]))))
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
    fn particles_keep_each_named_effect_and_its_values_as_sent() {
        // An NPC rising in level: five effects, as EQEmu's
        // Mob::SendLevelAppearance fills them.
        let level_up = words(&[
            41, 0x4d, 1, 0, 0x4e, 2, 0, 0x4f, 1, 1, 0x50, 1, 1, 0x51, 2, 0,
        ]);
        let expected = [
            (0x4d, 1, 0),
            (0x4e, 2, 0),
            (0x4f, 1, 1),
            (0x50, 1, 1),
            (0x51, 2, 0),
        ]
        .map(|(effect, slot, ground)| Particle {
            effect,
            slot,
            ground,
        });
        assert_eq!(
            titanium_update(LEVEL_APPEARANCE_OPCODE, &level_up).unwrap(),
            Some(WorldEvent::Effect(Effect::Particles {
                spawn_id: 41,
                effects: expected.to_vec(),
            }))
        );
        // A quest's two effects, the second on the ground; the rest name
        // none.
        let quest = words(&[41, 0, 0, 0, 78, 2, 0, 0, 0, 0, 12, 9, 1, 0, 0, 0]);
        assert_eq!(
            titanium(LEVEL_APPEARANCE_OPCODE, &quest).unwrap(),
            Some(Effect::Particles {
                spawn_id: 41,
                effects: vec![
                    Particle {
                        effect: 78,
                        slot: 2,
                        ground: 0,
                    },
                    Particle {
                        effect: 12,
                        slot: 9,
                        ground: 1,
                    },
                ],
            })
        );
        assert!(titanium(LEVEL_APPEARANCE_OPCODE, &level_up[..63]).is_err());
        for spawn_id in [0, 70_000] {
            let mut bad = level_up.clone();
            bad[..4].copy_from_slice(&u32::to_le_bytes(spawn_id));
            assert!(
                titanium(LEVEL_APPEARANCE_OPCODE, &bad).is_err(),
                "{spawn_id}"
            );
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

    #[test]
    fn sounds_name_a_file_on_titanium_and_a_number_on_eqmac() {
        assert_eq!(
            titanium_update(PLAY_MP3_OPCODE, b"thunder.mp3\0").unwrap(),
            Some(WorldEvent::Effect(Effect::Sound(Cue::File(
                "thunder.mp3".into()
            ))))
        );
        // A quest may name no file; the record is then its NUL alone.
        assert_eq!(
            titanium(PLAY_MP3_OPCODE, &[0]).unwrap(),
            Some(Effect::Sound(Cue::File(String::new())))
        );
        assert!(titanium(PLAY_MP3_OPCODE, b"thunder.mp3").is_err());
        assert!(titanium(PLAY_MP3_OPCODE, &[]).is_err());
        // EQMac's: the player's own spawn, then the number, read as sent,
        // past the signed field's half too.
        assert_eq!(
            crate::quarm::updates(EQMAC_PLAY_SOUND_OPCODE, &[73, 0, 0x39, 0x01]).unwrap(),
            [WorldEvent::Effect(Effect::Sound(Cue::Number(313)))]
        );
        assert_eq!(
            eqmac_sound(&[73, 0, 0xff, 0xff]).unwrap(),
            Effect::Sound(Cue::Number(65_535))
        );
        assert!(eqmac_sound(&[73, 0, 0x39]).is_err());
    }
}
