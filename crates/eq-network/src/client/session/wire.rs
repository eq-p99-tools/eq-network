//! A client generation's packets: what the bytes of each generation's zone
//! packets say, behind one interface the zone session reads through.
//!
//! A generation is what the client speaks, not who runs the server: P99 and
//! `EQEmu` both speak Titanium, and Project Quarm speaks `EQMac`. What a
//! generation does not read yet is simply absent, as with every
//! [`ServerType`](super::servers::ServerType) feature.

use super::admission::{Admission, EqMacAdmission, Handshake, TitaniumAdmission};
use crate::chat::{self, ChatEvent};
use anyhow::{bail, Result};
use eq_network_game::{
    command::EncodedCommand,
    message::Message,
    request::{self, Request, Sender},
    GameDialect,
};
use eq_network_transport::Transport;
use std::{net::SocketAddr, sync::atomic::AtomicBool};

/// One client generation's zone packets.
pub(super) trait Wire: Sync {
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
    fn encode(&self, request: &Request, _sender: Sender<'_>) -> Result<EncodedCommand> {
        bail!("this client generation cannot send {request:?} yet")
    }

    /// Connects to a zone server the way the generation's client does.
    ///
    /// # Errors
    /// Returns an error when the connection fails or the generation has no
    /// zone connection yet.
    fn connect_zone(&self, _address: SocketAddr, _stop: &AtomicBool) -> Result<Box<dyn Transport>> {
        bail!("this client generation cannot connect to a zone yet")
    }

    /// Asks the zone to admit the character, starting the generation's
    /// handshake.
    ///
    /// # Errors
    /// Returns an error when the entry cannot be sent, or the generation has
    /// no handshake yet.
    fn admit(
        &self,
        _character: &str,
        _revolution: f32,
        _handshake: &mut Handshake<'_, '_>,
    ) -> Result<Box<dyn Admission>> {
        bail!("this client generation cannot enter a zone yet")
    }
}

/// The Titanium client's packets, which P99 and `EQEmu` speak.
pub(super) struct Titanium;

impl Wire for Titanium {
    fn messages(&self, opcode: u16, body: &[u8]) -> Vec<Message> {
        eq_network_game::message::titanium(opcode, body)
    }

    fn chat(&self, opcode: u16, body: &[u8], include_raw: bool) -> Result<Option<ChatEvent>> {
        chat::parse(opcode, body, include_raw)
    }

    fn encode(&self, request: &Request, sender: Sender<'_>) -> Result<EncodedCommand> {
        request::titanium(request, sender)
    }

    /// Titanium zones speak the modern transport, answering session
    /// requests.
    fn connect_zone(&self, address: SocketAddr, stop: &AtomicBool) -> Result<Box<dyn Transport>> {
        Ok(Box::new(crate::transport::Session::connect_cancellable(
            address, true, stop,
        )?))
    }

    fn admit(
        &self,
        character: &str,
        revolution: f32,
        handshake: &mut Handshake<'_, '_>,
    ) -> Result<Box<dyn Admission>> {
        Ok(Box::new(TitaniumAdmission::start(
            character, revolution, handshake,
        )?))
    }
}

/// The `EQMac` client's packets, which Project Quarm speaks. Only its
/// communication is read through this interface so far; Quarm's own zone
/// loop reads the rest until it runs on the shared session.
pub(super) struct EqMac;

impl Wire for EqMac {
    fn chat(&self, opcode: u16, body: &[u8], include_raw: bool) -> Result<Option<ChatEvent>> {
        chat::parse_for(GameDialect::EqMac, opcode, body, include_raw)
    }

    fn encode(&self, request: &Request, sender: Sender<'_>) -> Result<EncodedCommand> {
        request::eqmac(request, sender)
    }

    /// `EQMac` zones speak the legacy transport.
    fn connect_zone(&self, address: SocketAddr, stop: &AtomicBool) -> Result<Box<dyn Transport>> {
        Ok(Box::new(
            crate::old_transport::OldSession::connect_cancellable(address, stop)?,
        ))
    }

    /// `EQMac`'s handshake; the player stands where their own spawn puts
    /// them, so the profile's turn goes unused.
    fn admit(
        &self,
        character: &str,
        _revolution: f32,
        handshake: &mut Handshake<'_, '_>,
    ) -> Result<Box<dyn Admission>> {
        Ok(Box::new(EqMacAdmission::start(character, handshake)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A player with a spawn.
    const PLAYER: Sender<'static> = Sender {
        name: "Tester",
        spawn_id: Some(7),
    };

    #[test]
    fn titanium_reads_what_the_titanium_decoders_read() {
        // A logout reply: the server ends the session.
        let read = Titanium.messages(0x3cdc, &[]);
        assert!(matches!(read.as_slice(), [Message::LoggedOut]), "{read:?}");
    }

    #[test]
    fn titanium_sends_what_the_titanium_codecs_build_and_eqmac_nothing_yet() {
        assert_eq!(
            Titanium.encode(&Request::Camp, PLAYER).unwrap(),
            eq_network_game::command::titanium_camp()
        );
        assert!(EqMac.encode(&Request::Camp, PLAYER).is_err());
    }

    #[test]
    fn each_generation_sends_chat_its_own_way() {
        use eq_network_game::chat::OutboundChat;
        let say = Request::Command(eq_network_game::command::GameCommand::SendChat(
            OutboundChat::Say("Hail".into()),
        ));
        assert_eq!(Titanium.encode(&say, PLAYER).unwrap().opcode, 0x1004);
        assert_eq!(EqMac.encode(&say, PLAYER).unwrap().opcode, 0x0741);
    }

    #[test]
    fn eqmac_reads_only_its_communication_so_far() {
        assert!(EqMac.messages(0x3cdc, &[]).is_empty());
        // An opcode that carries no communication in either generation.
        assert!(EqMac.chat(0x0001, &[], false).unwrap().is_none());
        assert!(Titanium.chat(0x0001, &[], false).unwrap().is_none());
    }
}
