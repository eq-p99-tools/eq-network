//! What `/played` reads of the Titanium profile (read R49-8 a): the
//! account's entitled days, the character's creation time and its played
//! minutes. The program copies the raw `OP_PlayerProfile` body before its
//! loader reads these, so the offsets are body offsets. The Mac-era
//! program's `/played` is unread, so its profile isn't read here.
use anyhow::Result;

/// The account's entitled time, in days (body +0x3314, copied at
/// 0x4ba16d); `/played` shows it in years.
const ENTITLED_DAYS: usize = 0x3314;
/// The character's creation time, Unix seconds (body +0x32ec, copied at
/// 0x4ba27c; inferred from its `localtime` use).
const CREATED: usize = 0x32ec;
/// The character's played minutes (body +0x32f4, copied at 0x4ba41f).
const PLAYED_MINUTES: usize = 0x32f4;

/// `/played`'s three fields as a profile gives them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct PlayedTime {
    /// The account's entitled time, in days. The program divides it by 365
    /// for its line (0xe24).
    pub entitled_days: u32,
    /// When the character was made, in Unix seconds (line 0x32a8).
    pub created: u32,
    /// Minutes the character has played (line 0x32af). The program adds one
    /// each played minute between profiles itself (0x422526); the host
    /// keeps that count.
    pub played_minutes: u32,
}

/// Reads `/played`'s fields from a Titanium profile.
///
/// # Errors
/// Rejects a profile too short to hold them.
pub fn titanium_profile(profile: &[u8]) -> Result<PlayedTime> {
    let word = |at: usize| {
        profile
            .get(at..at + 4)
            .map(|field| u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
            .ok_or_else(|| anyhow::anyhow!("profile too short for its played time"))
    };
    Ok(PlayedTime {
        entitled_days: word(ENTITLED_DAYS)?,
        created: word(CREATED)?,
        played_minutes: word(PLAYED_MINUTES)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_gives_played_times_three_words_at_their_body_offsets() {
        let mut profile = vec![0_u8; 19_592];
        profile[ENTITLED_DAYS..ENTITLED_DAYS + 4].copy_from_slice(&730_u32.to_le_bytes());
        profile[CREATED..CREATED + 4].copy_from_slice(&1_100_000_000_u32.to_le_bytes());
        profile[PLAYED_MINUTES..PLAYED_MINUTES + 4].copy_from_slice(&3_001_u32.to_le_bytes());
        assert_eq!(
            titanium_profile(&profile).unwrap(),
            PlayedTime {
                entitled_days: 730,
                created: 1_100_000_000,
                played_minutes: 3_001,
            }
        );
        assert!(titanium_profile(&profile[..PLAYED_MINUTES + 3]).is_err());
    }
}
