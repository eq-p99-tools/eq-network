//! The task journal stock `EQEmu` keeps for a Titanium player, as the zone
//! sends it (gap list section 5, recording checklist 6.546).
//!
//! `EQEmu` sends an entry's description before its steps every time it
//! sends the entry: `TaskManager::SendActiveTaskToClient` and
//! `SendSingleActiveTaskToClient` (`zone/task_manager.cpp`) send the
//! description, then each step, long or short, in order; a count that
//! moves sends that one step alone (`SendTaskActivityLong`), to an entry
//! already described. A step for a slot that holds no entry of its task
//! is therefore stale, and is told as a gap by its opcode and length
//! alone. After a step ends `EQEmu` sends `OP_TaskActivityComplete`, then
//! the whole entry again; after the whole task ends, or fails, it sends
//! `OP_TaskActivityComplete`, the finished list (on a completion), and then
//! takes the entry out with `OP_CancelTask` (`ClientTaskState::IncrementDoneCount`,
//! `FailTask` and `TaskPeriodicChecks`, `zone/task_client_state.cpp`). So
//! the session ends an entry only on that removal. Whether the program
//! ends it earlier, on the completion, is unread (read R49-38).
//!
//! A zone sends the whole journal as it admits the player
//! (`Client::CompleteConnect` → `LoadClientTaskState`), so the journal
//! starts empty in each zone. Durations and start times are kept as sent:
//! the session does no clock arithmetic. The task's words reach the host
//! in the events only; no gap or diagnostic carries them.
//!
//! An NPC's offer and a finished task's steps come only as answers to
//! the admitted player, so one heard before the admission is refused. The
//! player's requests are in [`requests`].

use super::feature::{Feature, Out, World};
use super::ClientCommand;
use crate::client::{ClientEvent, GapKind};
use anyhow::Result;
use eq_network_game::{
    message::Message,
    tasks::{self, News, TaskActivity, TaskDescription, TaskSlot},
    world::WorldEvent,
};
use std::collections::{BTreeMap, HashMap};

mod requests;

/// The most refusals the journal keeps before the admission, so that a
/// zone that sends stale steps without end holds no growing list. A whole
/// journal as `EQEmu` sends it refuses none; past this bound the journal
/// keeps only how many more came, and tells that count at the admission.
const MAX_REFUSED: usize = 64;

/// An entry the journal holds: its description and the steps heard since.
#[derive(Clone, Debug)]
struct Entry {
    description: TaskDescription,
    steps: BTreeMap<u32, TaskActivity>,
}

/// A message the journal refused, told by its opcode and length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Refused {
    opcode: (u16, &'static str),
    length: usize,
    why: &'static str,
}

/// The task journal of the zone's player.
#[derive(Debug, Default)]
pub(super) struct Journal {
    entries: HashMap<TaskSlot, Entry>,
    completed: Option<Vec<tasks::CompletedTask>>,
    /// Refusals heard before the admission, told once it comes: at most
    /// [`MAX_REFUSED`].
    refused: Vec<Refused>,
    /// How many refusals before the admission came past the bound.
    dropped: usize,
    /// The player's requests, where the session offers them.
    requests: Option<requests::Requests>,
}

impl Journal {
    /// A journal that also takes the player's requests (6.555).
    pub(super) fn with_requests() -> Self {
        Self {
            requests: Some(requests::Requests::default()),
            ..Self::default()
        }
    }

    /// Applies one message, returning the event the host hears, or the
    /// refusal to tell.
    fn apply(
        &mut self,
        news: &News,
        session_id: u64,
        admitted: bool,
    ) -> std::result::Result<WorldEvent, &'static str> {
        match news {
            News::Offered(_) if !admitted => Err("a task offer before the admission"),
            News::History(_) if !admitted => Err("a task history before the admission"),
            News::Offered(offers) => {
                if let Some(requests) = &mut self.requests {
                    requests.offer = Some(offers.clone());
                }
                Ok(WorldEvent::TasksOffered {
                    session_id,
                    offers: offers.clone(),
                })
            }
            News::History(history) => Ok(WorldEvent::TaskHistory {
                session_id,
                history: history.clone(),
            }),
            News::Described(description) => {
                let entry = self.entries.entry(description.at).or_insert_with(|| Entry {
                    description: description.clone(),
                    steps: BTreeMap::new(),
                });
                if entry.description.task_id != description.task_id {
                    entry.steps.clear();
                }
                entry.description = description.clone();
                Ok(WorldEvent::TaskDescribed {
                    session_id,
                    description: description.clone(),
                })
            }
            News::Step(activity) => {
                let entry = self
                    .entries
                    .get_mut(&activity.at)
                    .filter(|entry| entry.description.task_id == activity.task_id)
                    .ok_or("a task step for no journal entry of its task")?;
                entry.steps.insert(activity.activity_id, activity.clone());
                Ok(WorldEvent::TaskStepUpdated {
                    session_id,
                    activity: activity.clone(),
                })
            }
            News::Concluded(completion) => {
                self.entries
                    .get(&completion.at)
                    .filter(|entry| entry.description.task_id == completion.task_id)
                    .ok_or("a task's end for no journal entry of its task")?;
                Ok(WorldEvent::TaskConcluded {
                    session_id,
                    completion: *completion,
                })
            }
            News::Completed(list) => {
                self.completed = Some(list.clone());
                Ok(WorldEvent::CompletedTasks {
                    session_id,
                    list: list.clone(),
                })
            }
            News::Removed(at) => {
                self.entries.remove(at);
                if let Some(requests) = &mut self.requests {
                    requests.cancelling.remove(at);
                }
                Ok(WorldEvent::TaskRemoved {
                    session_id,
                    at: *at,
                })
            }
        }
    }

    /// The message's opcode and name, for a refusal's gap.
    const fn opcode(news: &News) -> (u16, &'static str) {
        match news {
            News::Described(_) => (tasks::DESCRIPTION_OPCODE, "OP_TaskDescription"),
            News::Step(_) => (tasks::ACTIVITY_OPCODE, "OP_TaskActivity"),
            News::Concluded(_) => (tasks::COMPLETION_OPCODE, "OP_TaskActivityComplete"),
            News::Completed(_) => (tasks::COMPLETED_OPCODE, "OP_CompletedTasks"),
            News::Removed(_) => (tasks::REMOVAL_OPCODE, "OP_CancelTask"),
            News::Offered(_) => (tasks::offers::OFFERS_OPCODE, "OP_TaskSelectWindow"),
            News::History(_) => (tasks::offers::HISTORY_OPCODE, "OP_TaskHistoryReply"),
        }
    }

    fn tell(refused: Refused, out: &mut Out<'_, '_>) -> Result<()> {
        out.log.gap_unsampled(
            refused.opcode.0,
            Some(refused.opcode.1),
            refused.length,
            GapKind::Unread {
                reason: refused.why.to_owned(),
            },
        )
    }
}

impl Feature for Journal {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        if self.requests.is_some() {
            vec![crate::world::Capability::Tasks]
        } else {
            Vec::new()
        }
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        self.requests.is_some() && requests::Requests::owns(command)
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let Some(requests) = &mut self.requests else {
            return Ok(());
        };
        requests.handle(
            command,
            &self.entries,
            self.completed.as_deref(),
            world,
            out,
        )
    }

    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        if let Message::Journal { news, length } = message {
            if let Err(why) = self.apply(news, world.session_id, false) {
                if self.refused.len() == MAX_REFUSED {
                    self.dropped += 1;
                    return Ok(());
                }
                self.refused.push(Refused {
                    opcode: Self::opcode(news),
                    length: *length,
                    why,
                });
            }
        }
        Ok(())
    }

    /// Tells the host the journal the zone sent before admitting the
    /// player: each entry's description, then its steps, by slot.
    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        for refused in std::mem::take(&mut self.refused) {
            Self::tell(refused, out)?;
        }
        let dropped = std::mem::take(&mut self.dropped);
        if dropped > 0 {
            out.log.diagnostic(format!(
                "the task journal dropped {dropped} refused messages heard before \
                 the admission, past the {MAX_REFUSED} it keeps"
            ))?;
        }
        let mut slots: Vec<_> = self.entries.keys().copied().collect();
        slots.sort_by_key(|at| (at.kind == tasks::TaskKind::Quest, at.slot));
        for at in slots {
            let entry = &self.entries[&at];
            out.log.send(ClientEvent::World(WorldEvent::TaskDescribed {
                session_id: world.session_id,
                description: entry.description.clone(),
            }))?;
            for activity in entry.steps.values() {
                out.log
                    .send(ClientEvent::World(WorldEvent::TaskStepUpdated {
                        session_id: world.session_id,
                        activity: activity.clone(),
                    }))?;
            }
        }
        if let Some(list) = &self.completed {
            out.log
                .send(ClientEvent::World(WorldEvent::CompletedTasks {
                    session_id: world.session_id,
                    list: list.clone(),
                }))?;
        }
        Ok(())
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let Message::Journal { news, length } = message else {
            return Ok(());
        };
        match self.apply(news, world.session_id, true) {
            Ok(event) => out.log.send(ClientEvent::World(event)),
            Err(why) => Self::tell(
                Refused {
                    opcode: Self::opcode(news),
                    length: *length,
                    why,
                },
                out,
            ),
        }
    }

    /// Each zone sends the whole journal again as it admits the player,
    /// and a new zone holds no offer and no cancel.
    fn connection_ended(&mut self, _world: &mut World) {
        let requests = self.requests.is_some();
        *self = Self::default();
        if requests {
            self.requests = Some(requests::Requests::default());
        }
    }
}

#[cfg(test)]
mod tests;
