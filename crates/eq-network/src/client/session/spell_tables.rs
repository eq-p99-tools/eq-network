//! Server-selected spell-table facts, outside profile/admission staging.
use super::{servers::ServerType, ClientEvent, Events};
use anyhow::{Context as _, Result};
use eq_network_game::{
    spell_tables::{self, Context, Event, Kind, Stage, Unavailable, Update},
    world::WorldEvent,
};

/// A server policy's interpretation of its spell-table messages.
pub(super) trait Policy: Sync {
    /// A complete update or visible failure; `None` for another opcode.
    fn read(&self, stage: Stage, opcode: u16, body: &[u8]) -> Option<Update>;
}

/// P99's source registration and cached-definition replay policy.
pub(super) struct P99;

impl Policy for P99 {
    fn read(&self, stage: Stage, opcode: u16, body: &[u8]) -> Option<Update> {
        let kind = Kind::from_opcode(opcode)?;
        if !matches!(
            (kind, stage),
            (Kind::Registration, Stage::World) | (Kind::Replay, Stage::Zone { .. })
        ) {
            return Some(Update::Unavailable {
                kind,
                reason: Unavailable::UnexpectedStage,
            });
        }
        Some(match spell_tables::decode(opcode, body) {
            Ok(Some(table)) => Update::Table(table),
            Ok(None) => return None,
            Err(error) => Update::Unavailable {
                kind,
                reason: Unavailable::Malformed(error),
            },
        })
    }
}

/// One network attempt's ordering, shared across its world/zone transports.
/// This carries no installed data and is not the host's provider epoch.
pub(in crate::client) struct Sequence {
    run_id: u64,
    revision: u64,
}

impl Sequence {
    /// Start an attempt's table ordering.
    pub(in crate::client) const fn new(run_id: u64) -> Self {
        Self {
            run_id,
            revision: 0,
        }
    }

    /// Number every table or failure without wrapping into an older event.
    fn next(&mut self, stage: Stage, update: Update) -> Result<Event> {
        let revision = self
            .revision
            .checked_add(1)
            .context("spell table event revision exhausted")?;
        self.revision = revision;
        Ok(Event::new(
            Context::new(self.run_id, revision, stage),
            update,
        ))
    }
}

/// Deliver a server-policy table before ordinary world/zone admission gates.
/// Returns true when the policy consumed this packet. Malformed recognized
/// packets emit unavailability without delivering a valid prefix or raw bytes.
///
/// # Errors
/// Returns an error when ordering exhausts or the host cannot accept the event.
pub(super) fn receive(
    server: &dyn ServerType,
    stage: Stage,
    opcode: u16,
    body: &[u8],
    log: &mut Events<'_>,
) -> Result<bool> {
    let Some(update) = server
        .spell_tables()
        .and_then(|policy| policy.read(stage, opcode, body))
    else {
        return Ok(false);
    };
    let event = log.spell_tables.next(stage, update)?;
    log.send(ClientEvent::World(WorldEvent::SpellTable(event)))?;
    Ok(true)
}

#[cfg(test)]
mod tests;
