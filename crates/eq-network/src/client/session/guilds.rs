//! The player's guild: asking for its message of the day, answering an
//! invitation into it, and, as an officer or its leader, inviting and
//! removing members, setting the message, handing on the lead and demoting.
//!
//! The server decides who may do what (`EQEmu`'s `Client::Handle_OP_GuildInvite`,
//! `Handle_OP_GuildRemove`, `Handle_OP_SetGuildMOTD`, `Handle_OP_GuildLeader`
//! and `Handle_OP_GuildDemote`) and says so in its own lines, so the session
//! checks only what a request cannot be built without: a guild to name, an
//! invitation to answer, and, for a member the request names, that the
//! member list the server sent holds them, spelled as the list spells them
//! (inferred, recording checklist 6.411). An answer goes once, to the
//! invitation that waits; a new invitation replaces an older one.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::{
    guild::{GuildMember, GuildUpdate},
    listing::GuildRank,
    message::Message,
    request::Request,
    world::WorldEvent,
};

/// The official client's line when `/getguildmotd` finds no guild
/// (0x4667dc).
const NO_GUILD: u32 = 0x32a6;
/// The highest guild number the official client asks about.
const LAST_GUILD: u32 = 1499;

/// The invitation waiting for an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Invitation {
    inviter: String,
    guild_id: u32,
    rank: GuildRank,
}

/// The player's guild as the session follows it.
#[derive(Default)]
pub(super) struct Guilds {
    /// The members the server last listed, by name.
    members: Option<Vec<String>>,
    invitation: Option<Invitation>,
}

impl Guilds {
    /// Follows a guild update: a full member list, a member joining or
    /// leaving, or an invitation.
    fn follow(&mut self, message: &Message) {
        let Message::Event(WorldEvent::Guild(update)) = message else {
            return;
        };
        match update {
            GuildUpdate::Members(members) => {
                self.members = Some(
                    members
                        .iter()
                        .map(|GuildMember { name, .. }| name.clone())
                        .collect(),
                );
            }
            GuildUpdate::MemberAdded { name, .. } => {
                if let Some(members) = &mut self.members {
                    if !members.iter().any(|member| member == name) {
                        members.push(name.clone());
                    }
                }
            }
            GuildUpdate::MemberRemoved { name, .. } => {
                if let Some(members) = &mut self.members {
                    members.retain(|member| member != name);
                }
            }
            GuildUpdate::Invite {
                inviter,
                guild_id,
                rank,
            } => {
                self.invitation = Some(Invitation {
                    inviter: inviter.clone(),
                    guild_id: *guild_id,
                    rank: *rank,
                });
            }
            _ => (),
        }
    }

    /// The member a request names, spelled as the member list spells them.
    fn member(&self, name: &str) -> Result<String, &'static str> {
        let Some(members) = &self.members else {
            return Err("Wait for your guild's member list.");
        };
        members
            .iter()
            .find(|member| member.eq_ignore_ascii_case(name.trim()))
            .cloned()
            .ok_or("No member of your guild has that name.")
    }

    /// The request a command makes, or why it cannot be made, with the
    /// official client's line where it has one.
    fn request(
        &mut self,
        command: &ClientCommand,
        world: &World,
    ) -> Result<Request, (&'static str, Option<u32>)> {
        let unofficial = |reason| (reason, None);
        let guild = world
            .player
            .as_ref()
            .and_then(|player| player.listing.guild);
        let in_guild = || guild.ok_or(unofficial("You are not in a guild."));
        Ok(match command {
            ClientCommand::RequestGuildMotd { .. } => Request::GuildMotd(
                guild
                    .filter(|guild| *guild <= LAST_GUILD)
                    .ok_or(("You are not in a guild.", Some(NO_GUILD)))?,
            ),
            ClientCommand::AnswerGuildInvite { accept, .. } => {
                let invitation = self
                    .invitation
                    .take()
                    .ok_or(unofficial("No guild invitation is waiting for an answer."))?;
                Request::AnswerGuildInvite {
                    inviter: invitation.inviter,
                    guild_id: invitation.guild_id,
                    rank: invitation.rank,
                    accept: *accept,
                }
            }
            ClientCommand::GuildInvite { name, .. } => {
                let guild_id = in_guild()?;
                let name = name.trim();
                if name.is_empty() {
                    return Err(unofficial("Name the player to invite."));
                }
                Request::GuildInvite {
                    name: name.to_owned(),
                    guild_id,
                }
            }
            ClientCommand::GuildRemove { name, .. } => {
                let guild_id = in_guild()?;
                Request::GuildRemove {
                    name: self.member(name).map_err(unofficial)?,
                    guild_id,
                }
            }
            ClientCommand::SetGuildMotd { motd, .. } => {
                in_guild()?;
                Request::SetGuildMotd(motd.trim().to_owned())
            }
            ClientCommand::GuildMakeLeader { name, .. } => {
                in_guild()?;
                Request::GuildMakeLeader(self.member(name).map_err(unofficial)?)
            }
            ClientCommand::GuildDemote { name, .. } => {
                in_guild()?;
                Request::GuildDemote(self.member(name).map_err(unofficial)?)
            }
            _ => return Err(unofficial("Not a guild command.")),
        })
    }
}

impl Feature for Guilds {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![Capability::GuildRoster, Capability::GuildManagement]
    }

    fn admit(&mut self, message: &Message, _world: &mut World) -> Result<()> {
        self.follow(message);
        Ok(())
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::RequestGuildMotd { .. }
                | ClientCommand::AnswerGuildInvite { .. }
                | ClientCommand::GuildInvite { .. }
                | ClientCommand::GuildRemove { .. }
                | ClientCommand::SetGuildMotd { .. }
                | ClientCommand::GuildMakeLeader { .. }
                | ClientCommand::GuildDemote { .. }
        )
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match self.request(command, world) {
            Ok(request) => match out.encode(&request) {
                Ok(packet) => out.send(&packet),
                Err(_) => actions::refuse(command, "That guild request cannot be sent.", out.log),
            },
            Err(official) => actions::refuse_officially(command, official, out.log),
        }
    }

    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.follow(message);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::guild;

    fn guild_world(guild_id: Option<u32>) -> World {
        let mut world = World::new(5);
        let mut player = testing::player(7);
        player.listing.guild = guild_id;
        world.player.admit(player);
        world.own_spawn = Some(7);
        world
    }

    fn members(names: &[&str]) -> Message {
        Message::Event(WorldEvent::Guild(GuildUpdate::Members(
            names
                .iter()
                .map(|name| GuildMember {
                    name: (*name).into(),
                    level: 10,
                    class: 1,
                    rank: GuildRank::Member,
                    banker: false,
                    alt: false,
                    last_on: 0,
                    tribute: guild::GuildTribute {
                        on: false,
                        total: 0,
                        last: 0,
                    },
                    note: String::new(),
                    zone_id: None,
                })
                .collect(),
        )))
    }

    fn refusal(events: &[crate::client::ClientEvent]) -> Option<String> {
        events.iter().find_map(|event| match event {
            crate::client::ClientEvent::World(WorldEvent::GuildRefused { reason, .. }) => {
                Some(reason.clone())
            }
            _ => None,
        })
    }

    #[test]
    fn an_invitation_is_answered_once_with_what_it_offered() {
        let mut feature = Guilds::default();
        let mut world = guild_world(None);
        let answer = |accept| ClientCommand::AnswerGuildInvite {
            session_id: 5,
            accept,
        };
        let outcome = testing::run(|out| feature.handle(&answer(true), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert_eq!(
            refusal(&outcome.events).as_deref(),
            Some("No guild invitation is waiting for an answer.")
        );
        let invite = Message::Event(WorldEvent::Guild(GuildUpdate::Invite {
            inviter: "Inviter".into(),
            guild_id: 3,
            rank: GuildRank::Member,
        }));
        testing::run(|out| feature.observe(&invite, &mut world, out))
            .result
            .unwrap();
        let outcome = testing::run(|out| feature.handle(&answer(false), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [
                guild::titanium_answer_invite("Inviter", "Tester", 3, GuildRank::Member, false)
                    .unwrap()
            ]
        );
        let outcome = testing::run(|out| feature.handle(&answer(true), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [], "a second answer must not go");
    }

    #[test]
    fn management_names_members_as_the_list_spells_them() {
        let mut feature = Guilds::default();
        let mut world = guild_world(Some(3));
        let remove = |name: &str| ClientCommand::GuildRemove {
            session_id: 5,
            name: name.into(),
        };
        let outcome = testing::run(|out| feature.handle(&remove("member"), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            refusal(&outcome.events).as_deref(),
            Some("Wait for your guild's member list.")
        );
        feature
            .admit(&members(&["Member", "Other"]), &mut world)
            .unwrap();
        let outcome = testing::run(|out| feature.handle(&remove("member"), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [guild::titanium_remove("Member", "Tester", 3).unwrap()]
        );
        let outcome = testing::run(|out| feature.handle(&remove("Nobody"), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        let removed = Message::Event(WorldEvent::Guild(GuildUpdate::MemberRemoved {
            guild_id: 3,
            name: "Other".into(),
        }));
        testing::run(|out| feature.observe(&removed, &mut world, out))
            .result
            .unwrap();
        let demote = ClientCommand::GuildDemote {
            session_id: 5,
            name: "Other".into(),
        };
        let outcome = testing::run(|out| feature.handle(&demote, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        let leader = ClientCommand::GuildMakeLeader {
            session_id: 5,
            name: "MEMBER".into(),
        };
        let outcome = testing::run(|out| feature.handle(&leader, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [guild::titanium_make_leader("Tester", "Member").unwrap()]
        );
    }

    #[test]
    fn guild_requests_need_a_guild_and_the_motd_question_names_it() {
        let mut feature = Guilds::default();
        let mut world = guild_world(None);
        let ask = ClientCommand::RequestGuildMotd { session_id: 5 };
        let outcome = testing::run(|out| feature.handle(&ask, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            crate::client::ClientEvent::World(WorldEvent::GuildRefused {
                string_id: Some(NO_GUILD),
                ..
            })
        )));
        for command in [
            ClientCommand::GuildInvite {
                session_id: 5,
                name: "Friend".into(),
            },
            ClientCommand::SetGuildMotd {
                session_id: 5,
                motd: "Hello".into(),
            },
        ] {
            let outcome = testing::run(|out| feature.handle(&command, &mut world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent, []);
            assert_eq!(
                refusal(&outcome.events).as_deref(),
                Some("You are not in a guild.")
            );
        }
        let mut world = guild_world(Some(3));
        let outcome = testing::run(|out| feature.handle(&ask, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [guild::titanium_get_motd(3, "Tester").unwrap()]
        );
        let invite = ClientCommand::GuildInvite {
            session_id: 5,
            name: " Friend ".into(),
        };
        let outcome = testing::run(|out| feature.handle(&invite, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [guild::titanium_invite("Friend", "Tester", 3, GuildRank::Member).unwrap()]
        );
        let motd = ClientCommand::SetGuildMotd {
            session_id: 5,
            motd: "x".repeat(600),
        };
        let outcome = testing::run(|out| feature.handle(&motd, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert_eq!(
            refusal(&outcome.events).as_deref(),
            Some("That guild request cannot be sent.")
        );
    }
}
