//! Admitted Titanium inventory actions; wire submission is separate from server confirmation.
use super::{ClientCommand, ClientEvent, Events, Session};
use anyhow::{Context, Result};
use eq_network_game::{
    inventory::{Inventory, InventoryActor},
    world::WorldEvent,
};
use std::time::Instant;

/// Applies one validated move and reports either prediction or a local rejection.
pub(super) fn handle(
    inventory: &mut Inventory,
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
    let result = actor
        .context("Character equipment data is unavailable")
        .and_then(|actor| {
            inventory.submit_move(request, session_id, actor, Instant::now(), |body| {
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
