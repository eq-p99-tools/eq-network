//! What the player says, in any channel, and the item links they read in
//! chat and ask the server to describe.
use super::{
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::{chat::OutboundChat, world::SpawnKind};

/// Sends chat and item-link inspections.
pub(super) struct Talk {
    /// The character's one pace for all chat (6.523).
    pace: super::chat_pace::SharedPace,
}

impl Talk {
    pub(super) const fn new(pace: super::chat_pace::SharedPace) -> Self {
        Self { pace }
    }
}

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
        let (ClientCommand::SendChat(chat)
        | ClientCommand::SendChatInAdmission { message: chat, .. }) = command
        else {
            return out.command(command).map(drop);
        };
        match super::chat_pace::paced(
            &self.pace,
            super::chat_pace::Kind::of(chat),
            std::time::Instant::now(),
            || out.command(command).map(drop),
        ) {
            Ok(sent) => sent,
            Err(paced) => {
                let reason = paced.reason();
                out.log.send(crate::client::ClientEvent::World(
                    eq_network_game::world::WorldEvent::ChatRefused {
                        session_id: world.session_id,
                        reason: reason.clone(),
                    },
                ))?;
                super::actions::refuse(command, &reason, out.log)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;

    #[test]
    fn chat_goes_out_in_the_dialect() {
        let mut talk = Talk::new(super::super::chat_pace::SharedPace::default());
        let mut world = World::new(5);
        let say = ClientCommand::SendChat(OutboundChat::Say("Hail".into()));
        let outcome = testing::run(|out| talk.handle(&say, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, 0x1004);
    }

    /// One pace holds all chat a host asks for, across zone sessions; a
    /// paced line sends nothing, is told by `ChatRefused` without its words
    /// and is not counted (6.523).
    #[test]
    fn chat_past_the_pace_is_refused_and_told_across_zones() {
        use super::super::chat_pace::{SharedPace, ANY_CHAT};
        use eq_network_game::world::WorldEvent;
        let pace = SharedPace::default();
        let mut talk = Talk::new(std::sync::Arc::clone(&pace));
        let mut world = World::new(5);
        let say = |text: &str| ClientCommand::SendChat(OutboundChat::Say(text.into()));
        for line in 0..ANY_CHAT - 1 {
            let outcome = testing::run(|out| talk.handle(&say("synthetic"), &mut world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent.len(), 1, "{line}");
        }
        // The next zone's session shares the character's pace.
        let mut next_zone = Talk::new(std::sync::Arc::clone(&pace));
        let mut world = World::new(6);
        let last = testing::run(|out| next_zone.handle(&say("synthetic"), &mut world, out));
        last.result.unwrap();
        assert_eq!(last.sent.len(), 1);
        for _ in 0..3 {
            let paced =
                testing::run(|out| next_zone.handle(&say("a private line"), &mut world, out));
            paced.result.unwrap();
            assert_eq!(paced.sent, []);
            let told = paced.events.iter().find_map(|event| match event {
                crate::client::ClientEvent::World(WorldEvent::ChatRefused {
                    session_id,
                    reason,
                }) => Some((*session_id, reason.clone())),
                _ => None,
            });
            let (session_id, reason) = told.expect("told");
            assert_eq!(session_id, 6);
            assert!(reason.starts_with("You are sending chat too fast. Wait "));
            assert!(!reason.contains("private"));
        }
    }

    #[test]
    fn scoped_chat_refuses_an_earlier_admission_even_at_the_talk_boundary() {
        let mut talk = Talk::new(super::super::chat_pace::SharedPace::default());
        let mut world = World::new(8);
        let queued = ClientCommand::SendChatInAdmission {
            session_id: 7,
            message: OutboundChat::Say("synthetic".into()),
            tongue: eq_network_game::chat::Tongue::COMMON,
        };
        let output = testing::run(|out| talk.handle(&queued, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent, []);
        assert!(output.events.iter().any(|event| matches!(event, crate::client::ClientEvent::Diagnostic(message) if message.contains("earlier zone visit"))));
    }
}
