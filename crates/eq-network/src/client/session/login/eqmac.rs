//! `EQMac`'s login, as the TAKP login service runs it: the client says it
//! is ready, sends its account and password under the Verant key, finishes,
//! asks for the server list, and plays on the configured world or the
//! player's choice, which hands back the session key.
use super::{Credentials, Next, WorldChoice, ANSWER_LIMIT};
use crate::{
    client::{
        session::{cstr, put_string},
        CancellationToken, ClientCommand, ClientConfig, ClientEvent, ConnectionStage, Events,
        LoginError,
    },
    old_transport::OldSession,
};
use anyhow::{bail, ensure, Context, Result};
use eq_network_game::servers::{ServerChoice, ServerRefusal, ServerStatus};
use eq_network_login::crypto::{des_encrypt, DesKeyIv};
use std::{sync::mpsc::Receiver, time::Instant};
use zeroize::Zeroizing;

const VERANT_DES: DesKeyIv = DesKeyIv {
    key: [0x13, 0xd9, 0x13, 0x6d, 0xd0, 0x34, 0x15, 0xfb],
    iv: [0x13, 0xd9, 0x13, 0x6d, 0xd0, 0x34, 0x15, 0xfb],
};

const LOGIN_SESSION_READY: u16 = 0x5900;
const LOGIN_PC: u16 = 0x0100;
const LOGIN_ERROR: u16 = 0x0200;
const LOGIN_ACCEPTED: u16 = 0x0400;
const LOGIN_SERVER_LIST: u16 = 0x4600;
const LOGIN_PLAY: u16 = 0x4700;
const LOGIN_COMPLETE: u16 = 0x8800;

/// Where a listed world is, which the play request names it by.
struct ServerEntry {
    ip: String,
}

/// `EQMac`'s login: authenticate through the legacy TAKP login service,
/// play on the configured world or the player's choice from the list, and
/// return the session's credentials and the world's address.
pub(in crate::client::session) fn login(
    config: &ClientConfig,
    stop: &CancellationToken,
    commands: Option<&Receiver<ClientCommand>>,
    log: &mut Events<'_>,
) -> Result<(Credentials, String)> {
    let mut session = OldSession::connect_cancellable(
        crate::client::endpoint(&config.host, config.port, config.local_only)?,
        stop.flag(),
    )?;
    session.send(LOGIN_SESSION_READY, &[])?;
    let mut deadline = Instant::now() + ANSWER_LIMIT;
    let mut account = None;
    let mut choice = WorldChoice::default();
    let mut sent_login = false;
    let mut sent_complete = false;
    let mut requested_list = false;
    loop {
        let packet = match choice.next(&mut session, &mut deadline, stop, commands)? {
            Next::Chosen(world) => {
                play(&mut session, world)?;
                continue;
            }
            Next::Packet(packet) => packet,
        };
        match packet.opcode {
            LOGIN_SESSION_READY if !sent_login => {
                log.send(ClientEvent::Progress(ConnectionStage::Authenticating))?;
                session.send(LOGIN_PC, &login_credentials(config)?)?;
                sent_login = true;
            }
            LOGIN_ACCEPTED if account.is_none() => {
                account = Some(login_account(&packet.body)?);
                session.send(LOGIN_COMPLETE, &[])?;
                sent_complete = true;
                log.diagnostic("TAKP login server authenticated the account".into())?;
            }
            LOGIN_COMPLETE if sent_complete && !requested_list => {
                log.send(ClientEvent::Progress(ConnectionStage::SelectingServer))?;
                session.send(LOGIN_SERVER_LIST, &[])?;
                requested_list = true;
            }
            LOGIN_SERVER_LIST if requested_list && !choice.listed() => {
                let servers = parse_server_list(&packet.body)?;
                if let Some(world) = choice.list(servers, &config.server, log)? {
                    play(&mut session, world)?;
                }
            }
            LOGIN_PLAY if choice.asking() => {
                ensure!(packet.body.len() >= 11, "truncated TAKP play response");
                let key: [u8; 10] = packet.body[1..11].try_into().unwrap();
                ensure!(
                    key.iter().all(u8::is_ascii_alphanumeric),
                    "invalid TAKP session key"
                );
                session.close()?;
                let ip = choice.accepted(log)?.ip.clone();
                log.send(ClientEvent::Progress(ConnectionStage::ConnectingWorld))?;
                return Ok((
                    Credentials {
                        account: account.context("play response before authentication")?,
                        key,
                    },
                    ip,
                ));
            }
            // TAKP's login server refuses a world with its own words, and
            // keeps the connection open for another choice.
            LOGIN_ERROR => {
                let message = String::from_utf8_lossy(cstr(&packet.body)).into_owned();
                if account.is_none() {
                    return Err(LoginError::InvalidCredentials.into());
                }
                if !choice.asking() {
                    bail!("TAKP login server ended the login: {}", message.trim());
                }
                choice.refused(ServerRefusal::Text(message), log)?;
            }
            _ => (),
        }
    }
}

/// Asks the `EQMac` login server to play on a listed world, by its address.
fn play(session: &mut OldSession, world: &ServerEntry) -> Result<()> {
    let mut request = world.ip.as_bytes().to_vec();
    request.push(0);
    session.send(LOGIN_PLAY, &request)
}

fn login_credentials(config: &ClientConfig) -> Result<Zeroizing<Vec<u8>>> {
    let mut clear = Zeroizing::new([0; 40]);
    put_string(&mut clear[..20], config.credentials.account())?;
    put_string(&mut clear[20..], config.credentials.password())?;
    Ok(Zeroizing::new(des_encrypt(&*clear, VERANT_DES)))
}

fn login_account(body: &[u8]) -> Result<String> {
    ensure!(body.len() >= 21, "truncated TAKP login response");
    let account = std::str::from_utf8(cstr(&body[..10]))?;
    ensure!(
        account.starts_with("LS#") && account.len() > 3,
        "invalid TAKP account session"
    );
    Ok(account.to_owned())
}

/// Reads `EQMac`'s list as TAKP's login server writes it
/// (`ServerManager::CreateServerListPacket`): a count, two bytes we skip
/// and whether to show player counts, then each world's name and address,
/// a byte that marks it preferred, two words we skip, and a word that is
/// its players or its status (see [`ServerStatus::eqmac`]).
fn parse_server_list(body: &[u8]) -> Result<Vec<(ServerEntry, ServerChoice)>> {
    ensure!(body.len() >= 5, "truncated TAKP server list");
    let count = usize::from(u16::from_le_bytes(body[..2].try_into().unwrap()));
    let counted = body[4] != 0;
    let mut position = 5;
    let mut servers = Vec::with_capacity(count);
    for _ in 0..count {
        let name = take_string(body, &mut position)?;
        let ip = take_string(body, &mut position)?;
        let flags = body
            .get(position..position + 13)
            .context("truncated TAKP server flags")?;
        position += 13;
        let (status, players) =
            ServerStatus::eqmac(i32::from_le_bytes(flags[9..13].try_into().unwrap()));
        servers.push((
            ServerEntry { ip },
            ServerChoice {
                name,
                status,
                players: players.filter(|_| counted),
                preferred: flags[0] != 0,
            },
        ));
    }
    Ok(servers)
}

fn take_string(body: &[u8], position: &mut usize) -> Result<String> {
    let tail = body.get(*position..).context("truncated TAKP string")?;
    let length = tail
        .iter()
        .position(|&byte| byte == 0)
        .context("unterminated TAKP string")?;
    let result = String::from_utf8_lossy(&tail[..length]).into_owned();
    *position += length + 1;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eq_network_login::crypto::des_decrypt;

    fn config() -> ClientConfig {
        ClientConfig::for_protocol(
            crate::client::ServerProtocol::Quarm,
            "EXAMPLE_ACCOUNT",
            "EXAMPLE_PASSWORD",
            "The Project Quarm Server",
            "ExampleCharacter",
        )
    }

    #[test]
    fn pc_login_uses_the_verant_key_and_fixed_20_byte_fields() {
        let encrypted = login_credentials(&config()).unwrap();
        assert_eq!(encrypted.len(), 40);
        let clear = des_decrypt(&encrypted, VERANT_DES).unwrap();
        assert_eq!(cstr(&clear[..20]), b"EXAMPLE_ACCOUNT");
        assert_eq!(cstr(&clear[20..]), b"EXAMPLE_PASSWORD");
    }

    #[test]
    fn old_server_list_layout_reads_names_addresses_and_status() {
        // Two worlds, player counts shown.
        let mut body = vec![3, 0, 0, 0, 0xff];
        for (name, ip, preferred, count) in [
            ("Example Up Server", "198.51.100.10", 1, 123i32),
            ("Example Down Server", "203.0.113.20", 0, -1),
            ("Example Locked Server", "203.0.113.30", 0, -2),
        ] {
            body.extend_from_slice(name.as_bytes());
            body.push(0);
            body.extend_from_slice(ip.as_bytes());
            body.push(0);
            body.push(preferred);
            body.extend_from_slice(&1u32.to_le_bytes());
            body.extend_from_slice(&7u32.to_le_bytes());
            body.extend_from_slice(&count.to_le_bytes());
        }
        body.extend_from_slice(&[0; 26]);
        let servers = parse_server_list(&body).unwrap();
        let shown: Vec<_> = servers
            .iter()
            .map(|(entry, choice)| (entry.ip.as_str(), choice.clone()))
            .collect();
        let world = |name: &str, status, players, preferred| ServerChoice {
            name: name.into(),
            status,
            players,
            preferred,
        };
        assert_eq!(
            shown,
            [
                (
                    "198.51.100.10",
                    world("Example Up Server", ServerStatus::Up, Some(123), true)
                ),
                (
                    "203.0.113.20",
                    world("Example Down Server", ServerStatus::Down, None, false)
                ),
                (
                    "203.0.113.30",
                    world("Example Locked Server", ServerStatus::Locked, None, false)
                ),
            ]
        );
        // A login server that hides the counts.
        body[4] = 0;
        let (_, hidden) = &parse_server_list(&body).unwrap()[0];
        assert_eq!((hidden.status, hidden.players), (ServerStatus::Up, None));
        // A world cut short.
        assert!(parse_server_list(&body[..30]).is_err());
    }
}
