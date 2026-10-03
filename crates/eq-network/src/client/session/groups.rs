//! Groups. The session keeps the invitation waiting for an answer, which
//! following or declining answers once, and refuses an answer with none
//! waiting; `/disband` with an invitation waiting declines it, as it does in
//! the official client (inferred). It keeps the player's group as the
//! server describes it, to refuse an invitation as the official client does
//! (inferred from its strings for them): one that names no one, one from a
//! member who is not the leader, and one to a full group. Who leaves, is
//! removed or disbands the group is the server's to decide, by its idea of
//! the player's target. The server does not answer what the session sends,
//! so the session says what it sent.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{group::GroupUpdate, message::Message, request::Request, world::WorldEvent};

/// The official client's strings for the invitations it refuses: one that
/// names no one, one from a member who is not the leader, and one to a full
/// group (`eqstr_us.txt`).
const NO_ONE_NAMED: u32 = 12268;
const NOT_THE_LEADER: u32 = 12267;
const FULL: u32 = 12275;

/// How many members a group holds besides the player.
const OTHERS: usize = 5;

/// The player's group, as the server described it.
#[derive(Default)]
struct Group {
    /// Its leader, once named.
    leader: Option<String>,
    /// The other members.
    members: Vec<String>,
}

/// Invites, joins, declines, leaves and disbands.
#[derive(Default)]
pub(super) struct Groups {
    /// Who invited the player last, until the player answers or is in a
    /// group.
    invitation: Option<String>,
    /// The player's group, while they are in one.
    group: Option<Group>,
}

impl Groups {
    /// Why the player may not invite someone, in this library's words and
    /// the official client's string for it; None when they may.
    fn refusal(&self, name: &str, player: &str) -> Option<(&'static str, u32)> {
        if name.is_empty() {
            return Some(("Name a player to invite, or target one.", NO_ONE_NAMED));
        }
        let group = self.group.as_ref()?;
        if group
            .leader
            .as_ref()
            .is_some_and(|leader| !leader.eq_ignore_ascii_case(player))
        {
            return Some(("Only the group's leader may invite.", NOT_THE_LEADER));
        }
        (group.members.len() >= OTHERS).then_some(("The group is full.", FULL))
    }

    /// Follows the server's word on the player's group.
    fn follow_news(&mut self, update: &GroupUpdate, player: &str) {
        match update {
            GroupUpdate::Invited { inviter } => self.invitation = Some(inviter.clone()),
            GroupUpdate::Formed => {
                self.group = Some(Group {
                    leader: Some(player.to_owned()),
                    members: Vec::new(),
                });
            }
            GroupUpdate::Joined { member } => {
                let members = &mut self.group.get_or_insert_with(Group::default).members;
                if !members.iter().any(|name| name.eq_ignore_ascii_case(member)) {
                    members.push(member.clone());
                }
            }
            GroupUpdate::Left { member } if member.eq_ignore_ascii_case(player) => {
                self.group = None;
            }
            GroupUpdate::Left { member } => {
                if let Some(group) = self.group.as_mut() {
                    group
                        .members
                        .retain(|name| !name.eq_ignore_ascii_case(member));
                }
            }
            GroupUpdate::Members { leader, members } => {
                self.group = Some(Group {
                    leader: Some(leader.clone()),
                    members: members.clone(),
                });
            }
            GroupUpdate::Leader { name } => {
                if let Some(group) = self.group.as_mut() {
                    group.leader = Some(name.clone());
                }
            }
            GroupUpdate::Disbanded => self.group = None,
            GroupUpdate::Inviting { .. }
            | GroupUpdate::Following { .. }
            | GroupUpdate::Declining { .. }
            | GroupUpdate::Accepted { .. }
            | GroupUpdate::Declined { .. } => {}
        }
        // Being in a group answers any invitation.
        if self.group.is_some() {
            self.invitation = None;
        }
    }
}

/// Sends a request, then says it was sent.
fn send(request: &Request, sent: GroupUpdate, out: &mut Out<'_, '_>) -> Result<()> {
    out.request(request)?;
    out.log.send(ClientEvent::World(WorldEvent::Group(sent)))
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
                if let Some((reason, string_id)) = self.refusal(name, out.sender.name) {
                    return actions::refuse_officially(command, (reason, Some(string_id)), out.log);
                }
                send(
                    &Request::InviteToGroup(name.to_owned()),
                    GroupUpdate::Inviting {
                        player: name.to_owned(),
                    },
                    out,
                )
            }
            ClientCommand::FollowGroup { .. } => {
                let Some(inviter) = self.invitation.take() else {
                    return actions::refuse(command, "No group invitation is waiting.", out.log);
                };
                send(
                    &Request::FollowGroup(inviter.clone()),
                    GroupUpdate::Following { inviter },
                    out,
                )
            }
            ClientCommand::DeclineGroup { .. } => {
                let Some(inviter) = self.invitation.take() else {
                    return actions::refuse(command, "No group invitation is waiting.", out.log);
                };
                send(
                    &Request::DeclineGroup(inviter.clone()),
                    GroupUpdate::Declining { inviter },
                    out,
                )
            }
            ClientCommand::Disband { .. } => match self.invitation.take() {
                Some(inviter) => send(
                    &Request::DeclineGroup(inviter.clone()),
                    GroupUpdate::Declining { inviter },
                    out,
                ),
                None => out.request(&Request::Disband),
            },
            _ => Ok(()),
        }
    }

    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::Event(WorldEvent::Group(update)) = message {
            self.follow_news(update, out.sender.name);
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

    /// The official string a refused command names, if it was refused.
    fn refused(groups: &mut Groups, world: &mut World, command: &ClientCommand) -> Option<u32> {
        let outcome = testing::run(|out| groups.handle(command, world, out));
        assert_eq!(outcome.sent, []);
        outcome.events.iter().find_map(|event| match event {
            ClientEvent::World(WorldEvent::GroupRefused { string_id, .. }) => *string_id,
            _ => None,
        })
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
        // Following sends the answer and says so.
        let followed = testing::run(|out| groups.handle(&follow, &mut world, out));
        assert_eq!(followed.sent, [group::follow("Leader", "Tester").unwrap()]);
        assert!(followed.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Group(GroupUpdate::Following { inviter }))
                if inviter == "Leader"
        )));
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
        let invited = testing::run(|out| groups.handle(&invite(" Friend "), &mut world, out));
        assert_eq!(invited.sent, [group::invite("Friend", "Tester").unwrap()]);
        assert!(invited.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Group(GroupUpdate::Inviting { player }))
                if player == "Friend"
        )));
        assert_eq!(
            refused(&mut groups, &mut world, &invite("  ")),
            Some(NO_ONE_NAMED)
        );
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

    #[test]
    fn only_the_leader_of_a_group_with_room_invites() {
        let mut groups = Groups::default();
        let mut world = World::new(5);
        let invite = ClientCommand::InviteToGroup {
            session_id: 5,
            name: "Friend".into(),
        };
        // A member who is not the leader may not invite.
        hear(
            &mut groups,
            &mut world,
            GroupUpdate::Members {
                leader: "Other".into(),
                members: vec!["Other".into()],
            },
        );
        assert_eq!(
            refused(&mut groups, &mut world, &invite),
            Some(NOT_THE_LEADER)
        );
        // Made the leader, they may.
        hear(
            &mut groups,
            &mut world,
            GroupUpdate::Leader {
                name: "Tester".into(),
            },
        );
        assert_eq!(sent(&mut groups, &mut world, &invite).len(), 1);
        // Out of the group, anyone may form one.
        hear(
            &mut groups,
            &mut world,
            GroupUpdate::Left {
                member: "Tester".into(),
            },
        );
        assert_eq!(sent(&mut groups, &mut world, &invite).len(), 1);
        // A group the player formed holds five others at most.
        hear(&mut groups, &mut world, GroupUpdate::Formed);
        for member in ["One", "Two", "Three", "Four", "Five"] {
            hear(
                &mut groups,
                &mut world,
                GroupUpdate::Joined {
                    member: member.into(),
                },
            );
        }
        assert_eq!(refused(&mut groups, &mut world, &invite), Some(FULL));
        hear(
            &mut groups,
            &mut world,
            GroupUpdate::Left {
                member: "Two".into(),
            },
        );
        assert_eq!(sent(&mut groups, &mut world, &invite).len(), 1);
        // Disbanded, the player is in no group.
        hear(&mut groups, &mut world, GroupUpdate::Disbanded);
        assert!(groups.group.is_none());
    }
}
