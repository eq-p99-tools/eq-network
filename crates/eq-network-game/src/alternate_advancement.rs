//! Read-only Titanium AA selections and rank definitions. No purchases or activations.
//!
//! Layouts follow `EQEmu` `common/patches/titanium_structs.h` and the
//! `OP_SendAATable`/`OP_RespondAA` encoders in `common/patches/titanium.cpp`.
//! The native environmental cache (0x40feaa) selects definitions by exact ID,
//! checks class and required level, and adds their signed effect values.
use anyhow::{ensure, Result};

/// A received rank definition, not an owned-rank snapshot.
pub const DEFINITION_OPCODE: u16 = 0x367d;
/// Replaces the profile's selected AA records.
pub const SELECTIONS_OPCODE: u16 = 0x3af4;
/// Withdraws the previous AA view; a replacement selection array must follow.
pub const CLEAR_OPCODE: u16 = 0x5918;
/// The number of eight-byte entries in Titanium's selection array.
pub const SELECTION_SLOTS: usize = 240;
/// Local bound on one definition's effect list; excess remains unavailable.
pub const MAX_EFFECTS: usize = 256;

/// A nonempty entry of a complete received selection array.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Selection {
    /// Exact selected definition ID, never inferred from an ability sequence.
    pub id: u32,
    /// Raw profile rank value, or `RespondAA` total cost. Not an effect multiplier.
    pub value: u32,
}

/// One definition's raw effect record, including unsupported identifiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Effect {
    /// Spell-effect identifier.
    pub id: u32,
    /// Signed effect magnitude supplied by the server.
    pub value: i32,
    /// Effect-specific secondary parameter.
    pub limit: i32,
    /// Raw effect slot number.
    pub slot: u32,
}

/// A received Titanium rank definition.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Rank {
    /// Exact definition ID.
    pub id: u32,
    /// Required character level, named `class_type` in the wire structure.
    pub required_level: u32,
    /// Ability sequence; it does not select this rank.
    pub sequence: u32,
    /// Rank number supplied in `current_level`.
    pub rank: u32,
    /// Native class mask uses bit `1 << class`, with classes 1 through 16.
    pub classes: u32,
    /// Signed effect records in received order.
    pub effects: Vec<Effect>,
}

/// Received facts and explicit withdrawal after an unreadable AA packet.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub enum Update {
    /// The server cleared AA state. Its replacement is not yet known.
    Unknown,
    /// Complete selections; None withdraws a previously received snapshot.
    Selections(Option<Vec<Selection>>),
    /// One definition, or withdrawal of that exact ID after a malformed update.
    Definition {
        /// Exact definition ID whose previous contents are replaced or withdrawn.
        id: u32,
        /// Received definition, or explicit unresolved replacement.
        rank: Option<Rank>,
    },
    /// An unreadable definition without an identifiable ID invalidates the cache.
    DefinitionsUnknown,
}

fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(body[at..at + 4].try_into().unwrap())
}

fn selections(body: &[u8]) -> Result<Update> {
    ensure!(
        body.len() == SELECTION_SLOTS * 8,
        "unexpected Titanium AA selection layout"
    );
    Ok(Update::Selections(Some(
        body.as_chunks::<8>()
            .0
            .iter()
            .filter_map(|entry| {
                let selected = Selection {
                    id: word(entry, 0),
                    value: word(entry, 4),
                };
                (selected.id != 0 || selected.value != 0).then_some(selected)
            })
            .collect(),
    )))
}

/// Complete AA selection array from an already received Titanium profile.
///
/// # Errors
/// Rejects a profile with an unexpected size; no empty snapshot is invented.
pub fn titanium_profile(body: &[u8]) -> Result<Update> {
    ensure!(
        body.len() == 19_592,
        "unexpected Titanium AA profile layout"
    );
    selections(&body[304..2224])
}

/// Decodes one complete AA definition or replacement selection array.
///
/// # Errors
/// Rejects malformed/trailing data, unsupported opcodes and excessive effect counts.
pub fn titanium(opcode: u16, body: &[u8]) -> Result<Update> {
    if opcode == CLEAR_OPCODE {
        ensure!(body.is_empty(), "unexpected Titanium AA clear layout");
        return Ok(Update::Unknown);
    }
    if opcode == SELECTIONS_OPCODE {
        return selections(body);
    }
    ensure!(
        opcode == DEFINITION_OPCODE && body.len() >= 96,
        "unexpected Titanium AA definition layout"
    );
    let count = usize::try_from(word(body, 92))?;
    ensure!(
        count <= MAX_EFFECTS && body.len() == 96 + count * 16,
        "unexpected Titanium AA effect list"
    );
    let id = word(body, 0);
    ensure!(id != 0, "invalid Titanium AA definition ID");
    let effects = body[96..]
        .as_chunks::<16>()
        .0
        .iter()
        .map(|effect| Effect {
            id: word(effect, 0),
            value: word(effect, 4).cast_signed(),
            limit: word(effect, 8).cast_signed(),
            slot: word(effect, 12),
        })
        .collect();
    Ok(Update::Definition {
        id,
        rank: Some(Rank {
            id,
            required_level: word(body, 24),
            sequence: word(body, 32),
            rank: word(body, 36),
            classes: word(body, 64),
            effects,
        }),
    })
}

/// Withdrawal accompanying an unreadable packet, retaining unknown versus empty.
#[must_use]
pub fn invalidated(opcode: u16, body: &[u8]) -> Update {
    if opcode == CLEAR_OPCODE {
        Update::Unknown
    } else if opcode == SELECTIONS_OPCODE || opcode == 0x75df {
        Update::Selections(None)
    } else if body.len() >= 4 && word(body, 0) != 0 {
        Update::Definition {
            id: word(body, 0),
            rank: None,
        }
    } else {
        Update::DefinitionsUnknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_uses_the_actual_header_and_preserves_signed_values() {
        let mut body = vec![0; 112];
        for (at, value) in [
            (0, 42u32),
            (24, 28),
            (32, 900),
            (36, 3),
            (64, 1 << 2),
            (92, 1),
            (96, 14),
            (108, 7),
        ] {
            body[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        body[100..104].copy_from_slice(&(-2i32).to_le_bytes());
        body[104..108].copy_from_slice(&(-1i32).to_le_bytes());
        let Update::Definition {
            id,
            rank: Some(rank),
        } = titanium(DEFINITION_OPCODE, &body).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (
                id,
                rank.required_level,
                rank.sequence,
                rank.rank,
                rank.classes
            ),
            (42, 28, 900, 3, 1 << 2)
        );
        assert_eq!(
            rank.effects,
            [Effect {
                id: 14,
                value: -2,
                limit: -1,
                slot: 7
            }]
        );
        let trailing = [body.as_slice(), &[0]].concat();
        for bad in [&body[..111], trailing.as_slice()] {
            assert!(titanium(DEFINITION_OPCODE, bad).is_err());
            assert_eq!(
                invalidated(DEFINITION_OPCODE, bad),
                Update::Definition { id: 42, rank: None }
            );
        }
    }

    #[test]
    fn profile_and_replacement_selections_retain_nonempty_and_unknown_records() {
        let mut profile = vec![0; 19_592];
        profile[304..308].copy_from_slice(&42u32.to_le_bytes());
        profile[312 + 4..312 + 8].copy_from_slice(&3u32.to_le_bytes());
        let expected = Update::Selections(Some(vec![
            Selection { id: 42, value: 0 },
            Selection { id: 0, value: 3 },
        ]));
        assert_eq!(titanium_profile(&profile).unwrap(), expected);
        assert_eq!(
            titanium(SELECTIONS_OPCODE, &profile[304..2224]).unwrap(),
            expected
        );
        assert!(titanium_profile(&profile[..19_591]).is_err());
        assert_eq!(
            invalidated(SELECTIONS_OPCODE, &[]),
            Update::Selections(None)
        );
        assert_eq!(
            titanium(SELECTIONS_OPCODE, &[0; 1920]).unwrap(),
            Update::Selections(Some(Vec::new()))
        );
    }

    #[test]
    fn malformed_definition_and_clear_withdraw_instead_of_preserving_old_effects() {
        let mut body = vec![0; 96];
        body[..4].copy_from_slice(&42u32.to_le_bytes());
        body[92..].copy_from_slice(&257u32.to_le_bytes());
        assert!(titanium(DEFINITION_OPCODE, &body).is_err());
        assert_eq!(
            invalidated(DEFINITION_OPCODE, &body),
            Update::Definition { id: 42, rank: None }
        );
        assert_eq!(
            invalidated(DEFINITION_OPCODE, &[1]),
            Update::DefinitionsUnknown
        );
        assert_eq!(titanium(CLEAR_OPCODE, &[]).unwrap(), Update::Unknown);
        assert!(titanium(CLEAR_OPCODE, &[0]).is_err());
        assert_eq!(invalidated(CLEAR_OPCODE, &[0]), Update::Unknown);
    }
}
