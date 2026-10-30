//! Answering the server's end of the player's hiding, where the server
//! waits for the client's word before the zone sees the player again.
//!
//! `EQEmu` ends a hidden player's sneaking as it sends `OP_CancelSneakHide`
//! (a spell on them, a trap, looting, an item's use or a trade ended it),
//! but keeps them hidden until the Titanium client answers with its own
//! appearance, invisibility 0, as its source says that client does
//! (`zone/client_process.cpp` `Client::CancelSneakHide`; not yet recorded
//! from the client). Without the answer the player stays hidden from the
//! zone, NPCs included, until they move. The session answers so where the
//! server type waits for it: `EQEmu`. P99 hears nothing back until its
//! hiding is checked live there, and TAKP and Quarm end the hiding
//! themselves before they tell the player.
use super::feature::{Feature, Out, World};
use anyhow::Result;
use eq_network_game::{message::Message, request::Request, world::WorldEvent};

/// Answers the server's end of the player's hiding.
pub(super) struct HideAnswer;

impl Feature for HideAnswer {
    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if !matches!(message, Message::Event(WorldEvent::HideAndSneakEnded)) {
            return Ok(());
        }
        // The server ends only a hidden player's hiding, so their spawn is
        // known; an answer that cannot be made all the same is only noted.
        match out.encode(&Request::Unhide) {
            Ok(packet) => out.send(&packet),
            Err(error) => out
                .log
                .diagnostic(format!("Did not answer the end of hiding: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;

    #[test]
    fn the_end_of_hiding_is_answered_with_the_players_own_invisibility_0() {
        let mut world = World::new(5);
        let ended = Message::Event(WorldEvent::HideAndSneakEnded);
        let outcome = testing::run(|out| HideAnswer.observe(&ended, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [eq_network_game::abilities::titanium_unhide(7).unwrap()]
        );
        // Nothing else is answered.
        let spun = Message::Event(WorldEvent::Spun);
        let outcome = testing::run(|out| HideAnswer.observe(&spun, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
    }

    #[test]
    fn an_answer_that_cannot_be_made_is_only_noted() {
        // EQMac's generation carries no such answer.
        let mut world = World::new(5);
        let ended = Message::Event(WorldEvent::HideAndSneakEnded);
        let outcome = testing::run_on(&super::super::wire::EqMac, |out| {
            HideAnswer.observe(&ended, &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert!(matches!(
            &outcome.events[..],
            [ClientEvent::Diagnostic(line)] if line.starts_with("Did not answer the end of hiding")
        ));
    }
}
