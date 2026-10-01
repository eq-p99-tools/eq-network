//! Doors: the zone's door table and the player's clicks on them.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{anyhow, ensure, Result};
use eq_network_game::{
    command::EncodedCommand,
    doors::DoorUpdate,
    message::Message,
    world::{WorldEvent, WorldEvent::DoorAction},
};
use std::time::Instant;

/// The zone's doors in this admission.
pub(super) struct Doors {
    table: eq_network_game::doors::Doors,
    /// When the table last gained or lost doors; older clicks are stale.
    changed_at: Instant,
}

impl Default for Doors {
    fn default() -> Self {
        Self {
            table: eq_network_game::doors::Doors::default(),
            changed_at: Instant::now(),
        }
    }
}

impl Doors {
    /// The click packet for a door in reach, clicked since the door table last
    /// changed.
    fn click(&self, command: &ClientCommand, world: &World) -> Result<EncodedCommand> {
        let ClientCommand::ClickDoor {
            door_id, created, ..
        } = command
        else {
            unreachable!("only door clicks are clicked");
        };
        ensure!(*created >= self.changed_at, "stale door request");
        let (spawn_id, position) = world
            .player_at()
            .ok_or_else(|| anyhow!("player is unavailable"))?;
        self.table.click_packet(*door_id, spawn_id, position)
    }
}

impl Feature for Doors {
    fn admit(&mut self, message: &Message, _world: &mut World) -> Result<()> {
        if let Message::Event(WorldEvent::Doors(update)) = message {
            self.table.apply(update);
        }
        Ok(())
    }

    fn admitted(&mut self, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        out.log.send(ClientEvent::World(WorldEvent::Doors(
            self.table.admission(),
        )))
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::ClickDoor { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::ClickDoor { door_id, .. } = command else {
            return Ok(());
        };
        let error = match self.click(command, world) {
            Ok(packet) => {
                out.send(&packet)?;
                None
            }
            Err(error) => Some(error.to_string()),
        };
        out.log.send(ClientEvent::World(DoorAction {
            session_id: world.session_id,
            door_id: *door_id,
            error,
        }))
    }

    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::Event(WorldEvent::Doors(update)) = message {
            if matches!(update, DoorUpdate::Spawn(_) | DoorUpdate::RemoveAll) {
                self.changed_at = Instant::now();
            }
            self.table.apply(update);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::{doors::Door, world::Position};
    use std::time::Duration;

    #[test]
    fn a_fresh_click_on_a_door_in_reach_sends_the_click_and_reports_it() {
        let mut doors = Doors::default();
        let spawn = WorldEvent::Doors(DoorUpdate::Spawn(vec![Door {
            id: 1,
            model: "DOOR".into(),
            position: Position::default(),
            incline: 0,
            size: 100,
            open_type: 0,
            state_at_spawn: 0,
            invert_state: 0,
            parameter: 0,
            action: None,
        }]));
        doors
            .admit(&Message::Event(spawn), &mut World::new(5))
            .unwrap();
        let mut world = World::new(5);
        let click = ClientCommand::ClickDoor {
            session_id: 5,
            door_id: 1,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| doors.handle(&click, &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty());
        assert!(matches!(
            &outcome.events[..],
            [ClientEvent::World(DoorAction { door_id: 1, error: Some(error), .. })]
                if error == "player is unavailable"
        ));
        world.player = Some(testing::player(7));
        let outcome = testing::run(|out| doors.handle(&click, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, eq_network_game::doors::CLICK_OPCODE);
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(DoorAction {
                door_id: 1,
                error: None,
                ..
            })]
        ));
    }

    #[test]
    fn clicks_made_before_the_doors_last_changed_are_stale() {
        let doors = Doors::default();
        let world = World::new(5);
        let click = |session_id, created| ClientCommand::ClickDoor {
            session_id,
            door_id: 1,
            created,
        };
        let error = |command: &ClientCommand| doors.click(command, &world).unwrap_err().to_string();
        let before = doors
            .changed_at
            .checked_sub(Duration::from_millis(1))
            .unwrap();
        assert_eq!(error(&click(5, before)), "stale door request");
        assert_eq!(error(&click(5, Instant::now())), "player is unavailable");
    }
}
