//! The world server: validating the client, the character list, choosing or
//! creating a character, and the handoff to its zone. Each client
//! generation runs it its own way: Titanium's is here, `EQMac`'s in
//! [`eqmac`].
use super::{
    creation::Creating, cstr, login::Credentials, put_string, servers, servers::Shield,
    CharacterSession, ZoneDestination,
};
use crate::{
    assets::Assets,
    chat,
    client::{ClientEvent, ClientIdentity, ConnectionStage, Events, RecordEvent},
    transport::Session,
    world::WorldEvent,
};
use anyhow::{anyhow, ensure, Context, Result};
use eq_network_game::{channels, rules, zoning};
use std::{
    net::IpAddr,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub(super) mod eqmac;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorldOpcode {
    ApprovalChallenge,
    FileManifest,
    ValidationResult,
    CharacterList,
    ZoneHandoff,
    ApproveName,
    Guilds,
    EnterWorld,
    ZoneUnavailable,
    Unknown(u16),
}

impl From<u16> for WorldOpcode {
    fn from(value: u16) -> Self {
        match value {
            0x3c25 => Self::ApprovalChallenge,
            0x52a4 => Self::FileManifest,
            0x1251 => Self::ValidationResult,
            0x4513 => Self::CharacterList,
            0x61b6 => Self::ZoneHandoff,
            0x3ea6 => Self::ApproveName,
            eq_network_game::listing::GUILDS_OPCODE => Self::Guilds,
            0x7cba => Self::EnterWorld,
            zoning::TITANIUM_UNAVAILABLE_OPCODE => Self::ZoneUnavailable,
            value => Self::Unknown(value),
        }
    }
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

/// Decode the endpoint and, behind protection, the file manifest before
/// rekeying for destination admission.
pub(super) fn decode_destination(
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
    session: &mut dyn eq_network_transport::Transport,
    server: &dyn servers::ServerType,
    credentials: &Credentials,
    zoning: bool,
) -> Result<Option<Box<dyn Shield>>> {
    let mut login_info = Zeroizing::new(vec![0; 464]);
    login_info[192] = 0xcc;
    login_info[188] = u8::from(zoning);
    let account = &credentials.account;
    login_info[..account.len()].copy_from_slice(account.as_bytes());
    login_info[account.len() + 1..account.len() + 1 + credentials.key.len()]
        .copy_from_slice(&credentials.key);
    let shield = server.protect(&login_info)?;
    session.send(0x4dd0, &login_info)?;
    Ok(shield)
}

/// Sends the selected server name and starts the zone-handoff deadline.
fn enter_character(
    session: &mut dyn eq_network_transport::Transport,
    name: &str,
    log: &mut Events<'_>,
) -> Result<Instant> {
    let mut enter = [0; 72];
    put_string(&mut enter[..64], name)?;
    session.send(0x7cba, &enter)?;
    name.clone_into(&mut log.character);
    log.send(ClientEvent::Progress(ConnectionStage::ConnectingZone))?;
    Ok(Instant::now() + Duration::from_secs(60))
}

/// Tells the host what the world says of itself as the player logs in, or
/// why `what` could not be read.
fn tell(read: Result<WorldEvent>, what: &str, log: &mut Events<'_>) -> Result<()> {
    match read {
        Ok(event) => log.send(ClientEvent::World(event)),
        Err(error) => log.diagnostic(format!("{what} unreadable: {error}")),
    }
}

/// Tells the host what the world says of itself, as `EQEmu`'s and P99's
/// worlds send it: its short name and rules, the expansions it lets the
/// account play and its chat server. Any other message says nothing of it.
///
/// # Errors
/// Returns an error when the host's event handler fails.
fn news(opcode: u16, body: &[u8], log: &mut Events<'_>) -> Result<()> {
    match opcode {
        // Consumers name per-character files after the world, as the
        // official client does.
        rules::TITANIUM_LOG_SERVER_OPCODE => {
            let name = crate::world::titanium_world_name(body);
            tell(
                name.map(|short_name| WorldEvent::WorldName { short_name }),
                "World short name",
                log,
            )?;
            tell(
                rules::titanium_rules(body).map(WorldEvent::Rules),
                "World rules",
                log,
            )
        }
        rules::TITANIUM_EXPANSIONS_OPCODE => tell(
            rules::expansions(body).map(WorldEvent::Expansions),
            "Expansion mask",
            log,
        ),
        _ => match channels::titanium(opcode, body).transpose() {
            Some(offer) => tell(offer.map(WorldEvent::ChatServer), "Chat server offer", log),
            None => Ok(()),
        },
    }
}

/// The world's word that the zone the player is entering is down: tells the
/// host which zone, as far as the world names it, and gives the error that
/// ends the attempt at once.
///
/// # Errors
/// Returns an error when the host's event handler fails.
fn unavailable(body: &[u8], log: &mut Events<'_>) -> Result<anyhow::Error> {
    let zone = zoning::unavailable(body).or_else(|error| {
        log.diagnostic(format!("Unavailable zone unnamed: {error}"))?;
        Ok::<_, anyhow::Error>(None)
    })?;
    log.send(ClientEvent::World(WorldEvent::ZoneUnavailable {
        zone: zone.clone(),
    }))?;
    Ok(zone.map_or_else(
        || anyhow!("the zone is unavailable"),
        |zone| anyhow!("zone {zone} is unavailable"),
    ))
}

/// Reads the character the world names as the player logs in: between
/// zones the one in play, which must be the session's own; otherwise
/// `EQEmu`'s automatic login (`World:EnableAutoLogin`), which the session
/// tells of and leaves to its own choice.
///
/// # Errors
/// Returns an error when the world names another character between zones,
/// or when the host's event handler fails.
fn named(body: &[u8], character: &str, zoning: bool, log: &mut Events<'_>) -> Result<()> {
    let name = cstr(body);
    if zoning {
        ensure!(
            name.is_empty() || name.eq_ignore_ascii_case(character.as_bytes()),
            "the world named another character between zones"
        );
    } else if !name.is_empty() {
        log.diagnostic(format!(
            "The world would enter {} by itself; the session keeps to its own choice",
            name.escape_ascii()
        ))?;
    }
    Ok(())
}

/// Titanium's world stage: complete world validation, select the
/// character, and follow its zone handoff.
// The linear handshake keeps packet ordering and state transitions together.
#[allow(clippy::too_many_lines)]
pub(super) fn titanium(
    context: &CharacterSession<'_>,
    assets: &Assets,
    ip: &str,
    (world_only, zoning): (bool, bool),
    log: &mut Events<'_>,
) -> Result<Option<ZoneDestination>> {
    let config = &context.config;
    let credentials = context.credentials;
    let stop = context.stop;
    let mut session = Session::connect_cancellable(
        crate::client::endpoint(ip, 9000, config.local_only)?,
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
    // A creation awaiting the world's approval of its name, or its new list.
    let mut creating: Option<Creating> = None;
    // Stock EQEmu marks a world session that created a character as bound for the
    // tutorial until the client sends OP_World_Client_CRC1 (world/client.cpp).
    let mut tutorial_pending = false;
    loop {
        ensure!(!stop.is_cancelled(), "shutdown requested");
        if !entered {
            match selection
                .as_ref()
                .and_then(|list: &crate::client::selection::Selection| list.poll(context.commands))
            {
                Some(crate::client::selection::Choice::Enter(name)) => chosen = Some(name),
                Some(crate::client::selection::Choice::Create(character)) if creating.is_none() => {
                    creating = Creating::start(server.creation(), character, &mut session, log)?;
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
            WorldOpcode::EnterWorld => named(&packet.body, &config.character, zoning, log)?,
            WorldOpcode::ZoneUnavailable => {
                let error = unavailable(&packet.body, log)?;
                session.close()?;
                return Err(error);
            }
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
                        let (list, automatic) = crate::client::selection::Selection::new(
                            entries,
                            &config.character,
                            log,
                        )?;
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
            WorldOpcode::ApproveName => {
                if let Some(attempt) = creating.take() {
                    creating = attempt.answered(&packet.body, &mut session, log)?;
                }
            }
            WorldOpcode::CharacterList
                if accepted && !entered && creating.as_ref().is_some_and(Creating::requested) =>
            {
                if let Some(attempt) = creating.take() {
                    tutorial_pending |= server.start_choice();
                    attempt.created(log)?;
                }
                let entries =
                    eq_network_game::characters::decode(config.protocol.into(), &packet.body)?;
                let (list, automatic) =
                    crate::client::selection::Selection::new(entries, &config.character, log)?;
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
                    crate::client::selection::Selection::new(entries, &config.character, log)?;
                selection = Some(list);
                chosen = automatic;
            }
            // The guilds players' spawns name by number.
            WorldOpcode::Guilds => match eq_network_game::listing::titanium_guilds(&packet.body) {
                Ok(names) => log.send(ClientEvent::World(crate::world::WorldEvent::GuildNames(
                    names,
                )))?,
                Err(error) => log.diagnostic(format!("Guild list unreadable: {error}"))?,
            },
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
            _ => news(packet.opcode, &packet.body, log)?,
        }
        // Stock EQEmu accepts the client without the V62 validation reply.
        if world_only && accepted {
            session.close()?;
            return Ok(None);
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::client::ClientConfig;
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
    /// A connection that keeps what the client sent.
    #[derive(Default)]
    struct Kept(Vec<(u16, Vec<u8>)>);

    impl eq_network_transport::Transport for Kept {
        fn send(&mut self, opcode: u16, body: &[u8]) -> Result<()> {
            self.0.push((opcode, body.to_vec()));
            Ok(())
        }

        fn receive(&mut self) -> Result<Option<crate::transport::Application>> {
            Ok(None)
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }

        fn last_received_seconds(&self) -> u64 {
            0
        }
    }

    #[test]
    fn the_world_login_carries_the_account_and_key_and_starts_p99s_protection() {
        let credentials = Credentials {
            account: "12345".into(),
            key: *b"0123456789",
        };
        for (protocol, zoning) in [
            (crate::client::ServerProtocol::Project1999, false),
            (crate::client::ServerProtocol::EqEmu, true),
        ] {
            let mut kept = Kept::default();
            let shield = open_world(
                &mut kept,
                servers::server_type(protocol),
                &credentials,
                zoning,
            )
            .unwrap();
            // Only P99 protects its worlds.
            assert_eq!(
                shield.is_some(),
                protocol == crate::client::ServerProtocol::Project1999
            );
            let (opcode, body) = &kept.0[0];
            assert_eq!(*opcode, 0x4dd0);
            assert_eq!(body.len(), 464);
            assert_eq!(&body[..5], b"12345");
            assert_eq!(&body[6..16], b"0123456789");
            assert_eq!((body[188], body[192]), (u8::from(zoning), 0xcc));
        }
    }

    /// What a host hears while `run` tells it things.
    pub(super) fn heard(run: impl FnOnce(&mut Events<'_>)) -> Vec<ClientEvent> {
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "Test Server", "Tester");
        let mut received = Vec::new();
        let mut handler = |event| {
            received.push(event);
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        run(&mut log);
        drop(log);
        received
    }

    #[test]
    fn a_titanium_world_tells_its_name_rules_expansions_and_chat_server() {
        use eq_network_game::{
            channels::{ChatServer, ChatService},
            rules::Expansions,
        };
        let mut settings = [0_u8; 264];
        settings[8] = 1;
        settings[32..41].copy_from_slice(b"testworld");
        let events = heard(|log| {
            news(0x0fa6, &settings, log).unwrap();
            news(0x04ec, &15_u32.to_le_bytes(), log).unwrap();
            news(
                0x6536,
                b"chat.example.test,7778,testworld.Tester,b0123ABCD\0",
                log,
            )
            .unwrap();
            news(0x00d7, b"garbled\0", log).unwrap();
            // Anything else says nothing of the world.
            news(0x4513, &[0; 8], log).unwrap();
        });
        assert!(matches!(
            &events[..],
            [
                ClientEvent::World(WorldEvent::WorldName { short_name }),
                ClientEvent::World(WorldEvent::Rules(rules)),
                ClientEvent::World(WorldEvent::Expansions(Expansions(15))),
                ClientEvent::World(WorldEvent::ChatServer(ChatServer {
                    service: ChatService::Mail,
                    port: 7778,
                    ..
                })),
                ClientEvent::Diagnostic(unreadable),
            ] if short_name == "testworld"
                && rules.no_drop_trades
                && unreadable == "Chat server offer unreadable: incomplete chat server offer"
        ));
    }

    #[test]
    fn an_unavailable_zone_ends_the_attempt_at_once_with_its_name() {
        let mut body = [0_u8; 24];
        body[..5].copy_from_slice(b"misty");
        let mut error = None;
        let events = heard(|log| error = Some(unavailable(&body, log).unwrap()));
        assert_eq!(
            error.take().unwrap().to_string(),
            "zone misty is unavailable"
        );
        assert!(matches!(
            &events[..],
            [ClientEvent::World(WorldEvent::ZoneUnavailable { zone: Some(zone) })] if zone == "misty"
        ));
        // A garbled one still ends the attempt, with no zone named.
        let events = heard(|log| error = Some(unavailable(&[0; 3], log).unwrap()));
        assert_eq!(error.unwrap().to_string(), "the zone is unavailable");
        assert!(matches!(
            &events[..],
            [
                ClientEvent::Diagnostic(_),
                ClientEvent::World(WorldEvent::ZoneUnavailable { zone: None })
            ]
        ));
    }

    #[test]
    fn between_zones_the_world_must_name_the_character_in_play() {
        let events = heard(|log| {
            named(b"tester\0", "Tester", true, log).unwrap();
            named(b"\0", "Tester", true, log).unwrap();
            named(&[], "Tester", true, log).unwrap();
            assert_eq!(
                named(b"Other\0", "Tester", true, log)
                    .unwrap_err()
                    .to_string(),
                "the world named another character between zones"
            );
            // From the list, a name is EQEmu's automatic login: told, and
            // left to the session's own choice.
            named(b"\0", "Tester", false, log).unwrap();
            named(b"Other\0", "Tester", false, log).unwrap();
        });
        assert!(matches!(
            &events[..],
            [ClientEvent::Diagnostic(line)] if line.starts_with("The world would enter Other by itself")
        ));
    }

    #[test]
    fn entering_names_the_character_in_its_72_byte_field() {
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "Test Server", "Tester");
        let mut handler = |_| Ok(());
        let mut log = Events::new(&config, &mut handler);
        let mut kept = Kept::default();
        enter_character(&mut kept, "Tester", &mut log).unwrap();
        let (opcode, body) = &kept.0[0];
        assert_eq!(*opcode, 0x7cba);
        assert_eq!(body.len(), 72);
        assert_eq!(cstr(&body[..64]), b"Tester");
        assert_eq!(log.character, "Tester");
    }
}
