//! Single-flight name suggestions. The wire has no request identity, so a
//! timeout retains its slot until the late reply is consumed. Manual creation
//! and entry remain available while a suggestion is pending.
use super::super::{ClientEvent, Events};
use anyhow::Result;
use eq_network_game::{creation::names, world::WorldEvent};
use eq_network_transport::Transport;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(in crate::client::session) struct Suggestions(Option<Pending>);

struct Pending {
    selection: u64,
    request: u64,
    race: u32,
    gender: u32,
    started: Instant,
    expired: bool,
}

impl Suggestions {
    pub fn request(
        &mut self,
        (selection, request, race, gender): (u64, u64, u32, u32),
        session: &mut dyn Transport,
        log: &mut Events<'_>,
    ) -> Result<()> {
        if self.0.is_some() {
            return answer(selection, request, None, log);
        }
        let Ok(packet) = names::request(race, gender) else {
            return answer(selection, request, None, log);
        };
        session.send(packet.opcode, &packet.body)?;
        self.0 = Some(Pending {
            selection,
            request,
            race,
            gender,
            started: Instant::now(),
            expired: false,
        });
        Ok(())
    }

    pub fn receive(&mut self, body: &[u8], log: &mut Events<'_>) -> Result<()> {
        if let Some(pending) = self.0.take() {
            if !pending.expired {
                let name = names::response(body, pending.race, pending.gender).ok();
                answer(pending.selection, pending.request, name, log)?;
            }
        }
        Ok(())
    }

    pub fn tick(&mut self, log: &mut Events<'_>) -> Result<()> {
        if let Some(pending) = &mut self.0 {
            if !pending.expired && pending.started.elapsed() >= Duration::from_secs(15) {
                pending.expired = true;
                answer(pending.selection, pending.request, None, log)?;
            }
        }
        Ok(())
    }
}

pub(in crate::client::session) fn answer(
    selection_id: u64,
    request_id: u64,
    name: Option<String>,
    log: &mut Events<'_>,
) -> Result<()> {
    log.send(ClientEvent::World(WorldEvent::NameSuggestion {
        selection_id,
        request_id,
        name,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ClientConfig;

    #[derive(Default)]
    struct Sent(Vec<(u16, Vec<u8>)>);
    impl Transport for Sent {
        fn send(&mut self, opcode: u16, body: &[u8]) -> Result<()> {
            self.0.push((opcode, body.into()));
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
    fn late_and_unsolicited_answers_cannot_complete_another_request() {
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "Test Server", "");
        let mut heard = Vec::new();
        let mut handler = |event| {
            heard.push(event);
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        let mut wire = Sent::default();
        let mut suggestions = Suggestions::default();
        suggestions
            .request((7, 10, 1, 0), &mut wire, &mut log)
            .unwrap();
        suggestions
            .request((7, 11, 1, 0), &mut wire, &mut log)
            .unwrap();
        suggestions.0.as_mut().unwrap().started =
            Instant::now().checked_sub(Duration::from_secs(16)).unwrap();
        suggestions.tick(&mut log).unwrap();
        suggestions.tick(&mut log).unwrap();
        suggestions
            .request((8, 12, 1, 0), &mut wire, &mut log)
            .unwrap();
        assert_eq!(wire.0.len(), 1);
        let mut reply = wire.0[0].1.clone();
        reply[8..16].copy_from_slice(b"Testhero");
        suggestions.receive(&reply, &mut log).unwrap();
        suggestions.receive(&reply, &mut log).unwrap();
        suggestions
            .request((8, 13, 1, 0), &mut wire, &mut log)
            .unwrap();
        suggestions.receive(&reply, &mut log).unwrap();
        drop(log);
        let answers: Vec<_> = heard
            .into_iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::NameSuggestion {
                    selection_id,
                    request_id,
                    name,
                }) => Some((selection_id, request_id, name)),
                _ => None,
            })
            .collect();
        assert_eq!(
            answers,
            [
                (7, 11, None),
                (7, 10, None),
                (8, 12, None),
                (8, 13, Some("Testhero".into()))
            ]
        );
        assert_eq!(wire.0.len(), 2);
        assert!(wire
            .0
            .iter()
            .all(|(opcode, body)| *opcode == names::OPCODE && body.len() == 72));
    }
}
