//! Admitted Titanium inventory actions; wire submission is separate from server confirmation.
use super::{ClientCommand, ClientEvent, Events, Session};
use anyhow::{Context, Result};
use eq_network_game::{
    inventory::{Inventory, InventoryActor, InventoryUpdate},
    world::WorldEvent,
};
use std::time::{Duration, Instant};

/// How long the server has to refuse a move. `EQEmu` refuses at once, by
/// resending the move's slots, and never acknowledges a success.
const REFUSAL_WINDOW: Duration = Duration::from_secs(2);

/// Settles moves the server has not refused in time.
#[derive(Default)]
pub(super) struct Settlement(Option<Instant>);

impl Settlement {
    /// Notes a move sent at `now`; every move restarts the wait.
    fn sent(&mut self, now: Instant) {
        self.0 = Some(now);
    }

    /// The settling update, once the last move has gone unrefused for the window
    /// and predictions remain.
    pub(super) fn due(&mut self, inventory: &Inventory, now: Instant) -> Option<InventoryUpdate> {
        let sent = self.0?;
        if now.saturating_duration_since(sent) < REFUSAL_WINDOW {
            return None;
        }
        self.0 = None;
        inventory.predicted().then_some(InventoryUpdate::Settled)
    }
}

/// Applies one validated move and reports either prediction or a local rejection.
pub(super) fn handle(
    inventory: &mut Inventory,
    settlement: &mut Settlement,
    actor: Option<InventoryActor>,
    session_id: u64,
    command: &ClientCommand,
    session: &mut Session,
    log: &mut Events<'_>,
) -> Result<bool> {
    let ClientCommand::MoveInventory(request) = command else {
        return Ok(false);
    };
    let mut transport_failed = false;
    let now = Instant::now();
    let result = actor
        .context("Character equipment data is unavailable")
        .and_then(|actor| {
            inventory.submit_move(request, session_id, actor, now, |body| {
                let sent = session.send(0x420f, body);
                transport_failed = sent.is_err();
                sent
            })
        });
    // Transport errors terminate this admission; never retry an uncertain item move.
    if transport_failed {
        result?;
        return Ok(true);
    }
    let error = match result {
        Ok(update) => {
            settlement.sent(now);
            log.send(ClientEvent::World(WorldEvent::Inventory(update)))?;
            None
        }
        Err(error) => Some(error.to_string()),
    };
    log.send(ClientEvent::World(WorldEvent::InventoryAction {
        session_id: request.session_id,
        revision: request.revision,
        error,
    }))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An inventory with one item moved to the cursor by an unanswered prediction.
    fn predicted() -> Inventory {
        let mut item = eq_network_game::inventory::InventoryItem {
            activation: eq_network_game::inventory::ItemActivation::default(),
            scroll_spell: None,
            rules: eq_network_game::inventory::ItemPlacement::default(),
            slot: eq_network_game::inventory::InventorySlot(22),
            icon: 0,
            stack_count: None,
            charges: 0,
            bag_slots: 0,
            details: eq_network_game::items::ItemDetails {
                equipment: None,
                bonuses: None,
                id: 42,
                name: "Synthetic item".into(),
                lore: String::new(),
                weight_tenths: 0,
                slots: 0,
                classes: 0,
                races: 0,
                flags: vec![],
                stats: vec![],
            },
        };
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item.clone()]));
        item.slot = eq_network_game::inventory::InventorySlot(30);
        inventory.apply(InventoryUpdate::Prediction(vec![item]));
        inventory
    }

    #[test]
    fn moves_settle_once_the_last_one_goes_unrefused_for_the_window() {
        let start = Instant::now();
        let mut inventory = predicted();
        let mut settlement = Settlement::default();
        assert_eq!(settlement.due(&inventory, start), None);
        settlement.sent(start);
        settlement.sent(start + Duration::from_secs(1));
        assert_eq!(settlement.due(&inventory, start + REFUSAL_WINDOW), None);
        let due = start + Duration::from_secs(1) + REFUSAL_WINDOW;
        assert_eq!(
            settlement.due(&inventory, due),
            Some(InventoryUpdate::Settled)
        );
        assert_eq!(settlement.due(&inventory, due), None);
        // Nothing left to settle: moves the server already answered stay quiet.
        settlement.sent(start);
        inventory.apply(InventoryUpdate::Settled);
        assert_eq!(settlement.due(&inventory, due), None);
    }
}
