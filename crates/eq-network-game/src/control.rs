//! Server control notifications. Receiving one does not send an answer or
//! authorize a local action.
use crate::{
    wire_read::{spawn_id, word},
    GameDialect,
};
use anyhow::{ensure, Result};
use serde::Serialize;

/// A change imposed by the server, separate from local action submission.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ControlUpdate {
    /// Stun the local player for this many milliseconds; zero clears a stun.
    Stun {
        /// Remaining duration, measured from receipt with a monotonic clock.
        duration_ms: u32,
    },
    /// Establish or release a charm relationship. This is not a complete
    /// report of whether the local player is controlled by the server.
    Charm {
        /// Controller's zone-local spawn ID.
        owner_id: u16,
        /// Controlled entity's zone-local spawn ID.
        pet_id: u16,
        /// Relationship change, retaining unsupported commands.
        command: CharmCommand,
    },
    /// The server selects a target; this is not an acknowledgment of a click.
    Target {
        /// Target spawn, or None to clear the target.
        spawn_id: Option<u16>,
    },
}

/// The command in `OP_Charm`, independent of the pet-command request opcode.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum CharmCommand {
    /// Release the relationship between this owner and pet.
    Release,
    /// Establish the relationship.
    Attach,
    /// Unsupported command; must not be treated as attachment or release.
    Unknown(u32),
}

impl From<u32> for CharmCommand {
    fn from(value: u32) -> Self {
        match value {
            0 => Self::Release,
            1 => Self::Attach,
            other => Self::Unknown(other),
        }
    }
}

/// Reads a control notice in the given wire generation; None for other opcodes.
///
/// # Errors
/// Rejects malformed lengths and spawn IDs that do not fit the zone's ID space.
pub fn decode(dialect: GameDialect, opcode: u16, body: &[u8]) -> Result<Option<ControlUpdate>> {
    use ControlUpdate::{Charm, Stun, Target};
    let (stun, charm, target) = match dialect {
        GameDialect::Titanium => (0x1e51, 0x12e5, 0x1477),
        GameDialect::EqMac => (0x5b41, 0x4442, 0xfe41),
    };
    Ok(Some(if opcode == stun {
        ensure!(body.len() == 4, "invalid stun length");
        Stun {
            duration_ms: word(body, 0),
        }
    } else if opcode == charm {
        let values = match dialect {
            GameDialect::Titanium => {
                ensure!(body.len() == 12, "invalid Titanium charm length");
                [word(body, 0), word(body, 4), word(body, 8)]
            }
            GameDialect::EqMac => {
                ensure!(body.len() == 6, "invalid EQMac charm length");
                [0, 2, 4].map(|at| u32::from(u16::from_le_bytes([body[at], body[at + 1]])))
            }
        };
        Charm {
            owner_id: spawn_id(values[0])?,
            pet_id: spawn_id(values[1])?,
            command: values[2].into(),
        }
    } else if opcode == target {
        let id = match dialect {
            GameDialect::Titanium => {
                ensure!(body.len() == 4, "invalid Titanium target length");
                u16::try_from(word(body, 0))?
            }
            GameDialect::EqMac => {
                ensure!(body.len() == 2, "invalid EQMac target length");
                u16::from_le_bytes([body[0], body[1]])
            }
        };
        Target {
            spawn_id: (id != 0).then_some(id),
        }
    } else {
        return Ok(None);
    }))
}
