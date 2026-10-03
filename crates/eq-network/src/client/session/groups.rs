//! Groups. The session keeps the invitation waiting for an answer, which
//! following or declining answers once, and refuses an answer with none
//! waiting; `/disband` with an invitation waiting declines it, as it does in
//! the official client (inferred). Inviting needs a name. Who leaves, is
//! removed or disbands the group is the server's to decide, by its idea of
//! the player's target.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::{group::GroupUpdate, message::Message, request::Request, world::WorldEvent};

/// Invites, joins, declines, leaves and disbands.
#[derive(Default)]
pub(super) struct Groups {
    /// Who invited the player last, until the player answers or joins a
    /// group.
    invitation: Option<String>,
}

impl Feature for Groups {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Grouping]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::InviteToGroup { .. }
                | ClientCommand::FollowGroup { .. }
                | ClientCommand::DeclineGroup { .. }
                | ClientCommand::Disband { .. }
        )
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match command {
            ClientCommand::InviteToGroup { name, .. } => {
                let name = name.trim();
                if name.is_empty() {
                    return actions::refuse(command, "Name a player to invite.", out.log);
                }
                out.request(&Request::InviteToGroup(name.to_owned()))
            }
            ClientCommand::FollowGroup { .. } => {
                let Some(inviter) = self.invitation.take() else {
                    return actions::refuse(command, "No group invitation is waiting.", out.log);
                };
                out.request(&Request::FollowGroup(inviter))
            }
            ClientCommand::DeclineGroup { .. } => {
                let Some(inviter) = self.invitation.take() else {
                    return actions::refuse(command, "No group invitation is waiting.", out.log);
                };
                out.request(&Request::DeclineGroup(inviter))
            }
            ClientCommand::Disband { .. } => match self.invitation.take() {
                Some(inviter) => out.request(&Request::DeclineGroup(inviter)),
                None => out.request(&Request::Disband),
            },
            _ => Ok(()),
        }
    }

    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::Event(WorldEvent::Group(update)) = message {
            match update {
                GroupUpdate::Invited { inviter } => self.invitation = Some(inviter.clone()),
                // Being in a group answers any invitation.
                GroupUpdate::Members { .. } => self.invitation = None,
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;
    use eq_network_game::group;

    /// What a command sends.
    fn sent(
        groups: &mut Groups,
        world: &mut World,
        command: &ClientCommand,
    ) -> Vec<eq_network_game::command::EncodedCommand> {
        testing::run(|out| groups.handle(command, world, out)).sent
    }

    /// The server's word on groups, as the session hears it.
    fn hear(groups: &mut Groups, world: &mut World, update: GroupUpdate) {
        testing::run(|out| groups.observe(&Message::Event(WorldEvent::Group(update)), world, out))
            .result
            .unwrap();
    }

    #[test]
    fn an_invitation_is_answered_once_and_disband_declines_one() {
        let mut groups = Groups::default();
        let mut world = World::new(5);
        let follow = ClientCommand::FollowGroup { session_id: 5 };
        // Nothing waits yet, so following is refused.
        let refused = testing::run(|out| groups.handle(&follow, &mut world, out));
        assert_eq!(refused.sent, []);
        assert!(refused
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::GroupRefused { .. }))));
        let invited = || GroupUpdate::Invited {
            inviter: "Leader".into(),
        };
        hear(&mut groups, &mut world, invited());
        assert_eq!(
            sent(&mut groups, &mut world, &follow),
            [group::follow("Leader", "Tester").unwrap()]
        );
        // The answer was given once.
        assert_eq!(sent(&mut groups, &mut world, &follow), []);
        // With an invitation waiting, /disband declines it.
        hear(&mut groups, &mut world, invited());
        let disband = ClientCommand::Disband { session_id: 5 };
        assert_eq!(
            sent(&mut groups, &mut world, &disband),
            [group::decline("Leader", "Tester").unwrap()]
        );
        // Without one, it leaves or disbands as the server decides.
        assert_eq!(
            sent(&mut groups, &mut world, &disband),
            [group::disband("Tester").unwrap()]
        );
    }

    #[test]
    fn an_invitation_names_a_player_and_being_in_a_group_answers_any() {
        let mut groups = Groups::default();
        let mut world = World::new(5);
        let invite = |name: &str| ClientCommand::InviteToGroup {
            session_id: 5,
            name: name.into(),
        };
        assert_eq!(
            sent(&mut groups, &mut world, &invite(" Friend ")),
            [group::invite("Friend", "Tester").unwrap()]
        );
        assert_eq!(sent(&mut groups, &mut world, &invite("  ")), []);
        hear(
            &mut groups,
            &mut world,
            GroupUpdate::Invited {
                inviter: "Leader".into(),
            },
        );
        hear(
            &mut groups,
            &mut world,
            GroupUpdate::Members {
                leader: "Other".into(),
                members: vec!["Other".into()],
            },
        );
        let decline = ClientCommand::DeclineGroup { session_id: 5 };
        assert_eq!(sent(&mut groups, &mut world, &decline), []);
    }
}
