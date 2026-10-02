mod presentation;

use super::session::{
    admission::{Admission, EqMacAdmission, Handshake},
    feature::World,
    login::Credentials,
};
use super::{
    CancellationToken, ClientCommand, ClientConfig, ClientEvent, ConnectionStage, ConnectionState,
    DecodeError, Events, RecordEvent, RunOptions,
};
use crate::{chat, old_transport::OldSession, transport::Application};
use anyhow::{bail, ensure, Result};
use eq_network_game::{
    command,
    quarm::{dll_version_reply, ZONE_CHANGE_REQUEST, ZONE_LOGOUT, ZONE_SPAWN_APPEARANCE},
};
use std::{
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const WORLD_LOGIN: u16 = 0x5818;
const WORLD_CHARACTER_LIST: u16 = 0x4740;
const WORLD_ENTER: u16 = 0x0180;
const WORLD_ZONE_SERVER: u16 = 0x0480;

/// Run one complete TAKP/EQMac login, world selection, and Quarm zone attempt.
pub(super) fn run(
    config: &ClientConfig,
    stop: &CancellationToken,
    options: &RunOptions,
    commands: Option<&Receiver<ClientCommand>>,
    log: &mut Events<'_>,
) -> Result<()> {
    ensure!(!stop.is_cancelled(), "shutdown requested");
    let (credentials, world_ip) = super::session::login(config, stop, log)?;
    world(
        config,
        &credentials,
        &world_ip,
        stop,
        options,
        commands,
        log,
    )
}

/// Authenticate to the `MacPC` world server, choose a character, and follow its handoff.
fn world(
    config: &ClientConfig,
    credentials: &Credentials,
    ip: &str,
    stop: &CancellationToken,
    options: &RunOptions,
    commands: Option<&Receiver<ClientCommand>>,
    log: &mut Events<'_>,
) -> Result<()> {
    let mut session = OldSession::connect_cancellable(
        super::endpoint(ip, 9000, config.local_only)?,
        stop.flag(),
    )?;
    let mut config = config.clone();
    session.send(WORLD_LOGIN, &*world_login(credentials)?)?;
    let mut deadline = Instant::now() + Duration::from_secs(60);
    let mut entered = false;
    let mut selection = None;
    let mut chosen = None;
    loop {
        ensure!(!stop.is_cancelled(), "shutdown requested");
        if !entered {
            if let Some(choice) = selection
                .as_ref()
                .and_then(|list: &super::selection::Selection| list.poll(commands))
                .and_then(super::selection::Choice::entered)
            {
                chosen = Some(choice);
            }
            if let Some(name) = chosen.take() {
                let mut enter = [0; 64];
                put_string(&mut enter, &name)?;
                session.send(WORLD_ENTER, &enter)?;
                config.character = name;
                log.character.clone_from(&config.character);
                entered = true;
                deadline = Instant::now() + Duration::from_secs(60);
                log.send(ClientEvent::Progress(ConnectionStage::ConnectingZone))?;
            }
        }
        ensure!(
            selection.is_some() && !entered || Instant::now() < deadline,
            "world handshake timed out"
        );
        let Some(packet) = session.receive()? else {
            continue;
        };
        record_chat(&config, "", &packet, log)?;
        log.diagnostic(format!(
            "Quarm world received 0x{:04x} ({} bytes)",
            packet.opcode,
            packet.body.len()
        ))?;
        match packet.opcode {
            WORLD_CHARACTER_LIST if !entered && selection.is_none() => {
                let entries =
                    eq_network_game::characters::decode(config.protocol.into(), &packet.body)?;
                log.send(ClientEvent::Progress(ConnectionStage::SelectingCharacter))?;
                if options.world_only {
                    session.close()?;
                    return Ok(());
                }
                let (list, automatic) =
                    super::selection::Selection::new(entries, &config.character, log)?;
                selection = Some(list);
                chosen = automatic;
            }
            WORLD_ZONE_SERVER if entered => {
                let (host, port) = zone_destination(&packet.body)?;
                session.close()?;
                return zone(&config, stop, options, commands, log, &host, port);
            }
            _ => (),
        }
    }
}

#[cfg(test)]
fn character_exists(body: &[u8], character: &str) -> Result<bool> {
    ensure!(body.len() >= 640, "truncated EQMac character list");
    Ok(body[..640]
        .as_chunks::<64>()
        .0
        .iter()
        .any(|name| cstr(name).eq_ignore_ascii_case(character.as_bytes())))
}

fn world_login(credentials: &Credentials) -> Result<Zeroizing<[u8; 200]>> {
    let mut body = Zeroizing::new([0; 200]);
    put_string(&mut body[..127], &credentials.account)?;
    let key_start = credentials.account.len() + 1;
    ensure!(
        key_start + credentials.key.len() <= 127,
        "TAKP session fields are too large"
    );
    body[key_start..key_start + credentials.key.len()].copy_from_slice(&credentials.key);
    Ok(body)
}

/// Read the world-to-zone endpoint advertised by an `EQMac` world server.
fn zone_destination(body: &[u8]) -> Result<(String, u16)> {
    ensure!(body.len() >= 130, "truncated EQMac zone handoff");
    let host = std::str::from_utf8(cstr(&body[..128]))?.to_owned();
    // Unlike the Titanium handoff, EQMac carries this port in network byte order.
    let port = u16::from_be_bytes(body[128..130].try_into().unwrap());
    ensure!(!host.is_empty() && port != 0, "invalid EQMac zone endpoint");
    Ok((host, port))
}

/// Complete the `EQMac` zone admission sequence and collect communications.
// The linear handshake mirrors the order required by the legacy zone server.
#[allow(clippy::too_many_lines)]
fn zone(
    config: &ClientConfig,
    stop: &CancellationToken,
    options: &RunOptions,
    commands: Option<&Receiver<ClientCommand>>,
    log: &mut Events<'_>,
    host: &str,
    port: u16,
) -> Result<()> {
    let mut session = OldSession::connect_cancellable(
        super::endpoint(host, port, config.local_only)?,
        stop.flag(),
    )?;
    let session_id = rand::random();
    // The shared session's EQMac handshake, which this loop runs until Quarm
    // moves onto that session; its world only serves the handshake.
    let mut world = World::new(session_id);
    let mut shield = None;
    let mut admission = EqMacAdmission::start(
        &config.character,
        &mut Handshake {
            session: &mut session,
            shield: &mut shield,
            key: &[],
            checksums: &mut [],
            log: &mut *log,
        },
    )?;

    let mut presentation = presentation::Presentation::default();
    let mut rejected_layouts = std::collections::HashSet::new();
    let connected = Instant::now();
    let mut ready = false;
    let mut zone_name = String::new();
    let mut packets = 0u64;
    let mut progress = Instant::now();
    loop {
        if stop.is_cancelled()
            || options
                .zone_duration
                .is_some_and(|duration| connected.elapsed() >= duration)
        {
            session.close()?;
            return Ok(());
        }
        ensure!(
            ready || connected.elapsed() < Duration::from_secs(60),
            "zone admission timed out while {}",
            admission.stage()
        );
        if progress.elapsed() >= Duration::from_secs(30) {
            log.status(
                if ready && session.last_received_seconds() < 60 {
                    ConnectionState::Connected
                } else {
                    ConnectionState::Zoning
                },
                packets,
                Some(session.last_received_seconds()),
            )?;
            log.diagnostic(format!(
                "Quarm zone session: {packets} application packets, {} communication records",
                log.messages
            ))?;
            progress = Instant::now();
        }
        if ready {
            if let Some(commands) = commands {
                for command in commands.try_iter().take(64) {
                    match command::encode(config.protocol.into(), &command, &config.character) {
                        Ok(packet) => session.send(packet.opcode, &packet.body)?,
                        Err(error) => log.diagnostic(format!(
                            "Rejected invalid outbound client command: {error}"
                        ))?,
                    }
                }
            }
        }
        let Some(mut packet) = session.receive()? else {
            continue;
        };
        packets += 1;
        world.packets = packets;
        if !ready {
            log.diagnostic(format!(
                "Quarm zone received 0x{:04x} ({} bytes)",
                packet.opcode,
                packet.body.len()
            ))?;
        }
        if ready {
            match packet.opcode {
                ZONE_SPAWN_APPEARANCE => {
                    if let Some(response) = dll_version_reply(&packet.body) {
                        session.send(ZONE_SPAWN_APPEARANCE, &response)?;
                    }
                }
                ZONE_LOGOUT => bail!("server logged the character out"),
                ZONE_CHANGE_REQUEST => {
                    bail!("server requested a new zone; reconnecting through world")
                }
                _ => (),
            }
        } else if let Some((_, zone)) = admission.read(
            &mut packet,
            &mut world,
            &mut Handshake {
                session: &mut session,
                shield: &mut shield,
                key: &[],
                checksums: &mut [],
                log: &mut *log,
            },
        )? {
            zone_name = zone.name;
            ready = true;
            log.send(ClientEvent::Progress(ConnectionStage::Ready))?;
            log.status(
                ConnectionState::Connected,
                packets,
                Some(session.last_received_seconds()),
            )?;
            log.diagnostic(format!(
                "Quarm zone login sequence complete for {zone_name}; waiting for ongoing server traffic"
            ))?;
        }
        match presentation.receive(packet.opcode, &packet.body, &config.character) {
            Ok(events) => {
                for event in events {
                    log.send(ClientEvent::World(event))?;
                }
            }
            Err(error) if rejected_layouts.insert(packet.opcode) => {
                log.diagnostic(format!(
                    "Quarm world-state decode rejected 0x{:04x} ({} bytes): {error}",
                    packet.opcode,
                    packet.body.len()
                ))?;
            }
            Err(_) => (),
        }
        if ready {
            for event in presentation.enter(session_id, &zone_name) {
                log.send(ClientEvent::World(event))?;
            }
        }
        record_chat(config, &zone_name, &packet, log)?;
    }
}

fn record_chat(
    config: &ClientConfig,
    zone: &str,
    packet: &Application,
    log: &mut Events<'_>,
) -> Result<()> {
    match chat::parse_for(
        config.protocol.into(),
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
    &bytes[..bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len())]
}

#[cfg(test)]
mod zone_tests;

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn config() -> ClientConfig {
        ClientConfig::for_protocol(
            super::super::ServerProtocol::Quarm,
            "EXAMPLE_ACCOUNT",
            "EXAMPLE_PASSWORD",
            "The Project Quarm Server",
            "ExampleCharacter",
        )
    }

    #[test]
    fn zone_handoff_uses_network_byte_order_for_the_port() {
        // EQMacEmu world/client.cpp serializes ntohs(GetCPort()) in this field.
        let mut body = [0; 130];
        put_string(&mut body[..128], "203.0.113.20").unwrap();
        body[128..].copy_from_slice(&7000u16.to_be_bytes());
        assert_eq!(
            zone_destination(&body).unwrap(),
            ("203.0.113.20".into(), 7000)
        );
        assert!(zone_destination(&body[..129]).is_err());
        body[128..].fill(0);
        assert!(zone_destination(&body).is_err());
    }

    #[test]
    fn world_login_is_the_200_byte_windows_eqmac_form() {
        let credentials = Credentials {
            account: "LS#12345".into(),
            key: *b"ABCDEFGHIJ",
        };
        let body = world_login(&credentials).unwrap();
        assert_eq!(body.len(), 200);
        assert_eq!(cstr(&body[..127]), b"LS#12345");
        assert_eq!(&body[9..19], b"ABCDEFGHIJ");
        assert!(body[19..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn character_list_uses_ten_fixed_64_byte_name_fields() {
        let mut body = vec![0; 1620];
        put_string(&mut body[3 * 64..4 * 64], "ExampleCharacter").unwrap();
        assert!(character_exists(&body, "examplecharacter").unwrap());
        assert!(!character_exists(&body, "MissingCharacter").unwrap());
        assert!(character_exists(&body[..639], "ExampleCharacter").is_err());
    }
}
