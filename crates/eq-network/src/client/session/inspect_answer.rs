//! The `EQMac` program's answer to another player's inspection (TAKP and
//! Quarm; checklist 6.621, TAKP program read row 11e: 0x4e769d).
//!
//! The servers pass a request on to the player inspected, and that
//! player's program answers itself: `OP_InspectAnswer`, 1,744 bytes, with
//! the name and icon of each of its 22 worn places and its inspect message
//! ([`inspection::eqmac::answer`]), which the servers pass on unread to the
//! one who asked (`Client::Handle_OP_InspectAnswer`, TAKP 44afcbf, Quarm
//! fc75fcf). Source-derived, not live.
//!
//! It answers at most once a second, whoever asks: a request inside the
//! second after the last answer is dropped (the program's [0x7f6f68]
//! stamp). Each answer says line 0x57c with the asker's name when they are
//! a spawn the session knows other than the player; the program says it
//! only while an option ([0x7cf208]) is on, which is unread, so here it is
//! always said (labelled, 6.621).
//!
//! Asking to inspect another player, and showing an answer received, are
//! unread on the `EQMac` program and not offered.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    inspection::{self, Equipment},
    inventory::InventorySlot,
    message::Message,
    world::{Capability, WorldEvent},
};
use std::time::{Duration, Instant};

/// How soon after an answer the program answers again.
const ANSWER_EVERY: Duration = Duration::from_millis(1000);
/// The line each answer says.
const ANSWERED_LINE: u32 = 0x57c;

/// The player's inspect message and when the last answer went.
#[derive(Default)]
pub(super) struct MacInspectAnswer {
    note: Option<String>,
    last: Option<Instant>,
}

impl MacInspectAnswer {
    /// The 22 worn places, in the program's order: `EQMac` slots 0 to 21.
    fn equipment(world: &World) -> Vec<Equipment> {
        (0..i32::try_from(inspection::eqmac::PLACES).unwrap_or(22))
            .map(|place| {
                InventorySlot::from_eqmac(place)
                    .and_then(|slot| world.inventory.items().get(&slot))
                    .map_or(
                        Equipment {
                            name: String::new(),
                            icon: None,
                        },
                        |item| Equipment {
                            name: item.details.name.clone(),
                            icon: item.details.icon,
                        },
                    )
            })
            .collect()
    }

    /// Answers `inspector` at `now`, unless an answer went within the
    /// second.
    fn answer_at(
        &mut self,
        inspector: u16,
        now: Instant,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) < ANSWER_EVERY)
        {
            return out.log.diagnostic(format!(
                "Inspect request from {inspector} dropped: answered within the second"
            ));
        }
        let Some(own) = world.own_spawn else {
            return Ok(());
        };
        let answer = match inspection::eqmac::answer(
            inspector,
            own,
            &Self::equipment(world),
            self.note.as_deref().unwrap_or_default(),
        ) {
            Ok(answer) => answer,
            Err(error) => {
                return out.log.diagnostic(format!(
                    "Inspect request from {inspector} not answered: {error}"
                ))
            }
        };
        out.send(&answer)?;
        self.last = Some(now);
        if inspector == own || !(1..5000).contains(&inspector) {
            return Ok(());
        }
        let Some(asker) = world.spawns.get(inspector) else {
            return Ok(());
        };
        out.log.send(ClientEvent::World(WorldEvent::InspectedBy {
            session_id: world.session_id,
            inspector_id: inspector,
            name: asker.name.clone(),
            string_id: ANSWERED_LINE,
        }))
    }
}

impl Feature for MacInspectAnswer {
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::InspectAnswer]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::SetInspectNote { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let ClientCommand::SetInspectNote { session_id, note } = command {
            if *session_id != world.session_id {
                return super::actions::refuse(command, "That is for another session.", out.log);
            }
            self.note.clone_from(note);
        }
        Ok(())
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::Event(WorldEvent::InspectRequest { inspector_id, .. }) = message {
            return self.answer_at(*inspector_id, Instant::now(), world, out);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::{
        command::EncodedCommand,
        inventory::{Inventory, InventoryUpdate},
        world::SpawnKind,
    };

    /// Player 7 wearing an item in `EQMac`'s slot 0 and a ring in slot 15,
    /// beside player 12, "Asker".
    fn world() -> World {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        let mut asker = testing::spawn(12, SpawnKind::Player);
        asker.name = "Asker".into();
        world.spawns.insert(asker);
        let mut first = testing::item(InventorySlot::from_eqmac(0).unwrap().0);
        first.details.name = "First".into();
        first.details.icon = Some(640);
        let mut ring = testing::item(15);
        ring.details.name = "Ring".into();
        ring.details.icon = Some(512);
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![first, ring]));
        world.inventory = inventory.into();
        world
    }

    fn expected(world: &World, inspector: u16, note: &str) -> EncodedCommand {
        let mut equipment = vec![
            Equipment {
                name: String::new(),
                icon: None,
            };
            22
        ];
        equipment[0] = Equipment {
            name: "First".into(),
            icon: Some(640),
        };
        equipment[15] = Equipment {
            name: "Ring".into(),
            icon: Some(512),
        };
        assert_eq!(MacInspectAnswer::equipment(world), equipment);
        inspection::eqmac::answer(inspector, 7, &equipment, note).unwrap()
    }

    fn lines(events: &[ClientEvent]) -> Vec<(u16, String, u32)> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::InspectedBy {
                    inspector_id,
                    name,
                    string_id,
                    ..
                }) => Some((*inspector_id, name.clone(), *string_id)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_request_is_answered_with_the_worn_places_and_the_message() {
        let mut world = world();
        let mut feature = MacInspectAnswer::default();
        let note = ClientCommand::SetInspectNote {
            session_id: 5,
            note: Some("Hail".into()),
        };
        testing::run(|out| feature.handle(&note, &mut world, out))
            .result
            .unwrap();
        let request = Message::Event(WorldEvent::InspectRequest {
            target_id: 7,
            inspector_id: 12,
        });
        let outcome = testing::run(|out| feature.observe(&request, &mut world, out));
        assert!(outcome.result.is_ok());
        assert_eq!(outcome.sent, [expected(&world, 12, "Hail")]);
        assert_eq!(lines(&outcome.events), [(12, "Asker".into(), 0x57c)]);
    }

    /// N4: at most one answer a second, whoever asks; the rest are dropped.
    #[test]
    fn answers_come_at_most_once_a_second() {
        let world = world();
        let mut feature = MacInspectAnswer::default();
        let start = Instant::now();
        let at = |millis| start + Duration::from_millis(millis);
        for (millis, inspector, answered) in [
            (0, 12, true),
            (1, 12, false),
            (999, 40, false),
            (1000, 40, true),
            (1500, 12, false),
            (2000, 12, true),
        ] {
            let outcome = testing::run(|out| feature.answer_at(inspector, at(millis), &world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent.len(), usize::from(answered), "{millis} ms");
        }
    }

    /// The line names only a known spawn other than the player; the
    /// answer goes either way, with an empty message when none is set.
    #[test]
    fn the_line_names_only_a_known_other_spawn() {
        let world = world();
        for inspector in [7, 40, 5000] {
            let mut feature = MacInspectAnswer::default();
            let outcome =
                testing::run(|out| feature.answer_at(inspector, Instant::now(), &world, out));
            assert!(outcome.result.is_ok());
            assert_eq!(outcome.sent, [expected(&world, inspector, "")]);
            assert_eq!(lines(&outcome.events), [], "{inspector}");
        }
    }
}
