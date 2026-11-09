//! How `/who` lists the player: away from the keyboard, anonymous or
//! roleplaying. The player sends an appearance update. `EQEmu`'s
//! `Client::Handle_OP_SpawnAppearance` relays it to other players and can
//! drop repeated updates within 250 ms of each other. Its local display
//! follows the last submitted value. Project 1999 uses the Titanium slash
//! toggle rules and reports submission separately from received appearance.
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

/// Time between sends of the same kind. Twice `EQEmu`'s 250 ms receiver
/// interval and an inferred local pacing limit on P99 (6.404).
const SETTLING: Duration = Duration::from_millis(500);

/// Away, anonymous and roleplaying, as the player turns them.
#[derive(Default)]
pub(super) struct Listing {
    /// Whether the generation's away update is known, so `/afk` is sent.
    away_known: bool,
    /// Use the native P99 toggle rules and keep sent values separate from
    /// received listing state.
    project1999: bool,
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

    pub(super) fn project1999() -> Self {
        Self {
            away_known: true,
            project1999: true,
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
        let player = world.player.as_ref();
        if self.project1999 && player.is_none_or(|player| player.spawn_id == 0) {
            return Err(("Your listing is not ready yet.", None));
        }
        let listed = player.map(|player| player.listing).unwrap_or_default();
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
                        if self.project1999 && anonymity == Anonymity::Roleplaying {
                            // Titanium's slash command toggles any nonzero value off
                            // (0x472578); its options control is a different path.
                            Anonymity::Open
                        } else {
                            anonymity.anonymous_toggled()
                        }
                    } else if self.project1999 && anonymity == Anonymity::Anonymous {
                        // The Titanium slash command refuses enabling roleplay
                        // from anonymous (0x46a302).
                        return Err(("Turn off anonymity before enabling roleplay.", Some(8873)));
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
        out.log.send(ClientEvent::World(if self.project1999 {
            WorldEvent::ListingRequested {
                session_id: world.session_id,
                change,
            }
        } else {
            WorldEvent::ListingSet {
                session_id: world.session_id,
                change,
            }
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
    fn p99_slash_toggles_follow_the_native_anonymity_transitions() {
        let mut feature = Listing::project1999();
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        let anonymous = ClientCommand::ToggleAnonymous { session_id: 5 };
        let roleplay = ClientCommand::ToggleRoleplay { session_id: 5 };
        feature.anonymity = Some(Anonymity::Anonymous);
        assert_eq!(
            run(&mut feature, &mut world, &roleplay),
            Outcome::Refused(Some(8873))
        );
        feature.anonymity = Some(Anonymity::Roleplaying);
        assert_eq!(
            run(&mut feature, &mut world, &anonymous),
            Outcome::Sent(vec![
                listing::titanium_anonymity(7, Anonymity::Open).unwrap()
            ])
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
