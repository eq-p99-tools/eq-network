//! Who is online: `/who all` asks the world, whose answer reaches the host
//! as news (`WorldEvent::WhoList`). A plain `/who` lists the zone's players
//! from what the host already knows, as the Titanium client does.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::{message::Message, request::Request, world::WorldEvent};
use std::time::{Duration, Instant};

/// Asks the world who is online.
pub(super) struct Who;

impl Feature for Who {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Who]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::WhoAll { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::WhoAll { filter, .. } = command else {
            return Ok(());
        };
        match out.encode(&Request::Who(filter.clone())) {
            Ok(request) => out.send(&request),
            // The host checks the text first; a request it let through
            // anyway is not worth the session.
            Err(error) => out.log.diagnostic(format!("Who request not sent: {error}")),
        }
    }
}

/// Asks the world which of the player's friends are online: `/who all
/// friends`, whose answer comes as an ordinary who listing.
pub(super) struct FriendsWho;

impl Feature for FriendsWho {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::FriendsWho]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::WhoFriends { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::WhoFriends { names, .. } = command else {
            return Ok(());
        };
        match out.encode(&Request::WhoFriends(names.clone())) {
            Ok(request) => out.send(&request),
            // The host keeps the list; one it let through that cannot be
            // sent is not worth the session.
            Err(error) => out
                .log
                .diagnostic(format!("Friends who request not sent: {error}")),
        }
    }
}

/// How long a who query waits for its answer before another may be asked
/// (6.583, ours).
const ANSWER_WAIT: Duration = Duration::from_secs(5);
/// The least time between two who queries (6.583, ours): Titanium's who has
/// no pace of its own to be slower than. The `EQMac` program's own pace is
/// unread (row 6.583a).
const QUERY_FLOOR: Duration = Duration::from_secs(1);

/// `/who all` and friends on an `EQMac` server, one query at a time
/// (6.583). Neither Quarm nor TAKP paces who queries itself, so the pace is
/// the session's own: a query waits for the last one's answer (the who
/// list, or the zone's string 5029 that no player matched) or
/// [`ANSWER_WAIT`], and comes no sooner than [`QUERY_FLOOR`] after it. One
/// asked sooner is refused, and can be asked again.
#[derive(Default)]
pub(super) struct PacedWho {
    /// When the last query was sent, and whether its answer is still to
    /// come.
    last: Option<(Instant, bool)>,
}

impl PacedWho {
    /// [`Feature::handle`] at a given time.
    fn handle_at(
        &mut self,
        command: &ClientCommand,
        out: &mut Out<'_, '_>,
        now: Instant,
    ) -> Result<()> {
        let request = match command {
            ClientCommand::WhoAll { filter, .. } => Request::Who(filter.clone()),
            ClientCommand::WhoFriends { names, .. } => Request::WhoFriends(names.clone()),
            _ => return Ok(()),
        };
        if let Some((sent, awaiting)) = self.last {
            let since = now.saturating_duration_since(sent);
            if awaiting && since < ANSWER_WAIT {
                return actions::refuse(command, "Wait for the last who answer.", out.log);
            }
            if since < QUERY_FLOOR {
                return actions::refuse(command, "Wait a moment before asking again.", out.log);
            }
        }
        match out.encode(&request) {
            Ok(packet) => {
                out.send(&packet)?;
                self.last = Some((now, true));
                Ok(())
            }
            Err(error) => actions::refuse(
                command,
                &format!("That who query cannot be sent: {error}"),
                out.log,
            ),
        }
    }
}

impl Feature for PacedWho {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![
            crate::world::Capability::Who,
            crate::world::Capability::FriendsWho,
        ]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::WhoAll { .. } | ClientCommand::WhoFriends { .. }
        )
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.handle_at(command, out, Instant::now())
    }

    /// The answer, or the zone's word that nobody matched, ends the wait.
    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if matches!(
            message,
            Message::Event(WorldEvent::WhoList(_)) | Message::WhoUnmatched
        ) {
            if let Some((_, awaiting)) = self.last.as_mut() {
                *awaiting = false;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::who;

    #[test]
    fn who_all_asks_the_world_with_the_filter() {
        let mut world = World::new(5);
        let ask = |text: &str| ClientCommand::WhoAll {
            session_id: 5,
            filter: who::WhoFilter {
                text: text.into(),
                ..who::WhoFilter::default()
            },
        };
        let outcome = testing::run(|out| Who.handle(&ask("qeynos"), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [who::request(&who::WhoFilter {
                text: "qeynos".into(),
                ..who::WhoFilter::default()
            })
            .unwrap()]
        );
        let outcome = testing::run(|out| Who.handle(&ask(&"x".repeat(64)), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
    }

    #[test]
    fn friends_who_sends_the_list_and_drops_one_it_cannot_carry() {
        let mut world = World::new(5);
        let ask = |names: &[&str]| ClientCommand::WhoFriends {
            session_id: 5,
            names: names.iter().map(|name| (*name).to_owned()).collect(),
        };
        let outcome =
            testing::run(|out| FriendsWho.handle(&ask(&["Alpha", "Beta"]), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [
                eq_network_game::friends::titanium_request(&["Alpha".into(), "Beta".into()])
                    .unwrap()
            ]
        );
        let outcome = testing::run(|out| FriendsWho.handle(&ask(&[]), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
    }

    #[test]
    fn eqmac_who_asks_one_query_at_a_time() {
        let wire = &super::super::wire::EqMac;
        let mut who = PacedWho::default();
        let start = Instant::now();
        let at = |millis| start + Duration::from_millis(millis);
        let all = ClientCommand::WhoAll {
            session_id: 5,
            filter: who::WhoFilter::default(),
        };
        let friends = ClientCommand::WhoFriends {
            session_id: 5,
            names: vec!["Alpha".into()],
        };
        let ask = |who: &mut PacedWho, command: &ClientCommand, millis| {
            let outcome = testing::run_on(wire, |out| who.handle_at(command, out, at(millis)));
            outcome.result.unwrap();
            outcome.sent
        };
        assert_eq!(
            ask(&mut who, &all, 0),
            [who::eqmac::request(&who::WhoFilter::default()).unwrap()]
        );
        // Unanswered: friends waits too, up to five seconds.
        assert_eq!(ask(&mut who, &friends, 2_000), []);
        assert_eq!(ask(&mut who, &all, 4_999), []);
        assert_eq!(ask(&mut who, &friends, 5_000).len(), 1);
        // Answered at once: still a second after the last.
        let mut world = World::new(5);
        let answered = |who: &mut PacedWho, world: &mut World, message: &Message| {
            testing::run_on(wire, |out| who.observe(message, world, out))
                .result
                .unwrap();
        };
        answered(&mut who, &mut world, &Message::WhoUnmatched);
        assert_eq!(ask(&mut who, &all, 5_500), []);
        assert_eq!(ask(&mut who, &all, 6_000).len(), 1);
        let list = Message::Event(WorldEvent::WhoList(who::WhoList {
            heading: 5001,
            rule: String::new(),
            closing: 5028,
            count: 0,
            players: Vec::new(),
        }));
        answered(&mut who, &mut world, &list);
        assert_eq!(
            ask(&mut who, &friends, 7_000),
            [eq_network_game::friends::eqmac_request(&["Alpha".into()]).unwrap()]
        );
        // One that cannot be sent is refused and starts no wait.
        let mut who = PacedWho::default();
        let long = ClientCommand::WhoFriends {
            session_id: 5,
            names: vec!["x".repeat(65)],
        };
        let outcome = testing::run_on(wire, |out| who.handle_at(&long, out, start));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert!(!outcome.events.is_empty());
        assert!(who.last.is_none());
    }

    /// Quarm's and TAKP's answer reads as the who list, the zone's 5029 as
    /// the query answered, and an answer that doesn't read as a gap that
    /// keeps no sample of the players' names (6.583).
    #[test]
    fn eqmac_answers_reach_the_pace_and_a_bad_one_keeps_no_sample() {
        use super::super::coverage::{Stage, Unheard};
        use super::super::wire::Wire;
        use crate::client::{ClientConfig, ClientEvent, Gap, GapStage};
        let wire: &dyn Wire = &super::super::wire::EqMac;
        let rules = eq_network_game::inventory::ItemRules::Quarm;
        let mut heading = vec![0; 58];
        heading[4..6].copy_from_slice(&5001u16.to_le_bytes());
        heading[36..38].copy_from_slice(&5028u16.to_le_bytes());
        assert!(matches!(
            &wire.messages(who::eqmac::RESPONSE_OPCODE, &heading, rules)[..],
            [Message::Event(WorldEvent::WhoList(list))] if list.players.is_empty()
        ));
        let mut none = vec![0, 0];
        none.extend_from_slice(&5029u16.to_le_bytes());
        none.extend_from_slice(&[0; 8]);
        assert!(wire
            .messages(who::eqmac::FORMATTED_OPCODE, &none, rules)
            .iter()
            .any(|message| matches!(message, Message::WhoUnmatched)));
        // A player cut off inside the name.
        let mut cut = heading.clone();
        cut[56] = 1;
        cut.extend_from_slice(&[0x99, 0x13, 0xff, 0xff, b'S', b'e', b'c']);
        let messages = wire.messages(who::eqmac::RESPONSE_OPCODE, &cut, rules);
        assert!(matches!(&messages[..], [Message::Unreadable { .. }]));
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "Test Server", "Tester");
        let mut gaps: Vec<Gap> = Vec::new();
        let mut handler = |event| {
            if let ClientEvent::Gap(gap) = event {
                gaps.push(*gap);
            }
            Ok(())
        };
        let mut log = crate::client::session::Events::new(&config, &mut handler);
        log.begin_stage(GapStage::Zone).unwrap();
        Unheard::new(Stage::Zone)
            .sift(
                wire,
                (who::eqmac::RESPONSE_OPCODE, &cut),
                messages,
                &mut log,
            )
            .unwrap();
        log.end_stage().unwrap();
        drop(log);
        assert_eq!(gaps.len(), 1);
        assert!(gaps[0].sample.is_none());
        assert_eq!(gaps[0].kind.key(), "unreadable:World update");
    }
}
