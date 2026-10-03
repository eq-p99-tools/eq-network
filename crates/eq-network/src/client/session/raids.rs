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
//! a member, one from a member who is not the leader, and one while the
//! raid is locked. The server does not pass the player's own raid chat back
//! to them, so the session records it as the others hear it, as the official
//! client shows it (inferred).
//!
//! The raid's leader locks and unlocks it, moves members between raid
//! groups, hands on the lead and removes members. `EQEmu` checks only that
//! the one handing on the lead leads the raid; the session allows all of
//! them to the leader alone, moves only while the raid is locked and into a
//! group with room, as the official client's notes on raids and its Raid
//! window's tips describe the window (inferred).
//!
//! The server moves a member by taking them out of the raid and adding them
//! back in their new place, at once, and lists the raid again to the member
//! moved (`Raid::MoveMember`, `SendRaidMoveAll`); a member moved in another
//! zone hears the raid end first (`ServerOP_RaidChangeGroup`). So the
//! session holds each removal, and each end of the raid, until the next
//! message: the same member added back makes a removal a move, the raid
//! listed again to the player taken out makes them nothing, and anything
//! else, or a moment without a message, makes them what they said, told in
//! order before that message.
//!
//! The server lists the raid, after its creation, as the player joins,
//! enters a zone or moves, in one burst whose end no packet marks: the
//! leader comes last in some orders and early in others, and the join's
//! list holds health updates. So the session counts every member added
//! after a raid's creation as listed ([`RaidUpdate::Listed`]) until a moment
//! passes without a raid update; a member added later joined. The raid the
//! player forms by inviting lists nothing: the one added with them joined
//! (inferred: what the official client tells the inviter is not checked).
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use crate::client::RecordEvent;
use anyhow::Result;
use eq_network_game::{
    chat::OutboundChat,
    message::Message,
    raid::{RaidMember, RaidUpdate},
    request::Request,
    world::WorldEvent,
};
use std::time::{Duration, Instant};

/// The official client's strings for the invitations it refuses: one that
/// names no one, one from a member who is not the leader, one to the player
/// themself, one to a member, and one while the raid is locked
/// (`eqstr_us.txt`).
const NO_ONE_NAMED: u32 = 5074;
const NOT_THE_LEADER: u32 = 5073;
const ONESELF: u32 = 5076;
const ALREADY_IN: u32 = 5077;
const LOCKED: u32 = 8870;
/// Its string for one who is not in the player's raid, naming them.
const NOT_IN_RAID: u32 = 5082;

/// How many members a raid group holds (`EQEmu`'s move checks for fewer).
const GROUP_SIZE: usize = 6;

// The two waits stand in for the burst of packets one server call sends,
// which no packet ends: a move's removal and add, a re-listing's end and
// creation, and a list of the raid each come in one call, and joins in
// later ones. Neither changes what the session knows of the raid, only
// how it tells the host: a member listed or added is in the raid alike.

/// How long a held removal, or end of the raid, waits for the message that
/// would show what it was.
const HOLD: Duration = Duration::from_millis(300);

/// How long a list of the raid lasts without a raid update.
const LIST_GAP: Duration = Duration::from_secs(1);

/// The player's raid, as the server described it.
#[derive(Default)]
struct Raid {
    /// Its leader, once named.
    leader: Option<String>,
    /// Its members, the player among them, as the server gave them.
    members: Vec<RaidMember>,
    /// Whether it is locked.
    locked: bool,
}

impl Raid {
    /// The member by this name, as the server gave it.
    fn member(&self, name: &str) -> Option<&RaidMember> {
        self.members
            .iter()
            .find(|member| member.name.eq_ignore_ascii_case(name))
    }

    /// Whether this player leads the raid.
    fn led_by(&self, player: &str) -> bool {
        self.leader
            .as_deref()
            .is_some_and(|leader| leader.eq_ignore_ascii_case(player))
    }

    /// How many are in a raid group.
    fn in_group(&self, group: u8) -> usize {
        self.members
            .iter()
            .filter(|member| member.group == Some(group))
            .count()
    }
}

/// Updates the session holds until the next message shows what they were:
/// a removal, or the raid's end and then the player's removal.
struct Held {
    /// The updates, in the order they came.
    updates: Vec<RaidUpdate>,
    /// When the session first saw them held.
    since: Option<Instant>,
}

/// A moment the session waits through: since when, once a timer has seen it.
#[derive(Default)]
struct Quiet(Option<Instant>);

impl Quiet {
    /// Whether this long has passed since the timer first saw it.
    fn over(&mut self, now: Instant, wait: Duration) -> bool {
        now.saturating_duration_since(*self.0.get_or_insert(now)) >= wait
    }
}

/// Invites, accepts, declines and leaves; locks, moves, hands on the lead
/// and removes.
#[derive(Default)]
pub(super) struct Raids {
    /// Who invited the player last, until the player answers or is in a
    /// raid.
    invitation: Option<String>,
    /// The player's raid, while they are in one.
    raid: Option<Raid>,
    /// Updates held until the next message.
    held: Option<Held>,
    /// Held updates the message being heard showed to be what they said,
    /// told before it.
    released: Vec<RaidUpdate>,
    /// The player's name, as the session speaks for them.
    player: Option<String>,
    /// The list the server is sending, until it pauses.
    listing: Option<Quiet>,
    /// The player invited someone out of a raid, so the next raid created
    /// for them is the one they formed, which lists nothing.
    forming: bool,
}

/// A refusal: this library's words and the official client's string for
/// it, if it has one.
type Refusal = (&'static str, Option<u32>);

impl Raids {
    /// Why the player may not invite someone; None when they may.
    fn refusal(&self, name: &str, player: &str) -> Option<Refusal> {
        if name.is_empty() {
            return Some((
                "Name a player to invite, or target one.",
                Some(NO_ONE_NAMED),
            ));
        }
        if name.eq_ignore_ascii_case(player) {
            return Some(("You cannot invite yourself.", Some(ONESELF)));
        }
        let raid = self.raid.as_ref()?;
        if raid.member(name).is_some() {
            return Some(("They are in your raid already.", Some(ALREADY_IN)));
        }
        if raid.leader.is_some() && !raid.led_by(player) {
            return Some(("Only the raid's leader may invite.", Some(NOT_THE_LEADER)));
        }
        raid.locked
            .then_some(("The raid is locked; unlock it to invite.", Some(LOCKED)))
    }

    /// The player's raid, if they lead it; else why a leader's command is
    /// refused.
    fn led(&self, player: &str, only: &'static str) -> Result<&Raid, Refusal> {
        let raid = self.raid.as_ref().ok_or(("You are in no raid.", None))?;
        if raid.led_by(player) {
            Ok(raid)
        } else {
            Err((only, None))
        }
    }

    /// Follows the server's word on the player's raid.
    fn follow_news(&mut self, update: &RaidUpdate, player: &str) {
        match update {
            RaidUpdate::Invited { inviter } => self.invitation = Some(inviter.clone()),
            RaidUpdate::Created { leader } => {
                self.raid = Some(Raid {
                    leader: Some(leader.clone()),
                    ..Raid::default()
                });
            }
            RaidUpdate::Added(member) | RaidUpdate::Listed(member) | RaidUpdate::Moved(member) => {
                let members = &mut self.raid.get_or_insert_with(Raid::default).members;
                match members.iter_mut().find(|known| known.name == member.name) {
                    Some(known) => known.clone_from(member),
                    None => members.push(member.clone()),
                }
            }
            RaidUpdate::Removed { member } if member.eq_ignore_ascii_case(player) => {
                self.raid = None;
            }
            RaidUpdate::Removed { member } => {
                if let Some(raid) = self.raid.as_mut() {
                    raid.members.retain(|known| known.name != *member);
                }
            }
            RaidUpdate::Disbanded => self.raid = None,
            RaidUpdate::Leader { name } => {
                if let Some(raid) = self.raid.as_mut() {
                    raid.leader = Some(name.clone());
                }
            }
            RaidUpdate::Locked { locked, .. } => {
                if let Some(raid) = self.raid.as_mut() {
                    raid.locked = *locked;
                }
            }
            RaidUpdate::Inviting { .. }
            | RaidUpdate::Accepting { .. }
            | RaidUpdate::Declining { .. }
            | RaidUpdate::Leaving
            | RaidUpdate::Locking { .. } => {}
        }
        // Being in a raid answers any invitation.
        if self.raid.is_some() {
            self.invitation = None;
        }
    }

    /// Tells the host of held updates, as the server sent them.
    fn release(&mut self, updates: Vec<RaidUpdate>, out: &mut Out<'_, '_>) -> Result<()> {
        for update in updates {
            self.follow_news(&update, out.sender.name);
            said(update, out)?;
        }
        Ok(())
    }

    /// Whether this name is the player's.
    fn is_player(&self, name: &str) -> bool {
        self.player
            .as_deref()
            .is_some_and(|player| player.eq_ignore_ascii_case(name))
    }

    /// What a held removal or end of the raid turns out to be, as the next
    /// message shows; true when that message is to be held too.
    fn resolve(&mut self, held: Held, message: &mut Message) -> bool {
        let Message::Event(WorldEvent::Raid(update)) = message else {
            self.released = held.updates;
            return false;
        };
        match (held.updates.as_slice(), &*update) {
            // The same member added back: a move.
            ([RaidUpdate::Removed { member }], RaidUpdate::Added(added))
                if *member == added.name =>
            {
                *update = RaidUpdate::Moved(added.clone());
                false
            }
            // The player taken out, with the raid ended first or not, and
            // the raid listed again: nothing happened.
            (
                [RaidUpdate::Removed { member }]
                | [RaidUpdate::Disbanded, RaidUpdate::Removed { member }],
                RaidUpdate::Created { .. },
            ) if self.is_player(member) => false,
            // The raid ended for the player, then their removal: both wait.
            ([RaidUpdate::Disbanded], RaidUpdate::Removed { member }) if self.is_player(member) => {
                let mut updates = held.updates;
                updates.push(update.clone());
                self.held = Some(Held {
                    updates,
                    since: held.since,
                });
                *message = Message::Withheld;
                true
            }
            _ => {
                self.released = held.updates;
                false
            }
        }
    }

    /// Counts a member added while the server lists the raid as listed, and
    /// starts a list as a raid is created for the player, but for the raid
    /// they form by inviting.
    fn list(&mut self, update: &mut RaidUpdate) {
        match update {
            RaidUpdate::Created { leader } => {
                let formed = std::mem::take(&mut self.forming) && self.is_player(leader);
                self.listing = (!formed).then(Quiet::default);
            }
            RaidUpdate::Added(member) if self.listing.is_some() => {
                *update = RaidUpdate::Listed(member.clone());
            }
            _ => {}
        }
        // Any raid update keeps a list going.
        if let Some(listing) = self.listing.as_mut() {
            *listing = Quiet::default();
        }
    }

    /// Leaves the player's raid.
    fn leave(&self, command: &ClientCommand, out: &mut Out<'_, '_>) -> Result<()> {
        if self.raid.is_none() {
            return actions::refuse(command, "You are in no raid.", out.log);
        }
        out.request(&Request::RaidLeave)?;
        said(RaidUpdate::Leaving, out)
    }

    /// Carries out a raid leader's command: locking, moving, handing on the
    /// lead or removing.
    fn lead(&self, command: &ClientCommand, out: &mut Out<'_, '_>) -> Result<()> {
        let player = out.sender.name;
        if let ClientCommand::RaidMove { name, .. }
        | ClientCommand::RaidMakeLeader { name, .. }
        | ClientCommand::RaidRemove { name, .. } = command
        {
            if name.trim().is_empty() {
                return actions::refuse(command, "Choose a member of your raid first.", out.log);
            }
        }
        let request = match command {
            ClientCommand::RaidLock { locked, .. } => self
                .led(player, "Only the raid's leader may lock or unlock it.")
                .map(|_| Request::RaidLock(*locked)),
            ClientCommand::RaidMove { name, group, .. } => {
                match self.led(player, "Only the raid's leader may move members.") {
                    Ok(raid) => match raid.member(name) {
                        None => return not_in_raid(command, name, out),
                        Some(member) => moving(raid, member, *group),
                    },
                    Err(refusal) => Err(refusal),
                }
            }
            ClientCommand::RaidMakeLeader { name, .. } => {
                match self.led(player, "Only the raid's leader may hand on the lead.") {
                    Ok(raid) => match raid.member(name) {
                        None => return not_in_raid(command, name, out),
                        Some(member) if member.name.eq_ignore_ascii_case(player) => {
                            Err(("You lead the raid already.", None))
                        }
                        Some(member) => Ok(Request::RaidMakeLeader(member.name.clone())),
                    },
                    Err(refusal) => Err(refusal),
                }
            }
            ClientCommand::RaidRemove { name, .. } => {
                match self.led(player, "Only the raid's leader may remove members.") {
                    Ok(raid) => match raid.member(name) {
                        None => return not_in_raid(command, name, out),
                        Some(member) => Ok(Request::RaidRemove(member.name.clone())),
                    },
                    Err(refusal) => Err(refusal),
                }
            }
            _ => return Ok(()),
        };
        match request {
            Ok(request) => {
                out.request(&request)?;
                match request {
                    Request::RaidLock(locked) => said(RaidUpdate::Locking { locked }, out),
                    _ => Ok(()),
                }
            }
            Err(refusal) => actions::refuse_officially(command, refusal, out.log),
        }
    }
}

/// A move of this member into a raid group, or out of every group; refused
/// while the raid is unlocked, into the group they are in, and into a full
/// one.
fn moving(raid: &Raid, member: &RaidMember, group: Option<u8>) -> Result<Request, Refusal> {
    if !raid.locked {
        return Err(("Lock the raid to move its members.", None));
    }
    if member.group == group {
        return Err(("They are in that group already.", None));
    }
    if group.is_some_and(|group| raid.in_group(group) >= GROUP_SIZE) {
        return Err(("That raid group is full.", None));
    }
    Ok(Request::RaidMove {
        member: member.name.clone(),
        group,
    })
}

/// Refuses a command naming one who is not in the player's raid.
fn not_in_raid(command: &ClientCommand, name: &str, out: &mut Out<'_, '_>) -> Result<()> {
    actions::refuse_naming(
        command,
        ("They are not in your raid.", Some(NOT_IN_RAID)),
        &[name.to_owned()],
        out.log,
    )
}

/// Says what the session did, which the server does not answer.
fn said(update: RaidUpdate, out: &mut Out<'_, '_>) -> Result<()> {
    out.log.send(ClientEvent::World(WorldEvent::Raid(update)))
}

impl Feature for Raids {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Raiding]
    }

    /// Holds a removal, or the raid's end, until the next message says what
    /// it was: a move as the same member is added back, nothing as the raid
    /// is listed again to the player taken out, or else what it said, told
    /// before that message. Counts the members a list adds as listed.
    fn explain(&mut self, message: &mut Message, _world: &World) {
        if let Some(held) = self.held.take() {
            if self.resolve(held, message) {
                return;
            }
        }
        let Message::Event(WorldEvent::Raid(update)) = message else {
            return;
        };
        if matches!(update, RaidUpdate::Removed { .. } | RaidUpdate::Disbanded) {
            self.held = Some(Held {
                updates: vec![update.clone()],
                since: None,
            });
            *message = Message::Withheld;
            return;
        }
        self.list(update);
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::RaidInvite { .. }
                | ClientCommand::RaidAccept { .. }
                | ClientCommand::RaidDecline { .. }
                | ClientCommand::RaidLeave { .. }
                | ClientCommand::RaidLock { .. }
                | ClientCommand::RaidMove { .. }
                | ClientCommand::RaidMakeLeader { .. }
                | ClientCommand::RaidRemove { .. }
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
        self.player = Some(player.to_owned());
        match command {
            ClientCommand::RaidInvite { name, .. } => {
                let name = name.trim();
                if let Some(refusal) = self.refusal(name, player) {
                    return actions::refuse_officially(command, refusal, out.log);
                }
                out.request(&Request::RaidInvite(name.to_owned()))?;
                // Out of a raid, the player forms one as someone accepts.
                self.forming = self.raid.is_none();
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
            // Removing oneself is leaving.
            ClientCommand::RaidLeave { .. } => self.leave(command, out),
            ClientCommand::RaidRemove { name, .. } if name.eq_ignore_ascii_case(player) => {
                self.leave(command, out)
            }
            _ => self.lead(command, out),
        }
    }

    /// Tells held updates once a moment has passed without a message, and
    /// ends a list once it has paused.
    fn tick(&mut self, now: Instant, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if self
            .listing
            .as_mut()
            .is_some_and(|listing| listing.over(now, LIST_GAP))
        {
            self.listing = None;
        }
        let Some(held) = self.held.as_mut() else {
            return Ok(());
        };
        let since = *held.since.get_or_insert(now);
        if now.saturating_duration_since(since) < HOLD {
            return Ok(());
        }
        let Some(held) = self.held.take() else {
            return Ok(());
        };
        self.release(held.updates, out)
    }

    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.player = Some(out.sender.name.to_owned());
        let released = std::mem::take(&mut self.released);
        self.release(released, out)?;
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
    use eq_network_game::raid;

    /// What a command sends.
    fn sent(
        raids: &mut Raids,
        world: &mut World,
        command: &ClientCommand,
    ) -> Vec<eq_network_game::command::EncodedCommand> {
        testing::run(|out| raids.handle(command, world, out)).sent
    }

    /// The official string a refused command names, if any, and what it
    /// names.
    fn refused_naming(
        raids: &mut Raids,
        world: &mut World,
        command: &ClientCommand,
    ) -> (Option<u32>, Vec<String>) {
        let outcome = testing::run(|out| raids.handle(command, world, out));
        assert_eq!(outcome.sent, []);
        outcome
            .events
            .iter()
            .find_map(|event| match event {
                ClientEvent::World(WorldEvent::RaidRefused {
                    string_id,
                    arguments,
                    ..
                }) => Some((*string_id, arguments.clone())),
                _ => None,
            })
            .expect("a refusal")
    }

    /// The official string a refused command names, if any.
    fn refused(raids: &mut Raids, world: &mut World, command: &ClientCommand) -> Option<u32> {
        refused_naming(raids, world, command).0
    }

    /// The server's word on raids, as the session hears it, explained first
    /// as every message is; what the host hears of it.
    fn hear(raids: &mut Raids, world: &mut World, update: RaidUpdate) -> Vec<RaidUpdate> {
        let mut message = Message::Event(WorldEvent::Raid(update));
        raids.explain(&mut message, world);
        let outcome = testing::run(|out| raids.observe(&message, world, out));
        outcome.result.unwrap();
        let mut heard: Vec<RaidUpdate> = outcome
            .events
            .into_iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::Raid(update)) => Some(update),
                _ => None,
            })
            .collect();
        if let Message::Event(WorldEvent::Raid(update)) = message {
            heard.push(update);
        }
        heard
    }

    fn member(name: &str, group: Option<u8>) -> RaidMember {
        RaidMember {
            name: name.into(),
            group,
            class: 2,
            level: 30,
            group_leader: false,
        }
    }

    fn added(name: &str) -> RaidUpdate {
        RaidUpdate::Added(member(name, None))
    }

    fn removed(name: &str) -> RaidUpdate {
        RaidUpdate::Removed {
            member: name.into(),
        }
    }

    /// A raid of the player, who leads it, and two others.
    fn led_raid(raids: &mut Raids, world: &mut World) {
        for update in [
            RaidUpdate::Created {
                leader: "Tester".into(),
            },
            added("Tester"),
            added("Friend"),
            added("Other"),
        ] {
            hear(raids, world, update);
        }
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
        hear(&mut raids, &mut world, added("Leader"));
        hear(&mut raids, &mut world, added("Tester"));
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
        // Nor while the raid is locked.
        hear(
            &mut raids,
            &mut world,
            RaidUpdate::Locked {
                locked: true,
                by: "Tester".into(),
            },
        );
        assert_eq!(
            refused(&mut raids, &mut world, &invite("Friend")),
            Some(LOCKED)
        );
        // Leaving sends the player's own name twice; out of a raid, it is
        // refused.
        let leave = ClientCommand::RaidLeave { session_id: 5 };
        assert_eq!(
            sent(&mut raids, &mut world, &leave),
            [raid::remove("Tester", "Tester").unwrap()]
        );
        hear(&mut raids, &mut world, removed("Tester"));
        hear(&mut raids, &mut world, RaidUpdate::Disbanded);
        assert_eq!(sent(&mut raids, &mut world, &leave), []);
    }

    #[test]
    fn the_leader_locks_moves_hands_on_the_lead_and_removes() {
        let mut raids = Raids::default();
        let mut world = World::new(5);
        led_raid(&mut raids, &mut world);
        let lock = |locked| ClientCommand::RaidLock {
            session_id: 5,
            locked,
        };
        let shift = |name: &str, group| ClientCommand::RaidMove {
            session_id: 5,
            name: name.into(),
            group,
        };
        // A move waits for the lock.
        assert_eq!(
            refused(&mut raids, &mut world, &shift("friend", Some(0))),
            None
        );
        let locking = testing::run(|out| raids.handle(&lock(true), &mut world, out));
        assert_eq!(locking.sent, [raid::lock("Tester", true).unwrap()]);
        assert!(locking.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Raid(RaidUpdate::Locking { locked: true }))
        )));
        hear(
            &mut raids,
            &mut world,
            RaidUpdate::Locked {
                locked: true,
                by: "Tester".into(),
            },
        );
        // The server finds the member by the name it gave them.
        assert_eq!(
            sent(&mut raids, &mut world, &shift("friend", Some(0))),
            [raid::move_member("Tester", "Friend", Some(0)).unwrap()]
        );
        assert_eq!(
            refused_naming(&mut raids, &mut world, &shift("Stranger", Some(0))),
            (Some(NOT_IN_RAID), vec!["Stranger".to_owned()])
        );
        // With no member chosen, a move names no one.
        assert_eq!(refused(&mut raids, &mut world, &shift("", Some(0))), None);
        // Into the group they are in, or a full one, is refused.
        assert_eq!(
            refused(&mut raids, &mut world, &shift("Friend", None)),
            None
        );
        for name in ["A", "B", "C", "D", "E", "F"] {
            hear(
                &mut raids,
                &mut world,
                RaidUpdate::Added(member(name, Some(4))),
            );
        }
        assert_eq!(
            refused(&mut raids, &mut world, &shift("Friend", Some(4))),
            None
        );
        let lead = |name: &str| ClientCommand::RaidMakeLeader {
            session_id: 5,
            name: name.into(),
        };
        assert_eq!(
            sent(&mut raids, &mut world, &lead("Other")),
            [raid::make_leader("Tester", "Other").unwrap()]
        );
        assert_eq!(refused(&mut raids, &mut world, &lead("Tester")), None);
        let remove = |name: &str| ClientCommand::RaidRemove {
            session_id: 5,
            name: name.into(),
        };
        assert_eq!(
            sent(&mut raids, &mut world, &remove("Other")),
            [raid::remove("Tester", "Other").unwrap()]
        );
        // Removing oneself leaves.
        assert_eq!(
            sent(&mut raids, &mut world, &remove("tester")),
            [raid::remove("Tester", "Tester").unwrap()]
        );
        // Once another leads, the leader's commands are refused.
        hear(
            &mut raids,
            &mut world,
            RaidUpdate::Leader {
                name: "Other".into(),
            },
        );
        for command in [
            lock(false),
            shift("Friend", Some(1)),
            lead("Friend"),
            remove("Friend"),
        ] {
            assert_eq!(
                refused(&mut raids, &mut world, &command),
                None,
                "{command:?}"
            );
        }
    }

    #[test]
    fn a_member_taken_out_and_added_back_moved() {
        let mut raids = Raids::default();
        let mut world = World::new(5);
        led_raid(&mut raids, &mut world);
        // The server takes the member out, adds them back in their new
        // group, and names the leader again.
        assert_eq!(hear(&mut raids, &mut world, removed("Friend")), []);
        let moved = member("Friend", Some(2));
        assert_eq!(
            hear(&mut raids, &mut world, RaidUpdate::Added(moved.clone())),
            [RaidUpdate::Moved(moved)]
        );
        // The player moved is listed again: nothing was removed.
        assert_eq!(hear(&mut raids, &mut world, removed("Tester")), []);
        let relisted = RaidUpdate::Created {
            leader: "Tester".into(),
        };
        assert_eq!(hear(&mut raids, &mut world, relisted.clone()), [relisted]);
        // A removal followed by any other update is told before it.
        hear(&mut raids, &mut world, added("Friend"));
        assert_eq!(hear(&mut raids, &mut world, removed("Friend")), []);
        let leader = RaidUpdate::Leader {
            name: "Tester".into(),
        };
        assert_eq!(
            hear(&mut raids, &mut world, leader.clone()),
            [removed("Friend"), leader]
        );
        // So does any other message.
        assert_eq!(hear(&mut raids, &mut world, removed("Other")), []);
        let mut other = Message::LoggedOut;
        raids.explain(&mut other, &world);
        let told = testing::run(|out| raids.observe(&other, &mut world, out)).events;
        assert!(matches!(
            told.as_slice(),
            [ClientEvent::World(WorldEvent::Raid(RaidUpdate::Removed { member }))] if member == "Other"
        ));
        hear(&mut raids, &mut world, added("Other"));
        // And on its own, once a moment has passed.
        assert_eq!(hear(&mut raids, &mut world, removed("Other")), []);
        let start = Instant::now();
        let tell = |raids: &mut Raids, world: &mut World, now| {
            testing::run(|out| raids.tick(now, world, out)).events
        };
        assert!(matches!(tell(&mut raids, &mut world, start).as_slice(), []));
        assert!(matches!(
            tell(&mut raids, &mut world, start + HOLD / 2).as_slice(),
            []
        ));
        assert!(matches!(
            tell(&mut raids, &mut world, start + HOLD).as_slice(),
            [ClientEvent::World(WorldEvent::Raid(RaidUpdate::Removed { member }))] if member == "Other"
        ));
        assert!(raids
            .raid
            .as_ref()
            .is_some_and(|raid| raid.member("Other").is_none()));
    }

    #[test]
    fn a_list_counts_its_members_as_listed_until_it_pauses() {
        let mut raids = Raids::default();
        let mut world = World::new(5);
        let created = || RaidUpdate::Created {
            leader: "Leader".into(),
        };
        let leader = || RaidUpdate::Leader {
            name: "Leader".into(),
        };
        let listed = |name: &str| RaidUpdate::Listed(member(name, None));
        // The leader named early ends nothing: every member added in the
        // burst is listed.
        assert_eq!(hear(&mut raids, &mut world, created()), [created()]);
        assert_eq!(hear(&mut raids, &mut world, leader()), [leader()]);
        assert_eq!(
            hear(&mut raids, &mut world, added("Tester")),
            [listed("Tester")]
        );
        assert_eq!(
            hear(&mut raids, &mut world, added("Leader")),
            [listed("Leader")]
        );
        // Once the list pauses, a member added joined.
        let start = Instant::now();
        let tick = |raids: &mut Raids, world: &mut World, now| {
            testing::run(|out| raids.tick(now, world, out))
                .result
                .unwrap();
        };
        tick(&mut raids, &mut world, start);
        tick(&mut raids, &mut world, start + LIST_GAP);
        assert_eq!(
            hear(&mut raids, &mut world, added("Friend")),
            [added("Friend")]
        );
    }

    #[test]
    fn a_list_leaves_the_same_raid_however_long_it_pauses() {
        // As EQEmu lists the raid to the last of a group whose leader left
        // it: the player taken out, the raid created, the leader, the
        // player, then the rest.
        let order = || {
            [
                removed("Tester"),
                RaidUpdate::Created {
                    leader: "Leader".into(),
                },
                RaidUpdate::Leader {
                    name: "Leader".into(),
                },
                RaidUpdate::Added(member("Tester", Some(0))),
                RaidUpdate::Added(member("Leader", None)),
                RaidUpdate::Added(member("Other", Some(2))),
            ]
        };
        let roster = |gap: Option<usize>| {
            let mut raids = Raids::default();
            let mut world = World::new(5);
            led_raid(&mut raids, &mut world);
            let start = Instant::now();
            for (index, update) in order().into_iter().enumerate() {
                if gap == Some(index) {
                    for now in [start, start + Duration::from_secs(2)] {
                        testing::run(|out| raids.tick(now, &mut world, out))
                            .result
                            .unwrap();
                    }
                }
                hear(&mut raids, &mut world, update);
            }
            raids.raid.map(|raid| raid.members).expect("in a raid")
        };
        // A pause in the middle of the list tells the rest as joins, which
        // leaves the raid as it is.
        assert_eq!(roster(None), roster(Some(4)));
    }

    #[test]
    fn the_raid_the_player_forms_by_inviting_lists_nothing() {
        let mut raids = Raids::default();
        let mut world = World::new(5);
        let invite = ClientCommand::RaidInvite {
            session_id: 5,
            name: "Friend".into(),
        };
        assert_eq!(sent(&mut raids, &mut world, &invite).len(), 1);
        hear(
            &mut raids,
            &mut world,
            RaidUpdate::Created {
                leader: "Tester".into(),
            },
        );
        assert_eq!(
            hear(&mut raids, &mut world, added("Tester")),
            [added("Tester")]
        );
        assert_eq!(
            hear(&mut raids, &mut world, added("Friend")),
            [added("Friend")]
        );
    }

    #[test]
    fn the_raid_ended_and_listed_again_ended_nothing() {
        let mut raids = Raids::default();
        let mut world = World::new(5);
        led_raid(&mut raids, &mut world);
        // A member moved in another zone: the raid's end, their removal and
        // the raid listed again.
        assert_eq!(hear(&mut raids, &mut world, RaidUpdate::Disbanded), []);
        assert_eq!(hear(&mut raids, &mut world, removed("Tester")), []);
        let relisted = RaidUpdate::Created {
            leader: "Leader".into(),
        };
        assert_eq!(hear(&mut raids, &mut world, relisted.clone()), [relisted]);
        // The player removed: their removal, then the raid's end, each told
        // as it is.
        assert_eq!(hear(&mut raids, &mut world, removed("Tester")), []);
        assert_eq!(
            hear(&mut raids, &mut world, RaidUpdate::Disbanded),
            [removed("Tester")]
        );
        let mut other = Message::LoggedOut;
        raids.explain(&mut other, &world);
        let told = testing::run(|out| raids.observe(&other, &mut world, out)).events;
        assert!(matches!(
            told.as_slice(),
            [ClientEvent::World(WorldEvent::Raid(RaidUpdate::Disbanded))]
        ));
        assert!(raids.raid.is_none());
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
