//! Logging in: the login server checks the account, lists the worlds and
//! hands out the session key the world and its zones check. Each client
//! generation logs in its own way: Titanium's is here, `EQMac`'s in
//! [`eqmac`]. Both play on the configured world, or list the worlds for the
//! player to choose from when none is configured.
use super::{
    coverage::{Stage, Unheard},
    cstr, le32,
    servers::server_type,
    wire::Titanium,
};
use crate::{
    client::{
        selection::{configured_world, Worlds},
        CancellationToken, ClientCommand, ClientConfig, ClientEvent, ConnectionStage, Events,
        LoginError,
    },
    transport::{Application, Session},
};
use anyhow::{bail, ensure, Context, Result};
use eq_network_game::servers::{ServerChoice, ServerRefusal, ServerStatus};
use eq_network_login::{
    crypto::{des_decrypt, DesKeyIv},
    login::{encrypt_login_credentials, is_bad_password_login_result},
    server_list::{parse_server_list, ServerEntry},
};
use eq_network_transport::Transport;
use std::{
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub(super) mod eqmac;

/// How long the login server has to answer, from the connection or from
/// the player's choice of world. The wait pauses while the player chooses,
/// as the world's does while its character list is up.
const ANSWER_LIMIT: Duration = Duration::from_secs(45);

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

/// Titanium's login: authenticate, play on the configured world or the
/// player's choice from the list, and return the session's credentials and
/// the world's address.
pub(super) fn titanium(
    config: &ClientConfig,
    stop: &CancellationToken,
    commands: Option<&Receiver<ClientCommand>>,
    log: &mut Events<'_>,
) -> Result<(Credentials, String)> {
    let mut session = Session::connect_cancellable(
        crate::client::endpoint(&config.host, config.port, config.local_only)?,
        false,
        stop.flag(),
    )?;
    session.send(1, &session_ready())?;
    let mut deadline = Instant::now() + ANSWER_LIMIT;
    let mut credentials = None;
    let mut choice = WorldChoice::default();
    let mut sent_credentials = false;
    let mut unheard = Unheard::new(Stage::Login);
    loop {
        let packet = match choice.next(&mut session, &mut deadline, stop, commands)? {
            Next::Chosen(world) => {
                play(&mut session, world)?;
                continue;
            }
            Next::Packet(packet) => packet,
        };
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
            LoginOpcode::ServerList if !choice.listed() => {
                ensure!(packet.body.len() >= 20, "truncated server list");
                let mut body = 0x18u16.to_le_bytes().to_vec();
                body.extend(packet.body);
                let (servers, _) = parse_server_list(&body).context("invalid server list")?;
                let servers = servers
                    .into_iter()
                    .map(|server| {
                        let shown = titanium_choice(&server);
                        (server, shown)
                    })
                    .collect();
                if let Some(world) = choice.list(servers, config, log)? {
                    play(&mut session, world)?;
                }
            }
            LoginOpcode::PlayResponse if choice.asking() => {
                ensure!(packet.body.len() >= 20, "truncated world-entry response");
                if packet.body[10] == 0 {
                    let refusal = ServerRefusal::Message(le32(&packet.body[11..15]));
                    choice.refused(refusal, log)?;
                    continue;
                }
                session.close()?;
                let ip = choice.accepted(log)?.ip.clone();
                let credentials = credentials.context("play response before authentication")?;
                log.send(ClientEvent::Progress(ConnectionStage::ConnectingWorld))?;
                return Ok((credentials, ip));
            }
            _ => unheard.tell(&Titanium, packet.opcode, packet.body.len(), log)?,
        }
    }
}

/// The session-ready body: the 10-byte header with sequence 2, then a 16-bit
/// 8 at +10, where the Titanium client writes it (from the program:
/// titanium-login-l1-part2, L-b; the 09-27 P99 capture agrees).
fn session_ready() -> [u8; 12] {
    let mut ready = [0; 12];
    ready[0] = 2;
    ready[10] = 8;
    ready
}

/// Asks the Titanium login server to play on a listed world.
fn play(session: &mut Session, world: &ServerEntry) -> Result<()> {
    let mut request = vec![0; 14];
    request[0] = 5;
    request[10..14].copy_from_slice(&world.runtime_id.to_le_bytes());
    session.send(0x0d, &request)
}

/// A Titanium list's world as the player sees it. `EQEmu`'s login server
/// marks a preferred world with type flag 8 (`ServerTypeFlags::Preferred`).
fn titanium_choice(server: &ServerEntry) -> ServerChoice {
    ServerChoice {
        name: server.name.clone(),
        status: ServerStatus::titanium(server.status),
        players: Some(server.player_count),
        preferred: server.list_id & 8 != 0,
    }
}

/// The world a login plays on: the configured one, or the player's choice
/// from the login server's list, which stays up through the login server's
/// refusals. `T` is a world as the generation's list gives it, which says
/// how to ask for it and where it is.
struct WorldChoice<T> {
    /// The list as the generation gives it, once it has come.
    listed: Option<Vec<T>>,
    /// The list the player chooses from, when no world is configured.
    worlds: Option<Worlds>,
    /// The world asked for, by its place in the list, until the login
    /// server answers.
    asked: Option<usize>,
}

impl<T> Default for WorldChoice<T> {
    fn default() -> Self {
        Self {
            listed: None,
            worlds: None,
            asked: None,
        }
    }
}

/// What a login waits on next.
enum Next<'a, T> {
    /// A packet from the login server.
    Packet(Application),
    /// The world the player chose, now asked for.
    Chosen(&'a T),
}

impl<T> WorldChoice<T> {
    /// Whether the login server's list has come.
    const fn listed(&self) -> bool {
        self.listed.is_some()
    }

    /// Whether the login server is answering for a world asked for.
    const fn asking(&self) -> bool {
        self.asked.is_some()
    }

    /// Takes the login server's list, each world as the generation gives it
    /// and as the player sees it, keeping the worlds the configured server
    /// type plays on: the configured world, to ask for at once, or none
    /// while the player chooses.
    ///
    /// # Errors
    /// Returns an error when the configured world is not kept or takes no
    /// players, or the host's event handler fails.
    fn list(
        &mut self,
        servers: Vec<(T, ServerChoice)>,
        config: &ClientConfig,
        log: &mut Events<'_>,
    ) -> Result<Option<&T>> {
        let server_type = server_type(config.protocol);
        let (kept, hidden): (Vec<_>, Vec<_>) = servers
            .into_iter()
            .partition(|(_, shown)| server_type.plays_on(&shown.name));
        let (listed, servers): (Vec<T>, Vec<ServerChoice>) = kept.into_iter().unzip();
        let listed = self.listed.insert(listed);
        let configured = config.server.as_str();
        if configured.is_empty() {
            self.worlds = Some(Worlds::publish(servers, log)?);
            return Ok(None);
        }
        ensure!(
            !hidden
                .iter()
                .any(|(_, shown)| shown.name.eq_ignore_ascii_case(configured)),
            "configured server is listed but hidden: the server type doesn't play on it"
        );
        let index = configured_world(&servers, configured)?;
        self.asked = Some(index);
        Ok(Some(&listed[index]))
    }

    /// The login server's next packet, or the player's choice of world
    /// while the list is up and no world is asked for. The deadline holds
    /// only while the player isn't choosing, and a choice gives the login
    /// server the whole limit again.
    ///
    /// # Errors
    /// Returns an error on shutdown, when the deadline passes, or when the
    /// connection fails.
    fn next(
        &mut self,
        session: &mut dyn Transport,
        deadline: &mut Instant,
        stop: &CancellationToken,
        commands: Option<&Receiver<ClientCommand>>,
    ) -> Result<Next<'_, T>> {
        loop {
            ensure!(!stop.is_cancelled(), "shutdown requested");
            match self.worlds.as_ref().filter(|_| self.asked.is_none()) {
                Some(worlds) => {
                    if let Some(index) = worlds.poll(commands) {
                        self.asked = Some(index);
                        *deadline = Instant::now() + ANSWER_LIMIT;
                        let listed = self
                            .listed
                            .as_ref()
                            .context("a world chosen before the list")?;
                        return Ok(Next::Chosen(&listed[index]));
                    }
                }
                None => ensure!(
                    Instant::now() < *deadline,
                    "application handshake timed out"
                ),
            }
            if let Some(packet) = session.receive()? {
                return Ok(Next::Packet(packet));
            }
        }
    }

    /// The login server refused the world asked for: the player chooses
    /// again from the list, or the session ends for a configured world.
    ///
    /// # Errors
    /// Returns an error for a configured world, or when the host's event
    /// handler fails.
    fn refused(&mut self, refusal: ServerRefusal, log: &mut Events<'_>) -> Result<()> {
        self.asked = None;
        match &self.worlds {
            Some(worlds) => worlds.refused(refusal, log),
            None => bail!("login server refused the configured server: {refusal}"),
        }
    }

    /// The world the login server accepted, which the session plays on.
    ///
    /// # Errors
    /// Returns an error when no world was asked for.
    fn accepted(&mut self, log: &mut Events<'_>) -> Result<&T> {
        let index = self
            .asked
            .take()
            .context("play response before selection")?;
        if let Some(worlds) = &self.worlds {
            worlds.name(index).clone_into(&mut log.server);
        }
        let listed = self
            .listed
            .as_ref()
            .context("play response before the list")?;
        Ok(&listed[index])
    }
}

/// The error string `EQEmu`'s login server names in a refused login's reply
/// (`loginserver/client.cpp` `SendFailedLogin`): the account or password
/// was wrong.
const BAD_CREDENTIALS_STRING: u32 = 105;

/// Use the SSO crate's failure signature before parsing the successful session key.
/// `EQEmu`'s own refusal, success 0 with error string 105, also reads as a
/// wrong account or password, so the player is told so.
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
    if clear[0] == 0 && le32(&clear[1..5]) == BAD_CREDENTIALS_STRING {
        return Err(LoginError::InvalidCredentials.into());
    }
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
    use crate::world::WorldEvent;
    use eq_network_login::crypto::des_encrypt;

    /// A listed world as the player sees it, by name, taking players.
    fn world(name: &str) -> ServerChoice {
        ServerChoice {
            name: name.into(),
            status: ServerStatus::Up,
            players: Some(1),
            preferred: false,
        }
    }

    /// A list shared by P99 and other `EQEmu` worlds, each with its place
    /// in the login server's list.
    fn shared_list() -> Vec<(usize, ServerChoice)> {
        [
            "Example EQEmu Server",
            "Project 1999: Green (Velious, PvE)",
            "Another EQEmu Server",
            "An Interesting Hole",
            "Project 1999: Red (Velious, PvP)",
        ]
        .into_iter()
        .map(world)
        .enumerate()
        .collect()
    }

    #[test]
    fn a_p99_login_shows_and_plays_only_the_worlds_p99_plays_on() {
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "", "");
        let mut events = Vec::new();
        let mut handler = |event| {
            events.push(event);
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        let mut choice = WorldChoice::default();
        assert!(choice
            .list(shared_list(), &config, &mut log)
            .unwrap()
            .is_none());
        drop(log);
        let Some(ClientEvent::World(WorldEvent::ServerSelection {
            selection_id,
            servers,
        })) = events.pop()
        else {
            panic!("the list is published");
        };
        let shown: Vec<&str> = servers.iter().map(|server| server.name.as_str()).collect();
        assert_eq!(
            shown,
            [
                "Project 1999: Green (Velious, PvE)",
                "An Interesting Hole",
                "Project 1999: Red (Velious, PvP)",
            ]
        );
        // The player's choice of the second shown world asks for the
        // login server's fourth.
        let (sender, commands) = std::sync::mpsc::channel();
        sender
            .send(ClientCommand::SelectServer {
                selection_id,
                index: 1,
            })
            .unwrap();
        assert_eq!(
            choice.worlds.as_ref().unwrap().poll(Some(&commands)),
            Some(1)
        );
        assert_eq!(choice.listed.as_ref().unwrap()[1], 3);
    }

    #[test]
    fn a_configured_world_is_found_among_the_kept_worlds_only() {
        let mut handler = |_| Ok(());
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "an interesting hole", "");
        let mut log = Events::new(&config, &mut handler);
        let mut choice = WorldChoice::default();
        let asked = choice.list(shared_list(), &config, &mut log).unwrap();
        assert_eq!(asked, Some(&3));
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "Another EQEmu Server", "");
        let mut log = Events::new(&config, &mut handler);
        let error = WorldChoice::default()
            .list(shared_list(), &config, &mut log)
            .unwrap_err();
        assert!(error.to_string().contains("hidden"), "{error}");
        // A stock EQEmu login plays on every world its login server lists.
        let mut config = config;
        config.protocol = crate::client::ServerProtocol::EqEmu;
        let mut log = Events::new(&config, &mut handler);
        let mut choice = WorldChoice::default();
        assert_eq!(
            choice.list(shared_list(), &config, &mut log).unwrap(),
            Some(&2)
        );
    }

    #[test]
    fn session_ready_puts_its_8_at_10_and_nowhere_else() {
        let ready = session_ready();
        assert_eq!(ready.len(), 12);
        assert_eq!(ready[0], 2);
        assert_eq!(ready[10], 8);
        assert!(ready
            .iter()
            .enumerate()
            .all(|(at, &byte)| at == 0 || at == 10 || byte == 0));
    }

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

    #[test]
    fn eqemus_refusal_reads_as_a_wrong_account_or_password() {
        // `SendFailedLogin`: everything zero but success false and string 105.
        let mut clear = vec![0; 32];
        clear[1..5].copy_from_slice(&105u32.to_le_bytes());
        let mut body = vec![0; 10];
        body[0] = 3;
        body[5] = 2;
        body.extend(des_encrypt(&clear, DesKeyIv::default()));
        let error = login_credentials(&body).err().unwrap();
        assert!(matches!(
            error.downcast_ref::<LoginError>(),
            Some(LoginError::InvalidCredentials)
        ));
        // Another refusal is still only a rejection.
        clear[1..5].copy_from_slice(&106u32.to_le_bytes());
        body.truncate(10);
        body.extend(des_encrypt(&clear, DesKeyIv::default()));
        let error = login_credentials(&body).err().unwrap();
        assert!(!error.is::<LoginError>());
    }
}
