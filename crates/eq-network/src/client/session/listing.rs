//! How `/who` lists the player: away from the keyboard, anonymous or
//! roleplaying. The player says so with an appearance update, which Titanium
//! servers never echo (`EQEmu`'s `Client::Handle_OP_SpawnAppearance` queues
//! it to everyone else), so the session keeps the listing the admission gave
//! and each change since, its own or the server's, and reports each change
//! it sends. `EQEmu` drops a second change of either kind within 250 ms of
//! the last (`anon_toggle_timer`, `afk_toggle_timer`), so the session
//! refuses one sooner. The official client refuses `/anonymous` while
//! roleplaying and `/roleplay` while anonymous (inferred from its having
//! strings for them); `EQEmu` takes either.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    listing::{Anonymity, ListingChange},
    message::Message,
    request::Request,
    world::WorldEvent,
};
use std::time::{Duration, Instant};

/// The official client's strings for `/anonymous` while roleplaying and
/// `/roleplay` while anonymous (`eqstr_us.txt`).
const ANONYMOUS_WHILE_ROLEPLAYING: u32 = 13235;
const ROLEPLAYING_WHILE_ANONYMOUS: u32 = 8873;

/// How soon after a change of a kind the server takes another.
const SETTLING: Duration = Duration::from_millis(250);

/// Away, anonymous and roleplaying, as the player turns them.
#[derive(Default)]
pub(super) struct Listing {
    /// Whether the player is away, once a change says; until then, as the
    /// admission listed them.
    away: Option<bool>,
    /// How the player hides from `/who`, kept the same way.
    anonymity: Option<Anonymity>,
    /// When the session last sent a change of each kind.
    away_sent: Option<Instant>,
    anonymity_sent: Option<Instant>,
}

/// Whether a change sent then is too recent for the server to take another.
fn settling(sent: Option<Instant>, now: Instant) -> bool {
    sent.is_some_and(|sent| now.saturating_duration_since(sent) < SETTLING)
}

impl Listing {
    /// The change a command asks for, from the listing now; or why not, in
    /// this library's words and the official client's string where it has
    /// one.
    fn change(
        &self,
        command: &ClientCommand,
        world: &World,
        now: Instant,
    ) -> Result<ListingChange, (&'static str, Option<u32>)> {
        let listed = world
            .player
            .as_ref()
            .map(|player| player.listing)
            .unwrap_or_default();
        let anonymity = self.anonymity.unwrap_or(listed.anonymity);
        let wait = ("Wait a moment before changing that again.", None);
        match command {
            ClientCommand::ToggleAway { .. } => {
                if settling(self.away_sent, now) {
                    return Err(wait);
                }
                Ok(ListingChange::Away(!self.away.unwrap_or(listed.away)))
            }
            ClientCommand::ToggleAnonymous { .. } | ClientCommand::ToggleRoleplay { .. } => {
                if settling(self.anonymity_sent, now) {
                    return Err(wait);
                }
                let anonymous = matches!(command, ClientCommand::ToggleAnonymous { .. });
                let (wanted, other, refusal) = if anonymous {
                    (
                        Anonymity::Anonymous,
                        Anonymity::Roleplaying,
                        ("Stop roleplaying first.", ANONYMOUS_WHILE_ROLEPLAYING),
                    )
                } else {
                    (
                        Anonymity::Roleplaying,
                        Anonymity::Anonymous,
                        ("Stop being anonymous first.", ROLEPLAYING_WHILE_ANONYMOUS),
                    )
                };
                if anonymity == other {
                    return Err((refusal.0, Some(refusal.1)));
                }
                Ok(ListingChange::Anonymity(if anonymity == wanted {
                    Anonymity::Open
                } else {
                    wanted
                }))
            }
            _ => Err(("Not a listing command.", None)),
        }
    }

    /// Notes a change to the player's listing.
    fn note(&mut self, change: ListingChange, sent: Option<Instant>) {
        match change {
            ListingChange::Away(away) => {
                self.away = Some(away);
                self.away_sent = sent.or(self.away_sent);
            }
            ListingChange::Anonymity(anonymity) => {
                self.anonymity = Some(anonymity);
                self.anonymity_sent = sent.or(self.anonymity_sent);
            }
            _ => {}
        }
    }
}

impl Feature for Listing {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Listing]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::ToggleAway { .. }
                | ClientCommand::ToggleAnonymous { .. }
                | ClientCommand::ToggleRoleplay { .. }
        )
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if !self.owns(command) {
            return Ok(());
        }
        let now = Instant::now();
        let change = match self.change(command, world, now) {
            Ok(change) => change,
            Err((reason, string_id)) => {
                return actions::refuse_officially(command, (reason, string_id), out.log);
            }
        };
        out.request(&match change {
            ListingChange::Away(away) => Request::SetAway(away),
            ListingChange::Anonymity(anonymity) => Request::SetAnonymity(anonymity),
            _ => return Ok(()),
        })?;
        self.note(change, Some(now));
        out.log.send(ClientEvent::World(WorldEvent::ListingSet {
            session_id: world.session_id,
            change,
        }))
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        // The server's own changes, such as its away timer's.
        if let Message::Event(WorldEvent::Listing { spawn_id, change }) = message {
            if world.own_spawn == Some(*spawn_id) {
                self.note(*change, None);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;
    use eq_network_game::listing;

    /// What a command sent or, refused, the official string it named.
    #[derive(Debug, PartialEq)]
    enum Outcome {
        Sent(Vec<eq_network_game::command::EncodedCommand>),
        Refused(Option<u32>),
    }

    fn run(feature: &mut Listing, world: &mut World, command: &ClientCommand) -> Outcome {
        let outcome = testing::run(|out| feature.handle(command, world, out));
        outcome
            .events
            .iter()
            .find_map(|event| match event {
                ClientEvent::World(WorldEvent::ListingRefused { string_id, .. }) => {
                    Some(Outcome::Refused(*string_id))
                }
                _ => None,
            })
            .unwrap_or(Outcome::Sent(outcome.sent))
    }

    /// Lets the server take another change of each kind.
    fn settle(feature: &mut Listing) {
        let before = Instant::now().checked_sub(SETTLING * 2);
        feature.away_sent = before;
        feature.anonymity_sent = before;
    }

    #[test]
    fn away_turns_on_and_off_and_waits_for_the_server_between() {
        let mut feature = Listing::default();
        let mut world = World::new(5);
        let away = ClientCommand::ToggleAway { session_id: 5 };
        let outcome = testing::run(|out| feature.handle(&away, &mut world, out));
        assert_eq!(outcome.sent, [listing::titanium_away(7, true).unwrap()]);
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::ListingSet {
                change: ListingChange::Away(true),
                ..
            })
        )));
        // Too soon for the server to take another.
        assert_eq!(run(&mut feature, &mut world, &away), Outcome::Refused(None));
        settle(&mut feature);
        assert_eq!(
            run(&mut feature, &mut world, &away),
            Outcome::Sent(vec![listing::titanium_away(7, false).unwrap()])
        );
    }

    #[test]
    fn anonymous_and_roleplaying_each_wait_for_the_other_to_end() {
        let mut feature = Listing::default();
        let mut world = World::new(5);
        let anonymous = ClientCommand::ToggleAnonymous { session_id: 5 };
        let roleplay = ClientCommand::ToggleRoleplay { session_id: 5 };
        let sent =
            |anonymity| Outcome::Sent(vec![listing::titanium_anonymity(7, anonymity).unwrap()]);
        assert_eq!(
            run(&mut feature, &mut world, &anonymous),
            sent(Anonymity::Anonymous)
        );
        settle(&mut feature);
        assert_eq!(
            run(&mut feature, &mut world, &roleplay),
            Outcome::Refused(Some(ROLEPLAYING_WHILE_ANONYMOUS))
        );
        assert_eq!(
            run(&mut feature, &mut world, &anonymous),
            sent(Anonymity::Open)
        );
        settle(&mut feature);
        assert_eq!(
            run(&mut feature, &mut world, &roleplay),
            sent(Anonymity::Roleplaying)
        );
        settle(&mut feature);
        assert_eq!(
            run(&mut feature, &mut world, &anonymous),
            Outcome::Refused(Some(ANONYMOUS_WHILE_ROLEPLAYING))
        );
        // The server's word on the player counts too.
        world.own_spawn = Some(7);
        testing::run(|out| {
            feature.observe(
                &Message::Event(WorldEvent::Listing {
                    spawn_id: 7,
                    change: ListingChange::Anonymity(Anonymity::Open),
                }),
                &mut world,
                out,
            )
        })
        .result
        .unwrap();
        assert_eq!(
            run(&mut feature, &mut world, &anonymous),
            sent(Anonymity::Anonymous)
        );
    }
}
