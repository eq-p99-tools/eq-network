//! TAKP/EQMac world state, normalized to the same units as Titanium.
//!
//! Layout references: SecretsOTheP/EQMacEmu `common/patches/mac_structs.h`,
//! `mac.cpp`, `common/eq_packet_structs.h`, and `common/packet_functions.cpp`.
//! Decoders retain only presentation fields, never the full character profile.

use crate::world::{BaseAttributes, PlayerState, Position, SpawnKind, SpawnState, WorldEvent};
use anyhow::{ensure, Context, Result};
use zeroize::Zeroizing;

const PROFILE_SIZE: usize = 8460;
const SPAWN_SIZE: usize = 224;

/// Decrypt the full 64-bit words, preserve the tail, then inflate with a strict limit.
fn unpack(body: &[u8], profile: bool) -> Result<Zeroizing<Vec<u8>>> {
    ensure!(
        (8..=65536).contains(&body.len()),
        "invalid EQMac compressed length"
    );
    let mut bytes = Zeroizing::new(body.to_vec());
    let (constant, mut key, first_rotation, second_rotation) = if profile {
        (0x4224_37a9u64, 0x6593_65e7u64, 7, 39)
    } else {
        (0x6593_65e7u64, 0, 14, 29)
    };
    let (words, _) = bytes.as_chunks_mut::<8>();
    for word in words.iter_mut() {
        let plain = u64::from_le_bytes(*word)
            .wrapping_add(key)
            .rotate_right(first_rotation)
            .wrapping_sub(constant)
            .rotate_right(second_rotation);
        key = key.wrapping_add(plain).wrapping_sub(constant);
        *word = plain.to_le_bytes();
    }
    words.swap(0, words.len() / 2);
    let limit = if profile {
        PROFILE_SIZE
    } else {
        SPAWN_SIZE * 4096
    };
    miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&bytes, limit)
        .map(Zeroizing::new)
        .map_err(|_| anyhow::anyhow!("invalid or oversized EQMac compressed payload"))
}

/// Decode only the rendering fields of the encrypted player profile.
///
/// # Errors
/// Rejects malformed compression, unexpected layouts, and a different character.
/// The returned spawn ID is zero until the session receives the `SpawnID` appearance.
pub fn profile(body: &[u8], character: &str) -> Result<PlayerState> {
    let data = unpack(body, true)?;
    decoded_profile(&data, character)
}

/// Projects validated profile fields without retaining the decompressed payload.
fn decoded_profile(data: &[u8], character: &str) -> Result<PlayerState> {
    ensure!(
        data.len() == PROFILE_SIZE,
        "unexpected EQMac profile layout"
    );
    ensure!(
        cstr(&data[6..70]).eq_ignore_ascii_case(character.as_bytes()),
        "profile character mismatch"
    );
    Ok(PlayerState {
        name: String::from_utf8_lossy(cstr(&data[6..70])).into_owned(),
        base_attributes: Some(BaseAttributes {
            strength: signed_attribute(data, 164),
            stamina: signed_attribute(data, 166),
            charisma: signed_attribute(data, 168),
            dexterity: signed_attribute(data, 170),
            intelligence: signed_attribute(data, 172),
            agility: signed_attribute(data, 174),
            wisdom: signed_attribute(data, 176),
        }),
        deity: None,
        class: Some(u32::from(short(data, 144))),
        spawn_id: 0,
        race: u32::from(short(data, 142)),
        gender: u32::from(data[140]),
        level: data[148],
        position: Position {
            x: float(data, 2908)?,
            y: float(data, 2904)?,
            z: float(data, 2912)?,
            heading: (float(data, 2916)? * 2.0).rem_euclid(512.0),
        },
        mana: u32::from(short(data, 158)),
        // EQMac uses fatigue, not Titanium endurance; no equivalent value is supplied.
        endurance: None,
        skills: None,
        spell_refresh_ms: None,
        memorized_spells: std::array::from_fn(|i| {
            let id = short(data, 2870 + i * 2);
            (id != 0 && id != u16::MAX).then_some(u32::from(id))
        }),
        size: 0.0,
        walk_speed: 0.0,
        run_speed: 0.0,
        hp_percent: None,
    })
}

/// Reads a signed `EQMac` profile attribute after the profile length check.
fn signed_attribute(data: &[u8], offset: usize) -> i32 {
    i32::from(i16::from_le_bytes([data[offset], data[offset + 1]]))
}

/// The separate, uncompressed own-character zone-entry projection.
#[derive(Clone, Debug)]
pub struct OwnSpawn {
    /// Corrected zone-entry position, which can differ from the saved profile.
    pub position: Position,
    /// Server model dimensions.
    pub size: f32,
    /// Protocol walk speed, not world units per second.
    pub walk_speed: f32,
    /// Protocol run speed, not world units per second.
    pub run_speed: f32,
}

/// Decode the own-character entry without retaining names or unused profile bytes.
///
/// # Errors
/// Rejects wrong lengths, character mismatch, and invalid dimensions or positions.
pub fn own_spawn(body: &[u8], character: &str) -> Result<OwnSpawn> {
    ensure!(body.len() == 356, "unexpected EQMac own-spawn layout");
    ensure!(
        cstr(&body[5..69]).eq_ignore_ascii_case(character.as_bytes()),
        "own-spawn character mismatch"
    );
    Ok(OwnSpawn {
        position: Position {
            x: float(body, 80)?,
            y: float(body, 76)?,
            z: float(body, 84)?,
            heading: (float(body, 88)? * 2.0).rem_euclid(512.0),
        },
        size: nonnegative(body, 244)?,
        walk_speed: nonnegative(body, 260)?,
        run_speed: nonnegative(body, 264)?,
    })
}

/// Decode a compressed initial or incremental spawn batch.
///
/// # Errors
/// Rejects invalid compression, partial records, zero IDs, and invalid model sizes.
pub fn spawns(body: &[u8]) -> Result<Vec<SpawnState>> {
    let data = unpack(body, false)?;
    ensure!(
        !data.is_empty() && data.len().is_multiple_of(SPAWN_SIZE),
        "partial EQMac spawn batch"
    );
    data.as_chunks::<SPAWN_SIZE>()
        .0
        .iter()
        .map(|record| {
            let spawn_id = valid_id(u32::from(short(record, 76)))?;
            Ok(SpawnState {
                class: None,
                spawn_id,
                name: String::from_utf8_lossy(cstr(&record[127..191])).into_owned(),
                kind: match record[86] {
                    0 | 10 => SpawnKind::Player,
                    1 => SpawnKind::Npc,
                    2 => SpawnKind::PlayerCorpse,
                    3 => SpawnKind::NpcCorpse,
                    other => SpawnKind::Unknown(other),
                },
                race: u32::from(short(record, 84)),
                gender: u32::from(record[88]),
                size: nonnegative(record, 28)?,
                invisible: record[90] != 0,
                position: Position {
                    x: signed(record, 9),
                    y: signed(record, 7),
                    z: signed(record, 11) / 10.0,
                    heading: f32::from(record[5]) * 2.0,
                },
                // EQMac motion fields are not decoded yet.
                velocity: [0.0; 3],
            })
        })
        .collect()
}

/// Decode a single or batched authoritative position update.
fn position(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 15, "unexpected EQMac position layout");
    Ok(WorldEvent::Position {
        spawn_id: valid_id(u32::from(short(body, 0)))?,
        position: Position {
            x: signed(body, 7),
            y: signed(body, 5),
            z: signed(body, 9) / 10.0,
            heading: f32::from(body[3]) * 2.0,
        },
        // EQMac motion fields are not decoded yet.
        velocity: [0.0; 3],
    })
}

/// Decode ongoing entity and HUD packets; unknown opcodes produce no events.
///
/// # Errors
/// Rejects malformed recognized packets, invalid IDs, and invalid health values.
pub fn updates(opcode: u16, body: &[u8]) -> Result<Vec<WorldEvent>> {
    Ok(match opcode {
        0x5f41 | 0x6b42 => vec![WorldEvent::Spawns(spawns(body)?)],
        0xf340 => vec![position(body)?],
        0x9f40 => {
            ensure!(body.len() >= 4, "truncated EQMac movement batch");
            let count = usize::try_from(word(body, 0))?;
            ensure!(
                count <= 4096 && body.len() == 4 + count * 15,
                "invalid EQMac movement batch length"
            );
            body[4..]
                .as_chunks::<15>()
                .0
                .iter()
                .map(|record| position(record))
                .collect::<Result<_>>()?
        }
        0x2940 => {
            ensure!(body.len() == 2, "invalid EQMac despawn length");
            vec![WorldEvent::Despawn(valid_id(u32::from(short(body, 0)))?)]
        }
        0xb240 => {
            ensure!(body.len() == 12, "invalid EQMac health length");
            let spawn_id = valid_id(word(body, 0))?;
            let current = i32::from_le_bytes(body[4..8].try_into()?);
            let maximum = i32::from_le_bytes(body[8..12].try_into()?);
            ensure!(
                maximum > 0 && current <= maximum,
                "invalid EQMac health values"
            );
            let percent = u8::try_from(i64::from(current.max(0)) * 100 / i64::from(maximum))?;
            vec![
                WorldEvent::HitPoints {
                    spawn_id,
                    current,
                    maximum,
                    without_items: false,
                },
                WorldEvent::HealthPercent { spawn_id, percent },
            ]
        }
        0x7f41 => {
            ensure!(body.len() == 4, "invalid EQMac mana update");
            vec![WorldEvent::Mana(u32::from(short(body, 0)))]
        }
        0xf540 => crate::world::appearance(body)?.into_iter().collect(),
        0x9941 => {
            ensure!(
                body.len() == 4 && word(body, 0) <= 330,
                "invalid EQMac experience update"
            );
            vec![WorldEvent::Experience(word(body, 0))]
        }
        _ => Vec::new(),
    })
}

/// Read the separate own-spawn ID announcement, leaving unrelated appearances alone.
///
/// # Errors
/// Rejects truncated appearance packets and IDs outside the nonzero u16 range.
pub fn assigned_id(body: &[u8]) -> Result<Option<u16>> {
    ensure!(body.len() == 8, "invalid EQMac appearance length");
    if short(body, 2) == 16 {
        Ok(Some(valid_id(word(body, 4))?))
    } else {
        Ok(None)
    }
}

fn valid_id(value: u32) -> Result<u16> {
    let id = u16::try_from(value).context("EQMac spawn ID out of range")?;
    ensure!(id != 0, "zero EQMac spawn ID");
    Ok(id)
}
fn cstr(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())]
}
fn short(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}
fn signed(bytes: &[u8], at: usize) -> f32 {
    f32::from(i16::from_le_bytes([bytes[at], bytes[at + 1]]))
}
fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("validated layout"))
}
fn float(bytes: &[u8], at: usize) -> Result<f32> {
    let value = f32::from_le_bytes(bytes[at..at + 4].try_into()?);
    ensure!(
        value.is_finite(),
        "non-finite EQMac coordinate or dimension"
    );
    Ok(value)
}
fn nonnegative(bytes: &[u8], at: usize) -> Result<f32> {
    let value = float(bytes, at)?;
    ensure!(value >= 0.0, "negative EQMac dimension or speed");
    Ok(value)
}

#[cfg(test)]
mod tests;
