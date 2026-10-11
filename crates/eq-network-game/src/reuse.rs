//! Discipline and linked-spell reuse notices. Timer groups are not gem slots.
//! These are wire facts, not authorization to cast or use a discipline.
use crate::{wire_read::word, GameDialect};
use anyhow::{ensure, Result};
use serde::Serialize;

/// A server report about disciplines or a shared spell reuse group.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum ReuseUpdate {
    /// Titanium's remaining reuse time for one discipline timer group.
    DisciplineTimer {
        /// Timer group from the spell's data, not its spell ID or gem slot.
        timer_id: u32,
        /// Seconds remaining; zero resets this timer.
        remaining_seconds: u32,
    },
    /// Titanium's complete, ordered 100-entry discipline list. Zero means
    /// an empty slot; retain other values without assuming spell-file coverage.
    Disciplines(Vec<u32>),
    /// A linked spell reuse group. Both timestamps use the server timer's
    /// seconds counter, not Unix time or the client's monotonic clock.
    LinkedSpell {
        /// Shared timer group, not a gem slot.
        timer_id: u32,
        /// Server timer value when reuse ends, possibly after u32 wraparound.
        end_seconds: u32,
        /// Server timer value when reuse starts.
        start_seconds: u32,
    },
    /// `EQMac`'s newly activated discipline ID. This is neither a duration
    /// nor Titanium's full list of learned discipline spells.
    DisciplineActivated(u8),
}

/// Reads reuse notices, leaving unrelated opcodes to their own codecs.
///
/// # Errors
/// Rejects truncated or oversized known records.
pub fn decode(dialect: GameDialect, opcode: u16, body: &[u8]) -> Result<Option<ReuseUpdate>> {
    Ok(Some(match (dialect, opcode) {
        (GameDialect::Titanium, 0x53df) => {
            ensure!(body.len() == 12, "invalid discipline timer length");
            ReuseUpdate::DisciplineTimer {
                timer_id: word(body, 0),
                remaining_seconds: word(body, 4),
            }
        }
        (GameDialect::Titanium, 0x7180) => {
            ensure!(body.len() == 400, "invalid discipline list length");
            ReuseUpdate::Disciplines(
                body.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|v| u32::from_le_bytes(*v))
                    .collect(),
            )
        }
        (GameDialect::Titanium, 0x6a00) => {
            ensure!(body.len() == 12, "invalid linked reuse length");
            ReuseUpdate::LinkedSpell {
                timer_id: word(body, 0),
                end_seconds: word(body, 4),
                start_seconds: word(body, 8),
            }
        }
        (GameDialect::EqMac, 0xf241) => {
            ensure!(body.len() == 4, "invalid EQMac discipline length");
            ReuseUpdate::DisciplineActivated(body[0])
        }
        _ => return Ok(None),
    }))
}
