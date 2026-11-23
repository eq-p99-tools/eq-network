//! `RoF2`'s world stage, as stock `EQEmu`'s world server runs it, built
//! from its public source and inferred for `RoF2` (6.499).
//!
//! The client logs in with the account's session key
//! ([`eq_network_game::world_login::rof2`]); the world sends what it says of
//! itself and, unless the client is between zones, the character list. The
//! client enters a character and the world hands it off to its zone
//! (`world/client.cpp`: `Client::HandleSendLoginInfoPacket`,
//! `Client::HandleEnterWorldPacket`, `Client::EnterWorld`,
//! `Client::TellClientZoneUnavailable`; opcodes from
//! `utils/patches/patch_RoF2.conf`).
//!
//! Stock `EQEmu` needs nothing else from the client before the entry, and
//! ignores the approvals and readiness the `RoF2` client may send, so none
//! of those are sent: what the client sends, and when it enters between
//! zones, is not read yet (6.499). What the world says of itself (guilds,
//! world name, expansions, chat servers, membership) is left unread.
use super::{character_list, named, unavailable};
use crate::{
    assets::Assets,
    client::{
        selection::{Choice, Selection},
        session::{put_string, record_chat, servers, CharacterSession, ZoneDestination},
        ClientEvent, ConnectionStage, Events,
    },
    transport::Session,
};
use anyhow::{ensure, Context, Result};
use eq_network_game::{characters, world_login, GameDialect};
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

/// `OP_EnterWorld`: the client's entry of a character, and the world's
/// naming of one (inferred, 6.499).
const ENTER_WORLD: u16 = 0x578f;
/// `OP_PostEnterWorld`, the last of what the world sends after the login
/// info (inferred, 6.499).
const POST_ENTER_WORLD: u16 = 0x6259;
/// `OP_ZoneServerInfo`, the handoff to the character's zone (inferred,
/// 6.499).
const ZONE_SERVER: u16 = 0x4c44;
/// `OP_ZoneUnavail` (inferred, 6.499).
const ZONE_UNAVAILABLE: u16 = 0x4cb4;
/// `RoF2`'s handoff: the zone server's address in a 128-byte field, then
/// its port (`rof2_structs.h`, `ZoneServerInfo_Struct`; `rof2.cpp`,
/// `ENCODE(OP_ZoneServerInfo)`; inferred, 6.499).
const ZONE_SERVER_LENGTH: usize = 130;
const ZONE_SERVER_ADDRESS: usize = 128;
/// `RoF2`'s zone-unavailable notice, sent untranslated in the shared
/// layout: the zone's name in 16 bytes, then 8 more (`common/eq_packet_structs.h`,
/// `ZoneUnavail_Struct`; inferred, 6.499).
const ZONE_UNAVAILABLE_LENGTH: usize = 24;

/// `RoF2`'s world stage: log in, choose a character, and follow its handoff.
///
/// # Errors
/// Returns an error when the world refuses the client, the zone is
/// unavailable, the world's messages are malformed, or it falls silent.
/// Stock `EQEmu`'s handoff carries no file manifest, so the installation's
/// checksums aren't asked for.
pub(in crate::client::session) fn world(
    context: &CharacterSession<'_>,
    _assets: &Assets,
    ip: &str,
    flags: (bool, bool),
    log: &mut Events<'_>,
) -> Result<Option<ZoneDestination>> {
    let config = &context.config;
    world_at(
        context,
        crate::client::endpoint(ip, 9000, config.local_only)?,
        flags,
        log,
    )
}

/// Runs the world stage at a resolved endpoint.
fn world_at(
    context: &CharacterSession<'_>,
    address: SocketAddr,
    (world_only, zoning): (bool, bool),
    log: &mut Events<'_>,
) -> Result<Option<ZoneDestination>> {
    let config = &context.config;
    let stop = context.stop;
    let server = servers::server_type(config.protocol);
    let mut session = Session::connect_cancellable(address, true, stop.flag())?;
    let credentials = context.credentials;
    session.send(
        world_login::ROF2_OPCODE,
        &world_login::rof2(&credentials.account, &credentials.key, zoning)?,
    )?;
    let mut deadline = Instant::now() + Duration::from_secs(60);
    let mut entered = false;
    let mut selection = None;
    let mut chosen: Option<String> = None;
    loop {
        ensure!(!stop.is_cancelled(), "shutdown requested");
        if !entered {
            let choice = match &selection {
                // No server filters are kept: RoF2's filter packet is not built.
                Some(list) => Selection::poll(list, context.commands, None, log)?,
                None => None,
            };
            match choice {
                // No generator of RoF2's is built: the player is told none came.
                Some(Choice::GenerateName(list, request, _, _)) => {
                    super::super::creation::names::answer(list, request, None, log)?;
                }
                Some(Choice::Enter(name)) => chosen = Some(name),
                _ => (),
            }
            if let Some(name) = chosen.as_deref() {
                deadline = enter(&mut session, name, log)?;
                entered = true;
            }
        }
        ensure!(
            selection.is_some() && !entered || Instant::now() < deadline,
            "world handshake timed out"
        );
        let Some(packet) = session.receive()? else {
            continue;
        };
        record_chat(config, "", &packet, log)?;
        match packet.opcode {
            characters::ROF2_OPCODE if !entered && selection.is_none() => {
                let entries = character_list(server, GameDialect::Rof2, &packet.body)?;
                log.send(ClientEvent::Progress(ConnectionStage::SelectingCharacter))?;
                if world_only {
                    session.close()?;
                    return Ok(None);
                }
                // The server type offers neither creation nor deletion.
                let (list, automatic) =
                    Selection::new(entries, &config.character, None, 0, server.deletion(), log)?;
                selection = Some(list);
                chosen = automatic;
            }
            ENTER_WORLD => named(&packet.body, &config.character, zoning, log)?,
            // Between zones no list comes: the character in play is entered
            // once the world has said what it says after the login info
            // (inferred, 6.499).
            POST_ENTER_WORLD if zoning && !entered => {
                chosen = Some(config.character.clone());
            }
            ZONE_UNAVAILABLE => {
                ensure!(
                    packet.body.len() == ZONE_UNAVAILABLE_LENGTH,
                    "invalid RoF2 zone-unavailable length"
                );
                let error = unavailable(&packet.body, log)?;
                session.close()?;
                return Err(error);
            }
            ZONE_SERVER if entered => {
                let (host, port) = zone_server(&packet.body)?;
                session.close()?;
                return Ok(Some(ZoneDestination {
                    character: chosen.context("zone handoff before character selection")?,
                    host,
                    port,
                    // Stock EQEmu's handoff carries no file manifest.
                    checksums: Vec::new(),
                    shield: None,
                }));
            }
            opcode => log.diagnostic(format!(
                "RoF2 world sent 0x{opcode:04x} ({} bytes), left unread",
                packet.body.len()
            ))?,
        }
    }
}

/// Reads `RoF2`'s handoff: an address terminated inside its field, and a
/// port, in exactly [`ZONE_SERVER_LENGTH`] bytes.
///
/// # Errors
/// Refuses any other length, an unterminated, empty or non-ASCII address,
/// and port zero.
fn zone_server(body: &[u8]) -> Result<(String, u16)> {
    ensure!(
        body.len() == ZONE_SERVER_LENGTH,
        "invalid RoF2 zone handoff length"
    );
    let field = &body[..ZONE_SERVER_ADDRESS];
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .context("unterminated RoF2 zone address")?;
    let host = &field[..end];
    ensure!(
        !host.is_empty() && host.iter().all(u8::is_ascii_graphic),
        "invalid RoF2 zone address"
    );
    let port = u16::from_le_bytes([body[ZONE_SERVER_ADDRESS], body[ZONE_SERVER_ADDRESS + 1]]);
    ensure!(port != 0, "invalid RoF2 zone port");
    Ok((String::from_utf8_lossy(host).into_owned(), port))
}

/// The body that enters a character: its name, and neither the tutorial
/// nor home (`rof2_structs.h`, `EnterWorld_Struct`; inferred, 6.499).
///
/// # Errors
/// Refuses an empty name, one holding a NUL, and one too long for the
/// 64-byte field with its NUL, rather than cut it.
fn enter_body(name: &str) -> Result<[u8; 72]> {
    ensure!(!name.is_empty(), "no character to enter");
    let mut body = [0; 72];
    put_string(&mut body[..64], name)?;
    Ok(body)
}

/// Enters the character and starts the handoff's deadline.
fn enter(session: &mut Session, name: &str, log: &mut Events<'_>) -> Result<Instant> {
    let body = enter_body(name)?;
    session.send(ENTER_WORLD, &body)?;
    name.clone_into(&mut log.character);
    log.send(ClientEvent::Progress(ConnectionStage::ConnectingZone))?;
    Ok(Instant::now() + Duration::from_secs(60))
}

#[cfg(test)]
mod tests;
