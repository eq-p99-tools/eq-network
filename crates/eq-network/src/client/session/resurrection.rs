//! Being resurrected. The session keeps the latest offer, since the answer
//! repeats it, and sends the player's answer to it once; an answer with no
//! offer waiting is refused, as the server would only say the player was
//! resurrected already. On acceptance the server moves the player to the
//! corpse as it moves them anywhere, which the transfers feature follows.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::{
    message::Message, request::Request, resurrection::ResurrectionOffer, world::WorldEvent,
};

/// Answers offers to resurrect the player.
#[derive(Default)]
pub(super) struct Resurrection {
    /// The offer waiting for an answer, if any.
    offer: Option<ResurrectionOffer>,
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
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::AnswerResurrection { accept, .. } = *command else {
            return Ok(());
        };
        let Some(offer) = self.offer.take() else {
            return actions::refuse(
                command,
                "No resurrection is waiting for an answer.",
                out.log,
            );
        };
        out.request(&Request::AnswerResurrection { offer, accept })
    }

    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::Event(WorldEvent::Resurrection(offer)) = message {
            self.offer = Some(offer.clone());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;
    use eq_network_game::resurrection::{titanium_answer, titanium_offer};

    fn answer(accept: bool) -> ClientCommand {
        ClientCommand::AnswerResurrection {
            session_id: 5,
            accept,
        }
    }

    #[test]
    fn an_offer_is_answered_once() {
        let mut resurrection = Resurrection::default();
        let mut world = World::new(5);
        let outcome = testing::run(|out| resurrection.handle(&answer(true), &mut world, out));
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::ResurrectionRefused { .. })
        )));
        // An offer from Tester to Example's corpse: the caster's name at 92
        // and the corpse's at 160 of the 228 bytes.
        let mut body = vec![0; 228];
        body[92..98].copy_from_slice(b"Tester");
        body[160..177].copy_from_slice(b"Example's corpse0");
        let offer = titanium_offer(&body).unwrap();
        testing::run(|out| {
            resurrection.observe(
                &Message::Event(WorldEvent::Resurrection(offer.clone())),
                &mut world,
                out,
            )
        })
        .result
        .unwrap();
        let outcome = testing::run(|out| resurrection.handle(&answer(true), &mut world, out));
        assert_eq!(outcome.sent, [titanium_answer(&offer, true)]);
        // The offer is spent.
        let outcome = testing::run(|out| resurrection.handle(&answer(false), &mut world, out));
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
    }
}
