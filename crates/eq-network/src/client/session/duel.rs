//! A duel as the Titanium program keeps one (read R49-6 a, e and h:
//! handlers 0x453231, `/duel` at 0x472a67, `OP_Death` at 0x4a8cd5).
//!
//! The program has no challenge window: a challenge is a chat line, and the
//! player accepts by `/duel` at the challenger. It keeps three facts, the
//! challenge the player sent, the one waiting for them and whether a duel
//! is on, and turns a challenge down on its own (0x3ea0) when the player is
//! busy. It has no decline command, no time-out and no distance test.
//!
//! The host may also show a question for a waiting challenge (a `QoL`
//! departure the host owns, on by default in eq-client): its yes sends what
//! `/duel` sends, and its no forgets the challenge here and sends nothing.
//!
//! The program's side of the duel, [0x9238d8], is the one the player
//! challenged or accepted; it alone decides a death (0x4a8cd5), a refusal
//! (0x3ea0, 0x45d5af) and a departure (`OP_DeleteSpawn`, 0x4581b7), by read
//! R49-6d. When the player kills the opponent, or the opponent leaves during
//! the duel, the program sends 0x3bad, 8 bytes, the opponent then the
//! player; `EQEmu` drops it, taking 0x3bad as its 12-byte decline.
//!
//! The ignore list's refusal (reason 0) is not sent: the session keeps no
//! ignore list, so an ignored player's challenge is asked about like any
//! other.
use super::{
    actions,
    confirmations::closed,
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::{
    confirmations::{ConsentClosure, ConsentId, ConsentKind},
    message::Message,
    request::Request,
    socials::{DuelAutoDecline, DuelNews, DuelQuestion, DuelUpdate},
    world::{SpawnKind, WorldEvent},
};

/// The Titanium program's duel state, and the host's question about a
/// waiting challenge.
#[derive(Debug, Default)]
pub(super) struct TitaniumDuel {
    /// The player the player challenged (the program's 0x9238d8).
    outgoing: Option<u16>,
    /// The challenge waiting for the player, or their opponent (0x9238dc).
    incoming: Option<u16>,
    /// Whether a duel is on (0x9238e0).
    active: bool,
    /// The question about the waiting challenge, if the host was asked.
    question: Option<DuelQuestion>,
    sequence: u64,
}

impl TitaniumDuel {
    /// The program clears its duel state at death, zone-in and setup.
    fn clear(&mut self, out: &mut Out<'_, '_>) -> Result<()> {
        self.outgoing = None;
        self.incoming = None;
        self.active = false;
        self.close_question(out, ConsentClosure::Interrupted)
    }

    fn close_question(&mut self, out: &mut Out<'_, '_>, closure: ConsentClosure) -> Result<()> {
        if let Some(question) = self.question.take() {
            closed(out, question.id, closure)?;
        }
        Ok(())
    }

    fn news(out: &mut Out<'_, '_>, news: DuelNews) -> Result<()> {
        out.log
            .send(super::ClientEvent::World(WorldEvent::DuelNews(news)))
    }

    /// Whether the spawn is the player's side of the duel, [0x9238d8]: the
    /// one challenged, or accepted, and in a duel always the opponent.
    fn partner(&self, spawn: u16) -> bool {
        self.outgoing == Some(spawn)
    }

    /// A challenge reaching the player, as 0x453231 takes it.
    fn challenged(&mut self, from: u16, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        if world.is_player(from) || world.spawns.get(from).is_none() {
            return Ok(());
        }
        if self.active {
            if self.incoming != Some(from) {
                out.request(&Request::AutoDeclineDuel {
                    challenger: from,
                    reason: DuelAutoDecline::Dueling,
                })?;
            }
            return Ok(());
        }
        if self.outgoing == Some(from) {
            // The one the player challenged accepted: the duel is on, and
            // nothing is sent.
            self.active = true;
            self.incoming = Some(from);
            return Self::news(out, DuelNews::Begun { with: from });
        }
        match (self.outgoing, self.incoming) {
            (None, None) => {
                self.incoming = Some(from);
                Self::news(out, DuelNews::Challenged { by: from })?;
                self.ask(from, world, out)
            }
            (None, Some(waiting)) if waiting == from => Ok(()),
            _ => out.request(&Request::AutoDeclineDuel {
                challenger: from,
                reason: DuelAutoDecline::Considering,
            }),
        }
    }

    /// Opens the host's question about the waiting challenge.
    fn ask(&mut self, challenger: u16, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        let Some(sequence) = self.sequence.checked_add(1) else {
            return Ok(());
        };
        self.sequence = sequence;
        let question = DuelQuestion {
            id: ConsentId {
                session_id: world.session_id,
                kind: ConsentKind::Duel,
                sequence,
            },
            challenger,
        };
        self.question = Some(question);
        out.log
            .send(super::ClientEvent::World(WorldEvent::DuelOffer(question)))
    }

    /// `/duel` at the target, as 0x472a67 takes it; 0 is no target.
    fn duel(&mut self, target: u16, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        let player = world
            .spawns
            .get(target)
            .is_some_and(|spawn| spawn.kind == SpawnKind::Player);
        if world.is_player(target) {
            return Self::news(out, DuelNews::NotYourself);
        }
        if target == 0 || !player {
            return Self::news(out, DuelNews::NoPlayerTarget);
        }
        if self.active {
            return Self::news(
                out,
                DuelNews::AlreadyDueling {
                    at_opponent: self.incoming == Some(target),
                },
            );
        }
        match self.incoming {
            Some(challenger) if challenger == target => self.accept(target, out),
            Some(_) => Self::news(out, DuelNews::ConsideringAnother),
            None => {
                // A second challenge replaces the first, with no test.
                out.request(&Request::DuelRequest { other: target })?;
                self.outgoing = Some(target);
                Self::news(out, DuelNews::Challenging { target })
            }
        }
    }

    /// Accepts the waiting challenge: `OP_RequestDuel`, then
    /// `OP_DuelAccept`, each naming the challenger, then the player.
    fn accept(&mut self, challenger: u16, out: &mut Out<'_, '_>) -> Result<()> {
        out.request(&Request::DuelRequest { other: challenger })?;
        out.request(&Request::AnswerDuel {
            challenger,
            accept: true,
        })?;
        self.active = true;
        self.outgoing = Some(challenger);
        self.close_question(out, ConsentClosure::AnswerSent { accept: true })?;
        Self::news(out, DuelNews::Accepting { challenger })
    }

    /// The host's answer to its question.
    fn answer(
        &mut self,
        command: &ClientCommand,
        (offer_id, accept): (ConsentId, bool),
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let current = self.question.filter(|question| {
            question.id == offer_id
                && offer_id.session_id == world.session_id
                && self.incoming == Some(question.challenger)
                && !self.active
                // Ours: `EQEmu` looks the one named up without checking
                // they exist, so the card never names one gone from the
                // zone. `/duel` reaches only a target in it.
                && world.spawns.get(question.challenger).is_some()
        });
        let Some(question) = current else {
            return actions::refuse(
                command,
                "That challenge to a duel is no longer current.",
                out.log,
            );
        };
        if accept {
            return self.accept(question.challenger, out);
        }
        // The program has no decline: the challenge is forgotten here and
        // nothing is sent.
        self.incoming = None;
        self.close_question(out, ConsentClosure::AnswerSent { accept: false })
    }

    /// `OP_Death` (0x4a8cd5): the partner's death ends it. The program
    /// tests [0x9238d8] alone, so a challenge the player sent that was never
    /// accepted ends too (0x2f45), and a challenger's death changes nothing
    /// (R49-6d).
    fn died(
        &mut self,
        (dead, killer): (u16, u32),
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if !self.partner(dead) {
            return Ok(());
        }
        let by_player = u16::try_from(killer).is_ok_and(|killer| world.is_player(killer));
        let news = if by_player && self.active {
            out.request(&Request::DuelOver { opponent: dead })?;
            DuelNews::Won { over: dead }
        } else {
            DuelNews::Ended { opponent: dead }
        };
        self.clear(out)?;
        Self::news(out, news)
    }

    /// Another program's own refusal (0x45d5af): only from the one the
    /// player challenged. It ends the player's side and the duel, and
    /// keeps a challenge waiting for the player (R49-6d).
    fn turned_down(
        &mut self,
        by: u16,
        reason: Option<DuelAutoDecline>,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if !self.partner(by) {
            return Ok(());
        }
        self.active = false;
        self.outgoing = None;
        Self::news(out, DuelNews::TurnedDown { by, reason })
    }

    /// The partner leaving the zone (`OP_DeleteSpawn`, 0x4581b7): during
    /// the duel the player wins and the program says so to the server;
    /// before it the challenge lapses. Either way all is forgotten
    /// (R49-6d).
    fn departed(&mut self, spawn: u16, out: &mut Out<'_, '_>) -> Result<()> {
        if !self.partner(spawn) {
            return Ok(());
        }
        let news = if self.active {
            out.request(&Request::DuelOver { opponent: spawn })?;
            DuelNews::Forfeited { opponent: spawn }
        } else {
            DuelNews::Lapsed { target: spawn }
        };
        self.clear(out)?;
        Self::news(out, news)
    }
}

impl Feature for TitaniumDuel {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Dueling]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::AnswerDuel { .. } | ClientCommand::ChallengeDuel { .. }
        )
    }

    fn admitted(&mut self, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        self.clear(out)
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        // Ours: outside a zone there's no one to send to. The program has no
        // posture or motion test for `/duel`, so none is made here.
        if !world.ready() {
            return actions::refuse(command, "You can't do that now.", out.log);
        }
        match *command {
            ClientCommand::AnswerDuel {
                session_id,
                offer_id,
                accept,
            } if session_id == world.session_id => {
                self.answer(command, (offer_id, accept), world, out)
            }
            ClientCommand::ChallengeDuel {
                session_id, target, ..
            } if session_id == world.session_id => self.duel(target, world, out),
            _ => actions::refuse(command, "That is for another session.", out.log),
        }
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match message {
            Message::ZoneOffer(_)
            | Message::ZoneAnswer(_)
            | Message::Handoff(_)
            | Message::LoggedOut => self.clear(out),
            Message::Event(WorldEvent::Death(death)) => {
                let Ok(dead) = u16::try_from(death.spawn_id) else {
                    return Ok(());
                };
                if world.is_player(dead) {
                    return self.clear(out);
                }
                self.died((dead, death.killer_id), world, out)
            }
            Message::Event(WorldEvent::Duel(DuelUpdate::Challenge { from })) => {
                self.challenged(*from, world, out)
            }
            Message::Event(WorldEvent::Duel(DuelUpdate::AutoDeclined { by, reason })) => {
                self.turned_down(*by, *reason, out)
            }
            Message::Event(WorldEvent::Despawn(spawn)) => self.departed(*spawn, out),
            // The program has no case for `OP_DuelAccept`, and Titanium
            // carries no client's refusal.
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;
    use eq_network_game::{socials, zoning::Death};
    use std::time::Instant;

    const OWN: u16 = 7;

    fn world() -> World {
        let mut world = World::new(5);
        world.admitted = Some(Instant::now());
        world.own_spawn = Some(OWN);
        for id in [12, 13] {
            world.spawns.insert(testing::spawn(id, SpawnKind::Player));
        }
        world.spawns.insert(testing::spawn(30, SpawnKind::Npc));
        world
    }

    fn challenge(from: u16) -> Message {
        Message::Event(WorldEvent::Duel(DuelUpdate::Challenge { from }))
    }

    fn duel(target: u16) -> ClientCommand {
        ClientCommand::ChallengeDuel {
            session_id: 5,
            target,
            created: Instant::now(),
        }
    }

    fn news(outcome: &testing::Outcome<Result<()>>) -> Vec<DuelNews> {
        outcome
            .events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::DuelNews(news)) => Some(*news),
                _ => None,
            })
            .collect()
    }

    fn offer(outcome: &testing::Outcome<Result<()>>) -> Option<DuelQuestion> {
        outcome.events.iter().find_map(|event| match event {
            ClientEvent::World(WorldEvent::DuelOffer(question)) => Some(*question),
            _ => None,
        })
    }

    #[test]
    fn a_challenge_is_a_line_and_a_question_and_sends_nothing() {
        let (mut feature, mut world) = (TitaniumDuel::default(), world());
        let heard = testing::run(|out| feature.observe(&challenge(12), &mut world, out));
        assert_eq!(heard.sent, []);
        assert_eq!(news(&heard), [DuelNews::Challenged { by: 12 }]);
        assert_eq!(offer(&heard).map(|question| question.challenger), Some(12));
        // The same challenger again: nothing.
        let again = testing::run(|out| feature.observe(&challenge(12), &mut world, out));
        assert!(again.sent.is_empty() && again.events.is_empty());
        // One not in the zone, or the player: nothing.
        for from in [99, OWN] {
            let ignored = testing::run(|out| feature.observe(&challenge(from), &mut world, out));
            assert!(ignored.sent.is_empty() && ignored.events.is_empty());
        }
    }

    #[test]
    fn a_second_challenger_is_turned_down_by_the_program_itself() {
        let (mut feature, mut world) = (TitaniumDuel::default(), world());
        testing::run(|out| feature.observe(&challenge(12), &mut world, out));
        let busy = testing::run(|out| feature.observe(&challenge(13), &mut world, out));
        assert_eq!(
            (busy.sent[0].opcode, &busy.sent[0].body),
            (
                0x3ea0,
                &socials::titanium_duel_auto_decline(13, OWN, DuelAutoDecline::Considering).body
            )
        );
        assert_eq!(busy.sent[0].body, [13, 0, 0, 0, 7, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(news(&busy), []);
    }

    #[test]
    fn duel_accepts_the_waiting_challenge_in_two_packets() {
        let (mut feature, mut world) = (TitaniumDuel::default(), world());
        testing::run(|out| feature.observe(&challenge(12), &mut world, out));
        let accepted = testing::run(|out| feature.handle(&duel(12), &mut world, out));
        assert_eq!(
            accepted
                .sent
                .iter()
                .map(|packet| (packet.opcode, packet.body.clone()))
                .collect::<Vec<_>>(),
            [
                (0x28e1, vec![12, 0, 0, 0, 7, 0, 0, 0]),
                (0x1b09, vec![12, 0, 0, 0, 7, 0, 0, 0]),
            ]
        );
        assert_eq!(news(&accepted), [DuelNews::Accepting { challenger: 12 }]);
        assert!(feature.active);
        // During the duel, a third player's challenge is turned down as
        // dueling, and /duel says the duel is on.
        let busy = testing::run(|out| feature.observe(&challenge(13), &mut world, out));
        assert_eq!(busy.sent[0].body[8], 1);
        let on = testing::run(|out| feature.handle(&duel(12), &mut world, out));
        assert_eq!(on.sent, []);
        assert_eq!(news(&on), [DuelNews::AlreadyDueling { at_opponent: true }]);
        let elsewhere = testing::run(|out| feature.handle(&duel(13), &mut world, out));
        assert_eq!(
            news(&elsewhere),
            [DuelNews::AlreadyDueling { at_opponent: false }]
        );
    }

    #[test]
    fn duel_challenges_and_the_answer_in_kind_begins_it_with_nothing_sent() {
        let (mut feature, mut world) = (TitaniumDuel::default(), world());
        let sent = testing::run(|out| feature.handle(&duel(12), &mut world, out));
        assert_eq!(
            (sent.sent.len(), sent.sent[0].opcode, &sent.sent[0].body),
            (1, 0x28e1, &vec![12, 0, 0, 0, 7, 0, 0, 0])
        );
        assert_eq!(news(&sent), [DuelNews::Challenging { target: 12 }]);
        // A second /duel replaces the first, with no test.
        testing::run(|out| feature.handle(&duel(13), &mut world, out));
        assert_eq!(feature.outgoing, Some(13));
        // Another challenger while ours is out is turned down.
        let other = testing::run(|out| feature.observe(&challenge(12), &mut world, out));
        assert_eq!(other.sent[0].body[8], 2);
        let begun = testing::run(|out| feature.observe(&challenge(13), &mut world, out));
        assert_eq!(begun.sent, []);
        assert_eq!(news(&begun), [DuelNews::Begun { with: 13 }]);
        assert!(offer(&begun).is_none() && feature.active);
    }

    #[test]
    fn duel_refuses_in_the_programs_words_and_sends_nothing() {
        let (mut feature, mut world) = (TitaniumDuel::default(), world());
        for (target, said) in [
            (0, DuelNews::NoPlayerTarget),
            (30, DuelNews::NoPlayerTarget),
            (99, DuelNews::NoPlayerTarget),
            (OWN, DuelNews::NotYourself),
        ] {
            let refused = testing::run(|out| feature.handle(&duel(target), &mut world, out));
            assert_eq!(refused.sent, []);
            assert_eq!(news(&refused), [said], "{target}");
        }
        testing::run(|out| feature.observe(&challenge(12), &mut world, out));
        let waiting = testing::run(|out| feature.handle(&duel(13), &mut world, out));
        assert_eq!(waiting.sent, []);
        assert_eq!(news(&waiting), [DuelNews::ConsideringAnother]);
    }

    #[test]
    fn the_hosts_yes_sends_what_duel_does_and_its_no_sends_nothing() {
        let (mut feature, mut world) = (TitaniumDuel::default(), world());
        let question = offer(&testing::run(|out| {
            feature.observe(&challenge(12), &mut world, out)
        }))
        .unwrap();
        let answer = |accept| ClientCommand::AnswerDuel {
            offer_id: question.id,
            session_id: 5,
            accept,
        };
        let no = testing::run(|out| feature.handle(&answer(false), &mut world, out));
        assert_eq!(no.sent, []);
        assert!(feature.incoming.is_none() && feature.question.is_none());
        // Forgotten: the same challenger may challenge again, and the next
        // challenger is not turned down.
        let next = offer(&testing::run(|out| {
            feature.observe(&challenge(13), &mut world, out)
        }))
        .unwrap();
        let stale = testing::run(|out| feature.handle(&answer(true), &mut world, out));
        assert_eq!(stale.sent, []);
        let yes = ClientCommand::AnswerDuel {
            offer_id: next.id,
            session_id: 5,
            accept: true,
        };
        let accepted = testing::run(|out| feature.handle(&yes, &mut world, out));
        assert_eq!(
            accepted
                .sent
                .iter()
                .map(|packet| packet.opcode)
                .collect::<Vec<_>>(),
            [0x28e1, 0x1b09]
        );
        assert!(feature.active && feature.question.is_none());
    }

    #[test]
    fn a_partners_death_ends_the_duel_and_a_challengers_changes_nothing() {
        let died = |spawn_id: u32, killer_id: u32| {
            Message::Event(WorldEvent::Death(Death {
                spawn_id,
                killer_id,
                corpse_id: 0,
                bind_zone_id: 0,
                corpse_name: None,
            }))
        };
        let (mut feature, mut world) = (TitaniumDuel::default(), world());
        testing::run(|out| feature.observe(&challenge(12), &mut world, out));
        // The challenger isn't the program's side yet: their death is no
        // news (0x4a8cd5 tests [0x9238d8] alone).
        let challenger = testing::run(|out| feature.observe(&died(12, 30), &mut world, out));
        assert_eq!(challenger.sent, []);
        assert_eq!(news(&challenger), []);
        assert_eq!(feature.incoming, Some(12));
        testing::run(|out| feature.handle(&duel(12), &mut world, out));
        // Killed by the player during the duel: won, and 0x3bad goes.
        let won = testing::run(|out| feature.observe(&died(12, u32::from(OWN)), &mut world, out));
        assert_eq!(won.sent, [socials::titanium_duel_over(12, OWN)]);
        assert_eq!((won.sent[0].opcode, won.sent[0].body.len()), (0x3bad, 8));
        assert_eq!(news(&won), [DuelNews::Won { over: 12 }]);
        assert!(!feature.active && feature.incoming.is_none() && feature.outgoing.is_none());
        // A challenge never accepted ends with its target's death.
        testing::run(|out| feature.handle(&duel(13), &mut world, out));
        let ended = testing::run(|out| feature.observe(&died(13, 30), &mut world, out));
        assert_eq!(ended.sent, []);
        assert_eq!(news(&ended), [DuelNews::Ended { opponent: 13 }]);
        assert!(feature.outgoing.is_none());
        testing::run(|out| feature.observe(&challenge(12), &mut world, out));
        testing::run(|out| feature.observe(&Message::LoggedOut, &mut world, out));
        assert!(feature.incoming.is_none() && feature.question.is_none());
    }

    #[test]
    fn a_refusal_counts_only_from_the_one_challenged_and_keeps_the_waiting_challenge() {
        let refused =
            |by, reason| Message::Event(WorldEvent::Duel(DuelUpdate::AutoDeclined { by, reason }));
        let (mut feature, mut world) = (TitaniumDuel::default(), world());
        testing::run(|out| feature.handle(&duel(13), &mut world, out));
        // From anyone else: ignored.
        let other = testing::run(|out| {
            feature.observe(
                &refused(12, Some(DuelAutoDecline::Dueling)),
                &mut world,
                out,
            )
        });
        assert_eq!(news(&other), []);
        assert_eq!(feature.outgoing, Some(13));
        let turned = testing::run(|out| {
            feature.observe(
                &refused(13, Some(DuelAutoDecline::Considering)),
                &mut world,
                out,
            )
        });
        assert_eq!(turned.sent, []);
        assert_eq!(
            news(&turned),
            [DuelNews::TurnedDown {
                by: 13,
                reason: Some(DuelAutoDecline::Considering)
            }]
        );
        assert!(feature.outgoing.is_none() && !feature.active);
    }

    #[test]
    fn a_partner_leaving_forfeits_the_duel_or_lapses_the_challenge() {
        let gone = |spawn| Message::Event(WorldEvent::Despawn(spawn));
        let (mut feature, mut world) = (TitaniumDuel::default(), world());
        testing::run(|out| feature.handle(&duel(13), &mut world, out));
        // Someone else leaving changes nothing.
        assert_eq!(
            news(&testing::run(|out| feature.observe(
                &gone(12),
                &mut world,
                out
            ))),
            []
        );
        let lapsed = testing::run(|out| feature.observe(&gone(13), &mut world, out));
        assert_eq!(lapsed.sent, []);
        assert_eq!(news(&lapsed), [DuelNews::Lapsed { target: 13 }]);
        assert!(feature.outgoing.is_none());
        // During the duel the player wins, and 0x3bad goes.
        testing::run(|out| feature.observe(&challenge(12), &mut world, out));
        testing::run(|out| feature.handle(&duel(12), &mut world, out));
        let forfeited = testing::run(|out| feature.observe(&gone(12), &mut world, out));
        assert_eq!(forfeited.sent, [socials::titanium_duel_over(12, OWN)]);
        assert_eq!(news(&forfeited), [DuelNews::Forfeited { opponent: 12 }]);
        assert!(!feature.active && feature.incoming.is_none() && feature.outgoing.is_none());
    }

    #[test]
    fn the_card_never_names_a_challenger_gone_from_the_zone() {
        let (mut feature, mut world) = (TitaniumDuel::default(), world());
        let question = offer(&testing::run(|out| {
            feature.observe(&challenge(12), &mut world, out)
        }))
        .unwrap();
        // A challenger's departure doesn't clear the program's state.
        testing::run(|out| {
            feature.observe(&Message::Event(WorldEvent::Despawn(12)), &mut world, out)
        });
        let mut gone = World::new(5);
        gone.admitted = world.admitted;
        gone.own_spawn = Some(OWN);
        let yes = ClientCommand::AnswerDuel {
            offer_id: question.id,
            session_id: 5,
            accept: true,
        };
        let refused = testing::run(|out| feature.handle(&yes, &mut gone, out));
        assert_eq!(refused.sent, []);
    }
}
