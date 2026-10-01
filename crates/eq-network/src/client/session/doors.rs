//! Doors: the zone's door table and the player's clicks on them.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{anyhow, ensure, Result};
use eq_network_game::{
    doors::DoorUpdate,
    world::{WorldEvent, WorldEvent::DoorAction},
};
use std::time::{Duration, Instant};

/// Titanium's door click.
const CLICK_OPCODE: u16 = 0x043b;

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
    /// The click packet for a fresh request on a door in reach.
    fn click(&self, command: &ClientCommand, world: &World) -> Result<[u8; 16]> {
        let ClientCommand::ClickDoor {
            session_id,
            door_id,
            created,
        } = command
        else {
            unreachable!("only door clicks are clicked");
        };
        let now = Instant::now();
        ensure!(
            *session_id == world.session_id
                && *created >= self.changed_at
                && *created <= now
                && now.duration_since(*created) < Duration::from_secs(1),
            "stale door request"
        );
        let (spawn_id, position) = world
            .player_at()
            .ok_or_else(|| anyhow!("player is unavailable"))?;
        self.table.click_packet(*door_id, spawn_id, position)
    }
}

impl Feature for Doors {
    fn admit(&mut self, event: &WorldEvent) {
        if let WorldEvent::Doors(update) = event {
            self.table.apply(update);
        }
    }

    fn admission(&self) -> Option<WorldEvent> {
        Some(WorldEvent::Doors(self.table.admission()))
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<bool> {
        let ClientCommand::ClickDoor { door_id, .. } = command else {
            return Ok(false);
        };
        let error = match self.click(command, world) {
            Ok(body) => {
                out.session.send(CLICK_OPCODE, &body)?;
                None
            }
            Err(error) => Some(error.to_string()),
        };
        out.log.send(ClientEvent::World(DoorAction {
            session_id: world.session_id,
            door_id: *door_id,
            error,
        }))?;
        Ok(true)
    }

    fn observe(
        &mut self,
        event: &WorldEvent,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let WorldEvent::Doors(update) = event {
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
    use super::*;

    #[test]
    fn clicks_need_a_fresh_request_from_this_admission_after_the_doors_last_changed() {
        let doors = Doors::default();
        let world = World {
            session_id: 5,
            player: None,
            motion: None,
            inventory: eq_network_game::inventory::Inventory::default(),
        };
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
        assert_eq!(error(&click(4, Instant::now())), "stale door request");
        assert_eq!(error(&click(5, Instant::now())), "player is unavailable");
    }
}
