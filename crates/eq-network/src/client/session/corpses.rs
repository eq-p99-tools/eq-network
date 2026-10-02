//! Players' corpses: `/consent` and `/deny`, `/corpse`, `/corpsedrag` and
//! `/corpsedrop`. The host picks a corpse by its spawn; the session names it
//! as the server knows it (`Name's corpse12`) and refuses what the server
//! would drop without a word. The server says the rest in its own string
//! table messages, and a dragged corpse moves as any spawn does.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::{request::Request, world::SpawnKind};

/// Consents, summons and drags the player's and others' corpses.
pub(super) struct Corpses;

/// The official client's string (`eqstr_us.txt`) refusing a consent to no
/// name.
const INVALID_NAME: u32 = 397;
/// Its string refusing a consent to the player themselves.
const YOURSELF: u32 = 399;

/// Why a command is not sent: this library's words, and the official
/// client's string where it refuses in its own.
struct Refused(String, Option<u32>);

impl From<&str> for Refused {
    fn from(reason: &str) -> Self {
        Self(reason.into(), None)
    }
}

/// The request a command makes, or why it is not sent.
fn request(command: &ClientCommand, world: &World) -> Result<Request, Refused> {
    let player = world.player.as_ref().ok_or("Not in the zone yet")?;
    // Servers ignore anything but a player's corpse.
    let corpse = |spawn_id: u16| -> Result<String, Refused> {
        let spawn = world
            .spawns
            .all()
            .find(|spawn| spawn.spawn_id == spawn_id)
            .ok_or("That corpse is not here")?;
        if spawn.kind == SpawnKind::PlayerCorpse {
            Ok(spawn.name.clone())
        } else {
            Err("That is not a player's corpse".into())
        }
    };
    Ok(match command {
        // Names the official client will not send, which it refuses in its
        // own words (eqstr 397 and 399).
        ClientCommand::Consent { name, .. } if name.trim().is_empty() => {
            return Err(Refused(
                "That is not a name you can consent".into(),
                Some(INVALID_NAME),
            ));
        }
        ClientCommand::Consent { name, .. } if name.eq_ignore_ascii_case(&player.name) => {
            return Err(Refused(
                "You do not need to consent yourself".into(),
                Some(YOURSELF),
            ));
        }
        ClientCommand::Consent { name, given, .. } => Request::Consent {
            name: name.trim().into(),
            given: *given,
        },
        ClientCommand::SummonCorpse { spawn_id, .. } => Request::SummonCorpse(corpse(*spawn_id)?),
        ClientCommand::DragCorpse { spawn_id, .. } => Request::DragCorpse(corpse(*spawn_id)?),
        ClientCommand::DropCorpse { spawn_id, .. } => Request::DropCorpse {
            corpse: spawn_id.map(corpse).transpose()?,
        },
        _ => return Err("Not a corpse command".into()),
    })
}

impl Feature for Corpses {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Corpses]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::Consent { .. }
                | ClientCommand::SummonCorpse { .. }
                | ClientCommand::DragCorpse { .. }
                | ClientCommand::DropCorpse { .. }
        )
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        // A name the packet cannot carry is refused as the feature's own
        // reasons are.
        let packet = request(command, world).and_then(|request| {
            out.encode(&request)
                .map_err(|error| Refused(error.to_string(), None))
        });
        match packet {
            Ok(packet) => out.send(&packet),
            Err(Refused(reason, official)) => {
                actions::refuse_officially(command, (&reason, official), out.log)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::corpses;
    use eq_network_game::world::WorldEvent;

    fn refusals(events: &[super::super::ClientEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                super::super::ClientEvent::World(WorldEvent::CorpseRefused { reason, .. }) => {
                    Some(reason.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn official(events: &[super::super::ClientEvent]) -> Vec<Option<u32>> {
        events
            .iter()
            .filter_map(|event| match event {
                super::super::ClientEvent::World(WorldEvent::CorpseRefused {
                    string_id, ..
                }) => Some(*string_id),
                _ => None,
            })
            .collect()
    }

    /// Player 7, "Tester", with their corpse (4) and an NPC's (5) nearby.
    fn world() -> World {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.name = "Tester".into();
        world.player.admit(player);
        let mut corpse = testing::spawn(4, SpawnKind::PlayerCorpse);
        corpse.name = "Tester's corpse4".into();
        world.spawns.insert(corpse);
        world.spawns.insert(testing::spawn(5, SpawnKind::NpcCorpse));
        world
    }

    #[test]
    fn consents_name_another_player() {
        let mut world = world();
        let consent = |name: &str, given| ClientCommand::Consent {
            session_id: 5,
            name: name.into(),
            given,
        };
        let outcome = testing::run(|out| Corpses.handle(&consent("Helper", true), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [corpses::consent("Helper", true).unwrap()]);
        let outcome =
            testing::run(|out| Corpses.handle(&consent("Helper", false), &mut world, out));
        assert_eq!(outcome.sent, [corpses::consent("Helper", false).unwrap()]);
        for (name, reason, string_id) in [
            ("  ", "That is not a name you can consent", INVALID_NAME),
            ("tester", "You do not need to consent yourself", YOURSELF),
        ] {
            let outcome = testing::run(|out| Corpses.handle(&consent(name, true), &mut world, out));
            outcome.result.unwrap();
            assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
            assert_eq!(refusals(&outcome.events), [reason]);
            assert_eq!(official(&outcome.events), [Some(string_id)]);
        }
    }

    #[test]
    fn summons_and_drags_name_a_player_corpse() {
        let mut world = world();
        let summon = ClientCommand::SummonCorpse {
            session_id: 5,
            spawn_id: 4,
        };
        let outcome = testing::run(|out| Corpses.handle(&summon, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [corpses::summon("Tester's corpse4", "Tester").unwrap()]
        );
        let drag = |spawn_id| ClientCommand::DragCorpse {
            session_id: 5,
            spawn_id,
        };
        let outcome = testing::run(|out| Corpses.handle(&drag(4), &mut world, out));
        assert_eq!(
            outcome.sent,
            [corpses::drag("Tester's corpse4", "Tester").unwrap()]
        );
        for (spawn_id, reason) in [
            (5, "That is not a player's corpse"),
            (6, "That corpse is not here"),
        ] {
            let outcome = testing::run(|out| Corpses.handle(&drag(spawn_id), &mut world, out));
            outcome.result.unwrap();
            assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
            assert_eq!(refusals(&outcome.events), [reason]);
        }
        let drop = |spawn_id| ClientCommand::DropCorpse {
            session_id: 5,
            spawn_id,
        };
        let outcome = testing::run(|out| Corpses.handle(&drop(Some(4)), &mut world, out));
        assert_eq!(
            outcome.sent,
            [corpses::release(Some("Tester's corpse4")).unwrap()]
        );
        let outcome = testing::run(|out| Corpses.handle(&drop(None), &mut world, out));
        assert_eq!(outcome.sent, [corpses::release(None).unwrap()]);
    }
}
