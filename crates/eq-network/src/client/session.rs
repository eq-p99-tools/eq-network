mod actions;
mod book_edits;
mod camp;
mod casting;
mod inventory;
mod lifecycle;
mod merchant;
mod motion;
mod scribe_consumption;
mod spellbook;
use eq_network_game::spells::BookActionStatus;
use lifecycle::ZoneLifecycle;
use spellbook::{BookIntent, PendingBookAction};

use super::{
    CancellationToken, ClientCommand, ClientConfig, ClientEvent, ClientIdentity, ConnectionStage,
    ConnectionState, DecodeError, Events, LoginError, RecordEvent, RunOptions, ServerProtocol,
};
use crate::{
    assets::Assets,
    chat,
    p99::{self, WorldCodec},
    transport::{Application, Session},
};
use anyhow::{bail, ensure, Context, Result};
use eq_network_game::{command, movement::MotionSession, zoning};
use eq_network_login::{
    crypto::{des_decrypt, DesKeyIv},
    login::{encrypt_login_credentials, is_bad_password_login_result},
    server_list::parse_server_list,
};
use std::{
    collections::BTreeMap,
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
        match zone(
            &context,
            &mut next_zone.codec,
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
                    decode_destination(&next_zone.codec, &packet, assets, log)?;
                destination = Some(ZoneDestination {
                    host,
                    port,
                    checksums,
                    codec: next_zone.codec,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ZoneOpcode {
    PlayerProfile,
    Weather,
    PlayerSpawn,
    ZoneDescription,
    ExperienceUpdate,
    ValidationRejected,
    LoggedOut,
    ZoneHandoff,
    Unknown(u16),
}

impl From<u16> for ZoneOpcode {
    fn from(value: u16) -> Self {
        match value {
            0x75df => Self::PlayerProfile,
            0x254d => Self::Weather,
            0x7213 => Self::PlayerSpawn,
            0x0920 => Self::ZoneDescription,
            0x0587 => Self::ExperienceUpdate,
            0x1252 => Self::ValidationRejected,
            0x3cdc => Self::LoggedOut,
            0x61b6 => Self::ZoneHandoff,
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
    p99::session_xor(&mut body[..2048], key)?;
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
    codec: WorldCodec,
}

enum ZoneExit {
    Stopped,
    World,
    Direct(Vec<u8>),
    CharacterSelect,
}

/// Decode the endpoint and V62 manifest before rekeying for destination admission.
fn decode_destination(
    codec: &WorldCodec,
    packet: &[u8],
    assets: &Assets,
    log: &mut Events<'_>,
) -> Result<(String, u16, Vec<u8>)> {
    ensure!(packet.len() >= 130, "invalid zone handoff");
    let host = std::str::from_utf8(cstr(&packet[..128]))?.to_owned();
    let port = u16::from_le_bytes(packet[128..130].try_into()?);
    ensure!(!host.is_empty() && port != 0, "invalid zone endpoint");
    // Stock EQEmu hands off only the endpoint; P99 appends an encrypted manifest.
    if packet.len() == 130 {
        return Ok((host, port, Vec::new()));
    }
    let manifest = codec.zone_manifest(packet)?;
    Ok((host, port, file_response(assets, &manifest, log)?))
}

/// Initializes the world codec from the exact login body sent to the server.
fn open_world(
    session: &mut Session,
    credentials: &Credentials,
    zoning: bool,
) -> Result<WorldCodec> {
    let mut login_info = Zeroizing::new(vec![0; 464]);
    login_info[192] = 0xcc;
    login_info[188] = u8::from(zoning);
    let account = credentials.account.to_string();
    login_info[..account.len()].copy_from_slice(account.as_bytes());
    login_info[account.len() + 1..account.len() + 1 + credentials.key.len()]
        .copy_from_slice(&credentials.key);
    let codec = WorldCodec::new(&login_info)?;
    session.send(0x4dd0, &login_info)?;
    Ok(codec)
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
    let mut codec = open_world(&mut session, credentials, zoning)?;
    let mut deadline = Instant::now() + Duration::from_secs(60);
    let mut accepted = false;
    let mut entered = false;
    let mut selection = None;
    // After camping, the world sends the list once, before its empty file notice.
    let mut early_list: Option<Vec<u8>> = None;
    let mut chosen = None;
    // A creation request awaiting name approval (false) or its new list (true).
    let mut creating: Option<(eq_network_game::creation::NewCharacter, bool)> = None;
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
            // Stock EQEmu's ApproveWorld is informational; only P99 expects an answer.
            WorldOpcode::ApprovalChallenge if config.protocol == ServerProtocol::EqEmu => {}
            WorldOpcode::ApprovalChallenge => {
                session.send(0x3c25, &codec.approve(&packet.body)?)?;
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
                codec.manifest(&mut packet.body)?;
                let mut response = file_response(assets, &packet.body, log)?;
                codec.file_response(&mut response)?;
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
                if world_only {
                    session.close()?;
                    return Ok(None);
                }
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
                    decode_destination(&codec, &packet.body, assets, log)?;
                session.send(0x509d, &[])?;
                session.close()?;
                return Ok(Some(ZoneDestination {
                    character: chosen.context("zone handoff before character selection")?,
                    host,
                    port,
                    checksums,
                    codec,
                }));
            }
            _ => (),
        }
    }
}

/// Enter the zone, keep the character stationary, and collect communications.
// The linear handshake keeps packet ordering and state transitions together.
#[allow(clippy::too_many_lines)]
fn zone(
    context: &CharacterSession<'_>,
    codec: &mut WorldCodec,
    host: &str,
    port: u16,
    mut checksums: Vec<u8>,
    log: &mut Events<'_>,
) -> Result<ZoneExit> {
    let config = &context.config;
    let credentials = context.credentials;
    let stop = context.stop;
    let duration = context.duration;
    let mut session = Session::connect_cancellable(
        super::endpoint(host, port, config.local_only)?,
        true,
        stop.flag(),
    )?;
    session.send(0x7752, &0u32.to_le_bytes())?;
    let mut entry = vec![0; 68];
    put_string(&mut entry[4..], &config.character)?;
    let stock = config.protocol == ServerProtocol::EqEmu;
    if !stock {
        codec.zone_entry(&entry)?;
    }
    session.send(0x7213, &entry)?;
    log.send(ClientEvent::Progress(ConnectionStage::LoadingCharacter))?;
    let connected = Instant::now();
    let mut ready = false;
    let mut lifecycle = ZoneLifecycle::default();
    let mut motion: Option<MotionSession> = None;
    let mut saw_spawn = false;
    let mut saw_profile = false;
    let mut saw_weather = false;
    let mut requested = false;
    let mut got_zone = false;
    let mut replied_experience = false;
    let mut zone_name = String::new();
    let mut far_clip = None;
    let mut progress = Instant::now();
    let mut packets = 0u64;
    let mut stationary = [0u8; 36];
    let session_id = rand::random();
    let mut initial_experience = None;
    let mut initial_level = None;
    let mut initial_skills = std::collections::BTreeMap::new();
    let mut inventory = eq_network_game::inventory::Inventory::default();
    let mut inventory_actor: Option<eq_network_game::inventory::InventoryActor> = None;
    let mut admitted_player: Option<crate::world::PlayerState> = None;
    let mut admitted_book: Option<eq_network_game::spells::SpellBook> = None;
    let mut book_edits = book_edits::BookEdits::default();
    let mut cast_guard = casting::CastGuard::default();
    let mut trades = merchant::MerchantTrades::default();
    let mut camp = camp::Camp::default();
    let mut zone_points = zoning::ZonePoints::default();
    let mut current_zone = (0u16, 0u16);
    let mut pending_memorization: Option<PendingBookAction> = None;
    let mut scribe_consumption = scribe_consumption::ScribeConsumption::default();
    let mut initial_spawns = BTreeMap::new();
    let mut initial_postures = BTreeMap::new();
    let mut doors = eq_network_game::doors::Doors::default();
    let mut doors_changed_at = Instant::now();
    let mut profile_data = Vec::new();
    let mut spawn_data = Vec::new();
    let mut position_sequence = 0u16;
    let mut last_position = Instant::now()
        .checked_sub(Duration::from_secs(2))
        .unwrap_or_else(Instant::now);
    loop {
        if stop.is_cancelled() || duration.is_some_and(|limit| connected.elapsed() >= limit) {
            session.close()?;
            // The outer run reports Stopped after the session has closed.
            return Ok(ZoneExit::Stopped);
        }
        if cast_guard.expire(Instant::now()) {
            log.send(ClientEvent::World(crate::world::WorldEvent::CastPending {
                session_id,
                spell_id: None,
            }))?;
            log.diagnostic("Cast acknowledgement timed out; a manual retry is available".into())?;
        }
        if trades.expire(Instant::now()) {
            log.send(ClientEvent::World(
                crate::world::WorldEvent::MerchantRefused {
                    session_id,
                    reason: "The merchant did not accept that offer.".into(),
                },
            ))?;
            log.diagnostic("Merchant trade was not answered; released the inventory".into())?;
        }
        if progress.elapsed() >= Duration::from_secs(30) {
            log.status(
                if ready && lifecycle.pending().is_none() && session.last_received_seconds() < 60 {
                    ConnectionState::Connected
                } else {
                    ConnectionState::Zoning
                },
                packets,
                Some(session.last_received_seconds()),
            )?;
            log.diagnostic(format!(
                "Zone session: {packets} application packets, {} communication records",
                log.messages
            ))?;
            progress = Instant::now();
        }
        ensure!(
            ready || connected.elapsed() < Duration::from_secs(60),
            "zone admission timed out"
        );
        ensure!(
            !lifecycle.expired(Instant::now()),
            "zone transfer approval timed out"
        );
        ensure!(!book_edits.expired(Instant::now()), "Spellbook edit result is unknown; reconnect for a fresh book snapshot. The edit will not be retried.");
        if camp.logout_due(Instant::now()) {
            session.send(camp::LOGOUT_OPCODE, &[])?;
            camp.logout_sent(Instant::now());
            log.send(ClientEvent::World(crate::world::WorldEvent::Camp(
                crate::world::CampStatus::LoggingOut,
            )))?;
        }
        if camp.reply_overdue(Instant::now()) {
            log.send(ClientEvent::World(crate::world::WorldEvent::Camp(
                crate::world::CampStatus::Camped,
            )))?;
            session.close()?;
            return Ok(ZoneExit::CharacterSelect);
        }
        if ready && !lifecycle.blocks_motion() {
            if let Some(motion) = motion.as_mut() {
                motion.tick(Instant::now(), |body| session.send_unreliable(0x14cb, body))?;
            } else if last_position.elapsed() >= Duration::from_secs(2) {
                stationary[2..4].copy_from_slice(&position_sequence.to_le_bytes());
                session.send_unreliable(0x14cb, &stationary)?;
                position_sequence = position_sequence.wrapping_add(1);
                last_position = Instant::now();
            }
        }
        if lifecycle.blocks_motion() {
            if let Some(commands) = context.commands {
                for _ in commands.try_iter().take(64) {}
            }
        }
        if ready && !lifecycle.is_dead() && lifecycle.pending().is_none() {
            if let Some(commands) = context.commands {
                // Bound each pass so continuous producers cannot starve receive/ACK work.
                for command in commands.try_iter().take(64) {
                    // One table decides which in-flight actions a command must wait for.
                    if let Some(reason) = actions::Held::from_state(
                        &cast_guard,
                        &book_edits,
                        pending_memorization.as_ref(),
                        &trades,
                    )
                    .conflict(&command)
                    {
                        actions::refuse(&command, reason, log)?;
                        continue;
                    }
                    if camp::handle(&mut camp, session_id, &command, &mut session, log)? {
                        continue;
                    }
                    if let ClientCommand::ClickDoor {
                        session_id: requested,
                        door_id,
                        created,
                    } = &command
                    {
                        let result = (|| -> Result<[u8; 16]> {
                            let now = Instant::now();
                            ensure!(
                                *requested == session_id
                                    && *created >= doors_changed_at
                                    && *created <= now
                                    && now.duration_since(*created) < Duration::from_secs(1),
                                "stale door request"
                            );
                            let player = admitted_player
                                .as_ref()
                                .ok_or_else(|| anyhow::anyhow!("player is unavailable"))?;
                            let position = motion
                                .as_ref()
                                .map_or(player.position, MotionSession::position);
                            doors.click_packet(*door_id, player.spawn_id, position)
                        })();
                        let error = match result {
                            Ok(body) => {
                                session.send(0x043b, &body)?;
                                None
                            }
                            Err(error) => Some(error.to_string()),
                        };
                        log.send(ClientEvent::World(crate::world::WorldEvent::DoorAction {
                            session_id,
                            door_id: *door_id,
                            error,
                        }))?;
                        continue;
                    }
                    if let ClientCommand::CrossZoneLine {
                        session_id: requested,
                        destination,
                        position,
                        created,
                    } = &command
                    {
                        let now = Instant::now();
                        let request = (|| -> Result<_> {
                            ensure!(
                                *requested == session_id
                                    && *created <= now
                                    && now.duration_since(*created) < Duration::from_millis(250),
                                "stale zone-line request"
                            );
                            ensure!(
                                motion.as_ref().is_some_and(|m| m.position() == *position),
                                "zone-line position is no longer current"
                            );
                            ensure!(current_zone.0 != 0, "zone identity is unavailable");
                            zone_points.request(
                                destination,
                                *position,
                                current_zone.0,
                                current_zone.1,
                            )
                        })();
                        match request {
                            Ok(offer) => {
                                session.send(0x5dd8, &offer.response(&config.character)?)?;
                                lifecycle.offer(offer.clone(), now)?;
                                pending_memorization = None;
                                if let Some(motion) = motion.as_mut() {
                                    motion.suspend();
                                }
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::ZoneTransfer(offer),
                                ))?;
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::MotionState {
                                        session_id,
                                        units_per_second: None,
                                        strafe_units_per_second: None,
                                        walk_units_per_second: None,
                                        backward_units_per_second: None,
                                        falls: false,
                                    },
                                ))?;
                                log.status(
                                    ConnectionState::Zoning,
                                    packets,
                                    Some(session.last_received_seconds()),
                                )?;
                                break;
                            }
                            Err(error) => {
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::ZoneLineRejected {
                                        session_id: *requested,
                                        reason: error.to_string(),
                                    },
                                ))?;
                            }
                        }
                        continue;
                    }
                    if matches!(
                        &command,
                        ClientCommand::Move(_)
                            | ClientCommand::CastSpell { .. }
                            | ClientCommand::UseItem(_)
                            | ClientCommand::SetPosture { .. }
                    ) && pending_memorization.take().is_some()
                    {
                        log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                            BookActionStatus::Cancelled(
                                "Movement, casting or posture changed".into(),
                            ),
                        )))?;
                    }
                    if let ClientCommand::ScribeSpell {
                        session_id: requested,
                        revision,
                        slot,
                        spell_id,
                        created,
                    } = &command
                    {
                        let now = Instant::now();
                        let pending = PendingBookAction {
                            started: now,
                            intent: BookIntent::Scribe {
                                revision: *revision,
                                slot: *slot,
                                spell_id: *spell_id,
                            },
                        };
                        let packet = admitted_book
                            .as_ref()
                            .filter(|_| {
                                *requested == session_id
                                    && *created <= now
                                    && now.duration_since(*created) < Duration::from_secs(1)
                                    && pending_memorization.is_none()
                            })
                            .context("scribing is unavailable or stale")
                            .and_then(|book| pending.packet(book, &inventory));
                        match packet {
                            Ok(_) => {
                                if let Some(player) = admitted_player.as_ref() {
                                    let sit = command::encode(
                                        config.protocol.into(),
                                        &ClientCommand::SetPosture {
                                            session_id,
                                            spawn_id: player.spawn_id,
                                            posture: command::Posture::Sitting,
                                            created: now,
                                        },
                                        &config.character,
                                    )?;
                                    session.send(sit.opcode, &sit.body)?;
                                    pending_memorization = Some(pending);
                                    log.send(ClientEvent::World(
                                        crate::world::WorldEvent::BookAction(
                                            BookActionStatus::Preparing,
                                        ),
                                    ))?;
                                    log.diagnostic(
                                        "Scribing scroll; movement or posture changes cancel it"
                                            .into(),
                                    )?;
                                }
                            }
                            Err(error) => {
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::BookAction(
                                        BookActionStatus::Rejected(error.to_string()),
                                    ),
                                ))?;
                                log.diagnostic(format!("Rejected scribing: {error}"))?;
                            }
                        }
                        continue;
                    }
                    if let Some(packet) = spellbook::edit_packet(
                        &command,
                        admitted_book.as_ref(),
                        session_id,
                        pending_memorization.is_some(),
                        Instant::now(),
                    ) {
                        let status = match packet {
                            Ok((opcode, body)) => {
                                session.send(opcode, &body)?;
                                book_edits.sent(&command, Instant::now());
                                BookActionStatus::AwaitingReply
                            }
                            Err(error) => BookActionStatus::Rejected(error.to_string()),
                        };
                        log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                            status,
                        )))?;
                        continue;
                    }
                    if let ClientCommand::ForgetSpell {
                        session_id: requested,
                        gem,
                        spell_id,
                        created,
                    } = &command
                    {
                        let now = Instant::now();
                        let packet = admitted_player
                            .as_ref()
                            .filter(|_| {
                                *requested == session_id
                                    && *created <= now
                                    && now.duration_since(*created) < Duration::from_secs(1)
                            })
                            .context("forget request is unavailable or stale")
                            .and_then(|player| {
                                eq_network_game::spells::forget_packet(
                                    &player.memorized_spells,
                                    *gem,
                                    *spell_id,
                                )
                            });
                        match packet {
                            Ok(body) => {
                                pending_memorization = None;
                                session.send(0x308e, &body)?;
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::BookAction(
                                        BookActionStatus::Submitted,
                                    ),
                                ))?;
                            }
                            Err(error) => {
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::BookAction(
                                        BookActionStatus::Rejected(error.to_string()),
                                    ),
                                ))?;
                                log.diagnostic(format!("Rejected forgetting spell: {error}"))?;
                            }
                        }
                        continue;
                    }
                    if let ClientCommand::MemorizeSpell {
                        session_id: requested,
                        gem,
                        spell_id,
                        created,
                    } = &command
                    {
                        let valid = *requested == session_id
                            && *created <= Instant::now()
                            && created.elapsed() < Duration::from_secs(1)
                            && pending_memorization.is_none();
                        let packet = admitted_book
                            .as_ref()
                            .filter(|_| valid)
                            .context("memorization is unavailable or stale")
                            .and_then(|book| book.memorize_packet(*gem, *spell_id));
                        match packet {
                            Ok(_) => {
                                if let Some(player) = admitted_player.as_ref() {
                                    let sit = ClientCommand::SetPosture {
                                        session_id,
                                        spawn_id: player.spawn_id,
                                        posture: command::Posture::Sitting,
                                        created: Instant::now(),
                                    };
                                    let sit = command::encode(
                                        config.protocol.into(),
                                        &sit,
                                        &config.character,
                                    )?;
                                    session.send(sit.opcode, &sit.body)?;
                                    pending_memorization = Some(PendingBookAction {
                                        started: Instant::now(),
                                        intent: BookIntent::Memorize {
                                            gem: *gem,
                                            spell_id: *spell_id,
                                        },
                                    });
                                    log.send(ClientEvent::World(
                                        crate::world::WorldEvent::BookAction(
                                            BookActionStatus::Preparing,
                                        ),
                                    ))?;
                                    log.diagnostic(
                                        "Memorizing spell; movement or posture changes cancel it"
                                            .into(),
                                    )?;
                                }
                            }
                            Err(error) => {
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::BookAction(
                                        BookActionStatus::Rejected(error.to_string()),
                                    ),
                                ))?;
                                log.diagnostic(format!("Rejected memorization: {error}"))?;
                            }
                        }
                        continue;
                    }
                    if let ClientCommand::UseItem(request) = &command {
                        let target_available = admitted_player.as_ref().is_some_and(|player| {
                            request.target_id == player.spawn_id
                                || initial_spawns.get(&request.target_id).is_some_and(
                                    |spawn: &crate::world::SpawnState| !spawn.invisible,
                                )
                        });
                        let prepared = inventory_actor
                            .as_ref()
                            .context("Character level is unavailable")
                            .and_then(|actor| {
                                inventory.prepare_item_cast(
                                    request,
                                    session_id,
                                    actor.level,
                                    target_available,
                                    Instant::now(),
                                )
                            });
                        let error = match prepared {
                            Ok((spell_id, body)) => {
                                session.send(0x304b, &body)?;
                                cast_guard.submitted(spell_id, Instant::now());
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::CastPending {
                                        session_id,
                                        spell_id: Some(spell_id),
                                    },
                                ))?;
                                None
                            }
                            Err(error) => Some(error.to_string()),
                        };
                        log.send(ClientEvent::World(
                            crate::world::WorldEvent::ItemUseAction {
                                session_id: request.session_id,
                                request_id: request.request_id,
                                error,
                            },
                        ))?;
                        continue;
                    }
                    let action_valid = match &command {
                        ClientCommand::CastSpell {
                            session_id: requested,
                            gem,
                            spell_id,
                            target_id,
                            created,
                        } => {
                            *requested == session_id
                                && created.elapsed() < Duration::from_secs(1)
                                && *created <= Instant::now()
                                && admitted_player.as_ref().is_some_and(|player| {
                                    player.memorized_spells.get(usize::from(*gem))
                                        == Some(&Some(*spell_id))
                                        && (*target_id == player.spawn_id
                                            || initial_spawns.get(target_id).is_some_and(
                                                |spawn: &crate::world::SpawnState| !spawn.invisible,
                                            ))
                                })
                        }
                        ClientCommand::SetPosture {
                            session_id: requested,
                            spawn_id,
                            created,
                            ..
                        }
                        | ClientCommand::Consider {
                            session_id: requested,
                            own_id: spawn_id,
                            created,
                            ..
                        } => {
                            *requested == session_id
                                && created.elapsed() < Duration::from_secs(1)
                                && *created <= Instant::now()
                                && admitted_player
                                    .as_ref()
                                    .is_some_and(|player| player.spawn_id == *spawn_id)
                                && match &command {
                                    ClientCommand::Consider { target_id, .. } => {
                                        initial_spawns.get(target_id).is_some_and(
                                            |spawn: &crate::world::SpawnState| !spawn.invisible,
                                        )
                                    }
                                    _ => true,
                                }
                        }
                        ClientCommand::AutoAttack {
                            session_id: requested,
                            created,
                            ..
                        }
                        | ClientCommand::LootItem {
                            session_id: requested,
                            created,
                            ..
                        }
                        | ClientCommand::Buy {
                            session_id: requested,
                            created,
                            ..
                        }
                        | ClientCommand::Sell {
                            session_id: requested,
                            created,
                            ..
                        } => {
                            *requested == session_id
                                && created.elapsed() < Duration::from_secs(1)
                                && *created <= Instant::now()
                        }
                        ClientCommand::EndLoot {
                            session_id: requested,
                            ..
                        } => *requested == session_id,
                        ClientCommand::Loot {
                            session_id: requested,
                            corpse_id: entity,
                            created,
                        }
                        | ClientCommand::Shop {
                            session_id: requested,
                            merchant_id: entity,
                            created,
                            ..
                        } => {
                            *requested == session_id
                                && created.elapsed() < Duration::from_secs(1)
                                && *created <= Instant::now()
                                && initial_spawns.get(entity).is_some_and(
                                    |spawn: &crate::world::SpawnState| {
                                        !spawn.invisible
                                            && if matches!(command, ClientCommand::Loot { .. }) {
                                                matches!(
                                                    spawn.kind,
                                                    crate::world::SpawnKind::NpcCorpse
                                                        | crate::world::SpawnKind::PlayerCorpse
                                                )
                                            } else {
                                                spawn.kind == crate::world::SpawnKind::Npc
                                            }
                                    },
                                )
                        }
                        _ => true,
                    };
                    if !action_valid {
                        if let Some(event) = casting::rejected(
                            &command,
                            "Request expired, spell gem changed, or target is unavailable",
                        ) {
                            log.send(ClientEvent::World(event))?;
                        }
                        log.diagnostic(
                            "Rejected stale or unavailable spell/posture action".into(),
                        )?;
                        continue;
                    }
                    if inventory::handle(
                        &mut inventory,
                        inventory_actor.map(|mut actor| {
                            actor.bank_access = admitted_player.as_ref().is_some_and(|player| {
                                let position = motion
                                    .as_ref()
                                    .map_or(player.position, MotionSession::position);
                                initial_spawns.values().any(|spawn| {
                                    eq_network_game::inventory::banker_in_range(position, spawn)
                                })
                            });
                            actor
                        }),
                        session_id,
                        &command,
                        &mut session,
                        log,
                    )? {
                        continue;
                    }
                    if let Some(motion) = motion.as_mut() {
                        if motion::handle(motion, session_id, &command, &mut session, log)? {
                            continue;
                        }
                    }
                    if matches!(&command, ClientCommand::InspectItem { session_id: requested, .. } if *requested != session_id)
                    {
                        log.diagnostic("Rejected item inspection from an old session".into())?;
                        continue;
                    }
                    if let ClientCommand::SelectTarget {
                        session_id: requested_session,
                        spawn_id,
                    } = &command
                    {
                        if *requested_session != session_id
                            || spawn_id.is_some_and(|id| {
                                admitted_player
                                    .as_ref()
                                    .is_none_or(|player| player.spawn_id != id)
                                    && initial_spawns.get(&id).is_none_or(
                                        |spawn: &crate::world::SpawnState| spawn.invisible,
                                    )
                            })
                        {
                            log.send(ClientEvent::World(
                                crate::world::WorldEvent::TargetRejected {
                                    session_id: *requested_session,
                                    spawn_id: *spawn_id,
                                    reason: "Target is stale, invisible, or unavailable".into(),
                                },
                            ))?;
                            log.diagnostic("Rejected stale or unavailable target".into())?;
                            continue;
                        }
                    }
                    match command::encode(config.protocol.into(), &command, &config.character) {
                        Ok(packet) => {
                            session.send(packet.opcode, &packet.body)?;
                            trades.sent(&command, Instant::now());
                            if let ClientCommand::CastSpell { spell_id, .. } = &command {
                                cast_guard.submitted(*spell_id, Instant::now());
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::CastPending {
                                        session_id,
                                        spell_id: Some(*spell_id),
                                    },
                                ))?;
                            }
                            if let ClientCommand::SelectTarget { spawn_id, .. } = command {
                                log.send(ClientEvent::World(
                                    crate::world::WorldEvent::TargetSent(spawn_id),
                                ))?;
                            }
                        }
                        Err(error) => {
                            if let Some(event) = casting::rejected(&command, &error.to_string()) {
                                log.send(ClientEvent::World(event))?;
                            }
                            log.diagnostic(format!(
                                "Rejected invalid outbound client command: {error}"
                            ))?;
                        }
                    }
                }
            }
        }
        if !lifecycle.is_dead() && lifecycle.pending().is_none() {
            if pending_memorization
                .as_ref()
                .is_some_and(|pending| pending.ready(Instant::now()))
            {
                if let Some(pending) = pending_memorization.take() {
                    let packet = admitted_book
                        .as_ref()
                        .context("spellbook unavailable")
                        .and_then(|book| pending.packet(book, &inventory));
                    match packet {
                        Ok(body) => {
                            session.send(0x308e, &body)?;
                            book_edits.prepared_sent(&pending.intent, Instant::now());
                            scribe_consumption.sent(&pending.intent);
                            log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                                BookActionStatus::AwaitingReply,
                            )))?;
                        }
                        Err(error) => {
                            log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                                BookActionStatus::Cancelled(error.to_string()),
                            )))?;
                            log.diagnostic(format!("Cancelled spellbook action: {error}"))?;
                        }
                    }
                }
            }
        } else {
            spellbook::cancel_pending(
                &mut pending_memorization,
                "Character died or zone transfer started",
                log,
            )?;
        }
        let Some(mut packet) = session.receive()? else {
            continue;
        };
        packets += 1;
        if !stock && matches!(packet.opcode, 0x2e78 | 0x1860) {
            // V62 XOR runs continuously over the full batch, not per spawn.
            p99::session_xor(&mut packet.body, &credentials.key)?;
        }
        if !ready {
            log.diagnostic(format!(
                "Zone received 0x{:04x} ({} bytes)",
                packet.opcode,
                packet.body.len()
            ))?;
        }
        if ready {
            match eq_network_game::spells::decode(packet.opcode, &packet.body) {
                Ok(Some(update)) => {
                    let book_result = book_edits.observe(&update);
                    scribe_consumption
                        .observe(&update, book_result == Some(BookActionStatus::Confirmed));
                    if let Some(player) = admitted_player.as_ref() {
                        let pending = cast_guard.pending();
                        cast_guard.observe(player.spawn_id, &update);
                        if pending.is_some() && cast_guard.pending().is_none() {
                            log.send(ClientEvent::World(crate::world::WorldEvent::CastPending {
                                session_id,
                                spell_id: None,
                            }))?;
                        }
                        if cast_guard.active() && pending_memorization.take().is_some() {
                            log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                                BookActionStatus::Cancelled("Casting started".into()),
                            )))?;
                        }
                    }
                    if let Some(book) = admitted_book.as_mut() {
                        book.apply(&update);
                    }
                    if let Some(player) = admitted_player.as_mut() {
                        update.apply_gems(&mut player.memorized_spells);
                    }
                    log.send(ClientEvent::World(crate::world::WorldEvent::Spell(update)))?;
                    if let Some(status) = book_result {
                        log.send(ClientEvent::World(crate::world::WorldEvent::BookAction(
                            status,
                        )))?;
                    }
                }
                Ok(None) => (),
                Err(error) => log.diagnostic(format!("Invalid spell notification: {error}"))?,
            }
        }
        if matches!(packet.opcode, 0x5394 | 0x3397 | 0x420f | 0x4d81 | 0x1c4a) {
            match eq_network_game::inventory::decode(packet.opcode, &packet.body) {
                Ok(Some(update)) => {
                    let update = scribe_consumption.reconcile(&inventory, update);
                    inventory.apply(update.clone());
                    if ready {
                        log.send(ClientEvent::World(crate::world::WorldEvent::Inventory(
                            update,
                        )))?;
                    }
                    continue;
                }
                Err(error) => {
                    log.diagnostic(format!("Inventory decode rejected: {error}"))?;
                    let update = eq_network_game::inventory::InventoryUpdate::Invalidated;
                    inventory.apply(update.clone());
                    if ready {
                        log.send(ClientEvent::World(crate::world::WorldEvent::Inventory(
                            update,
                        )))?;
                    }
                    continue;
                }
                Ok(None) => (),
            }
        }
        if packet.opcode == 0x3eba {
            match zoning::ZonePoints::decode(&packet.body) {
                Ok(points) => zone_points = points,
                Err(error) => {
                    zone_points = zoning::ZonePoints::default();
                    log.diagnostic(format!("Zone-point table rejected: {error}"))?;
                }
            }
            continue;
        }
        if ready && matches!(packet.opcode, 0x385e | 0x7834) {
            let offer = zoning::offer(packet.opcode, &packet.body)?;
            if let Some(position) = offer.local_position(current_zone) {
                ensure!(
                    !lifecycle.blocks_motion(),
                    "same-zone relocation conflicts with death or transfer"
                );
                spellbook::cancel_pending(
                    &mut pending_memorization,
                    "Server relocated character",
                    log,
                )?;
                let player = admitted_player
                    .as_mut()
                    .context("relocation without admitted player")?;
                motion::correct_own(
                    player,
                    motion.as_mut(),
                    &mut stationary,
                    position_sequence,
                    position,
                    Instant::now(),
                )?;
                log.send(ClientEvent::World(crate::world::WorldEvent::MotionState {
                    session_id,
                    units_per_second: None,
                    strafe_units_per_second: None,
                    walk_units_per_second: None,
                    backward_units_per_second: None,
                    falls: false,
                }))?;
                log.send(ClientEvent::World(crate::world::WorldEvent::Position {
                    spawn_id: player.spawn_id,
                    position,
                }))?;
                continue;
            }
            if let Some(pending) = lifecycle.pending() {
                ensure!(pending == &offer, "conflicting zone transfer offer");
            } else {
                session.send(0x5dd8, &offer.response(&config.character)?)?;
                spellbook::cancel_pending(&mut pending_memorization, "Zone transfer started", log)?;
                log.send(ClientEvent::World(crate::world::WorldEvent::ZoneTransfer(
                    offer.clone(),
                )))?;
                log.status(
                    ConnectionState::Zoning,
                    packets,
                    Some(session.last_received_seconds()),
                )?;
                lifecycle.offer(offer, Instant::now())?;
                if let Some(motion) = motion.as_mut() {
                    motion.suspend();
                }
                log.send(ClientEvent::World(crate::world::WorldEvent::MotionState {
                    session_id,
                    units_per_second: None,
                    strafe_units_per_second: None,
                    walk_units_per_second: None,
                    backward_units_per_second: None,
                    falls: false,
                }))?;
            }
            continue;
        }
        if camp.logging_out() && packet.opcode == camp::LOGOUT_REPLY_OPCODE {
            log.send(ClientEvent::World(crate::world::WorldEvent::Camp(
                crate::world::CampStatus::Camped,
            )))?;
            session.close()?;
            return Ok(ZoneExit::CharacterSelect);
        }
        if ready && packet.opcode == 0x5dd8 {
            let pending = lifecycle
                .pending()
                .context("zone approval without a pending server offer")?;
            let heading = motion
                .as_ref()
                .map_or(0.0, |motion| motion.position().heading);
            let reply = zoning::reply(
                &packet.body,
                &config.character,
                pending,
                current_zone,
                heading,
            )?;
            if reply == zoning::ZoneReply::Approved {
                lifecycle.finish(true)?;
                log.send(ClientEvent::Progress(ConnectionStage::ConnectingWorld))?;
                session.close()?;
                return Ok(ZoneExit::World);
            }
            lifecycle.finish(false)?;
            let reason = match &reply {
                zoning::ZoneReply::Denied(reason) => *reason,
                zoning::ZoneReply::Rewind(_) => zoning::ZoneRejection::Cancelled,
                zoning::ZoneReply::Approved => unreachable!("approved transfer returned above"),
            };
            if let zoning::ZoneReply::Rewind(position) = reply {
                let player = admitted_player
                    .as_mut()
                    .context("rewind without admitted player")?;
                motion::correct_own(
                    player,
                    motion.as_mut(),
                    &mut stationary,
                    position_sequence,
                    position,
                    Instant::now(),
                )?;
                log.send(ClientEvent::World(crate::world::WorldEvent::Position {
                    spawn_id: u16::from_le_bytes([stationary[0], stationary[1]]),
                    position,
                }))?;
            }
            log.send(ClientEvent::World(
                crate::world::WorldEvent::ZoneTransferRejected { session_id, reason },
            ))?;
            if !lifecycle.is_dead() {
                if let Some(motion) = motion.as_mut() {
                    motion.resume_stationary(Instant::now());
                }
                log.status(
                    ConnectionState::Connected,
                    packets,
                    Some(session.last_received_seconds()),
                )?;
            }
            continue;
        }
        match ZoneOpcode::from(packet.opcode) {
            ZoneOpcode::PlayerProfile => {
                ensure!(
                    packet.body.len() == 19592,
                    "unexpected Titanium player profile size"
                );
                for offset in [13116, 13120, 13124, 13128] {
                    ensure!(
                        f32::from_le_bytes(packet.body[offset..offset + 4].try_into().unwrap())
                            .is_finite(),
                        "invalid player position"
                    );
                }
                // Preserve the server's saved coordinates. Velocity and
                // animation stay zero; this collector never navigates.
                stationary[4..8].copy_from_slice(&packet.body[13120..13124]);
                stationary[24..28].copy_from_slice(&packet.body[13116..13120]);
                stationary[28..32].copy_from_slice(&packet.body[13124..13128]);
                let heading = f32::from_le_bytes(packet.body[13128..13132].try_into().unwrap());
                let heading = heading.rem_euclid(512.0) * 8.0;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let heading = heading as u16;
                stationary[32..34].copy_from_slice(&(heading & 0x0fff).to_le_bytes());
                profile_data.clone_from(&packet.body);
                initial_skills.clear();
                initial_level = None;
                current_zone = (
                    u16::from_le_bytes(packet.body[13276..13278].try_into()?),
                    u16::from_le_bytes(packet.body[13278..13280].try_into()?),
                );
                saw_profile = true;
            }
            ZoneOpcode::Weather => saw_weather = true,
            ZoneOpcode::PlayerSpawn if !saw_spawn => {
                if !stock {
                    p99::session_xor(&mut packet.body, &credentials.key)?;
                }
                ensure!(
                    packet.body.len() == 385
                        && cstr(&packet.body[7..71])
                            .eq_ignore_ascii_case(config.character.as_bytes()),
                    "zone returned a different character spawn"
                );
                if !stock {
                    codec.zone_spawn(&packet.body)?;
                }
                let id = u16::try_from(le32(&packet.body[340..344]))
                    .context("spawn ID exceeds Titanium position field")?;
                stationary[..2].copy_from_slice(&id.to_le_bytes());
                spawn_data.clone_from(&packet.body);
                saw_spawn = true;
            }
            ZoneOpcode::ZoneDescription if !ready => {
                ensure!(packet.body.len() >= 96, "truncated zone description");
                zone_name = String::from_utf8_lossy(cstr(&packet.body[64..96])).into_owned();
                far_clip = crate::world::titanium_far_clip(&packet.body);
                log.zone.clone_from(&zone_name);
                log.status(
                    ConnectionState::Zoning,
                    packets,
                    Some(session.last_received_seconds()),
                )?;
                got_zone = true;
                session.send(0x067a, &0u32.to_le_bytes())?;
                session.send(0x5e3a, &0u32.to_le_bytes())?;
                session.send(0x7752, &0u32.to_le_bytes())?;
                session.send(0x0322, &[])?;
            }
            ZoneOpcode::ExperienceUpdate if got_zone && !ready && !replied_experience => {
                session.send(0x0587, &[])?;
                replied_experience = true;
            }
            ZoneOpcode::ExperienceUpdate if got_zone && !ready && replied_experience => {
                session.send(0x6563, &chat::server_filters())?;
                session.send(0x5e20, &[])?;
                session.send(0x0c11, &1u32.to_le_bytes())?;
                ready = true;
                match crate::world::titanium_player(&profile_data, &spawn_data) {
                    Ok(mut player) => {
                        if let Some(level) = initial_level.take() {
                            player.level = level;
                        }
                        for (skill, value) in std::mem::take(&mut initial_skills) {
                            player.apply_skill(skill, value);
                        }
                        admitted_player = Some(player.clone());
                        let spell_book =
                            eq_network_game::spells::SpellBook::titanium_profile(&profile_data)?;
                        admitted_book = Some(spell_book.clone());
                        inventory_actor = Some(eq_network_game::inventory::InventoryActor {
                            bank_access: false,
                            deity: player.deity,
                            dual_wield: player
                                .skills
                                .as_ref()
                                .and_then(|skills| skills.get(22))
                                .copied(),
                            class: player.class,
                            race: player.race,
                            level: player.level,
                        });
                        motion = Some(
                            MotionSession::new(
                                session_id,
                                player.spawn_id,
                                player.position,
                                Instant::now(),
                            )?
                            .with_falls(stock),
                        );
                        log.send(ClientEvent::World(crate::world::WorldEvent::Entered {
                            session_id,
                            zone: zone_name.clone(),
                            player: Box::new(player),
                            far_clip,
                        }))?;
                        log.send(ClientEvent::World(crate::world::WorldEvent::Spawns(
                            initial_spawns.values().cloned().collect(),
                        )))?;
                        for (spawn_id, posture) in std::mem::take(&mut initial_postures) {
                            log.send(ClientEvent::World(crate::world::WorldEvent::Posture {
                                spawn_id,
                                posture,
                            }))?;
                        }
                        log.send(ClientEvent::World(crate::world::WorldEvent::Doors(
                            doors.admission(),
                        )))?;
                        let admission = inventory.admission_updates();
                        log.send(ClientEvent::World(crate::world::WorldEvent::BuffSnapshot(
                            eq_network_game::buffs::titanium_profile(&profile_data)?,
                        )))?;
                        log.send(ClientEvent::World(crate::world::WorldEvent::SpellBook(
                            spell_book,
                        )))?;
                        log.send(ClientEvent::World(crate::world::WorldEvent::Coins(
                            crate::world::titanium_coins(&profile_data)?,
                        )))?;
                        inventory = eq_network_game::inventory::Inventory::default();
                        for update in admission {
                            inventory.apply(update.clone());
                            log.send(ClientEvent::World(crate::world::WorldEvent::Inventory(
                                update,
                            )))?;
                        }

                        if let Some(value) = initial_experience.take() {
                            log.send(ClientEvent::World(crate::world::WorldEvent::Experience(
                                value,
                            )))?;
                        }
                    }
                    Err(error) => {
                        log.diagnostic(format!("Player presentation unavailable: {error}"))?;
                    }
                }
                profile_data.clear();
                spawn_data.clear();
                log.send(ClientEvent::Progress(ConnectionStage::Ready))?;
                log.status(
                    ConnectionState::Connected,
                    packets,
                    Some(session.last_received_seconds()),
                )?;
                log.diagnostic(format!("Zone login sequence complete for {zone_name}; waiting for ongoing server traffic"))?;
            }
            ZoneOpcode::ValidationRejected => bail!("server rejected zone validation"),
            ZoneOpcode::LoggedOut => bail!("server logged the character out"),
            ZoneOpcode::ZoneHandoff => {
                ensure!(
                    ready && lifecycle.pending().is_some(),
                    "zone handoff without a pending transfer"
                );
                log.send(ClientEvent::Progress(ConnectionStage::ConnectingZone))?;
                session.close()?;
                return Ok(ZoneExit::Direct(packet.body));
            }
            _ => (),
        }
        if saw_spawn && saw_profile && saw_weather && !requested {
            if !stock {
                codec.file_response(&mut checksums)?;
                session.send(0x1251, &checksums)?;
            }
            session.send(0x7ac5, &[])?;
            session.send(0x367d, &[])?;
            session.send(0x5966, &[])?;
            requested = true;
            log.send(ClientEvent::Progress(ConnectionStage::EnteringWorld))?;
        }
        if !ready && matches!(packet.opcode, 0x4c24 | 0x700d | 0x77d0) {
            match eq_network_game::doors::decode(packet.opcode, &packet.body) {
                Ok(Some(update)) => doors.apply(&update),
                Err(error) => log.diagnostic(format!("Initial door update rejected: {error}"))?,
                Ok(None) => (),
            }
        }
        if !ready && packet.opcode == 0x6a93 {
            match crate::world::titanium_update(packet.opcode, &packet.body) {
                Ok(Some(crate::world::WorldEvent::Skill { skill_id, value })) if skill_id < 100 => {
                    initial_skills.insert(skill_id, value);
                }
                Err(error) => log.diagnostic(format!("Initial skill update rejected: {error}"))?,
                _ => (),
            }
        }
        if !ready && matches!(packet.opcode, 0x5ecd | 0x6d44) {
            match crate::world::titanium_update(packet.opcode, &packet.body) {
                Ok(Some(crate::world::WorldEvent::Level {
                    current,
                    experience,
                    ..
                })) => {
                    initial_level = Some(current);
                    initial_experience = Some(experience);
                }
                Ok(Some(crate::world::WorldEvent::Experience(value))) => {
                    initial_experience = Some(value);
                }
                Err(error) => log.diagnostic(format!("Initial experience rejected: {error}"))?,
                _ => (),
            }
        }
        if !ready && matches!(packet.opcode, 0x2e78 | 0x1860 | 0x55bc | 0x14cb | 0x7c32) {
            match crate::world::titanium_update(packet.opcode, &packet.body) {
                Ok(Some(crate::world::WorldEvent::Spawns(spawns))) => {
                    for spawn in spawns {
                        initial_spawns.insert(spawn.spawn_id, spawn);
                    }
                    ensure!(initial_spawns.len() <= 65535, "zone entity limit exceeded");
                }
                Ok(Some(crate::world::WorldEvent::Despawn(id))) => {
                    initial_spawns.remove(&id);
                    initial_postures.remove(&id);
                }
                Ok(Some(crate::world::WorldEvent::Posture { spawn_id, posture })) => {
                    initial_postures.insert(spawn_id, posture);
                }
                Ok(Some(crate::world::WorldEvent::Visibility {
                    spawn_id,
                    invisible,
                })) => {
                    if let Some(spawn) = initial_spawns.get_mut(&spawn_id) {
                        spawn.invisible = invisible;
                    }
                }
                Ok(Some(crate::world::WorldEvent::Position { spawn_id, position })) => {
                    if let Some(spawn) = initial_spawns.get_mut(&spawn_id) {
                        spawn.position = position;
                    }
                }
                Err(error) => log.diagnostic(format!("Initial entity decode rejected: {error}"))?,
                _ => (),
            }
        }
        if ready {
            match crate::world::titanium_update(packet.opcode, &packet.body) {
                Ok(Some(event)) => {
                    match &event {
                        crate::world::WorldEvent::Posture { spawn_id, posture }
                            if *spawn_id == u16::from_le_bytes([stationary[0], stationary[1]])
                                && *posture != crate::world::PostureState::Sitting =>
                        {
                            spellbook::cancel_pending(
                                &mut pending_memorization,
                                "Server changed character posture",
                                log,
                            )?;
                        }
                        crate::world::WorldEvent::Visibility {
                            spawn_id,
                            invisible,
                        } => {
                            if let Some(spawn) = initial_spawns.get_mut(spawn_id) {
                                spawn.invisible = *invisible;
                            }
                        }
                        crate::world::WorldEvent::Doors(update) => {
                            if matches!(
                                update,
                                eq_network_game::doors::DoorUpdate::Spawn(_)
                                    | eq_network_game::doors::DoorUpdate::RemoveAll
                            ) {
                                doors_changed_at = Instant::now();
                            }
                            doors.apply(update);
                        }
                        crate::world::WorldEvent::Level { current, .. } => {
                            if let Some(player) = admitted_player.as_mut() {
                                player.level = *current;
                            }
                            if let Some(actor) = inventory_actor.as_mut() {
                                actor.level = *current;
                            }
                        }
                        crate::world::WorldEvent::Skill { skill_id, value } => {
                            if let Some(player) = admitted_player.as_mut() {
                                player.apply_skill(*skill_id, *value);
                                if let Some(actor) = inventory_actor.as_mut() {
                                    actor.dual_wield = player
                                        .skills
                                        .as_ref()
                                        .and_then(|skills| skills.get(22))
                                        .copied();
                                }
                            }
                        }
                        crate::world::WorldEvent::Death(death)
                            if death.spawn_id
                                == u32::from(u16::from_le_bytes([
                                    stationary[0],
                                    stationary[1],
                                ])) =>
                        {
                            lifecycle.mark_dead();
                            if camp.cancel() {
                                log.send(ClientEvent::World(crate::world::WorldEvent::Camp(
                                    crate::world::CampStatus::Abandoned,
                                )))?;
                            }
                            spellbook::cancel_pending(
                                &mut pending_memorization,
                                "Character died",
                                log,
                            )?;
                            cast_guard.clear();
                            trades.clear();
                            if let Some(motion) = motion.as_mut() {
                                motion.suspend();
                            }
                            log.send(ClientEvent::World(crate::world::WorldEvent::MotionState {
                                session_id,
                                units_per_second: None,
                                strafe_units_per_second: None,
                                walk_units_per_second: None,
                                backward_units_per_second: None,
                                falls: false,
                            }))?;
                            log.diagnostic(
                                "Own character died; waiting for the server bind offer".into(),
                            )?;
                        }
                        crate::world::WorldEvent::Spawns(spawns) => {
                            for spawn in spawns {
                                initial_spawns.insert(spawn.spawn_id, spawn.clone());
                            }
                        }
                        crate::world::WorldEvent::Despawn(id) => {
                            initial_spawns.remove(id);
                        }
                        // Another entity died: its corpse keeps the spawn ID.
                        crate::world::WorldEvent::Death(death) => {
                            if let Some(spawn) = u16::try_from(death.spawn_id)
                                .ok()
                                .and_then(|id| initial_spawns.get_mut(&id))
                            {
                                spawn.kind = spawn.kind.corpse();
                            }
                        }
                        // A sale's echo is the only notice that the item left.
                        crate::world::WorldEvent::Merchant(update) => {
                            if let Some(change) = trades.observe(update) {
                                inventory.apply(change.clone());
                                log.send(ClientEvent::World(crate::world::WorldEvent::Inventory(
                                    change,
                                )))?;
                            }
                        }
                        _ => (),
                    }
                    if let crate::world::WorldEvent::Position { spawn_id, position } = &event {
                        if *spawn_id == u16::from_le_bytes([stationary[0], stationary[1]]) {
                            spellbook::cancel_pending(
                                &mut pending_memorization,
                                "Server corrected character position",
                                log,
                            )?;
                            let player = admitted_player
                                .as_mut()
                                .context("correction without admitted player")?;
                            motion::correct_own(
                                player,
                                motion.as_mut(),
                                &mut stationary,
                                position_sequence,
                                *position,
                                Instant::now(),
                            )?;
                            log.send(ClientEvent::World(crate::world::WorldEvent::MotionState {
                                session_id,
                                units_per_second: None,
                                strafe_units_per_second: None,
                                walk_units_per_second: None,
                                backward_units_per_second: None,
                                falls: false,
                            }))?;
                        }
                    }
                    log.send(ClientEvent::World(event))?;
                }
                Ok(None) => (),
                Err(error) => log.diagnostic(format!("World-state decode rejected: {error}"))?,
            }
        }
        match chat::parse(packet.opcode, &packet.body, config.include_raw) {
            Ok(Some(event)) => log.record(&zone_name, RecordEvent::Chat(event))?,
            Ok(None) => (),
            Err(error) => log.record(
                &zone_name,
                RecordEvent::DecodeError(DecodeError {
                    kind: "decode_error",
                    opcode: packet.opcode,
                    payload_hex: hex::encode(&packet.body),
                    error: error.to_string(),
                }),
            )?,
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
                decode_destination(&codec, &handoff, &assets, &mut log).unwrap();
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
