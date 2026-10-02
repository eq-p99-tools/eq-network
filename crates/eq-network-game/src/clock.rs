//! The time of day in Norrath and how a zone's sky looks. Servers send the
//! time as a zone admits the player and whenever someone changes it
//! (`OP_TimeOfDay`); between those the clock runs on its own, a Norrath
//! minute every three real seconds. A zone's header (`OP_NewZone`) names its
//! sky and gives four fog colors and distances.
//!
//! Layout reference: `EQEmu`'s `TimeOfDay_Struct` and Titanium
//! `NewZone_Struct` (`common/patches/titanium_structs.h`), and
//! `EQTime::GetCurrentEQTimeOfDay` (`common/eqtime.cpp`) for the calendar:
//! hours 1 to 24 on the wire, 28 days to a month, 12 months to a year.
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_TimeOfDay`: the time in Norrath.
pub const TIME_OPCODE: u16 = 0x1580;
/// Real seconds to a Norrath minute.
pub const SECONDS_PER_MINUTE: u64 = 3;
const MINUTES_PER_HOUR: u64 = 60;
const HOURS_PER_DAY: u64 = 24;
const DAYS_PER_MONTH: u64 = 28;
const MONTHS_PER_YEAR: u64 = 12;

/// A time in Norrath.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct GameTime {
    /// The hour, from 0 at midnight to 23.
    pub hour: u8,
    /// The minute, from 0 to 59.
    pub minute: u8,
    /// The day of the month, from 1 to 28.
    pub day: u8,
    /// The month, from 1 to 12.
    pub month: u8,
    /// The year.
    pub year: u32,
}

impl GameTime {
    /// The time once some real seconds have passed.
    #[must_use]
    pub fn after(self, seconds: u64) -> Self {
        let minutes = seconds / SECONDS_PER_MINUTE + u64::from(self.minute);
        let hours = minutes / MINUTES_PER_HOUR + u64::from(self.hour);
        let days = hours / HOURS_PER_DAY + u64::from(self.day.saturating_sub(1));
        let months = days / DAYS_PER_MONTH + u64::from(self.month.saturating_sub(1));
        let narrow = |value: u64, range: u64| u8::try_from(value % range).unwrap_or_default();
        Self {
            hour: narrow(hours, HOURS_PER_DAY),
            minute: narrow(minutes, MINUTES_PER_HOUR),
            day: narrow(days, DAYS_PER_MONTH) + 1,
            month: narrow(months, MONTHS_PER_YEAR) + 1,
            year: self
                .year
                .saturating_add(u32::try_from(months / MONTHS_PER_YEAR).unwrap_or(u32::MAX)),
        }
    }
}

/// Decodes `OP_TimeOfDay`, whose hours run from 1 (midnight) to 24.
///
/// # Errors
/// Rejects a malformed length.
pub fn decode(body: &[u8]) -> Result<GameTime> {
    ensure!(body.len() == 8, "invalid time of day length");
    Ok(GameTime {
        hour: (body[0] % 24 + 23) % 24,
        minute: body[1] % 60,
        day: body[2].clamp(1, 28),
        month: body[3].clamp(1, 12),
        year: u32::from_le_bytes([body[4], body[5], body[6], body[7]]),
    })
}

/// One of a zone's fog settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Fog {
    /// Red, green and blue.
    pub color: [u8; 3],
    /// Where it begins.
    pub near: f32,
    /// Where it hides everything.
    pub far: f32,
}

/// How a zone's sky and fog look, from its header.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ZoneSky {
    /// The sky the zone shows; zero for none, as below ground.
    pub sky: u8,
    /// The zone's time type.
    pub time_type: u8,
    /// The four fog settings, the first the zone's usual one.
    pub fog: [Fog; 4],
}

/// Reads the sky and fog from a Titanium zone header (`OP_NewZone`): fog
/// colors at 375, 379 and 383, fog distances at 388 and 404, the time type
/// at 424 and the sky at 474. None when the header is too short.
#[must_use]
pub fn titanium_zone_sky(new_zone: &[u8]) -> Option<ZoneSky> {
    let header = new_zone.get(..475)?;
    let float = |offset: usize| {
        let value = f32::from_le_bytes([
            header[offset],
            header[offset + 1],
            header[offset + 2],
            header[offset + 3],
        ]);
        if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        }
    };
    Some(ZoneSky {
        sky: header[474],
        time_type: header[424],
        fog: std::array::from_fn(|index| Fog {
            color: [
                header[375 + index],
                header[379 + index],
                header[383 + index],
            ],
            near: float(388 + index * 4),
            far: float(404 + index * 4),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_reads_from_the_wire_and_runs_on() {
        let time = decode(&[13, 30, 28, 12, 0xe8, 0x0c, 0, 0]).unwrap();
        assert_eq!(
            time,
            GameTime {
                hour: 12,
                minute: 30,
                day: 28,
                month: 12,
                year: 3304,
            }
        );
        assert_eq!(decode(&[24, 0, 1, 1, 0, 0, 0, 0]).unwrap().hour, 23);
        assert_eq!(decode(&[1, 0, 1, 1, 0, 0, 0, 0]).unwrap().hour, 0);
        assert!(decode(&[1, 0, 1, 1]).is_err());
        // Ninety real seconds is thirty minutes.
        assert_eq!(time.after(90).hour, 13);
        assert_eq!(time.after(90).minute, 0);
        // Half a real day later: midnight at the turn of the year.
        let new_year = time.after(3 * 60 * 11 + 90);
        assert_eq!(
            (
                new_year.hour,
                new_year.minute,
                new_year.day,
                new_year.month,
                new_year.year
            ),
            (0, 0, 1, 1, 3305)
        );
    }

    #[test]
    fn the_zone_header_says_how_the_sky_and_fog_look() {
        let mut header = vec![0; 700];
        header[474] = 1;
        header[424] = 2;
        header[375..387].copy_from_slice(&[200, 1, 2, 3, 150, 1, 2, 3, 100, 1, 2, 3]);
        header[388..392].copy_from_slice(&10.0f32.to_le_bytes());
        header[404..408].copy_from_slice(&900.0f32.to_le_bytes());
        let sky = titanium_zone_sky(&header).unwrap();
        assert_eq!(sky.sky, 1);
        assert_eq!(sky.time_type, 2);
        assert_eq!(
            sky.fog[0],
            Fog {
                color: [200, 150, 100],
                near: 10.0,
                far: 900.0,
            }
        );
        assert_eq!(sky.fog[1].color, [1, 1, 1]);
        assert!(titanium_zone_sky(&header[..474]).is_none());
    }
}
