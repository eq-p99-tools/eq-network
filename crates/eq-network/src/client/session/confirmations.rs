//! The server's questions the player answers: a sacrifice, which takes the
//! player's life for the caster's gain, and a translocate, which moves them.
//! Each feature keeps the latest offer, since the answer repeats it, and
//! sends the player's answer to it once; the session never answers by
//! itself. An answer with no offer waiting is refused, as the server would
//! ignore it. On a yes the server takes the player's life or moves them as
//! it does anywhere, which the transfers feature follows; TAKP leaves a
//! translocate's move to the client, and its word that the player may go
//! reads as the server's offer to move them
//! ([`eq_network_game::confirmations`]).
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::{
    confirmations::{SacrificeOffer, TranslocateOffer},
    message::Message,
    request::Request,
    world::WorldEvent,
};

/// Answers offers to sacrifice the player.
#[derive(Default)]
pub(super) struct Sacrifice {
    /// The offer waiting for an answer, if any.
    offer: Option<SacrificeOffer>,
}

impl Feature for Sacrifice {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Sacrifice]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::AnswerSacrifice { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::AnswerSacrifice { accept, .. } = *command else {
            return Ok(());
        };
        let Some(offer) = self.offer.take() else {
            return actions::refuse(command, "No sacrifice is waiting for an answer.", out.log);
        };
        out.request(&Request::AnswerSacrifice { offer, accept })
    }

    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::Event(WorldEvent::SacrificeOffer(offer)) = message {
            self.offer = Some(offer.clone());
        }
        Ok(())
    }
}

/// Answers offers to translocate the player.
#[derive(Default)]
pub(super) struct Translocate {
    /// The offer waiting for an answer, if any.
    offer: Option<TranslocateOffer>,
}

impl Feature for Translocate {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Translocate]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::AnswerTranslocate { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::AnswerTranslocate { accept, .. } = *command else {
            return Ok(());
        };
        let Some(offer) = self.offer.take() else {
            return actions::refuse(command, "No translocate is waiting for an answer.", out.log);
        };
        out.request(&Request::AnswerTranslocate { offer, accept })
    }

    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::Event(WorldEvent::TranslocateOffer(offer)) = message {
            self.offer = Some(offer.clone());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;
    use eq_network_game::confirmations::{
        eqmac_translocate_answer, sacrifice, titanium_sacrifice_answer, titanium_translocate,
    };

    #[test]
    fn a_sacrifice_is_answered_once_and_only_on_the_players_word() {
        let mut feature = Sacrifice::default();
        let mut world = World::new(5);
        let answer = |accept| ClientCommand::AnswerSacrifice {
            session_id: 5,
            accept,
        };
        let outcome = testing::run(|out| feature.handle(&answer(true), &mut world, out));
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::SacrificeRefused { .. })
        )));
        // A sacrifice from spawn 581 to the player, spawn 7.
        let body: Vec<u8> = [581u32, 7, 0]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let offer = sacrifice(&body).unwrap();
        let outcome = testing::run(|out| {
            feature.observe(
                &Message::Event(WorldEvent::SacrificeOffer(offer.clone())),
                &mut world,
                out,
            )
        });
        outcome.result.unwrap();
        // The offer alone sends nothing.
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        let outcome = testing::run(|out| feature.handle(&answer(false), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [titanium_sacrifice_answer(&offer, false)]);
        // The offer is spent.
        let outcome = testing::run(|out| feature.handle(&answer(true), &mut world, out));
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
    }

    #[test]
    fn a_translocate_is_answered_in_the_wires_own_terms() {
        let mut feature = Translocate::default();
        let mut world = World::new(5);
        // A translocate from Tester to zone 4.
        let mut body = vec![0; 92];
        body[..4].copy_from_slice(&4u32.to_le_bytes());
        body[12..18].copy_from_slice(b"Tester");
        let offer = titanium_translocate(&body).unwrap();
        testing::run(|out| {
            feature.observe(
                &Message::Event(WorldEvent::TranslocateOffer(offer.clone())),
                &mut world,
                out,
            )
        })
        .result
        .unwrap();
        let outcome = testing::run_on(&super::super::wire::EqMac, |out| {
            feature.handle(
                &ClientCommand::AnswerTranslocate {
                    session_id: 5,
                    accept: false,
                },
                &mut world,
                out,
            )
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [eqmac_translocate_answer(&offer, false)]);
    }
}
