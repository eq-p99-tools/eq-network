//! Being resurrected. The session offers one answer to the admitted player
//! for the server's exact packet. Movement and recovery come from the server,
//! but for TAKP's completion of an accepted offer, which moves the player as
//! the `EQMac` program's own transfer request (6.596).
use super::{
    actions,
    confirmations::{closed, interrupted, Pending, TITANIUM_TIMEOUT},
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use crate::client::GapKind;
use anyhow::Result;
use eq_network_game::{
    confirmations::{ConsentClosure, ConsentKind, ConsentOffer},
    message::Message,
    request::Request,
    resurrection::{ResurrectionOffer, EQMAC_COMPLETE_OPCODE},
    world::WorldEvent,
};
use std::time::{Duration, Instant};

/// Answers offers to resurrect the player.
pub(super) struct Resurrection {
    pending: Pending<ResurrectionOffer>,
    p99: bool,
    /// How long a question stays open before the session declines it.
    timeout: Duration,
    /// Whether the server may send its completion of an accepted offer for
    /// the client to follow (`EQMac`).
    follows_completion: bool,
    /// The offer this admission accepted, until its completion, a transfer,
    /// the player's death or its deadline.
    accepted: Option<ConsentOffer<ResurrectionOffer>>,
}

impl Default for Resurrection {
    fn default() -> Self {
        Self {
            pending: Pending::default(),
            p99: false,
            timeout: TITANIUM_TIMEOUT,
            follows_completion: false,
            accepted: None,
        }
    }
}

impl Resurrection {
    /// Uses Titanium consent for ordinary addressed P99 resurrection spells.
    pub(super) fn project1999() -> Self {
        Self {
            p99: true,
            ..Self::default()
        }
    }

    /// TAKP and Quarm: `EQMac`'s packets (6.596), a question left open an
    /// hour, and TAKP's completion followed only for the accepted offer.
    pub(super) fn eqmac(timeout: Duration) -> Self {
        Self {
            timeout,
            follows_completion: true,
            ..Self::default()
        }
    }

    /// Follows a completion of the offer this admission accepted, and
    /// records any other as a gap.
    fn complete(
        &mut self,
        completed: &ResurrectionOffer,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let reason = if !self.follows_completion {
            "a resurrection completion this server is not known to send"
        } else if let Some(accepted) = self.accepted.take_if(|accepted| {
            accepted.id.session_id == world.session_id && Instant::now() < accepted.expires_at
        }) {
            if let Some(go) = accepted.offer.eqmac_move(completed) {
                world.happened(Message::ZoneOffer(go));
                return Ok(());
            }
            "a resurrection completion unlike the offer accepted"
        } else {
            "a resurrection completion with no offer accepted in this admission"
        };
        out.log.gap(
            EQMAC_COMPLETE_OPCODE,
            Some("OP_RezzComplete"),
            completed.wire_identity(),
            GapKind::Unread {
                reason: reason.to_owned(),
            },
        )
    }
}

impl Feature for Resurrection {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Resurrection]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::AnswerResurrection { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::AnswerResurrection {
            offer_id, accept, ..
        } = *command
        else {
            return Ok(());
        };
        self.tick(Instant::now(), world, out)?;
        let Some(question) = self.pending.take(offer_id, world) else {
            return actions::refuse(
                command,
                "That resurrection offer is no longer current.",
                out.log,
            );
        };
        out.request(&Request::AnswerResurrection {
            offer: question.offer.clone(),
            accept,
        })?;
        if self.follows_completion && accept {
            self.accepted = Some(question);
        }
        closed(out, offer_id, ConsentClosure::AnswerSent { accept })
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.pending.invalidate(message, world, out)?;
        if interrupted(message, world) {
            self.accepted = None;
        }
        if let Message::ResurrectionCompleted(completed) = message {
            return self.complete(completed, world, out);
        }
        let Message::Resurrection(offer) = message else {
            return Ok(());
        };
        if self.p99 && !offer.valid_p99_question() {
            return Ok(());
        }
        if world
            .player
            .as_ref()
            .is_none_or(|player| !player.name.eq_ignore_ascii_case(&offer.recipient))
        {
            return Ok(());
        }
        if let Some(question) = self.pending.open(
            offer,
            world,
            ConsentKind::Resurrection,
            Instant::now() + self.timeout,
            self.p99.then(|| offer.wire_identity()),
        ) {
            out.log
                .send(ClientEvent::World(WorldEvent::Resurrection(question)))?;
        }
        Ok(())
    }

    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if world.lifecycle.blocks_motion()
            || self
                .accepted
                .as_ref()
                .is_some_and(|accepted| now >= accepted.expires_at)
        {
            self.accepted = None;
        }
        if let Some(ConsentOffer { id, offer, .. }) = self.pending.elapsed(now, world, out)? {
            out.request(&Request::AnswerResurrection {
                offer,
                accept: false,
            })?;
            closed(out, id, ConsentClosure::Expired)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::{
        confirmations::ConsentId,
        resurrection::{titanium_answer, OFFER_OPCODE},
    };

    fn answer(offer_id: ConsentId, accept: bool) -> ClientCommand {
        ClientCommand::AnswerResurrection {
            session_id: 5,
            offer_id,
            accept,
        }
    }

    fn offer(name: &[u8]) -> Message {
        let mut body = vec![0; 228];
        body[24..24 + name.len()].copy_from_slice(name);
        body[92..98].copy_from_slice(b"Helper");
        body[160..173].copy_from_slice(b"Tester's body");
        eq_network_game::message::titanium(OFFER_OPCODE, &body)
            .pop()
            .unwrap()
    }

    #[test]
    fn p99_resurrection_answers_the_current_addressed_offer_without_local_recovery() {
        let mut resurrection = Resurrection::project1999();
        let mut world = World::new(5);
        let mut player = testing::player(7);
        player.name = "Tester".into();
        world.player.admit(player);
        world.admitted = Some(Instant::now());
        let Message::Resurrection(missing) = offer(b"Tester") else {
            unreachable!();
        };
        let ignored = testing::run(|out| {
            resurrection.observe(&Message::Resurrection(missing), &mut world, out)
        });
        assert!(ignored.events.is_empty());
        let mut body = vec![0; 228];
        body[4..6].copy_from_slice(&8u16.to_le_bytes());
        body[24..30].copy_from_slice(b"Tester");
        body[92..98].copy_from_slice(b"Helper");
        body[156..160].copy_from_slice(&392u32.to_le_bytes());
        body[160..173].copy_from_slice(b"Tester's body");
        let message = eq_network_game::message::titanium(OFFER_OPCODE, &body).remove(0);
        let opened = testing::run(|out| resurrection.observe(&message, &mut world, out));
        let [ClientEvent::World(WorldEvent::Resurrection(question))] = opened.events.as_slice()
        else {
            panic!("expected the addressed offer");
        };
        let result =
            testing::run(|out| resurrection.handle(&answer(question.id, true), &mut world, out));
        assert_eq!(result.sent, [titanium_answer(&question.offer, true)]);
        assert!(matches!(
            result.events.as_slice(),
            [ClientEvent::World(WorldEvent::ConsentClosed {
                closure: ConsentClosure::AnswerSent { accept: true },
                ..
            })]
        ));
        assert!(world.take_news().is_empty());
        assert!(world.lifecycle.pending().is_none());
    }

    #[test]
    fn only_the_admitted_recipient_can_answer_and_expiry_only_declines_once() {
        let mut resurrection = Resurrection::default();
        let mut world = World::new(5);
        let mut player = testing::player(7);
        player.name = "Tester".into();
        world.player.admit(player);
        let valid = offer(b"tester");
        let foreign = offer(b"Another");
        let too_early = testing::run(|out| resurrection.observe(&valid, &mut world, out));
        assert!(too_early.events.is_empty());
        world.admitted = Some(Instant::now());
        let foreign = testing::run(|out| resurrection.observe(&foreign, &mut world, out));
        assert!(foreign.events.is_empty());
        assert_eq!(
            foreign.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );

        let opened = testing::run(|out| resurrection.observe(&valid, &mut world, out));
        opened.result.unwrap();
        assert_eq!(
            opened.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );
        let [ClientEvent::World(WorldEvent::Resurrection(question))] = opened.events.as_slice()
        else {
            panic!("expected the question");
        };
        let outcome =
            testing::run(|out| resurrection.handle(&answer(question.id, true), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [titanium_answer(&question.offer, true)]);
        assert!(!world.lifecycle.blocks_motion());

        // An identical offer after answering can be a fresh cast.
        let opened = testing::run(|out| resurrection.observe(&valid, &mut world, out));
        let [ClientEvent::World(WorldEvent::Resurrection(new_question))] = opened.events.as_slice()
        else {
            panic!("expected the new question");
        };
        assert_ne!(question.id, new_question.id);
        let now = new_question.expires_at;
        let before = now
            .checked_sub(std::time::Duration::from_nanos(1))
            .expect("the fixture deadline has a preceding nanosecond");
        let early = testing::run(|out| resurrection.tick(before, &mut world, out));
        early.result.unwrap();
        assert!(early.sent.is_empty() && early.events.is_empty());
        let expired = testing::run(|out| resurrection.tick(now, &mut world, out));
        expired.result.unwrap();
        assert_eq!(expired.sent, [titanium_answer(&new_question.offer, false)]);
        assert!(matches!(
            expired.events.as_slice(),
            [ClientEvent::World(WorldEvent::ConsentClosed { id, closure: ConsentClosure::Expired })] if *id == new_question.id
        ));
        let repeated = testing::run(|out| resurrection.tick(now, &mut world, out));
        repeated.result.unwrap();
        assert!(repeated.sent.is_empty() && repeated.events.is_empty());
        let late = testing::run(|out| {
            resurrection.handle(&answer(new_question.id, true), &mut world, out)
        });
        late.result.unwrap();
        assert_eq!(
            late.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );
    }

    /// A Mac offer to Tester at (263, 1748, 27) in zone 4, and its TAKP
    /// completion.
    fn eqmac_records() -> (Vec<u8>, Vec<u8>) {
        use eq_network_game::resurrection::EQMAC_OFFER_OPCODE;
        let mut body = vec![0; 228];
        body[4..6].copy_from_slice(&4u16.to_le_bytes());
        body[8..12].copy_from_slice(&1748.0f32.to_le_bytes());
        body[12..16].copy_from_slice(&263.0f32.to_le_bytes());
        body[16..20].copy_from_slice(&27.0f32.to_le_bytes());
        body[24..30].copy_from_slice(b"Tester");
        body[92..98].copy_from_slice(b"Helper");
        body[156..160].copy_from_slice(&392u32.to_le_bytes());
        body[160..173].copy_from_slice(b"Tester's body");
        let mut done = body.clone();
        done[224] = 1;
        assert!(matches!(
            eq_network_game::message::eqmac(EQMAC_OFFER_OPCODE, &body).as_slice(),
            [Message::Resurrection(_)]
        ));
        (body, done)
    }

    fn eqmac_world() -> World {
        let mut world = World::new(5);
        let mut player = testing::player(7);
        player.name = "Tester".into();
        world.player.admit(player);
        world.admitted = Some(Instant::now());
        world
    }

    fn completion(done: &[u8]) -> Message {
        use eq_network_game::resurrection::EQMAC_COMPLETE_OPCODE;
        eq_network_game::message::eqmac(EQMAC_COMPLETE_OPCODE, done).remove(0)
    }

    fn gap_reasons(events: &[ClientEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::Gap(gap) => match &gap.kind {
                    GapKind::Unread { reason } => Some(reason.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_mac_yes_echoes_the_offer_and_follows_only_its_own_completion() {
        use eq_network_game::resurrection::{eqmac_offer, EQMAC_ANSWER_OPCODE};
        let (body, done) = eqmac_records();
        let mut resurrection = Resurrection::eqmac(Duration::from_secs(3600));
        let mut world = eqmac_world();

        // A completion before any yes is a gap, and moves nobody.
        let early = testing::run_on(&super::super::wire::EqMac, |out| {
            resurrection.observe(&completion(&done), &mut world, out)
        });
        early.result.unwrap();
        assert_eq!(
            gap_reasons(&early.events),
            ["a resurrection completion with no offer accepted in this admission"]
        );
        assert!(world.take_news().is_empty());

        let offer = Message::Resurrection(eqmac_offer(&body).unwrap());
        let opened = testing::run_on(&super::super::wire::EqMac, |out| {
            resurrection.observe(&offer, &mut world, out)
        });
        let [ClientEvent::World(WorldEvent::Resurrection(question))] = opened.events.as_slice()
        else {
            panic!("expected the addressed offer");
        };
        let hour = question
            .expires_at
            .saturating_duration_since(Instant::now());
        assert!(hour > Duration::from_secs(3590) && hour <= Duration::from_secs(3600));
        let answered = testing::run_on(&super::super::wire::EqMac, |out| {
            resurrection.handle(&answer(question.id, true), &mut world, out)
        });
        answered.result.unwrap();
        let [sent] = answered.sent.as_slice() else {
            panic!("expected one answer");
        };
        assert_eq!((sent.opcode, &sent.body), (EQMAC_ANSWER_OPCODE, &done));

        // Another place is a gap and spends the acceptance.
        let mut elsewhere = done.clone();
        elsewhere[16..20].copy_from_slice(&28.0f32.to_le_bytes());
        let unlike = testing::run_on(&super::super::wire::EqMac, |out| {
            resurrection.observe(&completion(&elsewhere), &mut world, out)
        });
        assert_eq!(
            gap_reasons(&unlike.events),
            ["a resurrection completion unlike the offer accepted"]
        );
        assert!(world.take_news().is_empty());

        // Accept the same offer again and follow its completion once.
        let opened = testing::run_on(&super::super::wire::EqMac, |out| {
            resurrection.observe(&offer, &mut world, out)
        });
        let [ClientEvent::World(WorldEvent::Resurrection(question))] = opened.events.as_slice()
        else {
            panic!("expected the offer again");
        };
        testing::run_on(&super::super::wire::EqMac, |out| {
            resurrection.handle(&answer(question.id, true), &mut world, out)
        })
        .result
        .unwrap();
        let followed = testing::run_on(&super::super::wire::EqMac, |out| {
            resurrection.observe(&completion(&done), &mut world, out)
        });
        followed.result.unwrap();
        assert_eq!(gap_reasons(&followed.events), Vec::<String>::new());
        let news = world.take_news();
        let [Message::ZoneOffer(go)] = news.as_slice() else {
            panic!("expected the move to the corpse, got {news:?}");
        };
        assert_eq!(
            (
                go.zone_id,
                go.instance_id,
                go.reason,
                go.solicited,
                go.to_bind
            ),
            (4, 0, 9, true, false)
        );
        assert_eq!(
            [go.position.x, go.position.y, go.position.z],
            [263.0, 1748.0, 27.0]
        );
        let again = testing::run_on(&super::super::wire::EqMac, |out| {
            resurrection.observe(&completion(&done), &mut world, out)
        });
        assert_eq!(gap_reasons(&again.events).len(), 1, "followed once");
        assert!(world.take_news().is_empty());
    }

    #[test]
    fn a_mac_no_or_a_transfer_leaves_no_completion_to_follow() {
        use eq_network_game::resurrection::eqmac_offer;
        let (body, done) = eqmac_records();
        let offer = Message::Resurrection(eqmac_offer(&body).unwrap());
        for decline in [true, false] {
            let mut resurrection = Resurrection::eqmac(Duration::from_secs(3600));
            let mut world = eqmac_world();
            let opened = testing::run_on(&super::super::wire::EqMac, |out| {
                resurrection.observe(&offer, &mut world, out)
            });
            let [ClientEvent::World(WorldEvent::Resurrection(question))] = opened.events.as_slice()
            else {
                panic!("expected the offer");
            };
            testing::run_on(&super::super::wire::EqMac, |out| {
                resurrection.handle(&answer(question.id, !decline), &mut world, out)
            })
            .result
            .unwrap();
            if !decline {
                let zone = Message::ZoneOffer(eq_network_game::zoning::ZoneOffer {
                    zone_id: 2,
                    instance_id: 0,
                    position: eq_network_game::world::Position::default(),
                    reason: 0,
                    to_bind: false,
                    solicited: true,
                });
                testing::run_on(&super::super::wire::EqMac, |out| {
                    resurrection.observe(&zone, &mut world, out)
                })
                .result
                .unwrap();
            }
            let late = testing::run_on(&super::super::wire::EqMac, |out| {
                resurrection.observe(&completion(&done), &mut world, out)
            });
            assert_eq!(gap_reasons(&late.events).len(), 1);
            assert!(world.take_news().is_empty());
        }
    }

    #[test]
    fn titanium_never_follows_a_resurrection_completion() {
        let (_, done) = eqmac_records();
        let mut resurrection = Resurrection::default();
        let mut world = eqmac_world();
        let outcome = testing::run(|out| resurrection.observe(&completion(&done), &mut world, out));
        assert_eq!(
            gap_reasons(&outcome.events),
            ["a resurrection completion this server is not known to send"]
        );
        assert!(world.take_news().is_empty());
    }
}
