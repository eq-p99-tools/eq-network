//! Zone metadata from Titanium's 700-byte `NewZone_Struct`. Unknown bytes and
//! the redundant character name are not exposed. These are server settings,
//! not instructions to move to the safe point or apply a guessed gravity scale.
use crate::{
    clock::{self, ZoneSky},
    wire_read::word,
};
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// `OP_NewZone`, describing the zone during admission.
pub const TITANIUM_OPCODE: u16 = 0x0920;

/// The zone's four seasonal rain and snow settings, in the wire's units.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct SeasonalWeather {
    /// Rain chance, unscaled.
    pub rain_chance: u8,
    /// Rain duration; no time unit is inferred.
    pub rain_duration: u8,
    /// Snow chance, unscaled.
    pub snow_chance: u8,
    /// Snow duration; no time unit is inferred.
    pub snow_duration: u8,
}

/// Zone settings separate from the player's position and admission state.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ZoneEnvironment {
    /// Asset/zone short name.
    pub short_name: String,
    /// Display name.
    pub long_name: String,
    /// Server zone number.
    pub zone_id: u16,
    /// Instance number, normally zero on P99.
    pub instance_id: u16,
    /// Zone type, retained even when unknown.
    pub zone_type: u8,
    /// Sky and four fog settings.
    pub sky: ZoneSky,
    /// Gravity setting in the wire's units.
    pub gravity: f32,
    /// Experience multiplier reported in this record.
    pub experience_multiplier: f32,
    /// Zone safe point, normalized to X, Y, Z. This is not the player's position.
    pub safe_position: [f32; 3],
    /// Minimum view distance.
    pub near_clip: f32,
    /// Maximum view distance.
    pub far_clip: f32,
    /// Seasonal settings, not current precipitation.
    pub seasons: [SeasonalWeather; 4],
}

/// Reads known Titanium zone settings and normalizes the safe-point axes.
///
/// # Errors
/// Rejects an incorrect record length or non-finite floats in interpreted fields.
pub fn titanium(body: &[u8]) -> Result<ZoneEnvironment> {
    ensure!(
        body.len() == 700,
        "invalid Titanium zone description length"
    );
    let float = |at| {
        let value = f32::from_bits(word(body, at));
        ensure!(value.is_finite(), "non-finite zone setting");
        Ok(value)
    };
    for offset in (388..420).step_by(4) {
        let _ = float(offset)?;
    }
    let text = |start, end| {
        let bytes = &body[start..end];
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    };
    Ok(ZoneEnvironment {
        short_name: text(64, 96),
        long_name: text(96, 374),
        zone_id: u16::from_le_bytes([body[684], body[685]]),
        instance_id: u16::from_le_bytes([body[686], body[687]]),
        zone_type: body[374],
        sky: clock::titanium_zone_sky(body).context("missing zone sky")?,
        gravity: float(420)?,
        experience_multiplier: float(488)?,
        safe_position: [float(496)?, float(492)?, float(500)?],
        near_clip: float(512)?,
        far_clip: float(516)?,
        seasons: std::array::from_fn(|i| SeasonalWeather {
            rain_chance: body[425 + i],
            rain_duration: body[429 + i],
            snow_chance: body[433 + i],
            snow_duration: body[437 + i],
        }),
    })
}
