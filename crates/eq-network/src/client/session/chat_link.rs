//! The chat server link: one connection per character's stay in the world,
//! opened from the world's Titanium chat-channel offer where the server type
//! opens one, and the zone feature that takes its commands and tells what it
//! hears (recording checklist 6.509).
//!
//! The link runs on its own thread so that it can never stall the game
//! session (reviewer C1): commands reach it through a bounded queue, a full
//! one refusing the command; what it hears waits in a bounded queue the zone
//! drains, a full one dropping the oldest and telling so once. Whatever ends
//! it, refused address, failed connect, refused login, a lost connection or
//! something unreadable, is one [`ChannelLink::Lost`] and nothing more: the
//! world and zone go on. It sends nothing but the login and the commands the
//! player gives (reviewer C3); the transport sends its own keepalives.
//!
//! Nothing it logs or tells names the server's address, the key or the
//! mailbox (reviewer C2).

use super::{
    feature::{Feature, Out, World},
    ClientCommand,
};
use crate::{
    client::{ClientEvent, GapKind},
    world::{Capability, WorldEvent},
};
use anyhow::Result;
use eq_network_game::chat_server::{self, ChannelEvent, ChannelLink, LinkLoss, Login};
use eq_network_transport::Transport;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc, Mutex, PoisonError,
    },
    thread::JoinHandle,
    time::Duration,
};

/// Commands waiting for the link at most; this library's own bound.
pub(super) const OUTBOUND: usize = 16;
/// What the link heard, waiting for the zone, at most; this library's own
/// bound.
pub(super) const INBOUND: usize = 256;

/// Opens the transport to the chat server a login names.
pub(super) type Connect =
    dyn Fn(&Login, bool, &AtomicBool) -> Result<Box<dyn Transport + Send>, LinkLoss> + Send + Sync;

/// The production connection: the offered address through the same checks
/// as a zone's ([`crate::client::endpoint`]), then the modern transport.
fn connect(
    login: &Login,
    local_only: bool,
    stop: &AtomicBool,
) -> Result<Box<dyn Transport + Send>, LinkLoss> {
    let server = login.server();
    let address = crate::client::endpoint(&server.host, server.port, local_only)
        .map_err(|_| LinkLoss::Address)?;
    crate::transport::Session::connect_cancellable(address, false, stop)
        .map(|session| Box::new(session) as Box<dyn Transport + Send>)
        .map_err(|_| LinkLoss::Connect)
}

/// What the link heard, for the zone to tell.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum News {
    /// It came up or ended.
    Link(ChannelLink),
    /// The chat server told something.
    Event(ChannelEvent),
    /// The chat server sent something a channels link leaves unread: its
    /// opcode and length only.
    Unread { opcode: u16, length: usize },
    /// This many were dropped when the queue was full.
    Dropped(u64),
}

/// What the link's thread and the session share.
#[derive(Default)]
struct Inbound {
    news: VecDeque<News>,
    dropped: u64,
    up: bool,
}

impl Inbound {
    fn push(&mut self, news: News) {
        if self.news.len() >= INBOUND {
            self.news.pop_front();
            self.dropped += 1;
        }
        self.news.push_back(news);
    }
}

/// A link while its thread runs.
struct Running {
    outbound: SyncSender<Vec<u8>>,
    inbound: Arc<Mutex<Inbound>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Running {
    fn close(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            // The thread checks its stop at least every receive wait.
            let _ = thread.join();
        }
    }
}

/// The character's chat server link, kept across zones.
pub(in crate::client) struct ChatLink {
    connect: Arc<Connect>,
    running: Mutex<Option<Running>>,
    /// What a closed link left untold, kept for the next drain.
    left: Mutex<Vec<News>>,
}

impl Default for ChatLink {
    fn default() -> Self {
        Self::with(Arc::new(connect))
    }
}

impl ChatLink {
    /// A link that connects with `connect`.
    pub(super) fn with(connect: Arc<Connect>) -> Self {
        Self {
            connect,
            running: Mutex::new(None),
            left: Mutex::new(Vec::new()),
        }
    }

    /// Opens a link from a login, closing any open one first: one connect
    /// per offer, never a retry (reviewer C2).
    pub(super) fn open(&self, login: Login, local_only: bool) {
        self.close();
        let (outbound, commands) = mpsc::sync_channel(OUTBOUND);
        let inbound = Arc::new(Mutex::new(Inbound::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            // The thread holds the connector, never the link, so dropping
            // the link closes it.
            let (connect, inbound, stop) = (
                Arc::clone(&self.connect),
                Arc::clone(&inbound),
                Arc::clone(&stop),
            );
            std::thread::Builder::new()
                .name("chat-link".into())
                .spawn(move || {
                    let loss = run(&*connect, &login, local_only, &commands, &inbound, &stop);
                    if let Some(reason) = loss {
                        let mut inbound = inbound.lock().unwrap_or_else(PoisonError::into_inner);
                        inbound.up = false;
                        inbound.push(News::Link(ChannelLink::Lost { reason }));
                    }
                })
        };
        let Ok(thread) = thread else {
            self.left
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(News::Link(ChannelLink::Lost {
                    reason: LinkLoss::Connect,
                }));
            return;
        };
        *self.running.lock().unwrap_or_else(PoisonError::into_inner) = Some(Running {
            outbound,
            inbound,
            stop,
            thread: Some(thread),
        });
    }

    /// Closes the link and waits for its thread to end.
    pub(in crate::client) fn close(&self) {
        let running = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(running) = running {
            running.close();
        }
    }

    /// Whether a link was opened from an offer and not closed since, up,
    /// coming up or lost.
    pub(super) fn opened(&self) -> bool {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// Whether the link's thread is running.
    #[cfg(test)]
    pub(super) fn running(&self) -> bool {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|running| running.thread.as_ref())
            .is_some_and(|thread| !thread.is_finished())
    }

    /// Hands a command to the link; the reason it was not, in this
    /// library's words. Nothing is kept for later (reviewer C4).
    pub(super) fn send(&self, text: &str) -> Result<(), &'static str> {
        let running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(running) = running.as_ref() else {
            return Err("There is no chat server link");
        };
        if !running
            .inbound
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .up
        {
            return Err("The chat server has not taken the login");
        }
        let body =
            chat_server::command(text).map_err(|_| "That chat channel command cannot be sent")?;
        match running.outbound.try_send(body) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err("Too many chat channel commands are waiting"),
            Err(TrySendError::Disconnected(_)) => Err("The chat server link has ended"),
        }
    }

    /// What the link heard since the last drain.
    pub(super) fn drain(&self) -> Vec<News> {
        let mut news =
            std::mem::take(&mut *self.left.lock().unwrap_or_else(PoisonError::into_inner));
        if let Some(running) = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            let mut inbound = running
                .inbound
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            news.extend(inbound.news.drain(..));
            if inbound.dropped > 0 {
                news.push(News::Dropped(std::mem::take(&mut inbound.dropped)));
            }
        }
        news
    }
}

impl Drop for ChatLink {
    fn drop(&mut self) {
        self.close();
    }
}

/// The link's thread: connects, logs in, then carries commands out and what
/// the server says in until it is stopped or fails; why it failed, or None
/// when it was stopped.
fn run(
    connect: &Connect,
    login: &Login,
    local_only: bool,
    commands: &Receiver<Vec<u8>>,
    inbound: &Mutex<Inbound>,
    stop: &AtomicBool,
) -> Option<LinkLoss> {
    let mut transport = match connect(login, local_only, stop) {
        Ok(transport) => transport,
        Err(_) if stop.load(Ordering::Relaxed) => return None,
        Err(loss) => return Some(loss),
    };
    if transport
        .send(chat_server::LOGIN_OPCODE, &login.body())
        .is_err()
    {
        return Some(LinkLoss::Login);
    }
    let mut up = false;
    let lost = |up: bool| {
        if up {
            LinkLoss::Dropped
        } else {
            LinkLoss::Login
        }
    };
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = transport.close();
            return None;
        }
        loop {
            match commands.try_recv() {
                Ok(body) if transport.send(chat_server::COMMAND_OPCODE, &body).is_err() => {
                    return Some(lost(up));
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let _ = transport.close();
                    return None;
                }
            }
        }
        let packet = match transport.receive() {
            Ok(Some(packet)) => packet,
            Ok(None) => {
                if transport.termination().is_some() {
                    return Some(lost(up));
                }
                // The modern transport waits inside receive; a transport
                // that does not is still not spun on.
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            Err(_) => return Some(lost(up)),
        };
        let mut inbound = inbound.lock().unwrap_or_else(PoisonError::into_inner);
        if packet.opcode == chat_server::LOGIN_OPCODE {
            // The server's mailboxes answer a login it took.
            if !up {
                up = true;
                inbound.up = true;
                inbound.push(News::Link(ChannelLink::Up));
            }
            continue;
        }
        match chat_server::read(packet.opcode, &packet.body) {
            Ok(Some(event)) => inbound.push(News::Event(event)),
            Ok(None) => inbound.push(News::Unread {
                opcode: packet.opcode,
                length: packet.body.len(),
            }),
            Err(_) => {
                drop(inbound);
                let _ = transport.close();
                return Some(LinkLoss::Malformed);
            }
        }
    }
}

/// Opens the character's chat link from the world's Titanium chat-channel
/// offer, where the server type opens one; an offer of anything else, or a
/// malformed one, opens nothing (the offer's own reading reports it). True
/// when a link was opened.
pub(super) fn offered(
    server: &dyn super::servers::ServerType,
    link: &Arc<ChatLink>,
    body: &[u8],
    local_only: bool,
) -> bool {
    if !server.chat_channels() {
        return false;
    }
    match Login::titanium(body) {
        Ok(Some(login)) => {
            link.open(login, local_only);
            true
        }
        Ok(None) | Err(_) => false,
    }
}

/// The zone's side of the link: takes chat channel commands and tells what
/// the link heard.
pub(super) struct Channels {
    link: Arc<ChatLink>,
}

impl Channels {
    pub(super) const fn new(link: Arc<ChatLink>) -> Self {
        Self { link }
    }
}

/// The name a chat-server opcode a channels link leaves unread goes by in
/// the gap log.
const fn unread_name(opcode: u16) -> &'static str {
    match opcode {
        chat_server::BUDDY_OPCODE => "chat server OP_Buddy",
        chat_server::IGNORE_OPCODE => "chat server OP_Ignore",
        0x0d..=0x14 => "chat server mail",
        _ => "chat server",
    }
}

impl Feature for Channels {
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::ChatChannels]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::ChatChannel { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::ChatChannel {
            session_id, text, ..
        } = command
        else {
            return Ok(());
        };
        if *session_id != world.session_id {
            return super::actions::refuse(
                command,
                "That was meant for an earlier zone visit",
                out.log,
            );
        }
        match self.link.send(text) {
            Ok(()) => Ok(()),
            Err(reason) => super::actions::refuse(command, reason, out.log),
        }
    }

    fn tick(
        &mut self,
        _now: std::time::Instant,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        for news in self.link.drain() {
            match news {
                News::Link(link) => out
                    .log
                    .send(ClientEvent::World(WorldEvent::ChannelLink(link)))?,
                News::Event(event) => out
                    .log
                    .send(ClientEvent::World(WorldEvent::Channel(event)))?,
                News::Unread { opcode, length } => out.log.gap(
                    opcode,
                    Some(unread_name(opcode)),
                    &vec![0; length],
                    GapKind::Unread {
                        reason: "the chat server's mail, friends and ignore lists are not built"
                            .to_owned(),
                    },
                )?,
                News::Dropped(count) => out.log.diagnostic(format!(
                    "Chat server link: {count} messages dropped while the queue was full"
                ))?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use crate::transport::Application;
    use std::time::Instant;

    /// A key no test should find outside the login body.
    const KEY: &str = "0123ABCD";

    fn offer() -> Vec<u8> {
        format!("chat.example.test,7778,testworld.Tester,B{KEY}\0").into_bytes()
    }

    fn login() -> Login {
        Login::titanium(&offer()).unwrap().unwrap()
    }

    /// What a stub chat server does.
    #[derive(Clone, Copy)]
    enum Stub {
        /// Takes the login, then sends a channel message.
        Takes,
        /// Never answers.
        Stalls,
        /// Answers with something no chat server sends.
        Rubbish,
        /// Takes the login, then the connection fails.
        Drops,
        /// Takes the login, then floods messages.
        Floods,
        /// Takes the login, then waits long in each receive.
        Slow,
    }

    /// A stub transport, recording what the link sent.
    struct Fake {
        stub: Stub,
        script: VecDeque<Application>,
        sent: Sent,
        logged_in: bool,
    }

    fn strings(parts: &[&str]) -> Vec<u8> {
        parts
            .iter()
            .flat_map(|part| part.bytes().chain([0]))
            .collect()
    }

    impl Transport for Fake {
        fn send(&mut self, opcode: u16, body: &[u8]) -> Result<()> {
            self.sent.lock().unwrap().push((opcode, body.to_vec()));
            if opcode == chat_server::LOGIN_OPCODE && !self.logged_in {
                self.logged_in = true;
                match self.stub {
                    Stub::Takes | Stub::Drops | Stub::Floods | Stub::Slow => {
                        self.script.push_back(Application {
                            opcode: chat_server::LOGIN_OPCODE,
                            body: vec![1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                        });
                    }
                    Stub::Stalls => {}
                    Stub::Rubbish => self.script.push_back(Application {
                        opcode: 0x7777,
                        body: vec![0xff; 9],
                    }),
                }
                match self.stub {
                    Stub::Takes => self.script.push_back(Application {
                        opcode: chat_server::MESSAGE_OPCODE,
                        body: strings(&["test1", "testworld.Other", "hi"]),
                    }),
                    Stub::Floods => {
                        for n in 0..INBOUND + 20 {
                            self.script.push_back(Application {
                                opcode: chat_server::MESSAGE_OPCODE,
                                body: strings(&["", "", &format!("line {n}")]),
                            });
                        }
                        self.script.push_back(Application {
                            opcode: chat_server::BUDDY_OPCODE,
                            body: vec![9; 5],
                        });
                    }
                    Stub::Stalls | Stub::Rubbish | Stub::Drops | Stub::Slow => {}
                }
            }
            Ok(())
        }

        fn receive(&mut self) -> Result<Option<Application>> {
            if let Some(packet) = self.script.pop_front() {
                return Ok(Some(packet));
            }
            match self.stub {
                Stub::Drops if self.logged_in => anyhow::bail!("gone"),
                Stub::Slow => {
                    std::thread::sleep(Duration::from_millis(200));
                    Ok(None)
                }
                _ => Ok(None),
            }
        }

        fn close(&mut self) -> Result<()> {
            self.sent.lock().unwrap().push((CLOSED, Vec::new()));
            Ok(())
        }

        fn last_received_seconds(&self) -> u64 {
            0
        }
    }

    type Sent = Arc<Mutex<Vec<(u16, Vec<u8>)>>>;

    /// What the fake records when the link closes it.
    const CLOSED: u16 = 0xffff;

    /// A link to a stub server, with what it sent and how often it
    /// connected.
    fn link(stub: Option<Stub>) -> (Arc<ChatLink>, Sent, Arc<Mutex<usize>>) {
        let sent: Sent = Arc::default();
        let connects = Arc::new(Mutex::new(0));
        let (into, count) = (Arc::clone(&sent), Arc::clone(&connects));
        let link = ChatLink::with(Arc::new(move |_login, _local, _stop| {
            *count.lock().unwrap() += 1;
            let Some(stub) = stub else {
                return Err(LinkLoss::Connect);
            };
            Ok(Box::new(Fake {
                stub,
                script: VecDeque::new(),
                sent: Arc::clone(&into),
                logged_in: false,
            }) as Box<dyn Transport + Send>)
        }));
        (Arc::new(link), sent, connects)
    }

    /// Drains the link until `done` holds of all it told, or a second
    /// passes.
    fn until(link: &ChatLink, done: impl Fn(&[News]) -> bool) -> Vec<News> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut all = Vec::new();
        while Instant::now() < deadline {
            all.extend(link.drain());
            if done(&all) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        all
    }

    #[test]
    fn a_link_logs_in_comes_up_and_carries_commands_and_messages() {
        let (link, sent, connects) = link(Some(Stub::Takes));
        assert_eq!(link.send("list"), Err("There is no chat server link"));
        link.open(login(), true);
        let news = until(&link, |news| {
            news.iter().any(|n| matches!(n, News::Event(_)))
        });
        assert_eq!(news[0], News::Link(ChannelLink::Up));
        assert_eq!(
            news[1],
            News::Event(ChannelEvent::Message {
                channel: "test1".into(),
                sender: "Other".into(),
                text: "hi".into(),
            })
        );
        link.send("join test1").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while sent.lock().unwrap().len() < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let sent = sent.lock().unwrap().clone();
        // The login first, then only what the player asked for (C3).
        assert_eq!(sent[0].0, chat_server::LOGIN_OPCODE);
        assert_eq!(
            sent[1],
            (chat_server::COMMAND_OPCODE, b"\0join test1\0".to_vec())
        );
        assert_eq!(sent.len(), 2);
        assert_eq!(*connects.lock().unwrap(), 1);
        assert!(!format!("{news:?}").contains(KEY));
        link.close();
        assert!(!link.running());
        assert!(!link.opened());
    }

    #[test]
    fn refused_stalled_rubbish_and_dropped_links_end_once_and_never_retry() {
        for (stub, loss) in [
            (None, LinkLoss::Connect),
            (Some(Stub::Rubbish), LinkLoss::Malformed),
            (Some(Stub::Drops), LinkLoss::Dropped),
        ] {
            let (link, _, connects) = link(stub);
            link.open(login(), false);
            let news = until(&link, |news| {
                news.iter()
                    .any(|n| matches!(n, News::Link(ChannelLink::Lost { .. })))
            });
            let lost: Vec<_> = news
                .iter()
                .filter(|n| matches!(n, News::Link(ChannelLink::Lost { .. })))
                .collect();
            assert_eq!(lost, [&News::Link(ChannelLink::Lost { reason: loss })]);
            assert!(link.send("list").is_err());
            std::thread::sleep(Duration::from_millis(20));
            assert_eq!(*connects.lock().unwrap(), 1, "{loss:?}");
            assert_eq!(link.drain().len(), 0);
        }
        // A server that never answers leaves the link down, refusing
        // commands, until it is closed; closing ends the thread.
        let (link, _, _) = link(Some(Stub::Stalls));
        link.open(login(), false);
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(
            link.send("list"),
            Err("The chat server has not taken the login")
        );
        assert!(link.running());
        link.close();
        assert!(!link.running());
    }

    #[test]
    fn a_second_offer_closes_the_first_link_and_a_drop_ends_the_thread() {
        let (link, sent, connects) = link(Some(Stub::Stalls));
        link.open(login(), false);
        link.open(login(), false);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(*connects.lock().unwrap(), 2);
        assert!(link.running());
        // Dropping the session's link, as a camp, the character list, a
        // stop or a lost world or zone connection does, ends the thread:
        // it closes its transport before the drop returns.
        drop(link);
        let closes = sent
            .lock()
            .unwrap()
            .iter()
            .filter(|(opcode, _)| *opcode == CLOSED)
            .count();
        assert_eq!(closes, 2);
    }

    #[test]
    fn a_camp_or_the_character_list_ends_the_link_and_its_thread() {
        let config = crate::client::ClientConfig::for_protocol(
            crate::client::ServerProtocol::EqEmu,
            "ACCOUNT",
            "PASSWORD",
            "Test Server",
            "Tester",
        );
        let identity = crate::client::ClientIdentity::new("example-host", "example-user");
        let credentials = super::super::login::Credentials {
            account: "LS#1".into(),
            key: *b"0123456789",
        };
        let stop = crate::client::CancellationToken::default();
        let mut session = super::super::CharacterSession::new(
            &config,
            &identity,
            &credentials,
            &stop,
            None,
            None,
        );
        let (link, sent, _) = link(Some(Stub::Stalls));
        session.chat = Arc::clone(&link);
        link.open(login(), false);
        std::thread::sleep(Duration::from_millis(20));
        assert!(link.running());
        // A zone change keeps it.
        session.on_zone_exit(&super::super::ZoneExit::Direct(Vec::new()));
        assert!(link.running());
        session.on_zone_exit(&super::super::ZoneExit::CharacterSelect);
        assert!(!link.running());
        assert!(sent
            .lock()
            .unwrap()
            .iter()
            .any(|(opcode, _)| *opcode == CLOSED));
    }

    #[test]
    fn full_queues_drop_the_oldest_inbound_and_refuse_outbound() {
        let (link, _, _) = link(Some(Stub::Floods));
        link.open(login(), false);
        let news = until(&link, |news| {
            news.iter().any(|n| matches!(n, News::Unread { .. }))
        });
        // Up, then lines, the buddy list unread, and the count dropped.
        assert!(news.iter().any(|n| matches!(
            n,
            News::Unread {
                opcode: chat_server::BUDDY_OPCODE,
                length: 5
            }
        )));
        link.close();
        let (link, _, _) = link_slow();
        link.open(login(), false);
        until(&link, |news| news.contains(&News::Link(ChannelLink::Up)));
        let refused = (0..OUTBOUND * 3)
            .map(|_| link.send("list"))
            .filter(Result::is_err)
            .collect::<Vec<_>>();
        assert!(refused.contains(&Err("Too many chat channel commands are waiting")));
        link.close();
    }

    fn link_slow() -> (Arc<ChatLink>, Sent, Arc<Mutex<usize>>) {
        link(Some(Stub::Slow))
    }

    /// Whatever the chat server does, the zone's side of the link never
    /// fails: the zone loop goes on (reviewer C1).
    #[test]
    fn the_zone_goes_on_whatever_the_chat_server_does() {
        for stub in [
            None,
            Some(Stub::Stalls),
            Some(Stub::Rubbish),
            Some(Stub::Drops),
            Some(Stub::Floods),
        ] {
            let (link, _, _) = link(stub);
            let mut channels = Channels::new(Arc::clone(&link));
            let mut world = World::new(5);
            link.open(login(), false);
            let command = ClientCommand::ChatChannel {
                session_id: 5,
                text: "join test1".into(),
                created: Instant::now(),
            };
            for _ in 0..20 {
                let outcome = testing::run(|out| {
                    channels.tick(Instant::now(), &mut world, out)?;
                    channels.handle(&command, &mut world, out)
                });
                outcome.result.unwrap();
                // Nothing a chat link does goes to the zone's connection.
                assert_eq!(outcome.sent, []);
                std::thread::sleep(Duration::from_millis(5));
            }
            link.close();
        }
    }

    #[test]
    fn the_inbound_queue_is_bounded() {
        let mut inbound = Inbound::default();
        for n in 0..INBOUND + 3 {
            inbound.push(News::Unread {
                opcode: 1,
                length: n,
            });
        }
        assert_eq!(inbound.news.len(), INBOUND);
        assert_eq!(inbound.dropped, 3);
        assert_eq!(
            inbound.news[0],
            News::Unread {
                opcode: 1,
                length: 3
            }
        );
    }

    #[test]
    fn only_a_server_type_that_opens_links_connects_from_an_offer() {
        use crate::client::ServerProtocol;
        for (protocol, opens) in [
            (ServerProtocol::EqEmu, true),
            (ServerProtocol::Project1999, false),
            (ServerProtocol::Quarm, false),
            (ServerProtocol::Takp, false),
            (ServerProtocol::EqEmuRof2, false),
        ] {
            let (link, _, connects) = link(Some(Stub::Stalls));
            let server = super::super::servers::server_type(protocol);
            assert_eq!(
                offered(server, &link, &offer(), false),
                opens,
                "{protocol:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
            assert_eq!(
                *connects.lock().unwrap(),
                usize::from(opens),
                "{protocol:?}"
            );
            assert_eq!(link.opened(), opens);
            // The mail offer and a malformed one open nothing.
            link.close();
            let mail = format!("chat.example.test,7778,testworld.Tester,b{KEY}\0");
            assert!(!offered(server, &link, mail.as_bytes(), false));
            assert!(!offered(server, &link, b"garbled\0", false));
            assert!(!link.opened());
        }
    }

    #[test]
    fn the_zone_refuses_commands_without_an_up_link_and_tells_what_it_heard() {
        let (link, _, _) = link(Some(Stub::Takes));
        let mut channels = Channels::new(Arc::clone(&link));
        let mut world = World::new(5);
        let created = Instant::now();
        let command = |session_id| ClientCommand::ChatChannel {
            session_id,
            text: "list".into(),
            created,
        };
        for (session_id, reason) in [
            (4, "That was meant for an earlier zone visit"),
            (5, "There is no chat server link"),
        ] {
            let outcome =
                testing::run(|out| channels.handle(&command(session_id), &mut world, out));
            outcome.result.unwrap();
            assert!(outcome.events.iter().any(|event| matches!(
                event,
                ClientEvent::World(WorldEvent::ChatChannelRefused { reason: told, .. }) if told == reason
            )));
        }
        link.open(login(), false);
        // Wait without draining: the zone's tick is what drains.
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline
            && link
                .running
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|running| running.inbound.lock().unwrap().news.len() < 2)
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        let outcome = testing::run(|out| channels.tick(Instant::now(), &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::ChannelLink(ChannelLink::Up))
        )));
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Channel(ChannelEvent::Message { .. }))
        )));
        let outcome = testing::run(|out| channels.handle(&command(5), &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.events.iter().all(|event| !matches!(
            event,
            ClientEvent::World(WorldEvent::ChatChannelRefused { .. })
        )));
        link.close();
        // A lost or closed link refuses again, and keeps nothing for later.
        let outcome = testing::run(|out| channels.handle(&command(5), &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::ChatChannelRefused { .. })
        )));
    }
}
