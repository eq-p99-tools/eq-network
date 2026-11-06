//! Being resurrected. The session offers one answer to the admitted player
//! for the server's exact packet. Movement and recovery come from the server.
use super::{
    actions,
    confirmations::{closed, Pending, TITANIUM_TIMEOUT},
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    confirmations::{ConsentClosure, ConsentKind, ConsentOffer},
    message::Message,
    request::Request,
    resurrection::ResurrectionOffer,
    world::WorldEvent,
};
use std::time::Instant;

/// Answers offers to resurrect the player.
#[derive(Default)]
pub(super) struct Resurrection {
    pending: Pending<ResurrectionOffer>,
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
        let Some(offer) = self.pending.take(offer_id, world) else {
            return actions::refuse(
                command,
                "That resurrection offer is no longer current.",
                out.log,
            );
        };
        out.request(&Request::AnswerResurrection { offer, accept })?;
        closed(out, offer_id, ConsentClosure::AnswerSent { accept })
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.pending.invalidate(message, world, out)?;
        let Message::Resurrection(offer) = message else {
            return Ok(());
        };
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
            Instant::now() + TITANIUM_TIMEOUT,
        ) {
            out.log
                .send(ClientEvent::World(WorldEvent::Resurrection(question)))?;
        }
        Ok(())
    }

    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
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
        let early = testing::run(|out| {
            resurrection.tick(now - std::time::Duration::from_nanos(1), &mut world, out)
        });
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
}
