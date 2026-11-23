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
//! Once the login is taken, the link checks every 5 s how long the server
//! has been silent and ends after more than 30 s, as the Titanium program
//! does (read R49-12 a, checklist 6.616). [I] What counts as a word from the
//! server is unread, so anything the transport takes counts, its acks and
//! keepalives among them, which an idle healthy link still gets.
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
    time::{Duration, Instant},
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
    /// The thread has told the link lost, or ended.
    ended: bool,
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

/// How long a login may go unanswered before the link ends: this library's
/// own bound, inferred (the program's own is asked as R49-12 d).
pub(super) const LOGIN_WAIT: Duration = Duration::from_secs(30);

/// How often a logged-in link checks the server's silence (read R49-12 a:
/// every 5000 ms).
pub(super) const SILENCE_CHECK: Duration = Duration::from_secs(5);
/// The whole seconds of silence past which a logged-in link ends (read
/// R49-12 a: a value over 30000 ms).
pub(super) const SILENCE_LIMIT_SECS: u64 = 30;

/// A link while its thread runs.
struct Running {
    outbound: SyncSender<Vec<u8>>,
    inbound: Arc<Mutex<Inbound>>,
    stop: Arc<AtomicBool>,
    /// The thread is in its address and connect step, which a stop cannot
    /// cut short while the offered host resolves.
    connecting: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Running {
    fn close(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            // Past its connect step the thread checks its stop at least every
            // receive wait, so it is waited for. In the step it is left to end
            // on its own once the step returns, holding only the connector
            // and its queues, telling nothing (reviewer F2).
            if !self.connecting.load(Ordering::Acquire) {
                let _ = thread.join();
            }
        }
    }
}

/// The character's chat server link, kept across zones.
pub(in crate::client) struct ChatLink {
    connect: Arc<Connect>,
    /// How long a login may go unanswered.
    login_wait: Duration,
    /// How often a logged-in link checks the server's silence.
    silence_check: Duration,
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
            login_wait: LOGIN_WAIT,
            silence_check: SILENCE_CHECK,
            running: Mutex::new(None),
            left: Mutex::new(Vec::new()),
        }
    }

    /// The same link with another bound on an unanswered login.
    #[cfg(test)]
    pub(super) fn waiting(mut self, login_wait: Duration) -> Self {
        self.login_wait = login_wait;
        self
    }

    /// The same link checking the server's silence this often.
    #[cfg(test)]
    pub(super) fn checking(mut self, silence_check: Duration) -> Self {
        self.silence_check = silence_check;
        self
    }

    /// Opens a link from a login, closing any open one first: one connect
    /// per offer, never a retry (reviewer C2).
    pub(super) fn open(&self, login: Login, local_only: bool) {
        self.close();
        let (outbound, commands) = mpsc::sync_channel(OUTBOUND);
        let inbound = Arc::new(Mutex::new(Inbound::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let connecting = Arc::new(AtomicBool::new(true));
        let thread = {
            // The thread holds the connector, never the link, so dropping
            // the link closes it.
            let thread = Thread {
                connect: Arc::clone(&self.connect),
                login,
                local_only,
                login_wait: self.login_wait,
                silence_check: self.silence_check,
                commands,
                inbound: Arc::clone(&inbound),
                stop: Arc::clone(&stop),
                connecting: Arc::clone(&connecting),
            };
            std::thread::Builder::new()
                .name("chat-link".into())
                .spawn(move || {
                    let loss = run(&thread);
                    let mut inbound = thread
                        .inbound
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    inbound.up = false;
                    inbound.ended = true;
                    if let Some(reason) = loss {
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
            connecting,
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
        {
            let inbound = running
                .inbound
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if inbound.ended {
                return Err("The chat server link has ended");
            }
            if !inbound.up {
                return Err("The chat server has not taken the login");
            }
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

/// What the link's thread holds: the connector and its own queues, never
/// the link.
struct Thread {
    connect: Arc<Connect>,
    login: Login,
    local_only: bool,
    login_wait: Duration,
    silence_check: Duration,
    commands: Receiver<Vec<u8>>,
    inbound: Arc<Mutex<Inbound>>,
    stop: Arc<AtomicBool>,
    connecting: Arc<AtomicBool>,
}

/// The link's thread: connects, logs in, then carries commands out and what
/// the server says in until it is stopped or fails; why it failed, or None
/// when it was stopped.
fn run(thread: &Thread) -> Option<LinkLoss> {
    let Thread {
        connect,
        login,
        local_only,
        login_wait,
        silence_check,
        commands,
        inbound,
        stop,
        connecting,
    } = thread;
    let connected = connect(login, *local_only, stop);
    connecting.store(false, Ordering::Release);
    let mut transport = match connected {
        Ok(transport) => transport,
        Err(_) if stop.load(Ordering::Relaxed) => return None,
        Err(loss) => return Some(loss),
    };
    if stop.load(Ordering::Relaxed) {
        // Closed while it connected: nothing is sent, nothing told.
        let _ = transport.close();
        return None;
    }
    if transport
        .send(chat_server::LOGIN_OPCODE, &login.body())
        .is_err()
    {
        let _ = transport.close();
        return Some(LinkLoss::Login);
    }
    let asked = Instant::now();
    let mut up = false;
    let mut checked = Instant::now();
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
        if !up && asked.elapsed() >= *login_wait {
            // A server that keeps the connection but never takes the login
            // ends the link once (reviewer F1).
            let _ = transport.close();
            return Some(LinkLoss::Login);
        }
        if up && checked.elapsed() >= *silence_check {
            checked = Instant::now();
            if transport.last_received_seconds() > SILENCE_LIMIT_SECS {
                // The program's keep-alive check (read R49-12 a): a link
                // the server has gone silent on ends, once, with nothing
                // sent. Whether the program says anything then is unread.
                let _ = transport.close();
                return Some(LinkLoss::Dropped);
            }
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
    /// The character's one pace for all chat (6.523).
    pace: super::chat_pace::SharedPace,
}

impl Channels {
    pub(super) const fn new(link: Arc<ChatLink>, pace: super::chat_pace::SharedPace) -> Self {
        Self { link, pace }
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
        // Only what a host asked for is paced; the link's own login is not,
        // and a command the link refuses is not counted.
        match super::chat_pace::paced(
            &self.pace,
            super::chat_pace::Kind::of_channel(text),
            std::time::Instant::now(),
            || self.link.send(text),
        ) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(reason)) => super::actions::refuse(command, reason, out.log),
            Err(paced) => super::actions::refuse(command, &paced.reason(), out.log),
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
                News::Unread { opcode, length } => out.log.gap_unsampled(
                    opcode,
                    Some(unread_name(opcode)),
                    length,
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
        /// Takes the login, then says nothing for 31 whole seconds.
        Silent,
        /// Takes the login, then says nothing for 30 whole seconds.
        Quiet,
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
                    Stub::Takes
                    | Stub::Drops
                    | Stub::Floods
                    | Stub::Slow
                    | Stub::Silent
                    | Stub::Quiet => {
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
                    Stub::Stalls
                    | Stub::Rubbish
                    | Stub::Drops
                    | Stub::Slow
                    | Stub::Silent
                    | Stub::Quiet => {}
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
            match self.stub {
                Stub::Silent if self.logged_in => 31,
                Stub::Quiet if self.logged_in => 30,
                _ => 0,
            }
        }
    }

    type Sent = Arc<Mutex<Vec<(u16, Vec<u8>)>>>;

    /// What the fake records when the link closes it.
    const CLOSED: u16 = 0xffff;

    /// A link to a stub server, with what it sent and how often it
    /// connected.
    fn link(stub: Option<Stub>) -> (Arc<ChatLink>, Sent, Arc<Mutex<usize>>) {
        link_waiting(stub, LOGIN_WAIT)
    }

    fn link_waiting(
        stub: Option<Stub>,
        wait: Duration,
    ) -> (Arc<ChatLink>, Sent, Arc<Mutex<usize>>) {
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
        }))
        .waiting(wait);
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
    fn a_link_ends_once_after_more_than_30_seconds_of_silence_and_not_at_30() {
        let checking = |stub, every| {
            let (made, sent, connects) = link(Some(stub));
            let made = Arc::new(Arc::into_inner(made).unwrap().checking(every));
            (made, sent, connects)
        };
        // Past 30 whole seconds without a word, the check ends the link
        // once, closing it and sending nothing more (read R49-12 a).
        let (silent, sent, connects) = checking(Stub::Silent, Duration::from_millis(10));
        silent.open(login(), false);
        let news = until(&silent, |news| {
            news.iter()
                .any(|n| matches!(n, News::Link(ChannelLink::Lost { .. })))
        });
        assert_eq!(
            news,
            [
                News::Link(ChannelLink::Up),
                News::Link(ChannelLink::Lost {
                    reason: LinkLoss::Dropped
                })
            ]
        );
        std::thread::sleep(Duration::from_millis(20));
        assert!(!silent.running());
        let sent = sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 2, "the login and the close only");
        assert_eq!(sent[1].0, CLOSED);
        assert_eq!(*connects.lock().unwrap(), 1);
        // At 30 seconds it stays up, however often it checks.
        let (quiet, _, _) = checking(Stub::Quiet, Duration::from_millis(5));
        quiet.open(login(), false);
        until(&quiet, |news| news.contains(&News::Link(ChannelLink::Up)));
        std::thread::sleep(Duration::from_millis(60));
        assert!(quiet.running());
        assert_eq!(quiet.drain(), []);
        quiet.send("list").unwrap();
        quiet.close();
    }

    #[test]
    fn the_silence_check_runs_every_5_seconds_past_30() {
        assert_eq!(SILENCE_CHECK, Duration::from_secs(5));
        assert_eq!(SILENCE_LIMIT_SECS, 30);
    }

    #[test]
    fn an_unanswered_login_ends_once_past_its_bound_and_says_so() {
        let (link, sent, connects) = link_waiting(Some(Stub::Stalls), Duration::from_millis(40));
        link.open(login(), false);
        let news = until(&link, |news| {
            news.iter()
                .any(|n| matches!(n, News::Link(ChannelLink::Lost { .. })))
        });
        assert_eq!(
            news,
            [News::Link(ChannelLink::Lost {
                reason: LinkLoss::Login
            })]
        );
        std::thread::sleep(Duration::from_millis(20));
        assert!(!link.running());
        assert!(sent
            .lock()
            .unwrap()
            .iter()
            .any(|(opcode, _)| *opcode == CLOSED));
        // A lost link says it has ended, not that it is still coming up.
        assert_eq!(link.send("list"), Err("The chat server link has ended"));
        assert_eq!(*connects.lock().unwrap(), 1);
        assert_eq!(link.drain(), []);
    }

    #[test]
    fn closing_never_waits_on_a_connect_that_cannot_be_cut_short() {
        // A connector that ignores the stop until it is released, as the
        // offered host's resolution does.
        let (release, held) = mpsc::channel::<()>();
        let held = Mutex::new(held);
        let sent: Sent = Arc::default();
        let into = Arc::clone(&sent);
        let link = ChatLink::with(Arc::new(move |_login, _local, _stop| {
            let _ = held.lock().unwrap().recv();
            Ok(Box::new(Fake {
                stub: Stub::Takes,
                script: VecDeque::new(),
                sent: Arc::clone(&into),
                logged_in: false,
            }) as Box<dyn Transport + Send>)
        }));
        link.open(login(), false);
        std::thread::sleep(Duration::from_millis(20));
        let started = Instant::now();
        link.close();
        assert!(started.elapsed() < Duration::from_millis(200));
        assert!(!link.opened());
        release.send(()).unwrap();
        // After the release the thread closes what it connected, sends
        // nothing and tells nothing.
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && sent.lock().unwrap().is_empty() {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(*sent.lock().unwrap(), [(CLOSED, Vec::new())]);
        assert_eq!(link.drain(), []);
    }

    #[test]
    fn the_gap_log_keeps_an_unread_message_by_opcode_and_length_only() {
        let (link, _, _) = link(Some(Stub::Floods));
        let mut channels = Channels::new(
            Arc::clone(&link),
            super::super::chat_pace::SharedPace::default(),
        );
        let mut world = World::new(5);
        link.open(login(), false);
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut gaps = Vec::new();
        while Instant::now() < deadline && gaps.is_empty() {
            let outcome = testing::run(|out| channels.tick(Instant::now(), &mut world, out));
            outcome.result.unwrap();
            gaps.extend(outcome.events.into_iter().filter_map(|event| match event {
                ClientEvent::Gap(gap) => Some(gap),
                _ => None,
            }));
            std::thread::sleep(Duration::from_millis(5));
        }
        let [gap] = &gaps[..] else {
            panic!("one gap, not {gaps:?}");
        };
        assert_eq!(gap.opcode, chat_server::BUDDY_OPCODE);
        assert_eq!((gap.min_length, gap.max_length), (5, 5));
        assert_eq!(gap.sample, None);
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

    /// Only what a host asked for counts toward the pace for all chat, and
    /// only what the link took: refusals before the link is up and the
    /// link's own login do not (6.523, reviewer L2 and L4).
    #[test]
    fn the_pace_counts_only_what_a_host_asked_and_the_link_took() {
        let (link, _, _) = link(Some(Stub::Takes));
        let pace = super::super::chat_pace::SharedPace::default();
        let mut channels = Channels::new(Arc::clone(&link), Arc::clone(&pace));
        let mut world = World::new(5);
        let mut ask = |text: &str| {
            let command = ClientCommand::ChatChannel {
                session_id: 5,
                text: text.into(),
                created: Instant::now(),
            };
            let outcome = testing::run(|out| channels.handle(&command, &mut world, out));
            outcome.result.unwrap();
            outcome.events.into_iter().find_map(|event| match event {
                ClientEvent::World(WorldEvent::ChatChannelRefused { reason, .. }) => Some(reason),
                _ => None,
            })
        };
        for _ in 0..20 {
            assert_eq!(
                ask("2 early").as_deref(),
                Some("There is no chat server link")
            );
        }
        link.open(login(), false);
        until(&link, |news| news.contains(&News::Link(ChannelLink::Up)));
        for line in 0..super::super::chat_pace::ANY_CHAT {
            assert_eq!(ask("list"), None, "{line}");
        }
        let refused = ask("2 a synthetic private line").expect("paced");
        assert!(
            refused.starts_with("You are sending chat too fast."),
            "{refused}"
        );
        assert!(!refused.contains("private"));
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
            let mut channels = Channels::new(
                Arc::clone(&link),
                super::super::chat_pace::SharedPace::default(),
            );
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
        let mut channels = Channels::new(
            Arc::clone(&link),
            super::super::chat_pace::SharedPace::default(),
        );
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
