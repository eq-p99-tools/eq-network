//! Opt-in replies to server-driven P99 presence polls.
//!
//! The 3..=60 second delay is application policy, not evidence of native
//! timing, human behavior, server acceptance or detection properties.

use super::{actions, servers::ServerType, ClientCommand, Events};
use anyhow::Result;
use eq_network_game::list_poll::{self, Poll};
use rand::Rng;
use std::{
    cell::Cell,
    time::{Duration, Instant},
};

const SHORTEST_REPLY: u64 = 3;
const LONGEST_REPLY: u64 = 60;

/// Per-zone reply state. Dropping it cancels the owed reply.
#[derive(Default)]
pub(super) struct Replies {
    due: Option<Instant>,
}

impl Replies {
    /// Applies the runtime preference before ordinary lifecycle/action gates.
    pub(super) fn configure(
        &mut self,
        server: &dyn ServerType,
        command: &ClientCommand,
        session_id: u64,
        enabled: &Cell<bool>,
        log: &mut Events<'_>,
    ) -> Result<bool> {
        let ClientCommand::AnswerListPolls {
            session_id: requested,
            enabled: choice,
        } = command
        else {
            return Ok(false);
        };
        if *requested != session_id {
            actions::refuse(
                command,
                "The command belongs to another zone admission",
                log,
            )?;
        } else if !server.answers_list_polls() {
            actions::refuse(command, "Not available on this server", log)?;
        } else {
            enabled.set(*choice);
            if !choice {
                self.due = None;
            }
        }
        Ok(true)
    }

    /// Decodes messages even when automatic replies are disabled. A population
    /// notice never joins a list or owes a presence reply.
    pub(super) fn observe(
        &mut self,
        server: &dyn ServerType,
        allowed: bool,
        opcode: u16,
        body: &[u8],
        now: Instant,
        log: &mut Events<'_>,
    ) -> Result<bool> {
        if opcode != list_poll::OPCODE || !server.answers_list_polls() {
            return Ok(false);
        }
        self.due = None;
        match list_poll::decode(body) {
            Some(Poll::Population { players }) => log.diagnostic(format!(
                "/list population: {players}; joining requires an explicit player action."
            ))?,
            Some(Poll::Presence) => {
                log.diagnostic("Received a /list presence poll.".into())?;
                if allowed {
                    let wait = rand::thread_rng().gen_range(SHORTEST_REPLY..=LONGEST_REPLY);
                    self.due = Some(now + Duration::from_secs(wait));
                }
            }
            None => log.diagnostic(format!(
                "Unrecognized /list message ({} bytes); no reply scheduled.",
                body.len()
            ))?,
            Some(_) => {}
        }
        Ok(true)
    }

    /// Sends at most once; disabling, death or transfer cancels the owed reply.
    pub(super) fn tick(
        &mut self,
        now: Instant,
        allowed: bool,
        session: &mut dyn eq_network_transport::Transport,
        log: &mut Events<'_>,
    ) -> Result<()> {
        if !allowed {
            self.due = None;
        }
        if self.due.is_some_and(|due| due <= now) {
            self.due = None;
            let reply = list_poll::present();
            session.send(reply.opcode, &reply.body)?;
            log.diagnostic("Sent a /list presence reply; server acceptance is unverified.".into())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ClientConfig, ClientEvent, ServerProtocol};
    use eq_network_transport::{Application, Transport};

    #[derive(Default)]
    struct Sent(Vec<(u16, Vec<u8>)>);
    impl Transport for Sent {
        fn send(&mut self, opcode: u16, body: &[u8]) -> Result<()> {
            self.0.push((opcode, body.to_vec()));
            Ok(())
        }
        fn receive(&mut self) -> Result<Option<Application>> {
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
    fn only_opted_in_presence_polls_owe_one_delayed_reply() {
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "Test", "Tester");
        assert!(!config.answer_list_polls);
        let mut events = Vec::new();
        let mut handler = |event| {
            events.push(event);
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        let p99 = super::super::servers::server_type(ServerProtocol::Project1999);
        let mut replies = Replies::default();
        let now = Instant::now();
        let mut sent = Sent::default();
        replies
            .observe(p99, false, 0x1253, &[2], now, &mut log)
            .unwrap();
        assert!(replies.due.is_none());
        // Population numbers never become a captcha, join or presence reply.
        replies
            .observe(p99, true, 0x1253, &[1, 57, 0, 0, 0], now, &mut log)
            .unwrap();
        assert!(replies.due.is_none());
        replies
            .observe(p99, true, 0x1253, &[2], now, &mut log)
            .unwrap();
        let due = replies.due.unwrap();
        assert!(
            (Duration::from_secs(3)..=Duration::from_secs(60)).contains(&due.duration_since(now))
        );
        replies
            .tick(
                due.checked_sub(Duration::from_nanos(1)).unwrap(),
                true,
                &mut sent,
                &mut log,
            )
            .unwrap();
        assert_eq!(sent.0, []);
        replies.tick(due, true, &mut sent, &mut log).unwrap();
        replies
            .tick(due + Duration::from_secs(600), true, &mut sent, &mut log)
            .unwrap();
        assert_eq!(sent.0, [(0x1253, vec![3, 0, 0, 0, 0])]);
        assert!(events
            .iter()
            .all(|event| matches!(event, ClientEvent::Diagnostic(_))));
    }

    #[test]
    fn replacement_disable_and_lifecycle_changes_cancel_old_answers() {
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "Test", "Tester");
        let mut handler = |_| Ok(());
        let mut log = Events::new(&config, &mut handler);
        let p99 = super::super::servers::server_type(ServerProtocol::Project1999);
        let eqemu = super::super::servers::server_type(ServerProtocol::EqEmu);
        let mut replies = Replies::default();
        let enabled = Cell::new(true);
        let now = Instant::now();
        let mut sent = Sent::default();
        for body in [&[1, 0, 0, 0, 0][..], &[1], &[255]] {
            replies
                .observe(p99, true, 0x1253, &[2], now, &mut log)
                .unwrap();
            replies
                .observe(p99, true, 0x1253, body, now, &mut log)
                .unwrap();
            assert!(replies.due.is_none());
        }
        replies
            .observe(p99, true, 0x1253, &[2], now, &mut log)
            .unwrap();
        let later = now + Duration::from_secs(100);
        replies
            .observe(p99, true, 0x1253, &[2], later, &mut log)
            .unwrap();
        assert!(replies.due.unwrap() >= later + Duration::from_secs(3));
        let disable = ClientCommand::AnswerListPolls {
            session_id: 7,
            enabled: false,
        };
        replies
            .configure(p99, &disable, 8, &enabled, &mut log)
            .unwrap();
        assert!(
            enabled.get(),
            "a stale command cannot change another admission"
        );
        replies
            .configure(p99, &disable, 7, &enabled, &mut log)
            .unwrap();
        assert!(!enabled.get());
        assert!(replies.due.is_none());
        let enable = ClientCommand::AnswerListPolls {
            session_id: 7,
            enabled: true,
        };
        replies
            .configure(eqemu, &enable, 7, &enabled, &mut log)
            .unwrap();
        assert!(!enabled.get());
        replies
            .configure(p99, &enable, 7, &enabled, &mut log)
            .unwrap();
        assert!(enabled.get());
        assert!(replies.due.is_none(), "enabling never answers an old poll");
        replies
            .observe(p99, true, 0x1253, &[2], now, &mut log)
            .unwrap();
        replies
            .tick(now + Duration::from_secs(600), false, &mut sent, &mut log)
            .unwrap();
        assert!(replies.due.is_none());
        assert_eq!(sent.0, []);
        assert!(!replies
            .observe(eqemu, true, 0x1253, &[2], now, &mut log)
            .unwrap());
        assert!(!replies
            .observe(p99, true, 0x1252, &[2], now, &mut log)
            .unwrap());
    }
}
