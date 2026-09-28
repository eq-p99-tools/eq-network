//! Character-select lists contain fixed name slots, not searchable packet text.
use crate::GameDialect;
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// One occupied server-side character slot.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CharacterChoice {
    /// Slot in the server list; empty slots retain their indexes.
    pub slot: u8,
    /// Exact server spelling used for world entry.
    pub name: String,
    /// Level when supplied by a verified dialect layout.
    pub level: Option<u8>,
    /// Current zone identifier when supplied by a verified layout.
    pub zone_id: Option<u32>,
}

/// Decodes occupied slots and rejects malformed names or ambiguous duplicates.
///
/// # Errors
/// Rejects truncated records, unterminated/non-ASCII names and duplicate names.
pub fn decode(dialect: GameDialect, body: &[u8]) -> Result<Vec<CharacterChoice>> {
    let offset = match dialect {
        GameDialect::TitaniumP99 => {
            ensure!(body.len() == 1704, "invalid Titanium character list length");
            1024
        }
        GameDialect::EqMac => {
            ensure!(body.len() >= 640, "truncated EQMac character list");
            0
        }
    };
    let mut choices: Vec<CharacterChoice> = Vec::new();
    for (index, field) in body[offset..offset + 640]
        .as_chunks::<64>()
        .0
        .iter()
        .enumerate()
    {
        let end = field
            .iter()
            .position(|byte| *byte == 0)
            .context("unterminated character name")?;
        if end == 0 {
            continue;
        }
        let name = &field[..end];
        if name == b"<none>" {
            continue;
        }
        ensure!(
            name.iter().all(u8::is_ascii_alphabetic),
            "invalid character name"
        );
        let name = std::str::from_utf8(name)?.to_owned();
        ensure!(
            !choices
                .iter()
                .any(|choice| choice.name.eq_ignore_ascii_case(&name)),
            "duplicate character name"
        );
        let titanium = dialect == GameDialect::TitaniumP99;
        choices.push(CharacterChoice {
            slot: u8::try_from(index)?,
            name,
            level: titanium.then(|| body[1694 + index]),
            zone_id: if titanium {
                Some(u32::from_le_bytes(
                    body[964 + index * 4..968 + index * 4].try_into()?,
                ))
            } else {
                None
            },
        });
    }
    Ok(choices)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_only_come_from_occupied_slots_and_preserve_server_spelling() {
        let mut body = vec![0; 1704];
        body[10..15].copy_from_slice(b"Decoy");
        body[1024 + 192..1024 + 199].copy_from_slice(b"Example");
        body[1697] = 12;
        body[976..980].copy_from_slice(&22u32.to_le_bytes());
        let entries = decode(GameDialect::TitaniumP99, &body).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            (
                entries[0].slot,
                entries[0].name.as_str(),
                entries[0].level,
                entries[0].zone_id
            ),
            (3, "Example", Some(12), Some(22))
        );
        assert!(decode(GameDialect::TitaniumP99, &body[..1703]).is_err());
        body[1024..1031].copy_from_slice(b"example");
        assert!(decode(GameDialect::TitaniumP99, &body).is_err());
        let mut mac = vec![0; 1620];
        mac[64..71].copy_from_slice(b"Example");
        let entries = decode(GameDialect::EqMac, &mac).unwrap();
        assert_eq!(
            (entries[0].slot, entries[0].level, entries[0].zone_id),
            (1, None, None)
        );
        mac[..64].fill(b'A');
        assert!(decode(GameDialect::EqMac, &mac).is_err());
    }
}
