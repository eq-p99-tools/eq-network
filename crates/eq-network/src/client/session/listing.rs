//! How `/who` lists the player: away from the keyboard, anonymous or
//! roleplaying. The player says so with an appearance update, which Titanium
//! servers never echo (`EQEmu`'s `Client::Handle_OP_SpawnAppearance` queues
//! it to everyone else), so the session keeps the listing the admission gave
//! and each change since, its own or the server's, and reports each change
//! it sends. `EQEmu` drops a second change of either kind within 250 ms of
//! the last (`anon_toggle_timer`, `afk_toggle_timer`), measured between its
//! own receipts, so the session refuses one within twice that of its own
//! last send: jitter on the way cannot then make the server drop a change
//! the session reported as made. `/roleplay` flips the roleplaying bit and
//! keeps the anonymous one, and `/anonymous` sets anonymous or clears both,
//! as both programs do (Titanium sitting 9l; TAKP, read 10-31): neither
//! refuses one while the other is on.
//!
//! On a client generation whose away update is unread (the `EQMac` client),
//! the session lists anonymity and roleplaying only.
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

/// How soon after a change of a kind the session sends another: twice the
/// 250 ms the server waits between its receipts.
const SETTLING: Duration = Duration::from_millis(500);

/// Away, anonymous and roleplaying, as the player turns them.
#[derive(Default)]
pub(super) struct Listing {
    /// Whether the generation's away update is known, so `/afk` is sent.
    away_known: bool,
    /// Whether the player is away, once a change says; until then, as the
    /// admission listed them.
    away: Option<bool>,
    /// How the player hides from `/who`, kept the same way.
    anonymity: Option<Anonymity>,
    /// When the session last sent a change of each kind.
    away_sent: Option<Instant>,
    anonymity_sent: Option<Instant>,
}

impl Listing {
    /// The listing, with or without the away update.
    pub(super) fn new(away_known: bool) -> Self {
        Self {
            away_known,
            ..Self::default()
        }
    }
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
                if !self.away_known {
                    return Err(("Not available on this server", None));
                }
                if settling(self.away_sent, now) {
                    return Err(wait);
                }
                Ok(ListingChange::Away(!self.away.unwrap_or(listed.away)))
            }
            ClientCommand::ToggleAnonymous { .. } | ClientCommand::ToggleRoleplay { .. } => {
                if settling(self.anonymity_sent, now) {
                    return Err(wait);
                }
                Ok(ListingChange::Anonymity(
                    if matches!(command, ClientCommand::ToggleAnonymous { .. }) {
                        anonymity.anonymous_toggled()
                    } else {
                        anonymity.roleplay_toggled()
                    },
                ))
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

/// The Titanium client's show-helm option: sent on every change of it, as
/// the program sends it (sitting 8f). Nothing comes back; the server shows
/// the helm, or not, to everyone else.
pub(super) struct Helm;

impl Feature for Helm {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::ShowingHelm]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::ShowHelm { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::ShowHelm { show, .. } = *command else {
            return Ok(());
        };
        out.request(&Request::ShowHelm(show))
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
        let mut feature = Listing::new(true);
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
    fn roleplay_flips_its_bit_and_anonymous_sets_or_clears_both() {
        let mut feature = Listing::new(true);
        let mut world = World::new(5);
        let anonymous = ClientCommand::ToggleAnonymous { session_id: 5 };
        let roleplay = ClientCommand::ToggleRoleplay { session_id: 5 };
        let sent =
            |anonymity| Outcome::Sent(vec![listing::titanium_anonymity(7, anonymity).unwrap()]);
        // 0 → 1, then /roleplay 1 → 3, 3 → 1.
        assert_eq!(
            run(&mut feature, &mut world, &anonymous),
            sent(Anonymity::Anonymous)
        );
        settle(&mut feature);
        assert_eq!(
            run(&mut feature, &mut world, &roleplay),
            sent(Anonymity::AnonymousRoleplaying)
        );
        settle(&mut feature);
        assert_eq!(
            run(&mut feature, &mut world, &roleplay),
            sent(Anonymity::Anonymous)
        );
        settle(&mut feature);
        // /roleplay 1 → 3, then /anonymous from 3 writes 0.
        assert_eq!(
            run(&mut feature, &mut world, &roleplay),
            sent(Anonymity::AnonymousRoleplaying)
        );
        settle(&mut feature);
        assert_eq!(
            run(&mut feature, &mut world, &anonymous),
            sent(Anonymity::Open)
        );
        settle(&mut feature);
        // 0 → 2, and /anonymous from 2 writes 1.
        assert_eq!(
            run(&mut feature, &mut world, &roleplay),
            sent(Anonymity::Roleplaying)
        );
        settle(&mut feature);
        assert_eq!(
            run(&mut feature, &mut world, &anonymous),
            sent(Anonymity::Anonymous)
        );
        // The server's word on the player counts too.
        settle(&mut feature);
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
            run(&mut feature, &mut world, &roleplay),
            sent(Anonymity::Roleplaying)
        );
    }

    #[test]
    fn the_helm_option_sends_type_43_on_each_change() {
        let mut world = World::new(5);
        for show in [false, true] {
            let outcome = testing::run(|out| {
                Helm.handle(
                    &ClientCommand::ShowHelm {
                        session_id: 5,
                        show,
                    },
                    &mut world,
                    out,
                )
            });
            assert_eq!(
                outcome.sent,
                [listing::titanium_show_helm(7, show).unwrap()]
            );
            assert_eq!(&outcome.sent[0].body[2..4], &43u16.to_le_bytes());
        }
    }

    #[test]
    fn a_generation_without_the_away_update_refuses_afk_and_lists_the_rest() {
        let mut feature = Listing::new(false);
        let mut world = World::new(5);
        assert_eq!(
            run(
                &mut feature,
                &mut world,
                &ClientCommand::ToggleAway { session_id: 5 }
            ),
            Outcome::Refused(None)
        );
        assert!(matches!(
            run(
                &mut feature,
                &mut world,
                &ClientCommand::ToggleRoleplay { session_id: 5 }
            ),
            Outcome::Sent(_)
        ));
    }
}
