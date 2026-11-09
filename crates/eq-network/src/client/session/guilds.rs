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
//! (inferred, recording checklist 6.411). Each answer goes once, to the
//! invitation of the inviter it names; a new invitation from the same
//! inviter replaces their older one. The host decides the answer, as the
//! official client does from its dialog and its own state (program read
//! R11). `/guildmotd` with nothing, or with 512 bytes or more, sends an
//! empty message, as the program does.
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
    /// The invitations waiting for an answer, oldest first.
    invitations: Vec<Invitation>,
}

/// The most invitations kept waiting: the oldest goes when another comes.
const INVITATIONS: usize = 8;

/// The longest message of the day the official client sends; a longer one
/// goes empty (0x4666e3).
const MOTD_LIMIT: usize = 512;

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
                self.invitations
                    .retain(|invitation| invitation.inviter != *inviter);
                if self.invitations.len() == INVITATIONS {
                    self.invitations.remove(0);
                }
                self.invitations.push(Invitation {
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
            ClientCommand::AnswerGuildInvite {
                inviter, answer, ..
            } => {
                let index = self
                    .invitations
                    .iter()
                    .position(|invitation| invitation.inviter == *inviter)
                    .ok_or(unofficial(
                        "No guild invitation from them is waiting for an answer.",
                    ))?;
                let invitation = self.invitations.remove(index);
                Request::AnswerGuildInvite {
                    inviter: invitation.inviter,
                    guild_id: invitation.guild_id,
                    rank: invitation.rank,
                    answer: *answer,
                }
            }
            ClientCommand::GuildInvite { name, officer, .. } => {
                let guild_id = in_guild()?;
                let name = name.trim();
                if name.is_empty() {
                    return Err(unofficial("Name the player to invite."));
                }
                Request::GuildInvite {
                    name: name.to_owned(),
                    guild_id,
                    officer: *officer,
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
                let motd = motd.trim();
                Request::SetGuildMotd(if motd.len() >= MOTD_LIMIT {
                    String::new()
                } else {
                    motd.to_owned()
                })
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
    use eq_network_game::guild::{self, InviteAnswer};

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
        let answer = |inviter: &str, answer| ClientCommand::AnswerGuildInvite {
            session_id: 5,
            inviter: inviter.into(),
            answer,
        };
        let outcome = testing::run(|out| {
            feature.handle(&answer("Inviter", InviteAnswer::Accept), &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert_eq!(
            refusal(&outcome.events).as_deref(),
            Some("No guild invitation from them is waiting for an answer.")
        );
        let invite = Message::Event(WorldEvent::Guild(GuildUpdate::Invite {
            inviter: "Inviter".into(),
            guild_id: 3,
            rank: GuildRank::Member,
        }));
        testing::run(|out| feature.observe(&invite, &mut world, out))
            .result
            .unwrap();
        let other = Message::Event(WorldEvent::Guild(GuildUpdate::Invite {
            inviter: "Second".into(),
            guild_id: 4,
            rank: GuildRank::Officer,
        }));
        testing::run(|out| feature.observe(&other, &mut world, out))
            .result
            .unwrap();
        // Each invitation is answered by its inviter, with what it offered.
        let outcome = testing::run(|out| {
            feature.handle(&answer("Second", InviteAnswer::Busy), &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [guild::titanium_answer_invite(
                "Second",
                "Tester",
                4,
                GuildRank::Officer,
                InviteAnswer::Busy
            )
            .unwrap()]
        );
        let outcome = testing::run(|out| {
            feature.handle(&answer("Inviter", InviteAnswer::Decline), &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [guild::titanium_answer_invite(
                "Inviter",
                "Tester",
                3,
                GuildRank::Member,
                InviteAnswer::Decline
            )
            .unwrap()]
        );
        let outcome = testing::run(|out| {
            feature.handle(&answer("Inviter", InviteAnswer::Accept), &mut world, out)
        });
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
                officer: false,
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
            officer: true,
        };
        let outcome = testing::run(|out| feature.handle(&invite, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [guild::titanium_invite("Friend", "Tester", 3, GuildRank::Officer).unwrap()]
        );
        let motd = ClientCommand::SetGuildMotd {
            session_id: 5,
            motd: "x".repeat(600),
        };
        let outcome = testing::run(|out| feature.handle(&motd, &mut world, out));
        outcome.result.unwrap();
        // The program sends a message of 512 bytes or more as an empty one.
        assert_eq!(
            outcome.sent,
            [guild::titanium_set_motd("Tester", "").unwrap()]
        );
        let motd = ClientCommand::SetGuildMotd {
            session_id: 5,
            motd: "line\nbreak".into(),
        };
        let outcome = testing::run(|out| feature.handle(&motd, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            refusal(&outcome.events).as_deref(),
            Some("That guild request cannot be sent.")
        );
    }
}
