//! What the player says, in any channel, and the item links they read in
//! chat and ask the server to describe.
use super::{
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;

/// Sends chat and item-link inspections.
pub(super) struct Talk;

impl Feature for Talk {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![Capability::Talking]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::SendChat(_) | ClientCommand::InspectItem { .. }
        )
    }

    /// Needs nothing from the session: the server answers an inspection with
    /// the item's details, which reach the host with the rest of the zone's news.
    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        out.command(command).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::chat::OutboundChat;

    #[test]
    fn chat_goes_out_in_the_dialect() {
        let mut talk = Talk;
        let mut world = World::new(5);
        let say = ClientCommand::SendChat(OutboundChat::Say("Hail".into()));
        let outcome = testing::run(|out| talk.handle(&say, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, 0x1004);
    }
}
