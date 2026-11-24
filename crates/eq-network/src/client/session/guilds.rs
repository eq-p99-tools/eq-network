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
use std::sync::{Arc, Mutex};

/// The official client's line when `/getguildmotd` finds no guild
/// (0x4667dc).
const NO_GUILD: u32 = 0x32a6;
/// The highest guild number the official client asks about.
const LAST_GUILD: u32 = 1499;

/// Received canonical names awaiting this character's offered destination.
/// This belongs to one login attempt, is never persisted, and holds no rank
/// authority or invitations.
#[derive(Clone, Default)]
pub(in crate::client) struct RosterTransfer(Arc<Mutex<Option<Transfer>>>);

struct Transfer {
    character: String,
    guild_id: u32,
    session_id: u64,
    destination: (u16, u16),
    members: Vec<String>,
}

impl RosterTransfer {
    /// A character-list visit ends the previous character's evidence.
    pub(super) fn clear(&self) {
        *self.0.lock().expect("guild transfer poisoned") = None;
    }
}

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
    /// The admitted character and their current received guild ID.
    identity: Option<(String, u32)>,
    /// Canonical names may follow only a bound transfer of this character.
    transfer: RosterTransfer,
    /// Guild news received before the admitted identity can bind it.
    staged: Vec<GuildUpdate>,
    /// The invitations waiting for an answer, oldest first.
    invitations: Vec<Invitation>,
}

/// The most invitations kept waiting: the oldest goes when another comes.
const INVITATIONS: usize = 8;

/// The longest message of the day the official client sends; a longer one
/// goes empty (0x4666e3).
const MOTD_LIMIT: usize = 512;

impl Guilds {
    /// A zone's guild worker, sharing only explicitly bound transfer evidence.
    pub(super) fn new(transfer: RosterTransfer) -> Self {
        Self {
            transfer,
            ..Self::default()
        }
    }

    fn identity(world: &World) -> Option<(String, u32)> {
        let player = world.player.as_ref()?;
        Some((player.name.clone(), player.listing.guild?))
    }

    /// Only a received list supplies names; a different membership withdraws
    /// them even when a later update restores the old guild number.
    fn membership(&mut self, world: &World) {
        let identity = Self::identity(world);
        if self.identity != identity {
            self.members = None;
            self.invitations.clear();
            self.transfer.clear();
        }
        self.identity = identity;
    }

    /// Bind the last received names to the actual lifecycle's destination.
    /// Approval consumes the pending offer, so keep that binding while the
    /// old zone departs; a denial or ordinary active state withdraws it.
    fn transferring(&self, world: &World) {
        let mut retained = self.transfer.0.lock().expect("guild transfer poisoned");
        if let Some(offer) = world.lifecycle.pending() {
            *retained = self.identity.as_ref().zip(self.members.as_ref()).map(
                |((character, guild_id), members)| Transfer {
                    character: character.clone(),
                    guild_id: *guild_id,
                    session_id: world.session_id,
                    destination: (offer.zone_id, offer.instance_id),
                    members: members.clone(),
                },
            );
        } else if !world.lifecycle.departing() || self.members.is_none() {
            *retained = None;
        } else if let Some(retained) = retained.as_mut() {
            // Membership deltas received after approval remain authoritative.
            if let Some(members) = &self.members {
                retained.members.clone_from(members);
            }
        }
    }

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
        if let Message::Event(WorldEvent::Guild(update)) = message {
            self.staged.push(update.clone());
        }
        Ok(())
    }

    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        self.identity = Self::identity(world);
        let retained = self
            .transfer
            .0
            .lock()
            .expect("guild transfer poisoned")
            .take();
        if self.identity.is_none() {
            self.members = None;
        } else if self.members.is_none() {
            self.members = retained
                .filter(|retained| {
                    retained.session_id != world.session_id
                        && retained.destination == world.zone
                        && self.identity.as_ref().is_some_and(|(name, guild)| {
                            *name == retained.character && *guild == retained.guild_id
                        })
                })
                .map(|retained| retained.members);
        }
        // Apply admission-time changes after inheritance, in received order.
        // The host must hear them too, once its Entered event bound the roster.
        for update in std::mem::take(&mut self.staged) {
            let message = Message::Event(WorldEvent::Guild(update.clone()));
            self.observe(&message, world, out)?;
            out.log
                .send(super::ClientEvent::World(WorldEvent::Guild(update)))?;
        }
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

    fn after_command(&mut self, world: &World) {
        // A host crossing can be approved by the very next receive. Capture
        // its destination before that reply consumes the pending offer.
        self.membership(world);
        self.transferring(world);
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.membership(world);
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
        world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.membership(world);
        let guild = self.identity.as_ref().map(|(_, guild)| *guild);
        match message {
            Message::Event(WorldEvent::Guild(
                GuildUpdate::MemberAdded { guild_id, .. }
                | GuildUpdate::MemberRemoved { guild_id, .. }
                | GuildUpdate::MemberRenamed { guild_id, .. }
                | GuildUpdate::Disbanded { guild_id },
            )) if Some(*guild_id) != guild => return Ok(()),
            _ => (),
        }
        self.follow(message);
        match message {
            Message::Event(WorldEvent::Guild(GuildUpdate::MemberRenamed {
                old_name,
                new_name,
                ..
            })) => {
                if let Some(members) = &mut self.members {
                    for member in members.iter_mut().filter(|member| *member == old_name) {
                        member.clone_from(new_name);
                    }
                }
            }
            Message::Event(WorldEvent::Guild(GuildUpdate::Disbanded { .. })) => {
                self.members = None;
                self.transfer.clear();
            }
            Message::Event(WorldEvent::Guild(GuildUpdate::MemberRemoved { name, .. }))
                if self.identity.as_ref().is_some_and(|(own, _)| own == name) =>
            {
                self.members = None;
                self.transfer.clear();
            }
            _ => (),
        }
        self.transferring(world);
        Ok(())
    }

    fn tick(
        &mut self,
        _now: std::time::Instant,
        world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if !world.ready() {
            return Ok(());
        }
        self.membership(world);
        self.transferring(world);
        Ok(())
    }

    /// Invitations never outlive their zone. Only the lifecycle's bound
    /// departure can leave canonical names for the next admission.
    fn connection_ended(&mut self, world: &mut World) {
        if !world.lifecycle.departing() {
            self.transfer.clear();
        }
        self.members = None;
        self.invitations.clear();
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
        player.name = "Tester".into();
        player.listing.guild = guild_id;
        player.listing.guild_rank = guild_id.map(|_| GuildRank::Member);
        world.player.admit(player);
        world.own_spawn = Some(7);
        world.admitted = Some(std::time::Instant::now());
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

    fn bound_roster() -> (RosterTransfer, Guilds, World) {
        let transfer = RosterTransfer::default();
        let mut feature = Guilds::new(transfer.clone());
        let mut world = guild_world(Some(3));
        testing::run(|out| feature.admitted(&mut world, out))
            .result
            .unwrap();
        testing::run(|out| feature.observe(&members(&["Tester", "Member"]), &mut world, out))
            .result
            .unwrap();
        world
            .lifecycle
            .offer(
                eq_network_game::zoning::ZoneOffer {
                    zone_id: 2,
                    instance_id: 3,
                    position: eq_network_game::world::Position::default(),
                    reason: 0,
                    to_bind: false,
                    solicited: false,
                },
                std::time::Instant::now(),
            )
            .unwrap();
        testing::run(|out| feature.tick(std::time::Instant::now(), &mut world, out))
            .result
            .unwrap();
        (transfer, feature, world)
    }

    fn arrival() -> World {
        let mut world = guild_world(Some(3));
        world.session_id = 6;
        world.zone = (2, 3);
        world
    }

    #[test]
    fn inherited_names_require_the_same_character_guild_and_offered_destination_once() {
        for (name, guild, session, destination) in [
            ("Other", Some(3), 6, (2, 3)),
            ("Tester", Some(4), 6, (2, 3)),
            ("Tester", None, 6, (2, 3)),
            ("Tester", Some(3), 5, (2, 3)),
            ("Tester", Some(3), 6, (9, 3)),
            ("Tester", Some(3), 6, (2, 9)),
        ] {
            let (transfer, _, _) = bound_roster();
            let mut feature = Guilds::new(transfer.clone());
            let mut world = arrival();
            let mut player = testing::player(44);
            player.name = name.into();
            player.listing.guild = guild;
            world.player.admit(player);
            world.session_id = session;
            world.zone = destination;
            testing::run(|out| feature.admitted(&mut world, out))
                .result
                .unwrap();
            assert_eq!(
                feature.members, None,
                "{name} {guild:?} {session} {destination:?}"
            );
            // A mismatched admission consumes the transfer; a later match
            // cannot recover evidence from before this boundary.
            let mut later = Guilds::new(transfer);
            testing::run(|out| later.admitted(&mut arrival(), out))
                .result
                .unwrap();
            assert_eq!(later.members, None);
        }
    }

    #[test]
    fn unexpected_loss_denial_and_a_fresh_login_withdraw_transfer_names() {
        for denied in [false, true] {
            let (transfer, mut old, mut world) = bound_roster();
            if denied {
                world
                    .lifecycle
                    .finish(false, std::time::Instant::now())
                    .unwrap();
                testing::run(|out| old.tick(std::time::Instant::now(), &mut world, out))
                    .result
                    .unwrap();
            } else {
                old.connection_ended(&mut world);
            }
            let mut next = Guilds::new(transfer);
            testing::run(|out| next.admitted(&mut arrival(), out))
                .result
                .unwrap();
            assert_eq!(next.members, None);
        }
        let (_old_login, _, _) = bound_roster();
        let mut fresh = Guilds::new(RosterTransfer::default());
        testing::run(|out| fresh.admitted(&mut arrival(), out))
            .result
            .unwrap();
        assert_eq!(fresh.members, None);
    }

    #[test]
    fn an_approved_departure_keeps_names_but_never_invitations_or_rank_authority() {
        let (transfer, mut old, mut world) = bound_roster();
        old.follow(&Message::Event(WorldEvent::Guild(GuildUpdate::Invite {
            inviter: "Leader".into(),
            guild_id: 3,
            rank: GuildRank::Officer,
        })));
        world
            .lifecycle
            .finish(true, std::time::Instant::now())
            .unwrap();
        old.connection_ended(&mut world);
        let mut next = Guilds::new(transfer);
        let mut world = arrival();
        let rank = world.player.as_ref().unwrap().listing.guild_rank;
        testing::run(|out| next.admitted(&mut world, out))
            .result
            .unwrap();
        assert_eq!(next.members, Some(vec!["Tester".into(), "Member".into()]));
        assert_eq!(next.invitations, Vec::<Invitation>::new());
        assert_eq!(world.player.as_ref().unwrap().listing.guild_rank, rank);
        // Submission is not evidence of a removal, demotion or new leader.
        let before = next.members.clone();
        for command in [
            ClientCommand::GuildRemove {
                session_id: 6,
                name: "member".into(),
            },
            ClientCommand::GuildDemote {
                session_id: 6,
                name: "member".into(),
            },
            ClientCommand::GuildMakeLeader {
                session_id: 6,
                name: "member".into(),
            },
        ] {
            let outcome = testing::run(|out| next.handle(&command, &mut world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent.len(), 1);
            assert_eq!(refusal(&outcome.events), None);
            assert_eq!(next.members, before);
            assert_eq!(world.player.as_ref().unwrap().listing.guild_rank, rank);
        }
    }

    #[test]
    fn a_fresh_admission_list_replaces_inherited_names_and_is_published_once() {
        let (transfer, _, _) = bound_roster();
        let mut next = Guilds::new(transfer);
        let mut world = arrival();
        next.admit(&members(&["Replacement"]), &mut world).unwrap();
        let outcome = testing::run(|out| next.admitted(&mut world, out));
        outcome.result.unwrap();
        assert_eq!(next.members, Some(vec!["Replacement".into()]));
        assert_eq!(
            outcome
                .events
                .iter()
                .filter(|event| matches!(
                    event,
                    super::super::ClientEvent::World(WorldEvent::Guild(GuildUpdate::Members(_)))
                ))
                .count(),
            1
        );
    }

    #[test]
    fn admission_deltas_follow_inheritance_without_inventing_a_full_list() {
        for inherited in [false, true] {
            let transfer = if inherited {
                bound_roster().0
            } else {
                RosterTransfer::default()
            };
            let mut next = Guilds::new(transfer);
            let mut world = arrival();
            next.admit(
                &Message::Event(WorldEvent::Guild(GuildUpdate::MemberRemoved {
                    guild_id: 3,
                    name: "Member".into(),
                })),
                &mut world,
            )
            .unwrap();
            testing::run(|out| next.admitted(&mut world, out))
                .result
                .unwrap();
            assert_eq!(next.members, inherited.then(|| vec!["Tester".into()]));
        }
    }

    #[test]
    fn received_membership_boundaries_and_deltas_remain_authoritative() {
        for update in [
            GuildUpdate::Disbanded { guild_id: 3 },
            GuildUpdate::MemberRemoved {
                guild_id: 3,
                name: "Tester".into(),
            },
        ] {
            let (transfer, mut feature, mut world) = bound_roster();
            testing::run(|out| {
                feature.observe(&Message::Event(WorldEvent::Guild(update)), &mut world, out)
            })
            .result
            .unwrap();
            assert_eq!(feature.members, None);
            let mut next = Guilds::new(transfer);
            testing::run(|out| next.admitted(&mut arrival(), out))
                .result
                .unwrap();
            assert_eq!(next.members, None);
        }
        let (_, mut feature, mut world) = bound_roster();
        let foreign = Message::Event(WorldEvent::Guild(GuildUpdate::MemberRemoved {
            guild_id: 9,
            name: "Member".into(),
        }));
        testing::run(|out| feature.observe(&foreign, &mut world, out))
            .result
            .unwrap();
        assert_eq!(
            feature.members,
            Some(vec!["Tester".into(), "Member".into()])
        );
        let renamed = Message::Event(WorldEvent::Guild(GuildUpdate::MemberRenamed {
            guild_id: 3,
            old_name: "Member".into(),
            new_name: "Renamed".into(),
        }));
        testing::run(|out| feature.observe(&renamed, &mut world, out))
            .result
            .unwrap();
        assert_eq!(feature.member("renamed").unwrap(), "Renamed");
        assert!(feature.member("Member").is_err());
        let mut player = testing::player(7);
        player.listing.guild = Some(4);
        world.player.admit(player);
        let outcome = testing::run(|out| {
            feature.handle(
                &ClientCommand::GuildRemove {
                    session_id: 5,
                    name: "Renamed".into(),
                },
                &mut world,
                out,
            )
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert_eq!(feature.members, None);
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
        // An invitation from the zone the player left is not answered.
        testing::run(|out| feature.observe(&invite, &mut world, out))
            .result
            .unwrap();
        feature.connection_ended(&mut world);
        let outcome = testing::run(|out| {
            feature.handle(&answer("Inviter", InviteAnswer::Accept), &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [], "an old zone's invitation must not go");
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
        testing::run(|out| feature.admitted(&mut world, out))
            .result
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
