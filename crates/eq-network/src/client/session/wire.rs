//! A client generation's packets: what the bytes of each generation's zone
//! packets say, behind one interface the zone session reads through.
//!
//! A generation is what the client speaks, not who runs the server: P99 and
//! `EQEmu` both speak Titanium, and Project Quarm speaks `EQMac`. What a
//! generation does not read yet is simply absent, as with every
//! [`ServerType`](super::servers::ServerType) feature.

use crate::chat::{self, ChatEvent};
use anyhow::{bail, Result};
use eq_network_game::{
    command::EncodedCommand,
    message::Message,
    request::{self, Request},
    GameDialect,
};

/// One client generation's zone packets.
pub(super) trait Wire: Sync {
    /// The packet layouts commands are encoded in.
    fn dialect(&self) -> GameDialect;

    /// What a zone packet says; nothing where the generation does not read
    /// the packet yet.
    fn messages(&self, _opcode: u16, _body: &[u8]) -> Vec<Message> {
        Vec::new()
    }

    /// The communication a zone packet carries, if it carries any.
    ///
    /// # Errors
    /// Returns an error when a communication packet is malformed.
    fn chat(&self, _opcode: u16, _body: &[u8], _include_raw: bool) -> Result<Option<ChatEvent>> {
        Ok(None)
    }

    /// The packet that asks the server for what the session wants.
    ///
    /// # Errors
    /// Refuses a request the generation has not been built for, and one
    /// whose values its packet cannot carry.
    fn encode(&self, request: &Request) -> Result<EncodedCommand> {
        bail!("this client generation cannot send {request:?} yet")
    }
}

/// The Titanium client's packets, which P99 and `EQEmu` speak.
pub(super) struct Titanium;

impl Wire for Titanium {
    fn dialect(&self) -> GameDialect {
        GameDialect::Titanium
    }

    fn messages(&self, opcode: u16, body: &[u8]) -> Vec<Message> {
        eq_network_game::message::titanium(opcode, body)
    }

    fn chat(&self, opcode: u16, body: &[u8], include_raw: bool) -> Result<Option<ChatEvent>> {
        chat::parse(opcode, body, include_raw)
    }

    fn encode(&self, request: &Request) -> Result<EncodedCommand> {
        request::titanium(request)
    }
}

/// The `EQMac` client's packets, which Project Quarm speaks. Only its
/// communication is read through this interface so far; Quarm's own zone
/// loop reads the rest until it runs on the shared session.
pub(super) struct EqMac;

impl Wire for EqMac {
    fn dialect(&self) -> GameDialect {
        GameDialect::EqMac
    }

    fn chat(&self, opcode: u16, body: &[u8], include_raw: bool) -> Result<Option<ChatEvent>> {
        chat::parse_for(GameDialect::EqMac, opcode, body, include_raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titanium_reads_what_the_titanium_decoders_read() {
        // A logout reply: the server ends the session.
        let read = Titanium.messages(0x3cdc, &[]);
        assert!(matches!(read.as_slice(), [Message::LoggedOut]), "{read:?}");
        assert_eq!(Titanium.dialect(), GameDialect::Titanium);
    }

    #[test]
    fn titanium_sends_what_the_titanium_codecs_build_and_eqmac_nothing_yet() {
        assert_eq!(
            Titanium.encode(&Request::Camp).unwrap(),
            eq_network_game::command::titanium_camp()
        );
        assert!(EqMac.encode(&Request::Camp).is_err());
    }

    #[test]
    fn eqmac_reads_only_its_communication_so_far() {
        assert!(EqMac.messages(0x3cdc, &[]).is_empty());
        assert_eq!(EqMac.dialect(), GameDialect::EqMac);
        // An opcode that carries no communication in either generation.
        assert!(EqMac.chat(0x0001, &[], false).unwrap().is_none());
        assert!(Titanium.chat(0x0001, &[], false).unwrap().is_none());
    }
}
