//! The Titanium program's event saves: `OP_Save` (0x736b), which 0x4a1350
//! sends on certain events with the event's reason at +4 (reads R49-22g and
//! R49-22h, checklist 6.615). The program has no timed save: its 300-tick
//! test compares a byte with 300, so it never passes (R49-22g), and the
//! recordings show none.
//!
//! Every event save writes the player's place first (`write_position` 1),
//! so it carries where the player is at that moment (R49-22h §4), not the
//! last place reported. It goes from the same encoder as the zone save
//! ([`zoning::titanium_event_save`]).
//!
//! The events this session saves on (slice 1): `/roleplay` (0xa) and
//! `/anonymous` (0xb), after their appearance update; the player closing a
//! merchant's window (0x14, which window the program's site is, is
//! inferred); and the server's `OP_LootComplete` or `OP_EndLootRequest`
//! (0x15). The program's other sites wait on their reads (the plan's table).
//!
//! 0x4a1350 sends nothing when ([`withheld`]):
//! - there is no own spawn, or it is not a player's (type 0): the player is
//!   dead, and their spawn a corpse;
//! - a merchant or trade is pending ([0x9248d4] or [0x9248d0]): the save is
//!   deferred to a timer that never fires, so it is dropped.
//!
//! Its other gates are unread here and not modelled: [0x905ca4] being 1,
//! [0x923798] or [0x923884] set, character +0x104 at 4 or more, profile
//! +0x11b0 under 1, the game state not 5, or [0x923788], [0x923d24],
//! [0x9917e4] or `0x4f288c()` being 0.
//!
//! Which server types take it is [`Gates::taken`]: stock `EQEmu` saves the
//! character on any `OP_Save` and reads nothing in the body.
use super::feature::{Out, World};
use anyhow::Result;
use eq_network_game::{request::Request, zoning};

/// What the event saves read beyond the world.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Gates {
    /// Whether the server type takes the program's event saves.
    pub(super) taken: bool,
    /// Whether a merchant's window is open, which only the inventory
    /// feature changes: the program defers an event save then (0x4a1350,
    /// [0x9248d4]; which window the flag is, is inferred).
    pub(super) merchant_open: bool,
}

/// Why an event save was not sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Withheld {
    /// The server type doesn't take event saves here.
    NotTaken,
    /// The player has no own spawn or place yet.
    NoSpawn,
    /// The player is dead: their spawn is a corpse.
    Dead,
    /// A merchant's window or a trade is open.
    Pending,
}

/// Why the program would send no event save now, if it wouldn't.
pub(super) fn withheld(world: &World) -> Option<Withheld> {
    if !world.saves.taken {
        Some(Withheld::NotTaken)
    } else if world.own_spawn.is_none() || world.body.position().is_none() {
        Some(Withheld::NoSpawn)
    } else if world.lifecycle.is_dead() {
        Some(Withheld::Dead)
    } else if world.saves.merchant_open || world.exchange.is_some() {
        Some(Withheld::Pending)
    } else {
        None
    }
}

/// Sends the event save for `reason` ([`zoning::save_reason`]) where the
/// program would, from where the player is now.
pub(super) fn event(world: &World, reason: u8, out: &mut Out<'_, '_>) -> Result<()> {
    if let Some(why) = withheld(world) {
        if why == Withheld::NotTaken {
            return Ok(());
        }
        return out
            .log
            .diagnostic(format!("Event save 0x{reason:02x} not sent: {why:?}"));
    }
    let save = zoning::Save {
        position: world.body.position(),
        // The profile's boat name, as every save carries it. Aboard, which
        // frame the program's spawn place is in is unread.
        vehicle: world.vehicle.clone(),
    };
    out.request(&Request::EventSave { save, reason })
}

#[cfg(test)]
mod tests {
    use super::super::{
        exchange::{Exchange, Exchanging, Stage},
        feature::{testing, Feature},
        listing::Listing,
        looting::Looting,
        ClientCommand, ClientEvent,
    };
    use super::*;
    use crate::client::{ClientConfig, ServerProtocol};
    use eq_network_game::{
        command::EncodedCommand,
        exchange::Partner,
        loot::LootUpdate,
        message::Message,
        movement::MotionSession,
        world::{Position, SpawnKind, WorldEvent},
    };
    use std::time::Instant;

    /// Player 7, admitted and standing where the profile left them, on a
    /// server of this type.
    fn standing(protocol: ServerProtocol) -> World {
        let mut config = ClientConfig::new("Example", "Fixture", "Local", "Tester");
        config.protocol = protocol;
        let mut world = World::for_zone(5, &config);
        let player = testing::player(7);
        let motion = MotionSession::new(5, 7, player.position, Instant::now()).unwrap();
        world.body.admit(motion);
        world.player.admit(player);
        world.own_spawn = Some(7);
        world
    }

    /// The event save the program would send from where the player is now.
    fn saved(world: &World, reason: u8) -> EncodedCommand {
        zoning::titanium_event_save(
            &zoning::Save {
                position: world.body.position(),
                vehicle: None,
            },
            reason,
        )
    }

    fn sends(world: &World, reason: u8) -> Vec<EncodedCommand> {
        let outcome = testing::run(|out| event(world, reason, out));
        outcome.result.unwrap();
        outcome.sent
    }

    #[test]
    fn stock_eqemu_takes_each_event_save_from_where_the_player_is_now() {
        let mut world = standing(ServerProtocol::EqEmu);
        let profile = world.body.position().unwrap();
        // V1: the place is the spawn's at the save, not the profile's.
        let moved = Position {
            x: profile.x + 40.0,
            y: profile.y - 12.5,
            z: profile.z,
            heading: 100.75,
        };
        world.correct_own(moved, Instant::now()).unwrap();
        let now = world.body.position().unwrap();
        assert_eq!(
            (now.x, now.y, now.heading),
            (moved.x, moved.y, moved.heading)
        );
        for reason in [
            zoning::save_reason::ROLEPLAY,
            zoning::save_reason::ANONYMOUS,
            zoning::save_reason::MERCHANT_CLOSED,
            zoning::save_reason::LOOT_ENDED,
        ] {
            let sent = sends(&world, reason);
            assert_eq!(sent, [saved(&world, reason)], "reason {reason:#04x}");
            assert_eq!(sent[0].opcode, 0x736b);
            // The heading is cut to a whole number, as the zone save's is.
            assert_eq!(&sent[0].body[0x11..0x15], &100i32.to_le_bytes());
        }
    }

    #[test]
    fn other_servers_take_no_event_saves_and_say_nothing() {
        for protocol in [
            // V2: built, held off until an official capture shows 0x736b.
            ServerProtocol::Project1999,
            ServerProtocol::Quarm,
            ServerProtocol::Takp,
            ServerProtocol::EqEmuRof2,
        ] {
            let world = standing(protocol);
            assert_eq!(withheld(&world), Some(Withheld::NotTaken));
            let outcome = testing::run(|out| event(&world, 0x0a, out));
            outcome.result.unwrap();
            assert!(outcome.sent.is_empty(), "{protocol:?}");
            assert!(outcome.events.is_empty(), "{protocol:?}");
        }
    }

    #[test]
    fn the_dead_no_spawn_and_a_pending_merchant_or_trade_send_nothing() {
        let refused = |world: &World, why| {
            assert_eq!(withheld(world), Some(why));
            let outcome = testing::run(|out| event(world, 0x0b, out));
            outcome.result.unwrap();
            assert!(outcome.sent.is_empty(), "{why:?}");
            assert!(
                outcome
                    .events
                    .iter()
                    .any(|event| matches!(event, ClientEvent::Diagnostic(_))),
                "{why:?}"
            );
        };
        let mut world = standing(ServerProtocol::EqEmu);
        world.lifecycle.mark_dead();
        refused(&world, Withheld::Dead);

        let mut world = standing(ServerProtocol::EqEmu);
        world.own_spawn = None;
        refused(&world, Withheld::NoSpawn);
        let mut config = ClientConfig::new("Example", "Fixture", "Local", "Tester");
        config.protocol = ServerProtocol::EqEmu;
        refused(&World::for_zone(5, &config), Withheld::NoSpawn);

        let mut world = standing(ServerProtocol::EqEmu);
        world.saves.merchant_open = true;
        refused(&world, Withheld::Pending);

        let mut world = standing(ServerProtocol::EqEmu);
        world.exchange = Exchanging::from(Exchange {
            with: 42,
            partner: Partner::Npc,
            stage: Stage::Open,
        });
        refused(&world, Withheld::Pending);
    }

    #[test]
    fn roleplay_and_anonymous_save_after_their_appearance_update() {
        let mut world = standing(ServerProtocol::EqEmu);
        let mut listing = Listing::new(false);
        for (command, reason) in [
            (
                ClientCommand::ToggleRoleplay { session_id: 5 },
                zoning::save_reason::ROLEPLAY,
            ),
            (
                ClientCommand::ToggleAnonymous { session_id: 5 },
                zoning::save_reason::ANONYMOUS,
            ),
        ] {
            listing.settled_for_tests();
            let outcome = testing::run(|out| listing.handle(&command, &mut world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent.len(), 2, "{command:?}");
            assert_eq!(outcome.sent[0].opcode, 0x7c32);
            assert_eq!(outcome.sent[1], saved(&world, reason));
        }
        // Away is no event save.
        listing.settled_for_tests();
        let away = ClientCommand::ToggleAway { session_id: 5 };
        let outcome = testing::run(|out| listing.handle(&away, &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.sent.iter().all(|sent| sent.opcode != 0x736b));
    }

    #[test]
    fn the_servers_end_of_an_open_loot_saves_and_a_stray_one_does_not() {
        let mut world = standing(ServerProtocol::EqEmu);
        world.spawns.insert(testing::spawn(8, SpawnKind::NpcCorpse));
        let mut looting = Looting::default();
        let closed = || Message::Event(WorldEvent::Loot(LootUpdate::Closed));
        let deliver = |looting: &mut Looting, world: &mut World| {
            let mut message = closed();
            looting.explain(&mut message, world);
            let outcome = testing::run(|out| looting.observe(&message, world, out));
            outcome.result.unwrap();
            outcome.sent
        };
        // No loot open: the close is withheld, and nothing saves.
        assert_eq!(deliver(&mut looting, &mut world), []);
        let open = ClientCommand::Loot {
            session_id: 5,
            corpse_id: 8,
            created: Instant::now(),
        };
        testing::run(|out| looting.handle(&open, &mut world, out))
            .result
            .unwrap();
        assert_eq!(
            deliver(&mut looting, &mut world),
            [saved(&world, zoning::save_reason::LOOT_ENDED)]
        );
    }
}
