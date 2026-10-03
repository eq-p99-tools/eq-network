//! Raids. The server keeps no invitation (`EQEmu`'s
//! `Client::Handle_OP_RaidCommand`), so the session keeps the one waiting
//! for an answer: accepting sends the inviter's name back as the server gave
//! it, and declining sends nothing, as there is nothing to tell. The session
//! keeps the player's raid as the server describes it, to refuse an
//! invitation as the official client does (inferred from its having strings
//! for them, since `EQEmu` checks none of the inviter's side; it does refuse
//! an invitee already in a raid, and a grouped one who does not lead their
//! group): one that names no one, one to
//! the player themself, which `EQEmu` would turn into a broken raid, one to
//! a member, and one from a member who is not the leader. The server does
//! not pass the player's own raid chat back to them, so the session records
//! it as the others hear it, as the official client shows it (inferred).
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use crate::client::RecordEvent;
use anyhow::Result;
use eq_network_game::{
    chat::OutboundChat, message::Message, raid::RaidUpdate, request::Request, world::WorldEvent,
};

/// The official client's strings for the invitations it refuses: one that
/// names no one, one from a member who is not the leader, one to the player
/// themself, and one to a member (`eqstr_us.txt`).
const NO_ONE_NAMED: u32 = 5074;
const NOT_THE_LEADER: u32 = 5073;
const ONESELF: u32 = 5076;
const ALREADY_IN: u32 = 5077;

/// The player's raid, as the server described it.
#[derive(Default)]
struct Raid {
    /// Its leader, once named.
    leader: Option<String>,
    /// Its members, the player among them, by the names the server gave.
    members: Vec<String>,
}

/// Invites, accepts, declines and leaves.
#[derive(Default)]
pub(super) struct Raids {
    /// Who invited the player last, until the player answers or is in a
    /// raid.
    invitation: Option<String>,
    /// The player's raid, while they are in one.
    raid: Option<Raid>,
}

impl Raids {
    /// Why the player may not invite someone, in this library's words and
    /// the official client's string for it; None when they may.
    fn refusal(&self, name: &str, player: &str) -> Option<(&'static str, u32)> {
        if name.is_empty() {
            return Some(("Name a player to invite, or target one.", NO_ONE_NAMED));
        }
        if name.eq_ignore_ascii_case(player) {
            return Some(("You cannot invite yourself.", ONESELF));
        }
        let raid = self.raid.as_ref()?;
        if raid
            .members
            .iter()
            .any(|member| member.eq_ignore_ascii_case(name))
        {
            return Some(("They are in your raid already.", ALREADY_IN));
        }
        raid.leader
            .as_ref()
            .filter(|leader| !leader.eq_ignore_ascii_case(player))
            .map(|_| ("Only the raid's leader may invite.", NOT_THE_LEADER))
    }

    /// Follows the server's word on the player's raid.
    fn follow_news(&mut self, update: &RaidUpdate, player: &str) {
        match update {
            RaidUpdate::Invited { inviter } => self.invitation = Some(inviter.clone()),
            RaidUpdate::Created { leader } => {
                self.raid = Some(Raid {
                    leader: Some(leader.clone()),
                    members: Vec::new(),
                });
            }
            RaidUpdate::Added(member) => {
                let members = &mut self.raid.get_or_insert_with(Raid::default).members;
                if !members.contains(&member.name) {
                    members.push(member.name.clone());
                }
            }
            RaidUpdate::Removed { member } if member.eq_ignore_ascii_case(player) => {
                self.raid = None;
            }
            RaidUpdate::Removed { member } => {
                if let Some(raid) = self.raid.as_mut() {
                    raid.members.retain(|name| name != member);
                }
            }
            RaidUpdate::Disbanded => self.raid = None,
            RaidUpdate::Leader { name } => {
                if let Some(raid) = self.raid.as_mut() {
                    raid.leader = Some(name.clone());
                }
            }
            RaidUpdate::Inviting { .. }
            | RaidUpdate::Accepting { .. }
            | RaidUpdate::Declining { .. }
            | RaidUpdate::Leaving => {}
        }
        // Being in a raid answers any invitation.
        if self.raid.is_some() {
            self.invitation = None;
        }
    }
}

/// Says what the session did, which the server does not answer.
fn said(update: RaidUpdate, out: &mut Out<'_, '_>) -> Result<()> {
    out.log.send(ClientEvent::World(WorldEvent::Raid(update)))
}

impl Feature for Raids {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Raiding]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::RaidInvite { .. }
                | ClientCommand::RaidAccept { .. }
                | ClientCommand::RaidDecline { .. }
                | ClientCommand::RaidLeave { .. }
        )
    }

    /// The player's own raid chat, which the server passes to every other
    /// member but not back to them: recorded as the others hear it.
    fn notice(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if !matches!(command, ClientCommand::SendChat(OutboundChat::Raid(_))) || self.raid.is_none()
        {
            return Ok(());
        }
        // The line the player sends reads as the line the server would pass
        // on: the same struct, under the player's name.
        let packet = out.encode_command(command)?;
        if let Some(event) = out.wire.chat(packet.opcode, &packet.body, false)? {
            let zone = out.log.zone.clone();
            out.log.record(&zone, RecordEvent::Chat(event))?;
        }
        Ok(())
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let player = out.sender.name;
        match command {
            ClientCommand::RaidInvite { name, .. } => {
                let name = name.trim();
                if let Some((reason, string_id)) = self.refusal(name, player) {
                    return actions::refuse_officially(command, (reason, Some(string_id)), out.log);
                }
                out.request(&Request::RaidInvite(name.to_owned()))?;
                said(
                    RaidUpdate::Inviting {
                        player: name.to_owned(),
                    },
                    out,
                )
            }
            ClientCommand::RaidAccept { .. } => {
                let Some(inviter) = self.invitation.take() else {
                    return actions::refuse(command, "No raid invitation is waiting.", out.log);
                };
                out.request(&Request::RaidAccept(inviter.clone()))?;
                said(RaidUpdate::Accepting { inviter }, out)
            }
            ClientCommand::RaidDecline { .. } => {
                let Some(inviter) = self.invitation.take() else {
                    return actions::refuse(command, "No raid invitation is waiting.", out.log);
                };
                said(RaidUpdate::Declining { inviter }, out)
            }
            ClientCommand::RaidLeave { .. } => {
                if self.raid.is_none() {
                    return actions::refuse(command, "You are in no raid.", out.log);
                }
                out.request(&Request::RaidLeave)?;
                said(RaidUpdate::Leaving, out)
            }
            _ => Ok(()),
        }
    }

    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::Event(WorldEvent::Raid(update)) = message {
            self.follow_news(update, out.sender.name);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;
    use eq_network_game::raid::{self, RaidMember};

    /// What a command sends.
    fn sent(
        raids: &mut Raids,
        world: &mut World,
        command: &ClientCommand,
    ) -> Vec<eq_network_game::command::EncodedCommand> {
        testing::run(|out| raids.handle(command, world, out)).sent
    }

    /// The official string a refused command names, if any.
    fn refused(raids: &mut Raids, world: &mut World, command: &ClientCommand) -> Option<u32> {
        let outcome = testing::run(|out| raids.handle(command, world, out));
        assert_eq!(outcome.sent, []);
        outcome.events.iter().find_map(|event| match event {
            ClientEvent::World(WorldEvent::RaidRefused { string_id, .. }) => *string_id,
            _ => None,
        })
    }

    /// The server's word on raids, as the session hears it.
    fn hear(raids: &mut Raids, world: &mut World, update: RaidUpdate) {
        testing::run(|out| raids.observe(&Message::Event(WorldEvent::Raid(update)), world, out))
            .result
            .unwrap();
    }

    fn member(name: &str) -> RaidUpdate {
        RaidUpdate::Added(RaidMember {
            name: name.into(),
            group: None,
            class: 2,
            level: 30,
            group_leader: false,
        })
    }

    #[test]
    fn an_invitation_is_answered_once_by_accepting_or_declining() {
        let mut raids = Raids::default();
        let mut world = World::new(5);
        let accept = ClientCommand::RaidAccept { session_id: 5 };
        assert_eq!(sent(&mut raids, &mut world, &accept), []);
        let invited = || RaidUpdate::Invited {
            inviter: "Leader".into(),
        };
        hear(&mut raids, &mut world, invited());
        assert_eq!(
            sent(&mut raids, &mut world, &accept),
            [raid::accept("Leader", "Tester").unwrap()]
        );
        // Declining tells the server nothing, but answers the invitation.
        hear(&mut raids, &mut world, invited());
        let decline = ClientCommand::RaidDecline { session_id: 5 };
        let declined = testing::run(|out| raids.handle(&decline, &mut world, out));
        assert_eq!(declined.sent, []);
        assert!(declined.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Raid(RaidUpdate::Declining { .. }))
        )));
        assert_eq!(sent(&mut raids, &mut world, &accept), []);
    }

    #[test]
    fn invitations_are_refused_as_the_official_client_refuses_them() {
        let mut raids = Raids::default();
        let mut world = World::new(5);
        let invite = |name: &str| ClientCommand::RaidInvite {
            session_id: 5,
            name: name.into(),
        };
        assert_eq!(
            sent(&mut raids, &mut world, &invite(" Friend ")),
            [raid::invite("Friend", "Tester").unwrap()]
        );
        assert_eq!(
            refused(&mut raids, &mut world, &invite("")),
            Some(NO_ONE_NAMED)
        );
        assert_eq!(
            refused(&mut raids, &mut world, &invite("tester")),
            Some(ONESELF)
        );
        // In a raid someone else leads, the player may not invite.
        hear(
            &mut raids,
            &mut world,
            RaidUpdate::Created {
                leader: "Leader".into(),
            },
        );
        hear(&mut raids, &mut world, member("Leader"));
        hear(&mut raids, &mut world, member("Tester"));
        assert_eq!(
            refused(&mut raids, &mut world, &invite("Friend")),
            Some(NOT_THE_LEADER)
        );
        assert_eq!(
            refused(&mut raids, &mut world, &invite("Leader")),
            Some(ALREADY_IN)
        );
        hear(
            &mut raids,
            &mut world,
            RaidUpdate::Leader {
                name: "Tester".into(),
            },
        );
        assert_eq!(sent(&mut raids, &mut world, &invite("Friend")).len(), 1);
        // Leaving sends the player's own name twice; out of a raid, it is
        // refused.
        let leave = ClientCommand::RaidLeave { session_id: 5 };
        assert_eq!(
            sent(&mut raids, &mut world, &leave),
            [raid::remove("Tester", "Tester").unwrap()]
        );
        hear(
            &mut raids,
            &mut world,
            RaidUpdate::Removed {
                member: "Tester".into(),
            },
        );
        assert_eq!(sent(&mut raids, &mut world, &leave), []);
    }

    #[test]
    fn the_players_own_raid_chat_is_heard_as_the_others_hear_it() {
        let mut raids = Raids::default();
        let mut world = World::new(5);
        let chat = ClientCommand::SendChat(OutboundChat::Raid("Ready?".into()));
        let records = |raids: &mut Raids, world: &mut World| {
            testing::run(|out| raids.notice(&chat, world, out))
                .events
                .iter()
                .filter(|event| matches!(event, ClientEvent::Record(_)))
                .count()
        };
        // Out of a raid, the server drops it, and nothing is heard.
        assert_eq!(records(&mut raids, &mut world), 0);
        hear(
            &mut raids,
            &mut world,
            RaidUpdate::Created {
                leader: "Tester".into(),
            },
        );
        assert_eq!(records(&mut raids, &mut world), 1);
    }
}
