mod abilities;
mod actions;
pub(super) mod admission;
mod camp;
mod casting;
mod character;
mod clock;
mod combat;
mod corpses;
mod creation;
mod doors;
mod entities;
mod exchange;
pub(super) mod feature;
mod groups;
mod inventory;
mod lifecycle;
pub(super) mod login;
mod looting;
mod map;
mod motion;
mod objects;
mod offers;
mod pets;
mod posture;
mod reading;
mod resurrection;
mod servers;
mod spellbook;
mod talk;
mod targeting;
mod tradeskills;
mod training;
mod transfers;
mod who;
mod wire;
mod world;
pub(super) mod zone;

use super::{
    CancellationToken, ClientCommand, ClientConfig, ClientEvent, ClientIdentity, ConnectionStage,
    ConnectionState, DecodeError, Events, RecordEvent, RunOptions, ServerProtocol,
};
use crate::{assets::Assets, chat, transport::Application};
use anyhow::{bail, ensure, Context, Result};
use eq_network_game::zoning;
use servers::Shield;
use std::{
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

use login::Credentials;

/// Run one complete login/world/zone attempt with fresh session credentials,
/// each stage the way the configured server's type runs it.
pub(super) fn run(
    config: &ClientConfig,
    identity: &ClientIdentity,
    assets: &Assets,
    stop: &CancellationToken,
    options: &RunOptions,
    commands: Option<&Receiver<ClientCommand>>,
    log: &mut Events<'_>,
) -> Result<()> {
    ensure!(!stop.is_cancelled(), "shutdown requested");
    let server = servers::server_type(config.protocol);
    let (credentials, ip) = server.wire().login(config, stop, log)?;
    let mut context = CharacterSession {
        config: config.clone(),
        identity,
        credentials: &credentials,
        stop,
        duration: options.zone_duration,
        commands,
    };
    let mut destination =
        server
            .wire()
            .world(&context, assets, &ip, (options.world_only, false), log)?;
    while let Some(mut next_zone) = destination {
        context.config.character.clone_from(&next_zone.character);
        match server.zone(
            &context,
            &mut next_zone.shield,
            (&next_zone.host, next_zone.port),
            next_zone.checksums,
            log,
        )? {
            ZoneExit::Stopped => return Ok(()),
            ZoneExit::World => {
                destination = server
                    .wire()
                    .world(&context, assets, &ip, (false, true), log)?;
            }
            ZoneExit::CharacterSelect => {
                // Show the list again rather than re-entering the camped character.
                context.config.character.clear();
                destination = server
                    .wire()
                    .world(&context, assets, &ip, (false, false), log)?;
            }
            ZoneExit::Direct(packet) => {
                let (host, port, checksums) =
                    server
                        .wire()
                        .handoff(next_zone.shield.as_deref(), &packet, assets, log)?;
                destination = Some(ZoneDestination {
                    host,
                    port,
                    checksums,
                    shield: next_zone.shield,
                    character: context.config.character.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Wait for one application packet while enforcing shutdown and a deadline.
fn next(
    session: &mut dyn eq_network_transport::Transport,
    deadline: Instant,
    stop: &CancellationToken,
) -> Result<Application> {
    loop {
        ensure!(!stop.is_cancelled(), "shutdown requested");
        ensure!(Instant::now() < deadline, "application handshake timed out");
        if let Some(packet) = session.receive()? {
            return Ok(packet);
        }
    }
}

/// What a character's stay in the world and its zones works with.
pub(in crate::client) struct CharacterSession<'a> {
    pub(in crate::client) config: ClientConfig,
    pub(in crate::client) identity: &'a ClientIdentity,
    pub(in crate::client) credentials: &'a Credentials,
    pub(in crate::client) stop: &'a CancellationToken,
    pub(in crate::client) duration: Option<Duration>,
    pub(in crate::client) commands: Option<&'a Receiver<ClientCommand>>,
}

/// Where the world sends the character next.
pub(in crate::client) struct ZoneDestination {
    pub(in crate::client) character: String,
    pub(in crate::client) host: String,
    pub(in crate::client) port: u16,
    pub(in crate::client) checksums: Vec<u8>,
    /// The protection the world connection started, kept across zones.
    pub(in crate::client) shield: Option<Box<dyn Shield>>,
}

/// How a zone session ends.
pub(in crate::client) enum ZoneExit {
    Stopped,
    World,
    Direct(Vec<u8>),
    CharacterSelect,
}

/// Records the communication a packet carries, as the server's client
/// generation reads it; an unreadable one is recorded as such.
///
/// # Errors
/// Returns an error when the host's record handler fails.
pub(in crate::client) fn record_chat(
    config: &ClientConfig,
    zone: &str,
    packet: &Application,
    log: &mut Events<'_>,
) -> Result<()> {
    match servers::server_type(config.protocol).wire().chat(
        packet.opcode,
        &packet.body,
        config.include_raw,
    ) {
        Ok(Some(event)) => log.record(zone, RecordEvent::Chat(event)),
        Ok(None) => Ok(()),
        Err(error) => log.record(
            zone,
            RecordEvent::DecodeError(DecodeError {
                kind: "decode_error",
                opcode: packet.opcode,
                payload_hex: hex::encode(&packet.body),
                error: error.to_string(),
            }),
        ),
    }
}

fn put_string(destination: &mut [u8], value: &str) -> Result<()> {
    ensure!(
        value.len() < destination.len() && !value.contains('\0'),
        "string exceeds protocol field size"
    );
    destination[..value.len()].copy_from_slice(value.as_bytes());
    Ok(())
}
fn cstr(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.iter().position(|&v| v == 0).unwrap_or(bytes.len())]
}
fn le32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}
