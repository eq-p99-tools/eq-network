//! Titanium's world-owned name generator (official client 0x4f0508).
//! `EQMac` generates names locally and has no corresponding mapped opcode.
use crate::command::EncodedCommand;
use anyhow::{ensure, Context, Result};

/// Titanium `OP_RandomNameGenerator`, a 72-byte request and response.
pub const OPCODE: u16 = 0x23d4;

/// Builds a suggestion request. It does not reserve or approve a name.
///
/// # Errors
/// Rejects non-player races and genders.
pub fn request(race: u32, gender: u32) -> Result<EncodedCommand> {
    ensure!(
        matches!(race, 1..=12 | 128 | 130 | 330) && gender <= 1,
        "invalid name generation race or gender"
    );
    let mut body = vec![0; 72];
    body[..4].copy_from_slice(&race.to_le_bytes());
    body[4..8].copy_from_slice(&gender.to_le_bytes());
    Ok(EncodedCommand {
        opcode: OPCODE,
        body,
    })
}

/// Reads a bounded, terminated suggestion for the requested race and gender.
///
/// # Errors
/// Rejects malformed packets and mismatched requests.
pub fn response(body: &[u8], race: u32, gender: u32) -> Result<String> {
    ensure!(body.len() == 72, "invalid name suggestion length");
    ensure!(
        body[..4] == race.to_le_bytes() && body[4..8] == gender.to_le_bytes(),
        "name suggestion does not match the requested race and gender"
    );
    let bytes = &body[8..];
    let end = bytes
        .iter()
        .position(|b| *b == 0)
        .context("unterminated name suggestion")?;
    ensure!(
        (4..=15).contains(&end) && bytes[..end].iter().all(u8::is_ascii_alphabetic),
        "invalid suggested name"
    );
    let mut name = String::from_utf8(bytes[..end].to_vec())?.to_ascii_lowercase();
    name[..1].make_ascii_uppercase();
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_round_trip_without_reserving_a_name() {
        let mut packet = request(330, 1).unwrap();
        assert_eq!(packet.opcode, 0x23d4);
        assert_eq!(packet.body.len(), 72);
        assert!(packet.body[8..].iter().all(|b| *b == 0));
        packet.body[8..16].copy_from_slice(b"TESTHERO");
        assert_eq!(response(&packet.body, 330, 1).unwrap(), "Testhero");
        assert!(response(&packet.body, 1, 1).is_err());
        assert!(response(&packet.body[..71], 330, 1).is_err());
        packet.body[8] = b'!';
        assert!(response(&packet.body, 330, 1).is_err());
        packet.body[8..].fill(b'a');
        assert!(response(&packet.body, 330, 1).is_err());
        assert!(request(999, 0).is_err());
        assert!(request(1, 2).is_err());
    }
}

/// How a policy obtains suggestions. Mac-era worlds do not map the wire opcode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    /// Ask the world server.
    World,
    /// Generate locally, then use normal name approval at submission.
    Local,
}

impl super::Rules {
    /// The generator used by this client generation.
    #[must_use]
    pub const fn name_source(self) -> Source {
        match self {
            Self::Classic | Self::Titanium => Source::World,
            Self::EqMac => Source::Local,
        }
    }
}

/// Creates a local, race-aware suggestion for Mac-era worlds. These phonemes
/// are original data, not the proprietary name tables in the official client.
/// Suggestions never imply availability; the world still approves the name.
///
/// # Errors
/// Rejects an unavailable race or gender.
pub fn local(race: u32, gender: u32) -> Result<String> {
    local_with(race, gender, &mut rand::thread_rng())
}

fn local_with(race: u32, gender: u32, rng: &mut impl rand::Rng) -> Result<String> {
    // Race families select sounds rather than a list of complete names.
    const FAMILIES: [&[&str]; 6] = [
        &["b", "d", "f", "l", "m", "n", "r", "s", "t", "v"],
        &["ae", "el", "il", "la", "ly", "na", "ri", "sa", "tha", "va"],
        &["br", "dr", "gr", "kr", "og", "th", "tr", "uk", "ur", "vr"],
        &[
            "ik", "iss", "ka", "kes", "sil", "ss", "tak", "tek", "xis", "za",
        ],
        &[
            "ah", "ja", "ka", "kir", "ma", "rah", "sha", "shir", "ta", "vah",
        ],
        &[
            "ba", "bor", "da", "dor", "ga", "gi", "gran", "ka", "nor", "tor",
        ],
    ];
    const RACES: [(u32, usize); 15] = [
        (1, 0),
        (2, 2),
        (3, 0),
        (4, 1),
        (5, 1),
        (6, 1),
        (7, 1),
        (8, 5),
        (9, 2),
        (10, 2),
        (11, 0),
        (12, 5),
        (128, 3),
        (130, 4),
        (330, 3),
    ];
    request(race, gender)?;
    let family = RACES
        .iter()
        .find(|(id, _)| *id == race)
        .map_or(0, |(_, f)| *f);
    let sounds = FAMILIES[family];
    let vowels = ["a", "e", "i", "o", "u"];
    let endings = [["n", "r", "s", "th", "m"], ["a", "e", "ia", "is", "yn"]];
    let mut name = String::new();
    for _ in 0..rng.gen_range(2..=3) {
        name.push_str(sounds[rng.gen_range(0..sounds.len())]);
        if !name.ends_with(['a', 'e', 'i', 'o', 'u']) {
            name.push_str(vowels[rng.gen_range(0..vowels.len())]);
        }
    }
    name.push_str(endings[usize::try_from(gender)?][rng.gen_range(0..5)]);
    name.truncate(15);
    name[..1].make_ascii_uppercase();
    Ok(name)
}

#[cfg(test)]
mod local_tests {
    use super::*;
    use rand::SeedableRng;
    #[test]
    fn every_playable_race_and_gender_gets_valid_varied_suggestions() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(81);
        for (race, _) in super::super::RACES {
            for gender in 0..=1 {
                let mut names = std::collections::BTreeSet::new();
                for _ in 0..100 {
                    let name = local_with(*race, gender, &mut rng).unwrap();
                    assert!((4..=15).contains(&name.len()));
                    assert!(name.as_bytes()[0].is_ascii_uppercase());
                    assert!(name.as_bytes()[1..].iter().all(u8::is_ascii_lowercase));
                    names.insert(name);
                }
                assert!(names.len() > 90);
            }
        }
    }
}
