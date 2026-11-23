//! The Bazaar's search on stock `EQEmu` (6.550): the player's search and
//! welcome request, and the server's answers, read only. Nothing here buys,
//! browses a trader or turns trader mode on.
//!
//! `EQEmu` searches the traders of the player's own zone and instance
//! (`Bazaar::GetSearchResults`, `Local_Scope`), so a search outside the
//! Bazaar finds nothing; where the official client lets the player search
//! is not read yet (R49-42), so the session sends one only in the Bazaar.
//! One search or welcome request is in flight at a time, and the next waits
//! out a floor after the last was sent (ours, inferred), since each costs
//! the server a database search. An answer nobody asked for is withheld and
//! told as a gap, by its opcode and length alone: its rows list other
//! players' sales.
//!
//! Built, and not yet offered: stock `EQEmu` offers it once the local live
//! row passes (6.550a).
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "offered on stock EQEmu once row 6.550a passes")
)]
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::{
    bazaar::{self, BazaarEvent},
    message::Message,
    request::Request,
    world::WorldEvent,
};
use std::time::{Duration, Instant};

/// How long a search or welcome request waits for its answer before
/// another may go (inferred: `EQEmu` always ends a search, so this only
/// frees one whose end was lost).
const WAIT: Duration = Duration::from_secs(10);
/// The least time between one search or welcome request and the next
/// (ours, inferred).
const FLOOR: Duration = Duration::from_secs(2);

/// What the session is waiting to hear.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Asked {
    Search,
    Welcome,
}

/// The Bazaar's search and welcome request.
#[derive(Default)]
pub(super) struct Bazaar {
    /// The request in flight, and when it went.
    asked: Option<(Asked, Instant)>,
    /// When the last request went.
    sent: Option<Instant>,
    /// The length of each answer nobody asked for, to tell as gaps.
    unasked: Vec<usize>,
}

impl Bazaar {
    fn waiting(&self, now: Instant) -> Option<Asked> {
        self.asked
            .filter(|(_, at)| now.saturating_duration_since(*at) < WAIT)
            .map(|(asked, _)| asked)
    }
}

impl Feature for Bazaar {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::BazaarSearching]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::BazaarSearch { .. } | ClientCommand::BazaarWelcome { .. }
        )
    }

    /// A new admission forgets any request in flight.
    fn admitted(&mut self, _world: &mut World, _out: &mut Out<'_, '_>) -> Result<()> {
        self.asked = None;
        Ok(())
    }

    fn explain(&mut self, message: &mut Message, _world: &World) {
        let Message::Event(WorldEvent::Bazaar(event)) = message else {
            if matches!(
                message,
                Message::ZoneOffer(_) | Message::ZoneAnswer(_) | Message::Handoff(_)
            ) {
                self.asked = None;
            }
            return;
        };
        let waiting = self.waiting(Instant::now());
        match (event, waiting) {
            (BazaarEvent::Results(_), Some(Asked::Search)) => {}
            (BazaarEvent::SearchDone, Some(Asked::Search))
            | (BazaarEvent::Welcome { .. }, Some(Asked::Welcome)) => self.asked = None,
            (event, _) => {
                self.unasked.push(length(event));
                *message = Message::Withheld;
            }
        }
    }

    fn tick(&mut self, _now: Instant, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        for length in self.unasked.drain(..) {
            out.log.gap_unsampled(
                bazaar::SEARCH_OPCODE,
                Some("OP_BazaarSearch"),
                length,
                crate::client::GapKind::Unread {
                    reason: "a bazaar answer no request asked for".to_owned(),
                },
            )?;
        }
        Ok(())
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let now = Instant::now();
        let (request, asked) = match command {
            ClientCommand::BazaarSearch { criteria, .. } => {
                (Request::BazaarSearch(criteria.clone()), Asked::Search)
            }
            ClientCommand::BazaarWelcome { .. } => (Request::BazaarWelcome, Asked::Welcome),
            _ => return Ok(()),
        };
        if world.zone.0 != bazaar::ZONE_ID {
            return actions::refuse(command, "You can search only in the Bazaar.", out.log);
        }
        if self.waiting(now).is_some() {
            return actions::refuse(command, "Wait for the last search to finish.", out.log);
        }
        if self
            .sent
            .is_some_and(|sent| now.saturating_duration_since(sent) < FLOOR)
        {
            return actions::refuse(command, "Wait a moment before searching again.", out.log);
        }
        if let Err(error) = out.request(&request) {
            return actions::refuse(command, &format!("Cannot search: {error:#}."), out.log);
        }
        self.asked = Some((asked, now));
        self.sent = Some(now);
        Ok(())
    }
}

/// The length of the record an answer came in, as [`bazaar::answer`] reads
/// it.
fn length(event: &BazaarEvent) -> usize {
    match event {
        BazaarEvent::Results(rows) => rows.len() * bazaar::ROW_LENGTH,
        BazaarEvent::SearchDone | BazaarEvent::Welcome { .. } => bazaar::WELCOME_LENGTH,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;
    use eq_network_game::bazaar::{BazaarCriteria, BazaarListing};

    fn in_bazaar() -> World {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.zone = (bazaar::ZONE_ID, 0);
        world
    }

    fn search() -> ClientCommand {
        ClientCommand::BazaarSearch {
            session_id: 5,
            criteria: BazaarCriteria {
                name: "Bone Chips".into(),
                ..BazaarCriteria::default()
            },
        }
    }

    fn refused(outcome: &testing::Outcome<Result<()>>) -> Option<String> {
        if !outcome.sent.is_empty() {
            return None;
        }
        outcome.events.iter().find_map(|event| match event {
            ClientEvent::World(WorldEvent::BazaarRefused {
                session_id: 5,
                reason,
            }) => Some(reason.clone()),
            _ => None,
        })
    }

    fn results() -> Message {
        Message::Event(WorldEvent::Bazaar(BazaarEvent::Results(vec![
            BazaarListing {
                trader: 312,
                item_id: 1001,
                serial: 77,
                name: "Bone Chips".into(),
                count: Some(20),
                price: 1250,
                stat: 0,
            },
        ])))
    }

    fn done() -> Message {
        Message::Event(WorldEvent::Bazaar(BazaarEvent::SearchDone))
    }

    #[test]
    fn a_search_goes_only_in_the_bazaar() {
        let mut world = in_bazaar();
        world.zone = (202, 0);
        let mut feature = Bazaar::default();
        let outcome = testing::run(|out| feature.handle(&search(), &mut world, out));
        assert_eq!(
            refused(&outcome).as_deref(),
            Some("You can search only in the Bazaar.")
        );
        assert!(feature.asked.is_none() && feature.sent.is_none());
    }

    #[test]
    fn a_search_goes_as_built_and_its_answer_passes() {
        let mut world = in_bazaar();
        let mut feature = Bazaar::default();
        let outcome = testing::run(|out| feature.handle(&search(), &mut world, out));
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, bazaar::SEARCH_OPCODE);
        let ClientCommand::BazaarSearch { criteria, .. } = search() else {
            unreachable!()
        };
        assert_eq!(outcome.sent[0].body, bazaar::search(&criteria).unwrap());
        let mut rows = results();
        feature.explain(&mut rows, &world);
        assert!(matches!(rows, Message::Event(_)));
        // Results alone keep the search in flight; its end frees it.
        assert_eq!(feature.waiting(Instant::now()), Some(Asked::Search));
        let mut end = done();
        feature.explain(&mut end, &world);
        assert!(matches!(end, Message::Event(_)));
        assert!(feature.asked.is_none());
    }

    #[test]
    fn one_search_at_a_time_with_a_floor_between() {
        let mut world = in_bazaar();
        let mut feature = Bazaar::default();
        assert_eq!(
            testing::run(|out| feature.handle(&search(), &mut world, out))
                .sent
                .len(),
            1
        );
        // In flight.
        assert_eq!(
            refused(&testing::run(|out| feature.handle(
                &search(),
                &mut world,
                out
            )))
            .as_deref(),
            Some("Wait for the last search to finish.")
        );
        // Ended, but too soon after the last.
        feature.explain(&mut done(), &world);
        assert_eq!(
            refused(&testing::run(|out| feature.handle(
                &search(),
                &mut world,
                out
            )))
            .as_deref(),
            Some("Wait a moment before searching again.")
        );
        let welcome = ClientCommand::BazaarWelcome { session_id: 5 };
        assert!(refused(&testing::run(
            |out| feature.handle(&welcome, &mut world, out)
        ))
        .is_some());
        // Past the floor, the next goes.
        feature.sent = Instant::now().checked_sub(FLOOR);
        let outcome = testing::run(|out| feature.handle(&welcome, &mut world, out));
        assert_eq!(
            (outcome.sent[0].opcode, &outcome.sent[0].body),
            (bazaar::SEARCH_OPCODE, &bazaar::welcome())
        );
        // A lost end frees the search after the wait.
        let mut hello = Message::Event(WorldEvent::Bazaar(BazaarEvent::Welcome {
            traders: 2,
            items: 9,
        }));
        feature.explain(&mut hello, &world);
        assert!(matches!(hello, Message::Event(_)) && feature.asked.is_none());
        feature.sent = Instant::now().checked_sub(FLOOR);
        testing::run(|out| feature.handle(&search(), &mut world, out));
        feature.sent = Instant::now().checked_sub(WAIT);
        feature.asked = feature
            .asked
            .map(|(asked, _)| (asked, feature.sent.unwrap()));
        assert_eq!(
            testing::run(|out| feature.handle(&search(), &mut world, out))
                .sent
                .len(),
            1
        );
    }

    #[test]
    fn a_search_the_codec_refuses_is_refused_in_our_words() {
        let mut world = in_bazaar();
        let mut feature = Bazaar::default();
        let long = ClientCommand::BazaarSearch {
            session_id: 5,
            criteria: BazaarCriteria {
                name: "a".repeat(64),
                ..BazaarCriteria::default()
            },
        };
        let outcome = testing::run(|out| feature.handle(&long, &mut world, out));
        assert!(refused(&outcome).is_some_and(|reason| reason.starts_with("Cannot search")));
        assert!(feature.asked.is_none() && feature.sent.is_none());
    }

    #[test]
    fn an_answer_nobody_asked_for_is_withheld_and_a_gap_without_its_body() {
        let mut world = in_bazaar();
        let mut feature = Bazaar::default();
        let mut rows = results();
        feature.explain(&mut rows, &world);
        assert!(matches!(rows, Message::Withheld));
        // A welcome while a search is in flight is no answer to it.
        testing::run(|out| feature.handle(&search(), &mut world, out));
        let mut hello = Message::Event(WorldEvent::Bazaar(BazaarEvent::Welcome {
            traders: 2,
            items: 9,
        }));
        feature.explain(&mut hello, &world);
        assert!(matches!(hello, Message::Withheld));
        let told = testing::run(|out| feature.tick(Instant::now(), &mut world, out));
        let gaps: Vec<_> = told
            .events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::Gap(gap) => Some(gap),
                _ => None,
            })
            .collect();
        // The first is told at once; the second, of the same kind, is
        // counted as its repeat.
        assert_eq!(gaps.len(), 1);
        assert_eq!(feature.unasked, Vec::<usize>::new());
        assert!(gaps
            .iter()
            .all(|gap| gap.sample.is_none() && gap.opcode == bazaar::SEARCH_OPCODE));
        assert_eq!(gaps[0].min_length, bazaar::ROW_LENGTH);
    }

    #[test]
    fn zoning_or_a_new_admission_forgets_the_search_in_flight() {
        let mut world = in_bazaar();
        let mut feature = Bazaar::default();
        testing::run(|out| feature.handle(&search(), &mut world, out));
        testing::run(|out| feature.admitted(&mut world, out));
        assert!(feature.asked.is_none());
        let mut end = done();
        feature.explain(&mut end, &world);
        assert!(matches!(end, Message::Withheld));
    }
}
