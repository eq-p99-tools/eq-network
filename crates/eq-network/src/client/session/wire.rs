//! A client generation's packets: what the bytes of each generation's zone
//! packets say, behind one interface the zone session reads through.
//!
//! A generation is what the client speaks, not who runs the server: P99 and
//! `EQEmu` both speak Titanium, and Project Quarm speaks `EQMac`. What a
//! generation does not read yet is simply absent, as with every
//! [`ServerType`](super::servers::ServerType) feature.

use super::{
    admission::{Admission, EqMacAdmission, Handshake, TitaniumAdmission},
    login::{self, Credentials},
    servers::Shield,
    world, CharacterSession, ZoneDestination,
};
use crate::chat::{self, ChatEvent};
use crate::{
    assets::Assets,
    client::{CancellationToken, ClientCommand, ClientConfig, Events},
};
use anyhow::{bail, Result};
use eq_network_game::{
    command::EncodedCommand,
    message::Message,
    request::{self, Request, Sender},
    GameDialect,
};
use eq_network_transport::Transport;
use std::{
    net::SocketAddr,
    sync::{atomic::AtomicBool, mpsc::Receiver},
};

/// One client generation's zone packets.
pub(super) trait Wire: Sync {
    /// What a zone packet says; nothing where the generation does not read
    /// the packet yet.
    fn messages(&self, _opcode: u16, _body: &[u8]) -> Vec<Message> {
        Vec::new()
    }

    /// What the generation's client answers by itself whenever a packet
    /// arrives, during admission or normal play; nothing for most packets.
    fn answer(&self, _opcode: u16, _body: &[u8]) -> Option<EncodedCommand> {
        None
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

    /// Logs in the way the generation's client does, playing on the
    /// configured world or, when none is, the one the player chooses from
    /// the login server's list through `commands`: the session's
    /// credentials and the world server's address.
    ///
    /// # Errors
    /// Returns an error when the login server refuses the account or the
    /// configured server, or the generation has no login yet.
    fn login(
        &self,
        _config: &ClientConfig,
        _stop: &CancellationToken,
        _commands: Option<&Receiver<ClientCommand>>,
        _log: &mut Events<'_>,
    ) -> Result<(Credentials, String)> {
        bail!("this client generation cannot log in yet")
    }

    /// Enters the world the way the generation's client does: validation,
    /// the character list, and the handoff to the chosen character's zone.
    /// `world_only` stops at the list; `zoning` re-enters the world between
    /// zones. None when the session ends at the list.
    ///
    /// # Errors
    /// Returns an error when the world refuses the client or the character,
    /// or the generation has no world stage yet.
    fn world(
        &self,
        _context: &CharacterSession<'_>,
        _assets: &Assets,
        _ip: &str,
        _flags: (bool, bool),
        _log: &mut Events<'_>,
    ) -> Result<Option<ZoneDestination>> {
        bail!("this client generation cannot enter the world yet")
    }

    /// Reads a zone server's handoff of the player straight to another zone:
    /// its address, and the file checksums the protection answers with.
    ///
    /// # Errors
    /// Returns an error when the handoff is malformed, or the generation has
    /// no direct handoff.
    fn handoff(
        &self,
        _shield: Option<&dyn Shield>,
        _packet: &[u8],
        _assets: &Assets,
        _log: &mut Events<'_>,
    ) -> Result<(String, u16, Vec<u8>)> {
        bail!("this client generation cannot hand off between zones yet")
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

    fn login(
        &self,
        config: &ClientConfig,
        stop: &CancellationToken,
        commands: Option<&Receiver<ClientCommand>>,
        log: &mut Events<'_>,
    ) -> Result<(Credentials, String)> {
        login::titanium(config, stop, commands, log)
    }

    fn world(
        &self,
        context: &CharacterSession<'_>,
        assets: &Assets,
        ip: &str,
        flags: (bool, bool),
        log: &mut Events<'_>,
    ) -> Result<Option<ZoneDestination>> {
        world::titanium(context, assets, ip, flags, log)
    }

    fn handoff(
        &self,
        shield: Option<&dyn Shield>,
        packet: &[u8],
        assets: &Assets,
        log: &mut Events<'_>,
    ) -> Result<(String, u16, Vec<u8>)> {
        world::decode_destination(shield, packet, assets, log)
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

/// The `EQMac` client's packets, which Project Quarm and TAKP speak.
pub(super) struct EqMac;

impl Wire for EqMac {
    fn messages(&self, opcode: u16, body: &[u8]) -> Vec<Message> {
        eq_network_game::message::eqmac(opcode, body)
    }

    /// Quarm's DLL version checks.
    fn answer(&self, opcode: u16, body: &[u8]) -> Option<EncodedCommand> {
        eq_network_game::quarm::answer(opcode, body)
    }

    fn chat(&self, opcode: u16, body: &[u8], include_raw: bool) -> Result<Option<ChatEvent>> {
        chat::parse_for(GameDialect::EqMac, opcode, body, include_raw)
    }

    fn encode(&self, request: &Request, sender: Sender<'_>) -> Result<EncodedCommand> {
        request::eqmac(request, sender)
    }

    fn login(
        &self,
        config: &ClientConfig,
        stop: &CancellationToken,
        commands: Option<&Receiver<ClientCommand>>,
        log: &mut Events<'_>,
    ) -> Result<(Credentials, String)> {
        login::eqmac::login(config, stop, commands, log)
    }

    /// `EQMac`'s world sends no manifest and stays the same between zones.
    fn world(
        &self,
        context: &CharacterSession<'_>,
        _assets: &Assets,
        ip: &str,
        (world_only, _zoning): (bool, bool),
        log: &mut Events<'_>,
    ) -> Result<Option<ZoneDestination>> {
        world::eqmac::world(context, ip, world_only, log)
    }

    fn handoff(
        &self,
        _shield: Option<&dyn Shield>,
        packet: &[u8],
        _assets: &Assets,
        _log: &mut Events<'_>,
    ) -> Result<(String, u16, Vec<u8>)> {
        let (host, port) = world::eqmac::zone_destination(packet)?;
        Ok((host, port, Vec::new()))
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
    fn each_generation_sends_its_own_camp_and_eqmac_nothing_it_has_not_built() {
        assert_eq!(
            Titanium.encode(&Request::Camp, PLAYER).unwrap(),
            eq_network_game::command::titanium_camp()
        );
        assert_eq!(
            EqMac.encode(&Request::Camp, PLAYER).unwrap(),
            eq_network_game::quarm::camp()
        );
        assert!(EqMac.encode(&Request::Jump, PLAYER).is_err());
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
    fn each_generation_reads_its_own_layouts() {
        use eq_network_game::quarm::ZONE_LOGOUT;
        // Titanium's logout reply means nothing to EQMac, and EQMac's logout
        // nothing to Titanium.
        assert!(EqMac.messages(0x3cdc, &[]).is_empty());
        assert!(matches!(
            EqMac.messages(ZONE_LOGOUT, &[])[..],
            [Message::LoggedOut]
        ));
        assert!(Titanium.messages(ZONE_LOGOUT, &[]).is_empty());
        // An opcode that carries no communication in either generation.
        assert!(EqMac.chat(0x0001, &[], false).unwrap().is_none());
        assert!(Titanium.chat(0x0001, &[], false).unwrap().is_none());
    }

    #[test]
    fn only_eqmac_answers_version_checks_by_itself() {
        use eq_network_game::quarm::{dll_version_message, ZONE_SPAWN_APPEARANCE};
        let check = [0, 0, 0, 1, 0, 0, 4, 0];
        let reply = EqMac.answer(ZONE_SPAWN_APPEARANCE, &check).unwrap();
        assert_eq!(reply.body, dll_version_message(true));
        assert!(Titanium.answer(ZONE_SPAWN_APPEARANCE, &check).is_none());
    }
}
