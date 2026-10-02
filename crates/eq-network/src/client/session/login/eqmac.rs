//! `EQMac`'s login, as the TAKP login service runs it: the client says it
//! is ready, sends its account and password under the Verant key, finishes,
//! asks for the server list, and plays on the configured world, which hands
//! back the session key.
use super::Credentials;
use crate::{
    client::{
        session::{cstr, next, put_string},
        CancellationToken, ClientConfig, ClientEvent, ConnectionStage, Events, LoginError,
    },
    old_transport::OldSession,
};
use anyhow::{bail, ensure, Context, Result};
use eq_network_login::crypto::{des_encrypt, DesKeyIv};
use std::time::{Duration, Instant};
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

struct ServerEntry {
    name: String,
    ip: String,
}

/// `EQMac`'s login: authenticate through the legacy TAKP login service,
/// select the configured world, and return the session's credentials and
/// the world's address.
pub(in crate::client::session) fn login(
    config: &ClientConfig,
    stop: &CancellationToken,
    log: &mut Events<'_>,
) -> Result<(Credentials, String)> {
    let mut session = OldSession::connect_cancellable(
        crate::client::endpoint(&config.host, config.port, config.local_only)?,
        stop.flag(),
    )?;
    session.send(LOGIN_SESSION_READY, &[])?;
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut account = None;
    let mut selected = None;
    let mut sent_login = false;
    let mut sent_complete = false;
    let mut requested_list = false;
    loop {
        let packet = next(&mut session, deadline, stop)?;
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
            LOGIN_SERVER_LIST if requested_list && selected.is_none() => {
                let servers = parse_server_list(&packet.body)?;
                let server = servers
                    .into_iter()
                    .find(|server| server.name.eq_ignore_ascii_case(&config.server))
                    .context("configured server name was not found in the TAKP server list")?;
                let mut request = server.ip.as_bytes().to_vec();
                request.push(0);
                session.send(LOGIN_PLAY, &request)?;
                selected = Some(server.ip);
            }
            LOGIN_PLAY if selected.is_some() => {
                ensure!(packet.body.len() >= 11, "truncated TAKP play response");
                let key: [u8; 10] = packet.body[1..11].try_into().unwrap();
                ensure!(
                    key.iter().all(u8::is_ascii_alphanumeric),
                    "invalid TAKP session key"
                );
                session.close()?;
                log.send(ClientEvent::Progress(ConnectionStage::ConnectingWorld))?;
                return Ok((
                    Credentials {
                        account: account.context("play response before authentication")?,
                        key,
                    },
                    selected.unwrap(),
                ));
            }
            LOGIN_ERROR => {
                let message = String::from_utf8_lossy(cstr(&packet.body)).into_owned();
                if account.is_none() {
                    return Err(LoginError::InvalidCredentials.into());
                }
                bail!("TAKP login server rejected world entry: {message}");
            }
            _ => (),
        }
    }
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

fn parse_server_list(body: &[u8]) -> Result<Vec<ServerEntry>> {
    ensure!(body.len() >= 5, "truncated TAKP server list");
    let count = usize::from(u16::from_le_bytes(body[..2].try_into().unwrap()));
    let mut position = 5;
    let mut servers = Vec::with_capacity(count);
    for _ in 0..count {
        let name = take_string(body, &mut position)?;
        let ip = take_string(body, &mut position)?;
        ensure!(position + 13 <= body.len(), "truncated TAKP server flags");
        position += 13;
        servers.push(ServerEntry { name, ip });
    }
    ensure!(servers.len() == count, "TAKP server count mismatch");
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
    fn old_server_list_layout_selects_names_and_addresses() {
        let mut body = vec![2, 0, 0, 0, 0];
        for (name, ip, id) in [
            ("The Al'Kabor Project Server", "198.51.100.10", 1u32),
            ("The Project Quarm Server", "203.0.113.20", 2u32),
        ] {
            body.extend_from_slice(name.as_bytes());
            body.push(0);
            body.extend_from_slice(ip.as_bytes());
            body.push(0);
            body.push(0);
            body.extend_from_slice(&1u32.to_le_bytes());
            body.extend_from_slice(&id.to_le_bytes());
            body.extend_from_slice(&123u32.to_le_bytes());
        }
        body.extend_from_slice(&[0; 26]);
        let servers = parse_server_list(&body).unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[1].name, "The Project Quarm Server");
        assert_eq!(servers[1].ip, "203.0.113.20");
    }
}
