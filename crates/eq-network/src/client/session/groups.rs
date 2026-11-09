//! Group consent and server-reported membership. Titanium's native senders
//! (0x49d3cc, 0x49d654, 0x49d7b6, 0x49d8bd) distinguish initial and existing
//! groups. P99 uses that path; emulator compatibility senders stay separate.
//! A submitted answer is not membership. Consent uses an admission plus a
//! local monotonic generation, never a fabricated wire identifier.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    group::{GroupUpdate, InvitationId, InvitationKind, InvitationRefusal, TitaniumRequest},
    message::Message,
    request::Request,
    world::WorldEvent,
};

const NO_ONE_NAMED: u32 = 12268;
const NOT_THE_LEADER: u32 = 12267;
const FULL: u32 = 12275;
const OTHERS: usize = 5;

#[derive(Default)]
struct Group {
    leader: Option<String>,
    members: Vec<String>,
}

impl Group {
    fn listed(leader: &str, members: &[String], player: &str) -> Option<Self> {
        let mut group = Self {
            leader: (!leader.is_empty()).then(|| leader.to_owned()),
            members: Vec::new(),
        };
        for member in members {
            if !member.is_empty()
                && !member.eq_ignore_ascii_case(player)
                && !group
                    .members
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(member))
            {
                group.members.push(member.clone());
            }
        }
        (group.leader.is_some() || !group.members.is_empty()).then_some(group)
    }
}

struct Invitation {
    inviter: String,
    kind: InvitationKind,
    id: InvitationId,
}

/// One owner for this admission's invitations and membership guards.
#[derive(Default)]
pub(super) struct Groups {
    native: bool,
    invitation: Option<Invitation>,
    next_generation: u64,
    joining: bool,
    rejected: Option<(String, InvitationRefusal)>,
    group: Option<Group>,
    membership_known: bool,
    staged: Vec<GroupUpdate>,
}

impl Groups {
    /// Native Titanium requests, without changing emulator compatibility.
    pub(super) fn titanium() -> Self {
        Self {
            native: true,
            ..Self::default()
        }
    }

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

    fn follow_news(&mut self, update: &GroupUpdate, player: &str) {
        match update {
            GroupUpdate::Invited {
                inviter,
                player: invitee,
                kind,
                id: Some(id),
            } if invitee.eq_ignore_ascii_case(player)
                && !inviter.is_empty()
                && self.invitation.is_none()
                && self.group.is_none()
                && !self.joining =>
            {
                self.invitation = Some(Invitation {
                    inviter: inviter.clone(),
                    kind: *kind,
                    id: *id,
                });
            }
            GroupUpdate::Formed { leader } => {
                self.membership_known = true;
                self.group.get_or_insert_with(Group::default).leader =
                    (!leader.is_empty()).then(|| leader.clone());
            }
            GroupUpdate::Joined { member } if !member.is_empty() => {
                self.membership_known = true;
                let group = self.group.get_or_insert_with(Group::default);
                if !member.eq_ignore_ascii_case(player)
                    && !group
                        .members
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(member))
                {
                    group.members.push(member.clone());
                }
            }
            GroupUpdate::Left { member } if member.eq_ignore_ascii_case(player) => {
                self.membership_known = true;
                self.group = None;
                self.invitation = None;
                self.joining = false;
            }
            GroupUpdate::Left { member } => {
                if let Some(group) = self.group.as_mut() {
                    group
                        .members
                        .retain(|name| !name.eq_ignore_ascii_case(member));
                    if group
                        .leader
                        .as_ref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(member))
                    {
                        group.leader = None;
                    }
                }
            }
            GroupUpdate::Members { leader, members } => {
                self.membership_known = true;
                self.group = Group::listed(leader, members, player);
                self.joining = false;
                self.invitation = None;
            }
            GroupUpdate::Leader { name } => {
                if let Some(group) = self.group.as_mut() {
                    group.leader = (!name.is_empty()).then(|| name.clone());
                }
            }
            GroupUpdate::Disbanded => {
                self.membership_known = true;
                self.group = None;
                self.joining = false;
                self.invitation = None;
            }
            _ => {}
        }
        if self.group.is_some() {
            self.invitation = None;
            self.joining = false;
        }
    }

    fn answer(&mut self, accept: bool, out: &mut Out<'_, '_>) -> Result<()> {
        let Some(invitation) = &self.invitation else {
            return Ok(());
        };
        let inviter = invitation.inviter.clone();
        let request = if self.native {
            Request::TitaniumGroup(if accept {
                TitaniumRequest::Follow {
                    inviter: inviter.clone(),
                    kind: invitation.kind,
                }
            } else {
                TitaniumRequest::Decline {
                    inviter: inviter.clone(),
                }
            })
        } else if accept {
            Request::FollowGroup(inviter.clone())
        } else {
            Request::DeclineGroup(inviter.clone())
        };
        // A failed submission retains the offer. A successful submission
        // consumes consent once, while the outcome remains unknown.
        out.request(&request)?;
        self.invitation = None;
        self.joining = accept;
        let sent = if accept {
            GroupUpdate::Following { inviter }
        } else {
            GroupUpdate::Declining { inviter }
        };
        out.log.send(ClientEvent::World(WorldEvent::Group(sent)))
    }
}

impl Feature for Groups {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Grouping]
    }

    fn explain(&mut self, message: &mut Message, world: &World) {
        let Message::Event(WorldEvent::Group(GroupUpdate::Invited {
            inviter,
            player,
            id,
            ..
        })) = message
        else {
            return;
        };
        // Offers before admission, addressed elsewhere, or malformed cannot
        // acquire a consent identity or cause an automatic response.
        if !world.ready()
            || !world
                .player
                .as_ref()
                .is_some_and(|own| own.name.eq_ignore_ascii_case(player))
            || inviter.is_empty()
        {
            *message = Message::Withheld;
            return;
        }
        let refusal = if self.invitation.is_some() {
            Some(InvitationRefusal::Waiting)
        } else if self.group.is_some() || self.joining {
            Some(InvitationRefusal::Unavailable)
        } else {
            None
        };
        if let Some(reason) = refusal {
            self.rejected = Some((inviter.clone(), reason));
            *message = Message::Withheld;
        } else if let Some(next) = self
            .next_generation
            .checked_add(1)
            .and_then(InvitationId::new)
        {
            self.next_generation = next.get();
            *id = Some(next);
        } else {
            *message = Message::Withheld;
        }
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
                if self.native && !self.membership_known {
                    return actions::refuse(
                        command,
                        "Group membership has not been reported yet.",
                        out.log,
                    );
                }
                if let Some((reason, string_id)) = self.refusal(name, out.sender.name) {
                    return actions::refuse_officially(command, (reason, Some(string_id)), out.log);
                }
                if self.native
                    && (self.joining || self.group.as_ref().is_some_and(|g| g.leader.is_none()))
                {
                    return actions::refuse(
                        command,
                        "Group leadership has not been reported yet.",
                        out.log,
                    );
                }
                let leading = self
                    .group
                    .as_ref()
                    .and_then(|group| group.leader.as_ref())
                    .is_some_and(|leader| leader.eq_ignore_ascii_case(out.sender.name));
                let request = if self.native {
                    Request::TitaniumGroup(TitaniumRequest::Invite {
                        player: name.to_owned(),
                        leading,
                    })
                } else {
                    Request::InviteToGroup {
                        name: name.to_owned(),
                        leading,
                    }
                };
                out.request(&request)?;
                out.log.send(ClientEvent::World(WorldEvent::Group(
                    GroupUpdate::Inviting {
                        player: name.to_owned(),
                    },
                )))
            }
            ClientCommand::FollowGroup {
                inviter,
                invitation_id,
                ..
            }
            | ClientCommand::DeclineGroup {
                inviter,
                invitation_id,
                ..
            } => {
                if !self.invitation.as_ref().is_some_and(|offer| {
                    *invitation_id == Some(offer.id) && offer.inviter.eq_ignore_ascii_case(inviter)
                }) {
                    return actions::refuse(
                        command,
                        "That group invitation is no longer waiting.",
                        out.log,
                    );
                }
                self.answer(matches!(command, ClientCommand::FollowGroup { .. }), out)
            }
            ClientCommand::Disband { .. } if self.invitation.is_some() => self.answer(false, out),
            ClientCommand::Disband { .. } => out.request(&if self.native {
                Request::TitaniumGroup(TitaniumRequest::Leave)
            } else {
                Request::Disband
            }),
            _ => Ok(()),
        }
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Some((inviter, reason)) = self.rejected.take() {
            if self.native {
                out.request(&Request::TitaniumGroup(TitaniumRequest::Refuse {
                    inviter,
                    reason,
                }))?;
            }
        }
        if let Message::Event(WorldEvent::Group(update)) = message {
            self.follow_news(update, out.sender.name);
            world.grouped = self.group.is_some();
        }
        Ok(())
    }

    fn admit(&mut self, message: &Message, _world: &mut World) -> Result<()> {
        if let Message::Event(WorldEvent::Group(
            update @ (GroupUpdate::Members { .. }
            | GroupUpdate::Leader { .. }
            | GroupUpdate::Joined { .. }
            | GroupUpdate::Left { .. }
            | GroupUpdate::Formed { .. }
            | GroupUpdate::Disbanded),
        )) = message
        {
            self.staged.push(update.clone());
        }
        Ok(())
    }

    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        for update in std::mem::take(&mut self.staged) {
            self.follow_news(&update, out.sender.name);
            world.grouped = self.group.is_some();
            out.log
                .send(ClientEvent::World(WorldEvent::Group(update)))?;
        }
        Ok(())
    }

    fn connection_ended(&mut self, world: &mut World) {
        world.grouped = false;
        self.invitation = None;
        self.rejected = None;
        self.group = None;
        self.membership_known = false;
        self.staged.clear();
        self.joining = false;
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
        if world.player.is_none() {
            let mut player = testing::player(7);
            player.name = "Tester".into();
            world.player.admit(player);
        }
        world.admitted = Some(std::time::Instant::now());
        let mut message = Message::Event(WorldEvent::Group(update));
        groups.explain(&mut message, world);
        testing::run(|out| groups.observe(&message, world, out))
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
        let follow = ClientCommand::FollowGroup {
            session_id: 5,
            inviter: "Leader".into(),
            invitation_id: InvitationId::new(1),
        };
        // Nothing waits yet, so following is refused.
        let refused = testing::run(|out| groups.handle(&follow, &mut world, out));
        assert_eq!(refused.sent, []);
        assert!(refused
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::GroupRefused { .. }))));
        let invited = || GroupUpdate::Invited {
            inviter: "Leader".into(),
            player: "Tester".into(),
            kind: InvitationKind::Initial,
            id: None,
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
        assert!(groups.group.is_none(), "submission is not membership");
        // A server reset resolves the unanswered outcome before another offer.
        hear(&mut groups, &mut world, GroupUpdate::Disbanded);
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
                player: "Tester".into(),
                kind: InvitationKind::Initial,
                id: None,
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
        let decline = ClientCommand::DeclineGroup {
            session_id: 5,
            inviter: "Leader".into(),
            invitation_id: InvitationId::new(1),
        };
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
        // A list naming no leader leaves it unknown, so the server decides.
        hear(
            &mut groups,
            &mut world,
            GroupUpdate::Members {
                leader: String::new(),
                members: vec!["Other".into()],
            },
        );
        assert_eq!(sent(&mut groups, &mut world, &invite).len(), 1);
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
        hear(
            &mut groups,
            &mut world,
            GroupUpdate::Formed {
                leader: "Tester".into(),
            },
        );
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

    #[test]
    fn a_group_listed_before_the_admission_is_told_after_it() {
        let mut groups = Groups::default();
        let mut world = World::new(5);
        let listed = GroupUpdate::Members {
            leader: String::new(),
            members: ["One", "Two", "Three", "Four", "Five"]
                .map(String::from)
                .to_vec(),
        };
        groups
            .admit(
                &Message::Event(WorldEvent::Group(listed.clone())),
                &mut world,
            )
            .unwrap();
        let outcome = testing::run(|out| groups.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            &outcome.events[..],
            [ClientEvent::World(WorldEvent::Group(told))] if *told == listed
        ));
        // The session keeps it too: the group is full.
        let invite = ClientCommand::InviteToGroup {
            session_id: 5,
            name: "Friend".into(),
        };
        assert_eq!(refused(&mut groups, &mut world, &invite), Some(FULL));
        // Told once.
        let outcome = testing::run(|out| groups.admitted(&mut world, out));
        assert!(outcome.events.is_empty());
    }
    fn native_offer(
        groups: &mut Groups,
        world: &mut World,
        opcode: u16,
        inviter: &str,
    ) -> testing::Outcome<Result<()>> {
        if world.player.is_none() {
            let mut player = testing::player(7);
            player.name = "Tester".into();
            world.player.admit(player);
        }
        world.admitted = Some(std::time::Instant::now());
        let packet = group::invite("Tester", inviter).unwrap();
        let update = group::decode(opcode, &packet.body).unwrap().unwrap();
        let mut message = Message::Event(WorldEvent::Group(update));
        groups.explain(&mut message, world);
        testing::run(|out| groups.observe(&message, world, out))
    }

    fn consent(groups: &Groups, accept: bool) -> ClientCommand {
        let offer = groups.invitation.as_ref().unwrap();
        if accept {
            ClientCommand::FollowGroup {
                session_id: 5,
                inviter: offer.inviter.clone(),
                invitation_id: Some(offer.id),
            }
        } else {
            ClientCommand::DeclineGroup {
                session_id: 5,
                inviter: offer.inviter.clone(),
                invitation_id: Some(offer.id),
            }
        }
    }

    #[test]
    fn native_second_offers_preserve_consent_and_same_inviter_replacement_gets_a_new_id() {
        let mut groups = Groups::titanium();
        let mut world = World::new(5);
        let first = native_offer(&mut groups, &mut world, 0x1b48, "Leader");
        first.result.unwrap();
        assert!(
            first.sent.is_empty(),
            "an ordinary offer is not auto-accepted"
        );
        let old_accept = consent(&groups, true);
        let old_decline = consent(&groups, false);
        let first_id = groups.invitation.as_ref().unwrap().id;
        for inviter in ["Leader", "Other"] {
            let duplicate = native_offer(&mut groups, &mut world, 0x12d6, inviter);
            duplicate.result.unwrap();
            assert_eq!(duplicate.sent.len(), 1);
            assert_eq!(
                (duplicate.sent[0].opcode, duplicate.sent[0].body[128]),
                (0x1f27, 1)
            );
            let kept = groups.invitation.as_ref().unwrap();
            assert_eq!(
                (kept.inviter.as_str(), kept.id, kept.kind),
                ("Leader", first_id, InvitationKind::Initial)
            );
        }
        assert_eq!(sent(&mut groups, &mut world, &old_decline)[0].body[128], 3);
        native_offer(&mut groups, &mut world, 0x12d6, "Leader")
            .result
            .unwrap();
        assert!(groups.invitation.as_ref().unwrap().id.get() > first_id.get());
        for stale in [&old_accept, &old_decline] {
            let refused = testing::run(|out| groups.handle(stale, &mut world, out));
            refused.result.unwrap();
            assert_eq!(refused.sent, []);
            assert!(refused
                .events
                .iter()
                .any(|event| matches!(event, ClientEvent::World(WorldEvent::GroupRefused { .. }))));
        }
        let accept = consent(&groups, true);
        let answered = sent(&mut groups, &mut world, &accept);
        assert_eq!(answered.len(), 1);
        assert_eq!(answered[0].opcode, 0x42c9);
        assert!(groups.invitation.is_none());
        assert!(groups.group.is_none(), "submission is not membership");
        assert!(groups.joining);
        assert_eq!(sent(&mut groups, &mut world, &accept), []);
        let while_joining = native_offer(&mut groups, &mut world, 0x1b48, "Other");
        while_joining.result.unwrap();
        assert_eq!(while_joining.sent[0].body[128], 2);
        assert!(groups.group.is_none());
        hear(
            &mut groups,
            &mut world,
            GroupUpdate::Members {
                leader: "Leader".into(),
                members: vec!["Leader".into(), "Tester".into(), "leader".into()],
            },
        );
        assert!(!groups.joining);
        assert_eq!(groups.group.as_ref().unwrap().members, ["Leader"]);
        let grouped = native_offer(&mut groups, &mut world, 0x1b48, "Other");
        grouped.result.unwrap();
        assert_eq!(grouped.sent[0].body[128], 2);
        assert!(groups.invitation.is_none());
    }

    #[test]
    fn native_invites_follow_reported_leadership_and_leave_waits_for_server_news() {
        let mut groups = Groups::titanium();
        let mut world = World::new(5);
        let invite = ClientCommand::InviteToGroup {
            session_id: 5,
            name: "Friend".into(),
        };
        assert!(
            sent(&mut groups, &mut world, &invite).is_empty(),
            "absence is not known solo membership"
        );
        hear(&mut groups, &mut world, GroupUpdate::Disbanded);
        assert_eq!(sent(&mut groups, &mut world, &invite)[0].opcode, 0x1b48);
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
        hear(
            &mut groups,
            &mut world,
            GroupUpdate::Members {
                leader: String::new(),
                members: vec!["Other".into()],
            },
        );
        assert!(
            sent(&mut groups, &mut world, &invite).is_empty(),
            "unknown leader does not select the forming-group opcode"
        );
        hear(
            &mut groups,
            &mut world,
            GroupUpdate::Leader {
                name: "Tester".into(),
            },
        );
        assert_eq!(sent(&mut groups, &mut world, &invite)[0].opcode, 0x12d6);
        for member in ["Two", "Three", "Four", "Five"] {
            hear(
                &mut groups,
                &mut world,
                GroupUpdate::Joined {
                    member: member.into(),
                },
            );
        }
        assert_eq!(refused(&mut groups, &mut world, &invite), Some(FULL));
        let leave = ClientCommand::Disband { session_id: 5 };
        assert_eq!(sent(&mut groups, &mut world, &leave)[0].opcode, 0x0e76);
        assert!(
            groups.group.is_some(),
            "submission does not remove membership"
        );
        hear(
            &mut groups,
            &mut world,
            GroupUpdate::Left {
                member: "Tester".into(),
            },
        );
        assert!(groups.group.is_none());
        assert_eq!(sent(&mut groups, &mut world, &invite)[0].opcode, 0x1b48);
    }

    struct FailingSink;
    impl super::super::feature::Sink for FailingSink {
        fn send(&mut self, _: &eq_network_game::command::EncodedCommand) -> Result<()> {
            anyhow::bail!("synthetic submission failure")
        }
        fn send_unreliable(&mut self, _: &eq_network_game::command::EncodedCommand) -> Result<()> {
            anyhow::bail!("unexpected unreliable group send")
        }
        fn last_received_seconds(&self) -> u64 {
            0
        }
    }

    #[test]
    fn failed_answers_retain_consent_without_claiming_membership_or_retrying() {
        for accept in [false, true] {
            let mut groups = Groups::titanium();
            let mut world = World::new(5);
            native_offer(&mut groups, &mut world, 0x12d6, "Leader")
                .result
                .unwrap();
            let command = consent(&groups, accept);
            let before = groups.invitation.as_ref().unwrap().id;
            let failed = testing::run(|out| {
                let mut failure = FailingSink;
                let mut failing_out = Out {
                    sink: &mut failure,
                    log: &mut *out.log,
                    wire: out.wire,
                    sender: out.sender,
                };
                groups.handle(&command, &mut world, &mut failing_out)
            });
            assert!(failed.result.is_err());
            assert!(failed.events.is_empty());
            assert_eq!(groups.invitation.as_ref().unwrap().id, before);
            assert!(!groups.joining);
            assert!(groups.group.is_none());
            let idle = testing::run(|out| groups.tick(std::time::Instant::now(), &mut world, out));
            idle.result.unwrap();
            assert!(idle.sent.is_empty(), "no automatic retry");
            groups.connection_ended(&mut world);
            assert!(groups.invitation.is_none());
            assert_eq!(sent(&mut groups, &mut world, &command), []);
        }
    }

    #[test]
    fn malformed_unaddressed_preadmission_and_exhausted_offers_send_nothing() {
        let mut groups = Groups::titanium();
        let mut world = World::new(5);
        let packet = group::invite("Tester", "Leader").unwrap();
        let mut message = Message::Event(WorldEvent::Group(
            group::decode(packet.opcode, &packet.body).unwrap().unwrap(),
        ));
        groups.explain(&mut message, &world);
        assert!(matches!(message, Message::Withheld));
        let mut own = testing::player(7);
        own.name = "SomeoneElse".into();
        world.player.admit(own);
        let mut message = Message::Event(WorldEvent::Group(
            group::decode(packet.opcode, &packet.body).unwrap().unwrap(),
        ));
        groups.explain(&mut message, &world);
        assert!(matches!(message, Message::Withheld));
        let result = testing::run(|out| groups.observe(&message, &mut world, out));
        result.result.unwrap();
        assert_eq!(result.sent, []);
        let mut own = testing::player(7);
        own.name = "Tester".into();
        world.player.admit(own);
        groups.next_generation = u64::MAX;
        let exhausted = native_offer(&mut groups, &mut world, 0x1b48, "Leader");
        exhausted.result.unwrap();
        assert_eq!(exhausted.sent, []);
        assert!(groups.invitation.is_none());
        let malformed = Message::Unreadable {
            part: eq_network_game::message::Part::World,
            error: "synthetic malformed invite".into(),
        };
        let outcome = testing::run(|out| groups.observe(&malformed, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
    }
    #[test]
    fn native_profile_and_later_leader_news_are_staged_in_order() {
        let mut groups = Groups::titanium();
        let mut world = World::new(5);
        let mut profile = vec![0; 19592];
        profile[12940..12946].copy_from_slice(b"Tester");
        profile[13280..13286].copy_from_slice(b"Tester");
        profile[13344..13349].copy_from_slice(b"Other");
        let listed = group::titanium_profile(&profile).unwrap().unwrap();
        groups
            .admit(&Message::Event(WorldEvent::Group(listed)), &mut world)
            .unwrap();
        groups
            .admit(
                &Message::Event(WorldEvent::Group(GroupUpdate::Leader {
                    name: "Tester".into(),
                })),
                &mut world,
            )
            .unwrap();
        assert!(!groups.membership_known);
        let restored = testing::run(|out| groups.admitted(&mut world, out));
        restored.result.unwrap();
        assert_eq!(restored.sent, []);
        assert_eq!(
            groups.group.as_ref().unwrap().leader.as_deref(),
            Some("Tester")
        );
        let invite = ClientCommand::InviteToGroup {
            session_id: 5,
            name: "Friend".into(),
        };
        assert_eq!(sent(&mut groups, &mut world, &invite)[0].opcode, 0x12d6);
        groups.connection_ended(&mut world);
        assert!(!groups.membership_known);
        assert!(groups.group.is_none());
        assert_eq!(sent(&mut groups, &mut world, &invite), []);
    }
    #[test]
    fn formation_retains_earlier_members_and_uses_the_reported_leader() {
        let mut groups = Groups::titanium();
        let mut world = World::new(5);
        hear(
            &mut groups,
            &mut world,
            GroupUpdate::Joined {
                member: "Friend".into(),
            },
        );
        hear(
            &mut groups,
            &mut world,
            GroupUpdate::Formed {
                leader: "Other".into(),
            },
        );
        assert_eq!(groups.group.as_ref().unwrap().members, ["Friend"]);
        assert_eq!(
            groups.group.as_ref().unwrap().leader.as_deref(),
            Some("Other")
        );
        let invite = ClientCommand::InviteToGroup {
            session_id: 5,
            name: "Another".into(),
        };
        assert_eq!(
            refused(&mut groups, &mut world, &invite),
            Some(NOT_THE_LEADER)
        );
    }
}
