//! What the player says, in any channel, and the item links they read in
//! chat and ask the server to describe.
use super::{
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::{chat::OutboundChat, world::SpawnKind};

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
            ClientCommand::SendChat(_)
                | ClientCommand::SendChatInAdmission { .. }
                | ClientCommand::InspectItem { .. }
                | ClientCommand::DepopLocalNpc { .. }
        )
    }

    /// Chat and inspections are encoded directly. Fixture cleanup rechecks
    /// the admission and target before sending `EQEmu`'s bare depop command.
    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if matches!(command, ClientCommand::SendChatInAdmission { session_id, .. } if *session_id != world.session_id)
        {
            return super::actions::refuse(
                command,
                "That was meant for an earlier zone visit",
                out.log,
            );
        }
        if let ClientCommand::DepopLocalNpc {
            session_id, target, ..
        } = *command
        {
            if !world.local_eqemu() {
                return super::actions::refuse(
                    command,
                    "Fixture cleanup requires local EQEmu",
                    out.log,
                );
            }
            let current = world.target.state();
            if session_id != world.session_id
                || target != current
                || !current.available
                || !current
                    .spawn_id
                    .and_then(|id| world.spawns.visible(id))
                    .is_some_and(|spawn| {
                        spawn.kind == SpawnKind::Npc && !world.is_player(spawn.spawn_id)
                    })
            {
                return super::actions::refuse(
                    command,
                    "The fixture NPC target has changed or is unavailable",
                    out.log,
                );
            }
            return out
                .command(&ClientCommand::SendChat(OutboundChat::Say("#depop".into())))
                .map(drop);
        }
        out.command(command).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;

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

    #[test]
    fn scoped_chat_refuses_an_earlier_admission_even_at_the_talk_boundary() {
        let mut talk = Talk;
        let mut world = World::new(8);
        let queued = ClientCommand::SendChatInAdmission {
            session_id: 7,
            message: OutboundChat::Say("synthetic".into()),
        };
        let output = testing::run(|out| talk.handle(&queued, &mut world, out));
        output.result.unwrap();
        assert!(output.sent.is_empty());
        assert!(output.events.iter().any(|event| matches!(event, crate::client::ClientEvent::Diagnostic(message) if message.contains("earlier zone visit"))));
    }
}
