mod actions;
mod admission;
mod camp;
mod casting;
mod character;
mod combat;
mod doors;
mod entities;
mod feature;
mod inventory;
mod lifecycle;
mod looting;
mod motion;
mod objects;
mod posture;
mod servers;
mod spellbook;
mod talk;
mod targeting;
mod transfers;
mod zone;

use super::{
    CancellationToken, ClientCommand, ClientConfig, ClientEvent, ClientIdentity, ConnectionStage,
    ConnectionState, DecodeError, Events, LoginError, RecordEvent, RunOptions, ServerProtocol,
};
use crate::{
    assets::Assets,
    chat,
    transport::{Application, Session},
};
use anyhow::{bail, ensure, Context, Result};
use eq_network_game::zoning;
use eq_network_login::{
    crypto::{des_decrypt, DesKeyIv},
    login::{encrypt_login_credentials, is_bad_password_login_result},
    server_list::parse_server_list,
};
use servers::Shield;
use std::{
    net::IpAddr,
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

#[derive(Zeroize, ZeroizeOnDrop)]
struct Credentials {
    account: u32,
    key: [u8; 10],
}

/// Run one complete login/world/zone attempt with fresh session credentials.
pub(super) fn run(
    config: &ClientConfig,
    identity: &ClientIdentity,
    assets: &Assets,
    stop: &CancellationToken,
    options: &RunOptions,
    commands: Option<&Receiver<ClientCommand>>,
    log: &mut Events<'_>,
) -> Result<()> {
    match config.protocol {
        ServerProtocol::Project1999 | ServerProtocol::EqEmu => {
            run_p99(config, identity, assets, stop, options, commands, log)
        }
        ServerProtocol::Quarm => super::quarm::run(config, stop, options, commands, log),
    }
}

fn run_p99(
    config: &ClientConfig,
    identity: &ClientIdentity,
    assets: &Assets,
    stop: &CancellationToken,
    options: &RunOptions,
    commands: Option<&Receiver<ClientCommand>>,
    log: &mut Events<'_>,
) -> Result<()> {
    ensure!(!stop.is_cancelled(), "shutdown requested");
    let (credentials, ip) = login(config, stop, log)?;
    let mut context = CharacterSession {
        config: config.clone(),
        identity,
        credentials: &credentials,
        stop,
        duration: options.zone_duration,
        commands,
    };
    let mut destination = world(&context, assets, &ip, options.world_only, false, log)?;
    while let Some(mut next_zone) = destination {
        context.config.character.clone_from(&next_zone.character);
        match zone::run(
            &context,
            &mut next_zone.shield,
            &next_zone.host,
            next_zone.port,
            next_zone.checksums,
            log,
        )? {
            ZoneExit::Stopped => return Ok(()),
            ZoneExit::World => {
                destination = world(&context, assets, &ip, false, true, log)?;
            }
            ZoneExit::CharacterSelect => {
                // Show the list again rather than re-entering the camped character.
                context.config.character.clear();
                destination = world(&context, assets, &ip, false, false, log)?;
            }
            ZoneExit::Direct(packet) => {
                let (host, port, checksums) =
                    decode_destination(next_zone.shield.as_deref(), &packet, assets, log)?;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoginOpcode {
    Ready,
    Accepted,
    ServerList,
    PlayResponse,
    Unknown(u16),
}

impl From<u16> for LoginOpcode {
    fn from(value: u16) -> Self {
        match value {
            0x16 => Self::Ready,
            0x17 => Self::Accepted,
            0x18 => Self::ServerList,
            0x21 => Self::PlayResponse,
            value => Self::Unknown(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorldOpcode {
    LogServer,
    ApprovalChallenge,
    FileManifest,
    ValidationResult,
    CharacterList,
    ZoneHandoff,
    ApproveName,
    Unknown(u16),
}

impl From<u16> for WorldOpcode {
    fn from(value: u16) -> Self {
        match value {
            0x0fa6 => Self::LogServer,
            0x3c25 => Self::ApprovalChallenge,
            0x52a4 => Self::FileManifest,
            0x1251 => Self::ValidationResult,
            0x4513 => Self::CharacterList,
            0x61b6 => Self::ZoneHandoff,
            0x3ea6 => Self::ApproveName,
            value => Self::Unknown(value),
        }
    }
}

/// Wait for one application packet while enforcing shutdown and a deadline.
fn next(session: &mut Session, deadline: Instant, stop: &CancellationToken) -> Result<Application> {
    loop {
        ensure!(!stop.is_cancelled(), "shutdown requested");
        ensure!(Instant::now() < deadline, "application handshake timed out");
        if let Some(packet) = session.receive()? {
            return Ok(packet);
        }
    }
}

/// Authenticate, select the configured server, and return its world endpoint.
fn login(
    config: &ClientConfig,
    stop: &CancellationToken,
    log: &mut Events<'_>,
) -> Result<(Credentials, String)> {
    let mut session = Session::connect_cancellable(
        super::endpoint(&config.host, config.port, config.local_only)?,
        false,
        stop.flag(),
    )?;
    let mut ready = vec![0; 12];
    ready[0] = 2;
    ready[9] = 8;
    session.send(1, &ready)?;
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut credentials = None;
    let mut selected = None;
    let mut sent_credentials = false;
    loop {
        let packet = next(&mut session, deadline, stop)?;
        match LoginOpcode::from(packet.opcode) {
            LoginOpcode::Ready if !sent_credentials => {
                log.send(ClientEvent::Progress(ConnectionStage::Authenticating))?;
                let mut body = vec![0; 10];
                body[0] = 3;
                body[5] = 2;
                body.extend(encrypt_login_credentials(
                    config.credentials.account(),
                    config.credentials.password(),
                    DesKeyIv::default(),
                ));
                session.send(2, &body)?;
                sent_credentials = true;
            }
            LoginOpcode::Accepted => {
                credentials = Some(login_credentials(&packet.body)?);
                log.send(ClientEvent::Progress(ConnectionStage::SelectingServer))?;
                let mut request = vec![0; 10];
                request[0] = 4;
                session.send(4, &request)?;
                log.diagnostic("Login server authenticated the account".into())?;
            }
            LoginOpcode::ServerList => {
                ensure!(packet.body.len() >= 20, "truncated server list");
                let mut body = 0x18u16.to_le_bytes().to_vec();
                body.extend(packet.body);
                let (servers, _) = parse_server_list(&body).context("invalid server list")?;
                let server = servers
                    .into_iter()
                    .find(|server| server.name.eq_ignore_ascii_case(&config.server))
                    .context("configured server name was not found in the server list")?;
                ensure!(
                    matches!(server.status, 0 | 2),
                    "configured server is unavailable or locked"
                );
                let mut request = vec![0; 14];
                request[0] = 5;
                request[10..14].copy_from_slice(&server.runtime_id.to_le_bytes());
                session.send(0x0d, &request)?;
                selected = Some(server.ip);
            }
            LoginOpcode::PlayResponse => {
                ensure!(packet.body.len() >= 20, "truncated world-entry response");
                ensure!(
                    packet.body[10] > 0,
                    "login server denied world entry (message {})",
                    le32(&packet.body[11..15])
                );
                session.close()?;
                let selection = (
                    credentials.context("play response before authentication")?,
                    selected.context("play response before selection")?,
                );
                log.send(ClientEvent::Progress(ConnectionStage::ConnectingWorld))?;
                return Ok(selection);
            }
            _ => (),
        }
    }
}

/// Use the SSO crate's failure signature before parsing the successful session key.
fn login_credentials(body: &[u8]) -> Result<Credentials> {
    let mut application = 0x17u16.to_le_bytes().to_vec();
    application.extend_from_slice(body);
    if is_bad_password_login_result(&application, DesKeyIv::default()) {
        return Err(LoginError::InvalidCredentials.into());
    }
    ensure!(body.len() >= 34, "invalid login response");
    let ciphertext = &body[10..];
    let clear = Zeroizing::new(des_decrypt(
        &ciphertext[..ciphertext.len() / 8 * 8],
        DesKeyIv::default(),
    )?);
    ensure!(clear.len() >= 23, "invalid login response");
    let account = le32(&clear[8..12]);
    ensure!(
        account != 0 && account != u32::MAX && clear[0] == 1,
        "login was rejected"
    );
    let key = cstr(&clear[12..23])
        .try_into()
        .context("invalid login session key")?;
    Ok(Credentials { account, key })
}

/// Build the current P99 CRC1 client-validation response.
fn crc1(
    assets: &Assets,
    local_ip: IpAddr,
    identity: &ClientIdentity,
    key: &[u8],
) -> Result<Vec<u8>> {
    let spells = assets.spells()?;
    let mut body = vec![0; 2056];
    body[..4].copy_from_slice(&(!spells.crc32).to_le_bytes());
    body[4..8].copy_from_slice(&u32::try_from(spells.size)?.to_le_bytes());
    // Current V62 replaces the legacy random spell samples with this metadata
    // block. These fields also occur in successful, decoded stock responses.
    body[8..12].fill(0xff);
    body[45] = 1;
    body[46] = 27;
    put_string(&mut body[50..66], &identity.hostname)?;
    put_string(&mut body[66..82], &identity.username)?;
    body[82..86].copy_from_slice(&[127, 0, 0, 1]);
    if let IpAddr::V4(ip) = local_ip {
        if !ip.is_loopback() {
            body[86..90].copy_from_slice(&ip.octets());
        }
    }
    crate::p99::session_xor(&mut body[..2048], key)?;
    Ok(body)
}

struct CharacterSession<'a> {
    config: ClientConfig,
    identity: &'a ClientIdentity,
    credentials: &'a Credentials,
    stop: &'a CancellationToken,
    duration: Option<Duration>,
    commands: Option<&'a Receiver<ClientCommand>>,
}

/// Warn about unscanned files while allowing the server to evaluate checksum zero.
fn file_response(assets: &Assets, manifest: &[u8], log: &mut Events<'_>) -> Result<Vec<u8>> {
    let response = assets.file_response(manifest)?;
    if !response.unknown_files.is_empty() {
        log.diagnostic(format!(
            "Warning: asset inventory has no entry for {}; sending checksum 0",
            response.unknown_files.join(", ")
        ))?;
    }
    Ok(response.body)
}

struct ZoneDestination {
    character: String,
    host: String,
    port: u16,
    checksums: Vec<u8>,
    /// The protection the world connection started, kept across zones.
    shield: Option<Box<dyn Shield>>,
}

enum ZoneExit {
    Stopped,
    World,
    Direct(Vec<u8>),
    CharacterSelect,
}

/// Decode the endpoint and, behind protection, the file manifest before
/// rekeying for destination admission.
fn decode_destination(
    shield: Option<&dyn Shield>,
    packet: &[u8],
    assets: &Assets,
    log: &mut Events<'_>,
) -> Result<(String, u16, Vec<u8>)> {
    ensure!(packet.len() >= 130, "invalid zone handoff");
    let host = std::str::from_utf8(cstr(&packet[..128]))?.to_owned();
    let port = u16::from_le_bytes(packet[128..130].try_into()?);
    ensure!(!host.is_empty() && port != 0, "invalid zone endpoint");
    // Stock EQEmu hands off only the endpoint; P99 appends an encrypted manifest.
    let Some(shield) = shield.filter(|_| packet.len() > 130) else {
        return Ok((host, port, Vec::new()));
    };
    let manifest = shield.zone_manifest(packet)?;
    Ok((host, port, file_response(assets, &manifest, log)?))
}

/// Logs in to the world with the exact login body and starts the server's
/// protection, if it has any, from that body.
fn open_world(
    session: &mut Session,
    server: &dyn servers::ServerType,
    credentials: &Credentials,
    zoning: bool,
) -> Result<Option<Box<dyn Shield>>> {
    let mut login_info = Zeroizing::new(vec![0; 464]);
    login_info[192] = 0xcc;
    login_info[188] = u8::from(zoning);
    let account = credentials.account.to_string();
    login_info[..account.len()].copy_from_slice(account.as_bytes());
    login_info[account.len() + 1..account.len() + 1 + credentials.key.len()]
        .copy_from_slice(&credentials.key);
    let shield = server.protect(&login_info)?;
    session.send(0x4dd0, &login_info)?;
    Ok(shield)
}

/// Sends the selected server name and starts the zone-handoff deadline.
fn enter_character(session: &mut Session, name: &str, log: &mut Events<'_>) -> Result<Instant> {
    let mut enter = [0; 72];
    put_string(&mut enter[..64], name)?;
    session.send(0x7cba, &enter)?;
    name.clone_into(&mut log.character);
    log.send(ClientEvent::Progress(ConnectionStage::ConnectingZone))?;
    Ok(Instant::now() + Duration::from_secs(60))
}

/// Complete world validation, select the character, and follow its zone handoff.
// The linear handshake keeps packet ordering and state transitions together.
#[allow(clippy::too_many_lines)]
fn world(
    context: &CharacterSession<'_>,
    assets: &Assets,
    ip: &str,
    world_only: bool,
    zoning: bool,
    log: &mut Events<'_>,
) -> Result<Option<ZoneDestination>> {
    let config = &context.config;
    let credentials = context.credentials;
    let stop = context.stop;
    let mut session = Session::connect_cancellable(
        super::endpoint(ip, 9000, config.local_only)?,
        true,
        stop.flag(),
    )?;
    let server = servers::server_type(config.protocol);
    let mut shield = open_world(&mut session, server, credentials, zoning)?;
    let mut deadline = Instant::now() + Duration::from_secs(60);
    let mut accepted = false;
    let mut entered = false;
    let mut selection = None;
    // After camping, the world sends the list once, before its empty file notice.
    let mut early_list: Option<Vec<u8>> = None;
    let mut chosen = None;
    // A creation request awaiting name approval (false) or its new list (true).
    let mut creating: Option<(eq_network_game::creation::NewCharacter, bool)> = None;
    // Stock EQEmu marks a world session that created a character as bound for the
    // tutorial until the client sends OP_World_Client_CRC1 (world/client.cpp).
    let mut tutorial_pending = false;
    loop {
        ensure!(!stop.is_cancelled(), "shutdown requested");
        if !entered {
            match selection
                .as_ref()
                .and_then(|list: &super::selection::Selection| list.poll(context.commands))
            {
                Some(super::selection::Choice::Enter(name)) => chosen = Some(name),
                Some(super::selection::Choice::Create(character)) if creating.is_none() => {
                    match character.name_approval() {
                        Ok(body) => {
                            session.send(eq_network_game::creation::APPROVE_NAME_OPCODE, &body)?;
                            creating = Some((character, false));
                        }
                        Err(error) => {
                            log.diagnostic(format!("Rejected character creation: {error}"))?;
                            log.send(ClientEvent::World(
                                crate::world::WorldEvent::CharacterCreation {
                                    name: character.name,
                                    accepted: false,
                                },
                            ))?;
                        }
                    }
                }
                _ => (),
            }
            if let Some(name) = chosen.as_ref() {
                if std::mem::take(&mut tutorial_pending) {
                    // The official client sends it unless "Start Tutorial" was
                    // chosen; stock EQEmu checks its contents only when checksum
                    // verification is on, which it is not by default.
                    session.send(0x5072, &[0; 2056])?;
                }
                deadline = enter_character(&mut session, name, log)?;
                entered = true;
            }
        }
        ensure!(
            selection.is_some() && !entered || Instant::now() < deadline,
            "world handshake timed out"
        );
        let Some(mut packet) = session.receive()? else {
            continue;
        };
        if let Some(event) = chat::parse(packet.opcode, &packet.body, config.include_raw)? {
            log.record("", RecordEvent::Chat(event))?;
        }
        log.diagnostic(format!(
            "World received 0x{:04x} ({} bytes)",
            packet.opcode,
            packet.body.len()
        ))?;
        match WorldOpcode::from(packet.opcode) {
            // Consumers name per-character files after the world, as the official client does.
            WorldOpcode::LogServer => match crate::world::titanium_world_name(&packet.body) {
                Ok(short_name) => {
                    let event = crate::world::WorldEvent::WorldName { short_name };
                    log.send(ClientEvent::World(event))?;
                }
                Err(error) => {
                    log.diagnostic(format!("World short name unavailable: {error}"))?;
                }
            },
            // Only protected servers expect an answer; stock EQEmu's is informational.
            WorldOpcode::ApprovalChallenge => {
                if let Some(shield) = shield.as_mut() {
                    session.send(0x3c25, &shield.approve(&packet.body)?)?;
                }
            }
            // Captured Titanium zoning sessions receive an empty notification here,
            // with no world checksum response. The official client then repeats
            // its character entry before the server sends ZoneHandoff.
            WorldOpcode::FileManifest if zoning && packet.body.is_empty() => {
                if !entered {
                    chosen = Some(config.character.clone());
                }
            }
            // Returning after camp, the world skips file validation (official capture).
            WorldOpcode::FileManifest if packet.body.is_empty() => {
                if !accepted {
                    accepted = true;
                    log.send(ClientEvent::Progress(ConnectionStage::SelectingCharacter))?;
                    session.send(0x7752, &0u32.to_le_bytes())?;
                    session.send(0x5e99, &[])?;
                    if let Some(body) = early_list.take() {
                        let entries =
                            eq_network_game::characters::decode(config.protocol.into(), &body)?;
                        let (list, automatic) =
                            super::selection::Selection::new(entries, &config.character, log)?;
                        selection = Some(list);
                        chosen = automatic;
                    }
                }
            }
            WorldOpcode::FileManifest => {
                if let Some(shield) = shield.as_mut() {
                    shield.manifest(&mut packet.body)?;
                }
                let mut response = file_response(assets, &packet.body, log)?;
                if let Some(shield) = shield.as_ref() {
                    shield.answer(&mut response)?;
                }
                session.send(
                    0x5072,
                    &crc1(
                        assets,
                        session.local_address()?.ip(),
                        context.identity,
                        &credentials.key,
                    )?,
                )?;
                session.send(0x1251, &response)?;
            }
            WorldOpcode::ValidationResult => {
                ensure!(
                    packet.body == [0],
                    "world client validation returned {}",
                    hex::encode(&packet.body)
                );
                accepted = true;
                log.send(ClientEvent::Progress(ConnectionStage::SelectingCharacter))?;
                log.diagnostic("World accepted native V62 client validation".into())?;
                session.send(0x7752, &0u32.to_le_bytes())?;
                session.send(0x5e99, &[])?;
            }
            // Name approved: send the creation; any refusal ends this attempt.
            WorldOpcode::ApproveName if creating.is_some() => {
                let approved = packet.body.first() == Some(&1);
                match creating.take() {
                    Some((character, false)) if approved => {
                        session.send(
                            eq_network_game::creation::CREATE_OPCODE,
                            &character.create_request()?,
                        )?;
                        creating = Some((character, true));
                    }
                    Some((character, _)) => {
                        log.send(ClientEvent::World(
                            crate::world::WorldEvent::CharacterCreation {
                                name: character.name,
                                accepted: false,
                            },
                        ))?;
                    }
                    None => (),
                }
            }
            WorldOpcode::CharacterList
                if accepted && !entered && creating.as_ref().is_some_and(|(_, sent)| *sent) =>
            {
                if let Some((character, _)) = creating.take() {
                    tutorial_pending |= server.start_choice();
                    log.send(ClientEvent::World(
                        crate::world::WorldEvent::CharacterCreation {
                            name: character.name,
                            accepted: true,
                        },
                    ))?;
                }
                let entries =
                    eq_network_game::characters::decode(config.protocol.into(), &packet.body)?;
                let (list, automatic) =
                    super::selection::Selection::new(entries, &config.character, log)?;
                selection = Some(list);
                chosen = automatic;
            }
            WorldOpcode::CharacterList if !accepted && !entered => {
                early_list = Some(packet.body.clone());
            }
            WorldOpcode::CharacterList if accepted && !entered && selection.is_none() => {
                let entries =
                    eq_network_game::characters::decode(config.protocol.into(), &packet.body)?;
                let (list, automatic) =
                    super::selection::Selection::new(entries, &config.character, log)?;
                selection = Some(list);
                chosen = automatic;
            }
            WorldOpcode::ZoneHandoff => {
                ensure!(entered, "unsolicited zone handoff");
                let (host, port, checksums) =
                    decode_destination(shield.as_deref(), &packet.body, assets, log)?;
                session.send(0x509d, &[])?;
                session.close()?;
                return Ok(Some(ZoneDestination {
                    character: chosen.context("zone handoff before character selection")?,
                    host,
                    port,
                    checksums,
                    shield,
                }));
            }
            _ => (),
        }
        // Stock EQEmu accepts the client without the V62 validation reply.
        if world_only && accepted {
            session.close()?;
            return Ok(None);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p99::{self, WorldCodec};

    #[test]
    fn zoning_handoff_uses_fresh_login_key_without_world_manifest_state() {
        // Synthetic zoning admission: no approval or world manifest precedes the
        // handoff. Neither the previous world key nor its rolling state can help.
        let assets: Assets = serde_json::from_str(
            r#"{"client":"test","files":{"example.s3d":{"crc32":1234,"size":5678}}}"#,
        )
        .unwrap();
        let config = ClientConfig::new("EXAMPLE_ACCOUNT", "EXAMPLE_PASSWORD", "Test", "Example");
        let mut handler = |_| Ok(());
        let mut log = Events::new(&config, &mut handler);
        let manifest = b"\x11\x00\x01example.s3d\0";
        let expected = assets.file_response(manifest).unwrap().body;
        for marker in [1, 2] {
            let mut login = [0; 464];
            login[0] = marker;
            login[188] = 1;
            let mut codec = WorldCodec::new(&login).unwrap();
            let mut handoff = vec![0; 130 + manifest.len()];
            handoff[..9].copy_from_slice(b"192.0.2.1");
            handoff[128..130].copy_from_slice(&7000u16.to_le_bytes());
            let seed = p99::packet_digest(0x61b6, &handoff).unwrap();
            let mut encoded = manifest.to_vec();
            p99::encode(&mut encoded, &md5::compute(login).0, &seed);
            handoff[130..].copy_from_slice(&encoded);
            let (host, port, mut checksums) =
                decode_destination(Some(&codec), &handoff, &assets, &mut log).unwrap();
            assert_eq!((host.as_str(), port), ("192.0.2.1", 7000));
            assert_eq!(checksums, expected);

            let mut entry = [0; 68];
            entry[4..11].copy_from_slice(b"Example");
            codec.zone_entry(&entry).unwrap();
            assert!(codec.file_response(&mut checksums).is_err());
            let spawn = [marker; 385];
            codec.zone_spawn(&spawn).unwrap();
            codec.file_response(&mut checksums).unwrap();
            p99::decode(
                &mut checksums,
                &md5::compute(entry).0,
                &p99::packet_digest(0x7213, &spawn).unwrap(),
            );
            assert_eq!(checksums, expected);
        }
    }

    #[test]
    fn unknown_asset_warning_reaches_the_host_without_aborting_the_response() {
        let assets: Assets =
            serde_json::from_str(r#"{"client":"test","files":{"absent.eqg":null}}"#).unwrap();
        let config = ClientConfig::new(
            "EXAMPLE_ACCOUNT",
            "EXAMPLE_PASSWORD",
            "Test Server",
            "ExampleCharacter",
        );
        let mut received = Vec::new();
        let mut handler = |event| {
            received.push(event);
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        let manifest = b"\x11\x00\x01unknown.eqg\0\x12\x00\x01absent.eqg\0";
        let response = file_response(&assets, manifest, &mut log).unwrap();
        let mut expected = crc32fast::hash(manifest).to_le_bytes().to_vec();
        expected.extend(b"\x11\x00\x00\x00\x00\x00\x12\x00\x00\x00\x00\x00");
        assert_eq!(response, expected);
        assert_eq!(received.len(), 1);
        let super::super::ClientEvent::Diagnostic(message) = &received[0] else {
            panic!("expected diagnostic event");
        };
        assert!(message.contains("unknown.eqg") && message.contains("checksum 0"));
        assert!(!message.contains("absent.eqg"));
    }

    #[test]
    fn validation_uses_host_metadata_and_the_current_session_key() {
        let assets: Assets = serde_json::from_str(
            r#"{"client":"test","files":{"spells_us.txt":{"crc32":1234,"size":5678}}}"#,
        )
        .unwrap();
        let identity = ClientIdentity {
            hostname: "TEST-DEVICE".into(),
            username: "test-user".into(),
        };
        let ip = "192.0.2.10".parse().unwrap();
        let first = crc1(&assets, ip, &identity, b"0123456789").unwrap();
        let mut second = crc1(&assets, ip, &identity, b"abcdefghij").unwrap();
        assert_ne!(first, second);
        p99::session_xor(&mut second[..2048], b"abcdefghij").unwrap();
        assert_eq!(second.len(), 2056);
        assert_eq!(&second[..4], &(!1234u32).to_le_bytes());
        assert_eq!(&second[4..8], &5678u32.to_le_bytes());
        assert_eq!(cstr(&second[50..66]), b"TEST-DEVICE");
        assert_eq!(cstr(&second[66..82]), b"test-user");
        assert_eq!(&second[82..90], &[127, 0, 0, 1, 192, 0, 2, 10]);
    }
}

#[cfg(test)]
mod login_tests {
    use super::*;
    use eq_network_login::crypto::des_encrypt;

    #[test]
    fn valid_session_keys_and_malformed_responses_are_not_bad_passwords() {
        let mut clear = vec![0; 32];
        clear[0] = 1;
        clear[8..12].copy_from_slice(&12345u32.to_le_bytes());
        clear[12..22].copy_from_slice(b"EXAMPLEKEY");
        let mut body = vec![0; 10];
        body[0] = 3;
        body[5] = 2;
        body.extend(des_encrypt(&clear, DesKeyIv::default()));
        let credentials = login_credentials(&body).unwrap();
        assert_eq!(credentials.account, 12345);
        assert_eq!(&credentials.key, b"EXAMPLEKEY");
        for length in [0, 10, 18, 26] {
            let error = login_credentials(&body[..length]).err().unwrap();
            assert!(!error.is::<LoginError>());
        }
    }
}
