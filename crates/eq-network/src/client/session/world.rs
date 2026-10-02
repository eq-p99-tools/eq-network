//! The world server: validating the client, the character list, choosing or
//! creating a character, and the handoff to its zone. Each client
//! generation runs it its own way: Titanium's is here, `EQMac`'s in
//! [`eqmac`].
use super::{
    cstr, login::Credentials, put_string, servers, servers::Shield, CharacterSession,
    ZoneDestination,
};
use crate::{
    assets::Assets,
    chat,
    client::{ClientEvent, ClientIdentity, ConnectionStage, Events, RecordEvent},
    transport::Session,
};
use anyhow::{ensure, Context, Result};
use std::{
    net::IpAddr,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub(super) mod eqmac;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorldOpcode {
    LogServer,
    ApprovalChallenge,
    FileManifest,
    ValidationResult,
    CharacterList,
    ZoneHandoff,
    ApproveName,
    Guilds,
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
            eq_network_game::listing::GUILDS_OPCODE => Self::Guilds,
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
                .and_then(|list: &crate::client::selection::Selection| list.poll(context.commands))
            {
                Some(crate::client::selection::Choice::Enter(name)) => chosen = Some(name),
                Some(crate::client::selection::Choice::Create(character)) if creating.is_none() => {
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
            _ => (),
        }
        // Stock EQEmu accepts the client without the V62 validation reply.
        if world_only && accepted {
            session.close()?;
            return Ok(None);
        }
    }
}

#[cfg(test)]
mod tests {
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
