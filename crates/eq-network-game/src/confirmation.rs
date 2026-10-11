//! Translocate and sacrifice notifications. Offers are distinct from zoning
//! instructions; parsing one must never accept it or move the player.
use crate::{
    wire_read::{spawn_id, word},
    GameDialect,
};
use anyhow::{ensure, Result};
use serde::Serialize;

/// A spell exchange that requires the player's explicit answer.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Confirmation {
    /// A translocation exchange, with its own completion state.
    Translocate(Translocate),
    /// A sacrifice exchange. The server's initial prompt has confirm = 0;
    /// retain the field instead of treating every packet as a fresh offer.
    Sacrifice {
        /// Caster's zone-local spawn ID.
        caster_id: u16,
        /// Victim's zone-local spawn ID.
        target_id: u16,
        /// Raw confirmation field; zero is the initial server offer.
        confirm: u32,
    },
}

/// The destination and state carried by `OP_Translocate` in both generations.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Translocate {
    /// Destination zone number; zero can carry spell-specific meaning.
    pub zone_id: u32,
    /// Spell whose effect requests the transfer.
    pub spell_id: u32,
    /// Caster's character name.
    pub caster: String,
    /// Destination X coordinate, not the Y-first wire ordering.
    pub x: f32,
    /// Destination Y coordinate.
    pub y: f32,
    /// Destination Z coordinate; the packet does not supply a heading.
    pub z: f32,
    /// State of the exchange; not proof that a new zone admitted the player.
    pub state: TranslocateState,
    /// The word at offset 8. `EQMac` calls this Response but does not use it;
    /// Titanium's source leaves its meaning unknown. Never use as heading.
    pub response_word: u32,
}

/// `OP_Translocate`'s completion word, preserving unsupported values.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum TranslocateState {
    /// Display a confirmation prompt.
    Offered,
    /// The exchange was accepted; a transfer may follow separately.
    Accepted,
    /// `EQMac`'s decline sentinel; not assigned this meaning on Titanium.
    Declined,
    /// Unsupported generation-specific state.
    Unknown(u32),
}

/// Reads a spell confirmation without generating any response.
///
/// # Errors
/// Rejects malformed lengths, invalid entity IDs and non-finite coordinates.
pub fn decode(dialect: GameDialect, opcode: u16, body: &[u8]) -> Result<Option<Confirmation>> {
    let (translocate, sacrifice) = match dialect {
        GameDialect::Titanium => (0x8258, 0x727a),
        GameDialect::EqMac => (0x0642, 0xea41),
    };
    if opcode == sacrifice {
        ensure!(body.len() == 12, "invalid sacrifice length");
        return Ok(Some(Confirmation::Sacrifice {
            caster_id: spawn_id(word(body, 0))?,
            target_id: spawn_id(word(body, 4))?,
            confirm: word(body, 8),
        }));
    }
    if opcode != translocate {
        return Ok(None);
    }
    ensure!(body.len() == 92, "invalid translocate length");
    let [y, x, z] = [76, 80, 84].map(|at| f32::from_bits(word(body, at)));
    ensure!(
        [x, y, z].iter().all(|v| v.is_finite()),
        "invalid translocate coordinates"
    );
    let name = &body[12..76];
    let end = name.iter().position(|b| *b == 0).unwrap_or(name.len());
    let state = match (dialect, word(body, 88)) {
        (_, 0) => TranslocateState::Offered,
        (_, 1) => TranslocateState::Accepted,
        (GameDialect::EqMac, u32::MAX) => TranslocateState::Declined,
        (_, other) => TranslocateState::Unknown(other),
    };
    Ok(Some(Confirmation::Translocate(Translocate {
        zone_id: word(body, 0),
        spell_id: word(body, 4),
        caster: String::from_utf8_lossy(&name[..end]).into_owned(),
        x,
        y,
        z,
        state,
        response_word: word(body, 8),
    })))
}
