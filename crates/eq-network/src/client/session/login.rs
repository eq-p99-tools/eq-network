//! Logging in: the login server checks the account, lists the worlds and
//! hands out the session key the world and its zones check. Each client
//! generation logs in its own way: Titanium's is here, `EQMac`'s in
//! [`eqmac`].
use super::{cstr, le32, next};
use crate::{
    client::{CancellationToken, ClientConfig, ClientEvent, ConnectionStage, Events, LoginError},
    transport::Session,
};
use anyhow::{ensure, Context, Result};
use eq_network_login::{
    crypto::{des_decrypt, DesKeyIv},
    login::{encrypt_login_credentials, is_bad_password_login_result},
    server_list::parse_server_list,
};
use std::time::{Duration, Instant};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub(super) mod eqmac;

/// The login server's session for the world: the account as the login
/// server named it, and the key the world and its zones check.
#[derive(Zeroize, ZeroizeOnDrop)]
pub(in crate::client) struct Credentials {
    /// The account: Titanium's number, or `EQMac`'s `LS#` name.
    pub(in crate::client) account: String,
    /// The session key.
    pub(in crate::client) key: [u8; 10],
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

/// Titanium's login: authenticate, select the configured server, and
/// return the session's credentials and its world endpoint.
pub(super) fn titanium(
    config: &ClientConfig,
    stop: &CancellationToken,
    log: &mut Events<'_>,
) -> Result<(Credentials, String)> {
    let mut session = Session::connect_cancellable(
        crate::client::endpoint(&config.host, config.port, config.local_only)?,
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
    Ok(Credentials {
        account: account.to_string(),
        key,
    })
}

#[cfg(test)]
mod tests {
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
        assert_eq!(credentials.account, "12345");
        assert_eq!(&credentials.key, b"EXAMPLEKEY");
        for length in [0, 10, 18, 26] {
            let error = login_credentials(&body[..length]).err().unwrap();
            assert!(!error.is::<LoginError>());
        }
    }
}
