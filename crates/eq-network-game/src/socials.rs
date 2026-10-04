//! Rolling dice (`/random`), emoting (`/emote`) and assisting (`/assist`).
//!
//! Layout reference: `EQEmu`'s Titanium `RandomReq_Struct`,
//! `RandomReply_Struct`, `Emote_Struct` and `EntityId_Struct`
//! (`common/patches/titanium_structs.h`); the rules are
//! `Client::Handle_OP_RandomReq`, `Handle_OP_Emote` and `Handle_OP_Assist`
//! (`zone/client_packet.cpp`).
use crate::command::EncodedCommand;
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_RandomReq`: a die to roll, from the lowest to the highest number.
pub const RANDOM_OPCODE: u16 = 0x5534;
/// `OP_RandomReply`: a die the server rolled for a player nearby.
pub const ROLL_OPCODE: u16 = 0x6cd5;
/// `OP_Emote`: what a player does, in their own words.
pub const EMOTE_OPCODE: u16 = 0x547a;
/// `OP_Assist`: whose target to take; the server answers with that target.
pub const ASSIST_OPCODE: u16 = 0x7709;

/// The emote's text field; the server reads at most 512 bytes of it.
const EMOTE_TEXT: usize = 1024;
/// The longest emote the server passes on whole.
pub const EMOTE_LONGEST: usize = 511;
/// A roll's name field.
const NAME: usize = 64;

/// A die the server rolled for a player near the player, the player among
/// them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Roll {
    /// Who rolled it.
    pub name: String,
    /// The lowest number it could have turned up.
    pub low: u32,
    /// The highest.
    pub high: u32,
    /// What it turned up.
    pub result: u32,
}

/// The server's answer to an assist.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Assisted {
    /// The target to take; None to keep the player's own.
    pub target: Option<u16>,
}

fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

/// Rolls a die from the lowest to the highest number; the server orders
/// them, and rolls 0 to 100 for two zeros.
#[must_use]
pub fn random(low: u32, high: u32) -> EncodedCommand {
    let mut body = low.to_le_bytes().to_vec();
    body.extend_from_slice(&high.to_le_bytes());
    EncodedCommand {
        opcode: RANDOM_OPCODE,
        body,
    }
}

/// Emotes: the server puts the player's name before the text, as it is,
/// so the text starts with a space.
///
/// # Errors
/// Rejects an empty emote, one the server would cut, and one with a NUL.
pub fn emote(text: &str) -> Result<EncodedCommand> {
    ensure!(!text.trim().is_empty(), "an emote needs words");
    ensure!(
        text.len() < EMOTE_LONGEST,
        "an emote is at most {} bytes",
        EMOTE_LONGEST - 1
    );
    ensure!(!text.contains('\0'), "an emote cannot hold a NUL");
    let mut body = vec![0; 4 + EMOTE_TEXT];
    let words = format!(" {text}");
    body[4..4 + words.len()].copy_from_slice(words.as_bytes());
    Ok(EncodedCommand {
        opcode: EMOTE_OPCODE,
        body,
    })
}

/// An emote as the server passes it on to the players near the one who
/// made it: their name, then their words.
#[must_use]
pub fn heard_emote(name: &str, text: &str) -> Vec<u8> {
    let mut body = vec![0; 4];
    body.extend_from_slice(name.as_bytes());
    body.push(b' ');
    body.extend_from_slice(text.as_bytes());
    body.push(0);
    body
}

/// Takes the target of the spawn assisted.
#[must_use]
pub fn assist(spawn_id: u16) -> EncodedCommand {
    EncodedCommand {
        opcode: ASSIST_OPCODE,
        body: u32::from(spawn_id).to_le_bytes().to_vec(),
    }
}

/// Decodes a die rolled nearby; None for any other opcode.
///
/// # Errors
/// Rejects a malformed length.
pub fn decode_roll(opcode: u16, body: &[u8]) -> Result<Option<Roll>> {
    if opcode != ROLL_OPCODE {
        return Ok(None);
    }
    ensure!(body.len() == 12 + NAME, "invalid roll length");
    let field = &body[12..12 + NAME];
    let end = field.iter().position(|byte| *byte == 0).unwrap_or(NAME);
    Ok(Some(Roll {
        name: String::from_utf8_lossy(&field[..end]).into_owned(),
        low: word(body, 0),
        high: word(body, 4),
        result: word(body, 8),
    }))
}

/// Decodes the server's answer to an assist; None for any other opcode.
///
/// # Errors
/// Rejects a malformed length or spawn ID.
pub fn decode_assist(opcode: u16, body: &[u8]) -> Result<Option<Assisted>> {
    if opcode != ASSIST_OPCODE {
        return Ok(None);
    }
    ensure!(body.len() == 4, "invalid assist length");
    let target = u16::try_from(word(body, 0))?;
    Ok(Some(Assisted {
        target: (target != 0).then_some(target),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_the_servers_structs() {
        assert_eq!(random(1, 6).body, [1, 0, 0, 0, 6, 0, 0, 0]);
        let waved = emote("waves.").unwrap();
        assert_eq!((waved.opcode, waved.body.len()), (EMOTE_OPCODE, 1028));
        assert_eq!(&waved.body[..11], b"\0\0\0\0 waves.");
        assert!(emote(" ").is_err());
        assert!(emote(&"a".repeat(EMOTE_LONGEST)).is_err());
        assert_eq!(assist(300).body, [44, 1, 0, 0]);
        assert_eq!(heard_emote("Tester", "waves."), b"\0\0\0\0Tester waves.\0");
    }

    #[test]
    fn the_servers_answers_name_the_roll_and_the_target() {
        let mut roll = vec![0; 76];
        roll[..4].copy_from_slice(&1u32.to_le_bytes());
        roll[4..8].copy_from_slice(&6u32.to_le_bytes());
        roll[8..12].copy_from_slice(&4u32.to_le_bytes());
        roll[12..18].copy_from_slice(b"Tester");
        assert_eq!(
            decode_roll(ROLL_OPCODE, &roll).unwrap(),
            Some(Roll {
                name: "Tester".into(),
                low: 1,
                high: 6,
                result: 4
            })
        );
        assert!(decode_roll(ROLL_OPCODE, &roll[..70]).is_err());
        assert_eq!(decode_roll(ASSIST_OPCODE, &roll).unwrap(), None);
        assert_eq!(
            decode_assist(ASSIST_OPCODE, &[44, 1, 0, 0]).unwrap(),
            Some(Assisted { target: Some(300) })
        );
        assert_eq!(
            decode_assist(ASSIST_OPCODE, &[0; 4]).unwrap(),
            Some(Assisted { target: None })
        );
        assert!(decode_assist(ASSIST_OPCODE, &[0; 5]).is_err());
    }
}
