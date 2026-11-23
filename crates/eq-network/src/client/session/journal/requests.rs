//! What the player asks of the journal: to take an offered task, decline
//! an offer, cancel an entry, a finished task's steps, and `/tasktimers`
//! (checklist 6.555). Only a local `EQEmu` session offers them until the
//! live row passes, because a wrong acceptance can bring down a zone that
//! other players share.
//!
//! Each guard answers a fault in stock `EQEmu`'s handlers
//! (`zone/client_packet.cpp`, `zone/task_client_state.cpp`):
//!
//! - `ClientTaskState::CanAcceptNewTask` does
//!   `entity_list.GetID(giver)->CastToNPC()` without checking for null, so
//!   an acceptance after the giver has left the zone dereferences null.
//!   The session sends one only for a task in the current offer, heard in
//!   this admission, while the giver is an NPC spawn it still holds. A giver
//!   that leaves between this check and the server's read can't be guarded
//!   here.
//! - `ClientTaskState::RemoveTask` doesn't check that the slot holds a task:
//!   for a quest it empties `m_active_quests[slot]` and lowers the count of
//!   active quests, so a slot past 18 writes past the array and a second
//!   cancel lowers the count again. The session checks the slot itself,
//!   needs an entry there, and sends one cancel for a slot until the
//!   server's removal or the zone's end.
//!
//! Taking or cancelling a task only ever comes from the player's own
//! choice: a host doesn't send either from a script, social or hotbutton.
//! That is this library's own safeguard, not the program's, and it stays
//! until the question of lifting it is settled.
//! Declining sends nothing, since `EQEmu` ignores an acceptance of task 0;
//! whether the program sends one is unread (R49-38), so this is inferred.
//! A history request and `/tasktimers` are paced by this library's own
//! floor (inferred); `EQEmu` rate-limits `/tasktimers` itself.

use super::super::{
    actions,
    feature::{Out, World},
    ClientCommand,
};
use super::Entry;
use anyhow::Result;
use eq_network_game::{
    request::Request,
    tasks::{offers::TaskOffers, CompletedTask, TaskSlot},
    world::SpawnKind,
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

/// The least time between two history requests, and between two
/// `/tasktimers`: this library's floor (inferred, 6.555).
pub(super) const PACE: Duration = Duration::from_secs(1);

/// The player's requests and what they wait on.
#[derive(Debug, Default)]
pub(super) struct Requests {
    /// The last offer heard in this admission, until it is answered.
    pub(super) offer: Option<TaskOffers>,
    /// The slots a cancel was sent for, until the server's removal.
    pub(super) cancelling: HashSet<TaskSlot>,
    history_sent: Option<Instant>,
    timers_sent: Option<Instant>,
}

/// Whether `sent` was less than [`PACE`] before `now`.
fn too_soon(sent: Option<Instant>, now: Instant) -> bool {
    sent.is_some_and(|sent| now.saturating_duration_since(sent) < PACE)
}

impl Requests {
    pub(super) const fn owns(command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::AcceptTask { .. }
                | ClientCommand::DeclineTasks { .. }
                | ClientCommand::CancelTask { .. }
                | ClientCommand::AskTaskHistory { .. }
                | ClientCommand::ListTaskTimers { .. }
        )
    }

    pub(super) fn handle(
        &mut self,
        command: &ClientCommand,
        entries: &HashMap<TaskSlot, Entry>,
        completed: Option<&[CompletedTask]>,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let now = Instant::now();
        match *command {
            ClientCommand::AcceptTask { task_id, .. } => {
                self.accept(command, task_id, entries, world, out)
            }
            ClientCommand::DeclineTasks { .. } => {
                if self.offer.take().is_none() {
                    return actions::refuse(command, "No task is offered.", out.log);
                }
                Ok(())
            }
            ClientCommand::CancelTask { at, .. } => {
                if at.slot >= at.kind.slots() {
                    return actions::refuse(command, "That journal slot doesn't exist.", out.log);
                }
                if !entries.contains_key(&at) {
                    return actions::refuse(command, "That journal slot is empty.", out.log);
                }
                if self.cancelling.contains(&at) {
                    return actions::refuse(
                        command,
                        "That task is already being cancelled.",
                        out.log,
                    );
                }
                out.request(&Request::CancelTask(at))?;
                self.cancelling.insert(at);
                Ok(())
            }
            ClientCommand::AskTaskHistory { index, .. } => {
                let listed = completed.map_or(0, <[CompletedTask]>::len);
                if usize::try_from(index).map_or(true, |index| index >= listed) {
                    return actions::refuse(command, "No finished task is listed there.", out.log);
                }
                if too_soon(self.history_sent, now) {
                    return actions::refuse(
                        command,
                        "You just asked for a task's history.",
                        out.log,
                    );
                }
                out.request(&Request::TaskHistory(index))?;
                self.history_sent = Some(now);
                Ok(())
            }
            ClientCommand::ListTaskTimers { .. } => {
                if too_soon(self.timers_sent, now) {
                    return actions::refuse(
                        command,
                        "You just asked for your task timers.",
                        out.log,
                    );
                }
                out.request(&Request::TaskTimers)?;
                self.timers_sent = Some(now);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn accept(
        &mut self,
        command: &ClientCommand,
        task_id: u32,
        entries: &HashMap<TaskSlot, Entry>,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let Some(offer) = &self.offer else {
            return actions::refuse(command, "No task is offered.", out.log);
        };
        if task_id == 0 || !offer.tasks.iter().any(|task| task.task_id == task_id) {
            return actions::refuse(command, "That task isn't offered.", out.log);
        }
        if entries
            .values()
            .any(|entry| entry.description.task_id == task_id)
        {
            return actions::refuse(command, "You already have that task.", out.log);
        }
        let giver = u16::try_from(offer.giver)
            .ok()
            .filter(|giver| *giver != 0)
            .filter(|giver| {
                world
                    .spawns
                    .get(*giver)
                    .is_some_and(|spawn| spawn.kind == SpawnKind::Npc)
            });
        let Some(giver) = giver else {
            self.offer = None;
            return actions::refuse(
                command,
                "The one who offered that task is no longer here.",
                out.log,
            );
        };
        let kind = offer.kind;
        // EQEmu clears its offers on any acceptance, taken or not.
        self.offer = None;
        out.request(&Request::AcceptTask {
            kind,
            task_id,
            giver,
        })
    }
}

#[cfg(test)]
mod tests;
