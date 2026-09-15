//! Typed client actions and their dialect-specific packet encoding.

use crate::{chat, chat::OutboundChat, GameDialect};
use anyhow::Result;

/// An action requested by an application after zone admission.
///
/// Future movement, zoning, inventory, group, and character actions belong
/// here so application code never needs to carry raw opcodes or packet bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GameCommand {
    /// Send one chat message.
    SendChat(OutboundChat),
}

/// A command encoded as one game application packet.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct EncodedCommand {
    /// Little-endian application opcode understood by the selected dialect.
    pub opcode: u16,
    /// Application body without the opcode or reliable-UDP framing.
    pub body: Vec<u8>,
}

/// Encode a typed client action for one game dialect.
///
/// `character` supplies the active character name for packet layouts that
/// repeat the sender identity.
///
/// # Errors
///
/// Returns an error when a command contains values that cannot be represented
/// by the selected dialect.
pub fn encode(
    dialect: GameDialect,
    command: &GameCommand,
    character: &str,
) -> Result<EncodedCommand> {
    match command {
        GameCommand::SendChat(message) => Ok(EncodedCommand {
            opcode: match dialect {
                GameDialect::TitaniumP99 => 0x1004,
                GameDialect::EqMac => 0x0741,
            },
            body: chat::encode_outbound_for(dialect, message, character)?,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_command_selects_each_dialects_opcode_and_layout() {
        let command = GameCommand::SendChat(OutboundChat::Say("ok".into()));
        let titanium = encode(GameDialect::TitaniumP99, &command, "Example").unwrap();
        let eqmac = encode(GameDialect::EqMac, &command, "Example").unwrap();

        assert_eq!(titanium.opcode, 0x1004);
        assert_eq!(eqmac.opcode, 0x0741);
        assert_ne!(titanium.body, eqmac.body);
    }
}
