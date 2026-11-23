//! Read-only Titanium AA selections, rank definitions, points, AA experience
//! and reuse timers. No purchases or activations.
//!
//! Layouts follow `EQEmu` `common/patches/titanium_structs.h`, the
//! `OP_SendAATable`/`OP_RespondAA` encoders in `common/patches/titanium.cpp`
//! and `zone/aa.cpp`'s `SendAlternateAdvancementStats` and
//! `SendAlternateAdvancementTimer(s)` (checklist 6.547). Every field keeps
//! its raw value under its wire name.
//! The native environmental cache (0x40feaa) selects definitions by exact ID,
//! checks class and required level, and adds their signed effect values.
use anyhow::{ensure, Result};

/// A received rank definition, not an owned-rank snapshot.
pub const DEFINITION_OPCODE: u16 = 0x367d;
/// Replaces the profile's selected AA records.
pub const SELECTIONS_OPCODE: u16 = 0x3af4;
/// Withdraws the previous AA view; a replacement selection array must follow.
pub const CLEAR_OPCODE: u16 = 0x5918;
/// The server's AA reuse timer (`UseAA_Struct`); the client's 16-byte form
/// of this opcode is a request, not read here.
pub const TIMER_OPCODE: u16 = 0x0681;
/// The player's AA experience, unspent points and experience split
/// (`AltAdvStats_Struct`).
pub const PROGRESS_OPCODE: u16 = 0x5f58;
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

/// A received Titanium rank definition (`SendAA_Struct`, 96-byte header).
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Rank {
    /// Exact definition ID.
    pub id: u32,
    /// `hotkey_sid` at 8: a string id for the hot button's upper line.
    pub hotkey_sid: u32,
    /// `hotkey_sid2` at 12: a string id for the hot button's lower line.
    pub hotkey_sid2: u32,
    /// `title_sid` at 16: the string id of the ability's name.
    pub title_sid: u32,
    /// `desc_sid` at 20: the string id of its description.
    pub desc_sid: u32,
    /// Required character level, named `class_type` in the wire structure.
    pub required_level: u32,
    /// `cost` at 28: this rank's price in AA points.
    pub cost: u32,
    /// Ability sequence; it does not select this rank.
    pub sequence: u32,
    /// Rank number supplied in `current_level`.
    pub rank: u32,
    /// `prereq_skill` at 40, signed; `EQEmu`'s comment says a value below 0
    /// names a category. Not interpreted here.
    pub prereq_skill: i32,
    /// `prereq_minpoints` at 44: points the prerequisite needs.
    pub prereq_minpoints: u32,
    /// `type` at 48: the ability's window tab, as the server's data says.
    pub kind: u32,
    /// `spellid` at 52, signed: the spell an activation casts (`EQEmu` data
    /// uses -1 for none).
    pub spellid: i32,
    /// `spell_type` at 56: the reuse timer this ability shares, the id a
    /// [`Timer`] names.
    pub spell_type: u32,
    /// `spell_refresh` at 60: the reuse time, in seconds.
    pub spell_refresh: u32,
    /// Native class mask uses bit `1 << class`, with classes 1 through 16.
    pub classes: u32,
    /// `max_level` at 68: the ability's highest rank.
    pub max_level: u32,
    /// `last_id` at 72, signed: the previous rank's definition id.
    pub last_id: i32,
    /// `next_id` at 76, signed: the next rank's definition id; `EQEmu` sends
    /// -1 when the player may not use the next rank or the ability has
    /// charges.
    pub next_id: i32,
    /// `cost2` at 80: the total cost of the ability through this rank.
    pub cost2: u32,
    /// Signed effect records in received order.
    pub effects: Vec<Effect>,
}

/// The profile's AA points (`aapoints_spent` at 6152, `aapoints` at 6156,
/// `expAA` at 19512).
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Points {
    /// `aapoints_spent`: points spent.
    pub aapoints_spent: u32,
    /// `aapoints`: points unspent.
    pub aapoints: u32,
    /// `expAA`: raw experience inside the current AA point, not scaled.
    pub exp_aa: u32,
}

/// `OP_AAExpUpdate`'s `AltAdvStats_Struct`, 12 bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Progress {
    /// `experience` at 0: the server scales it to 0..330 of the next point.
    pub experience: u32,
    /// `unspent` at 4: points unspent.
    pub unspent: u16,
    /// `percentage` at 8: the share of experience that goes to AAs.
    pub percentage: u8,
}

/// The server's `OP_AAAction` `UseAA_Struct`, 12 bytes: one reuse timer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Timer {
    /// `begin` at 0: when the timer started, by the server's clock; 0 when
    /// the server reset it.
    pub begin: u32,
    /// `ability` at 4: the timer id, a definition's `spell_type`.
    pub ability: u32,
    /// `end` at 8: the server's clock when it sent this, not a deadline.
    pub end: u32,
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
    /// The profile's points; None withdraws them after an unreadable profile.
    Points(Option<Points>),
    /// AA experience and the split; None withdraws them after an unreadable
    /// update.
    Progress(Option<Progress>),
    /// One reuse timer as sent; nothing withdraws a timer.
    Timer(Timer),
}

impl Update {
    /// Whether this can change which effects the player's AAs add up to:
    /// the selections and definitions. Points, experience and timers don't.
    #[must_use]
    pub const fn changes_effects(&self) -> bool {
        match self {
            Self::Unknown
            | Self::Selections(_)
            | Self::Definition { .. }
            | Self::DefinitionsUnknown => true,
            Self::Points(_) | Self::Progress(_) | Self::Timer(_) => false,
        }
    }
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

/// Complete AA selection array and points from an already received
/// Titanium profile, in that order.
///
/// # Errors
/// Rejects a profile with an unexpected size; no empty snapshot is invented.
pub fn titanium_profile(body: &[u8]) -> Result<[Update; 2]> {
    ensure!(
        body.len() == 19_592,
        "unexpected Titanium AA profile length {}",
        body.len()
    );
    Ok([
        selections(&body[304..2224])?,
        Update::Points(Some(Points {
            aapoints_spent: word(body, 6152),
            aapoints: word(body, 6156),
            exp_aa: word(body, 19_512),
        })),
    ])
}

/// Whether a Titanium zone packet is one [`titanium`] reads.
#[must_use]
pub const fn is_titanium(opcode: u16) -> bool {
    matches!(
        opcode,
        DEFINITION_OPCODE | SELECTIONS_OPCODE | CLEAR_OPCODE | TIMER_OPCODE | PROGRESS_OPCODE
    )
}

/// Decodes one complete AA definition or replacement selection array.
///
/// # Errors
/// Rejects malformed/trailing data, unsupported opcodes and excessive effect counts.
pub fn titanium(opcode: u16, body: &[u8]) -> Result<Update> {
    if opcode == TIMER_OPCODE {
        ensure!(
            body.len() == 12,
            "unexpected Titanium AA timer length {}",
            body.len()
        );
        return Ok(Update::Timer(Timer {
            begin: word(body, 0),
            ability: word(body, 4),
            end: word(body, 8),
        }));
    }
    if opcode == PROGRESS_OPCODE {
        ensure!(
            body.len() == 12,
            "unexpected Titanium AA experience length {}",
            body.len()
        );
        return Ok(Update::Progress(Some(Progress {
            experience: word(body, 0),
            unspent: u16::from_le_bytes([body[4], body[5]]),
            percentage: body[8],
        })));
    }
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
            hotkey_sid: word(body, 8),
            hotkey_sid2: word(body, 12),
            title_sid: word(body, 16),
            desc_sid: word(body, 20),
            required_level: word(body, 24),
            cost: word(body, 28),
            sequence: word(body, 32),
            rank: word(body, 36),
            prereq_skill: word(body, 40).cast_signed(),
            prereq_minpoints: word(body, 44),
            kind: word(body, 48),
            spellid: word(body, 52).cast_signed(),
            spell_type: word(body, 56),
            spell_refresh: word(body, 60),
            classes: word(body, 64),
            max_level: word(body, 68),
            last_id: word(body, 72).cast_signed(),
            next_id: word(body, 76).cast_signed(),
            cost2: word(body, 80),
            effects,
        }),
    })
}

/// Withdrawal accompanying an unreadable packet, retaining unknown versus
/// empty. An unreadable timer withdraws nothing: no timer id in it can be
/// trusted.
#[must_use]
pub fn invalidated(opcode: u16, body: &[u8]) -> Option<Update> {
    Some(if opcode == CLEAR_OPCODE {
        Update::Unknown
    } else if opcode == TIMER_OPCODE {
        return None;
    } else if opcode == PROGRESS_OPCODE {
        Update::Progress(None)
    } else if opcode == SELECTIONS_OPCODE || opcode == 0x75df {
        Update::Selections(None)
    } else if body.len() >= 4 && word(body, 0) != 0 {
        Update::Definition {
            id: word(body, 0),
            rank: None,
        }
    } else {
        Update::DefinitionsUnknown
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_uses_the_actual_header_and_preserves_signed_values() {
        let mut body = vec![0; 112];
        for (at, value) in [
            (0, 42u32),
            (8, 1001),
            (12, 1002),
            (16, 1003),
            (20, 1004),
            (24, 28),
            (28, 5),
            (32, 900),
            (36, 3),
            (44, 6),
            (48, 2),
            (56, 9),
            (60, 4320),
            (64, 1 << 2),
            (68, 4),
            (72, 41),
            (80, 12),
            (92, 1),
            (96, 14),
            (108, 7),
        ] {
            body[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        for at in [40, 52, 76] {
            body[at..at + 4].copy_from_slice(&(-1i32).to_le_bytes());
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
            [
                rank.hotkey_sid,
                rank.hotkey_sid2,
                rank.title_sid,
                rank.desc_sid,
                rank.cost,
                rank.prereq_minpoints,
                rank.kind,
                rank.spell_type,
                rank.spell_refresh,
                rank.max_level,
                rank.cost2,
            ],
            [1001, 1002, 1003, 1004, 5, 6, 2, 9, 4320, 4, 12]
        );
        assert_eq!(
            (rank.prereq_skill, rank.spellid, rank.last_id, rank.next_id),
            (-1, -1, 41, -1)
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
                Some(Update::Definition { id: 42, rank: None })
            );
        }
    }

    #[test]
    fn profile_and_replacement_selections_retain_nonempty_and_unknown_records() {
        let mut profile = vec![0; 19_592];
        profile[304..308].copy_from_slice(&42u32.to_le_bytes());
        profile[312 + 4..312 + 8].copy_from_slice(&3u32.to_le_bytes());
        profile[6152..6156].copy_from_slice(&7u32.to_le_bytes());
        profile[6156..6160].copy_from_slice(&2u32.to_le_bytes());
        profile[19_512..19_516].copy_from_slice(&123_456u32.to_le_bytes());
        let expected = Update::Selections(Some(vec![
            Selection { id: 42, value: 0 },
            Selection { id: 0, value: 3 },
        ]));
        assert_eq!(
            titanium_profile(&profile).unwrap(),
            [
                expected.clone(),
                Update::Points(Some(Points {
                    aapoints_spent: 7,
                    aapoints: 2,
                    exp_aa: 123_456,
                })),
            ]
        );
        assert_eq!(
            titanium(SELECTIONS_OPCODE, &profile[304..2224]).unwrap(),
            expected
        );
        assert!(titanium_profile(&profile[..19_591]).is_err());
        assert_eq!(
            invalidated(SELECTIONS_OPCODE, &[]),
            Some(Update::Selections(None))
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
            Some(Update::Definition { id: 42, rank: None })
        );
        assert_eq!(
            invalidated(DEFINITION_OPCODE, &[1]),
            Some(Update::DefinitionsUnknown)
        );
        assert_eq!(titanium(CLEAR_OPCODE, &[]).unwrap(), Update::Unknown);
        assert!(titanium(CLEAR_OPCODE, &[0]).is_err());
        assert_eq!(invalidated(CLEAR_OPCODE, &[0]), Some(Update::Unknown));
    }

    #[test]
    fn timers_and_experience_read_exactly_twelve_bytes_as_sent() {
        let mut timer = [0u8; 12];
        timer[..4].copy_from_slice(&1_700_000_000u32.to_le_bytes());
        timer[4..8].copy_from_slice(&9u32.to_le_bytes());
        timer[8..].copy_from_slice(&1_700_000_060u32.to_le_bytes());
        assert_eq!(
            titanium(TIMER_OPCODE, &timer).unwrap(),
            Update::Timer(Timer {
                begin: 1_700_000_000,
                ability: 9,
                end: 1_700_000_060,
            })
        );
        let mut progress = [0xffu8; 12];
        progress[..4].copy_from_slice(&165u32.to_le_bytes());
        progress[4..6].copy_from_slice(&3u16.to_le_bytes());
        progress[8] = 75;
        // The bytes at 6 and 9 to 11 are unread, whatever they hold.
        assert_eq!(
            titanium(PROGRESS_OPCODE, &progress).unwrap(),
            Update::Progress(Some(Progress {
                experience: 165,
                unspent: 3,
                percentage: 75,
            }))
        );
        for (opcode, body) in [(TIMER_OPCODE, timer), (PROGRESS_OPCODE, progress)] {
            let long = [body.as_slice(), &[0]].concat();
            for bad in [&body[..11], long.as_slice()] {
                let error = titanium(opcode, bad).unwrap_err().to_string();
                assert!(error.ends_with(&format!("length {}", bad.len())), "{error}");
            }
        }
        // An unreadable timer withdraws nothing; unreadable experience
        // withdraws the last.
        assert_eq!(invalidated(TIMER_OPCODE, &timer[..11]), None);
        assert_eq!(
            invalidated(PROGRESS_OPCODE, &progress[..11]),
            Some(Update::Progress(None))
        );
    }
}
