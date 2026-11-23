//! The tasks an NPC offers a Titanium player, the steps of a finished task,
//! and what the player asks of the journal: to take an offered task, to
//! cancel an entry, a finished task's steps, and `/tasktimers`.
//!
//! `TaskManager::SendTaskSelector` (`zone/task_manager.cpp`) sends
//! `OP_TaskSelectWindow` (0x5e7c): it clears the player's last offers,
//! leaves out what the player can't take, sends nothing when none remain,
//! and records each task it sends as offered by that NPC. Its kind word is
//! always 2, even for a task of kind 0, and it sends at most 40
//! (`MAXCHOOSERENTRIES`). Each task is written by `TaskInformation` and
//! `ActivityInformation::SerializeSelector` (`common/tasks.h`), Titanium's
//! branch: no reward multiplier and no preview byte.
//! `ClientTaskState::SendTaskHistory` answers a finished task's steps with
//! `OP_TaskHistoryReply` (0x397d, `TaskHistoryReplyHeader_Struct`,
//! `TaskHistoryReplyData1_Struct`, `TaskHistoryReplyData2_Struct`).
//!
//! The requests are those `zone/client_packet.cpp` reads. No Titanium
//! decoder rewrites any of them (`common/patches/titanium_ops.h`), so each
//! must be the size of `EQEmu`'s own struct:
//! `Handle_OP_AcceptNewTask` takes 12 bytes (`AcceptNewTask_Struct`: kind
//! as offered, task, giver), `Handle_OP_CancelTask` 8 (slot, kind),
//! `Handle_OP_TaskHistoryRequest` 4 (the index in the last finished list),
//! and `Handle_OP_TaskTimers` reads no body. `EQEmu`'s Titanium header
//! still carries an older 8-byte accept, which its handler would refuse.
//! What the official program sends for each is unread (read R49-38): the
//! empty `/tasktimers` body and sending nothing to decline are inferred.

use super::{
    Cursor, TaskKind, TaskSlot, DESCRIPTION_LONGEST, OPEN_LONGEST, STEPS, TARGET_LONGEST,
    TITLE_LONGEST,
};
use crate::command::EncodedCommand;
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// Titanium's `OP_TaskSelectWindow`: the tasks an NPC offers.
pub const OFFERS_OPCODE: u16 = 0x5e7c;
/// Titanium's `OP_TaskHistoryReply`: the steps of a finished task.
pub const HISTORY_OPCODE: u16 = 0x397d;
/// Titanium's `OP_AcceptNewTask`.
pub const ACCEPT_OPCODE: u16 = 0x207f;
/// Titanium's `OP_CancelTask`, the player's request.
pub const CANCEL_OPCODE: u16 = super::REMOVAL_OPCODE;
/// Titanium's `OP_TaskHistoryRequest`.
pub const HISTORY_REQUEST_OPCODE: u16 = 0x5df4;
/// Titanium's `OP_TaskTimers` (`/tasktimers`).
pub const TIMERS_OPCODE: u16 = 0x6a1d;

/// The most tasks one offer holds (`MAXCHOOSERENTRIES`).
pub const OFFERED_MOST: u32 = 40;

/// A task an NPC offers: what the select window shows.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OfferedTask {
    /// The task's number.
    pub task_id: u32,
    /// How long it may take, in seconds; 0 when `duration_code` says.
    pub duration_seconds: u32,
    /// 1 short, 2 medium, 3 long, else unlimited, used when the duration
    /// is 0.
    pub duration_code: u32,
    /// Its title.
    pub title: String,
    /// Its description.
    pub description: String,
    /// The steps the window shows: the first the task opens with.
    pub steps: Vec<OfferedStep>,
}

/// One step an offered task shows.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OfferedStep {
    /// The step's number in the task, from 0.
    pub index: u32,
    /// `EQEmu`'s activity type, as [`super::Objective::activity_type`].
    pub activity_type: i32,
    /// Whom or what it names.
    pub target_name: String,
    /// The items it names.
    pub item_list: String,
    /// How many it asks for (1 for coin).
    pub goal: i32,
    /// The skill it names, or 0.
    pub skill_id: i32,
    /// The spell it names, or 0.
    pub spell_id: i32,
    /// The zone it is in, or 0.
    pub zone_id: i32,
    /// Its own words, when the task gives them.
    pub description: String,
}

/// The tasks one NPC offers (`OP_TaskSelectWindow`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskOffers {
    /// The kind word as sent, which an accept carries back.
    pub kind: TaskKind,
    /// The offering NPC's spawn.
    pub giver: u32,
    /// The tasks, in the server's order.
    pub tasks: Vec<OfferedTask>,
}

/// One done step of a finished task.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DoneStep {
    /// `EQEmu`'s activity type.
    pub activity_type: u32,
    /// Whom or what it named.
    pub target_name: String,
    /// The items it named.
    pub item_list: String,
    /// How many it asked for.
    pub goal: u32,
    /// The zone it was in, or 0.
    pub zone_id: u32,
    /// Its own words, when the task gives them.
    pub description: String,
}

/// A finished task's done steps (`OP_TaskHistoryReply`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskHistory {
    /// The index in the finished list that was asked for.
    pub index: u32,
    /// The steps done, in the task's order.
    pub steps: Vec<DoneStep>,
}

/// Reads `OP_TaskSelectWindow`.
///
/// # Errors
///
/// An offer that is short or long, holds no task or more than 40, names a
/// shared or unknown kind, shows more than 20 steps or a step past 20, or
/// has a string without its end or past its length.
pub fn offers(body: &[u8]) -> Result<TaskOffers> {
    let mut record = Cursor::new(body);
    let count = record.word("offered task count")?;
    ensure!(
        (1..=OFFERED_MOST).contains(&count),
        "{count} offered tasks, not 1 to {OFFERED_MOST}"
    );
    let kind = TaskKind::of(record.word("offered kind")?)?;
    let giver = record.word("offering spawn")?;
    let mut tasks = Vec::new();
    for _ in 0..count {
        let task_id = record.word("offered task's number")?;
        let duration_seconds = record.word("offered task's duration")?;
        let duration_code = record.word("offered task's duration code")?;
        let title = record.string(TITLE_LONGEST, "offered task's title")?;
        let description = record.string(DESCRIPTION_LONGEST, "offered task's description")?;
        let shown = record.word("offered task's step count")?;
        ensure!(
            shown <= STEPS,
            "an offered task showing {shown} steps, past {STEPS}"
        );
        let mut steps = Vec::new();
        for _ in 0..shown {
            let index = record.word("offered step's number")?;
            ensure!(index < STEPS, "offered step {index} past a task's {STEPS}");
            let activity_type = record.signed("offered step's type")?;
            // The request type, "no longer in live": EQEmu writes 0.
            record.word("offered step's request type")?;
            steps.push(OfferedStep {
                index,
                activity_type,
                target_name: record.string(TARGET_LONGEST, "offered step's target")?,
                item_list: record.string(OPEN_LONGEST, "offered step's items")?,
                goal: record.signed("offered step's goal")?,
                skill_id: record.signed("offered step's skill")?,
                spell_id: record.signed("offered step's spell")?,
                zone_id: record.signed("offered step's zone")?,
                description: record.string(OPEN_LONGEST, "offered step's description")?,
            });
        }
        tasks.push(OfferedTask {
            task_id,
            duration_seconds,
            duration_code,
            title,
            description,
            steps,
        });
    }
    record.end("task offer")?;
    Ok(TaskOffers { kind, giver, tasks })
}

/// Reads `OP_TaskHistoryReply`.
///
/// # Errors
///
/// A reply that is short or long, counts more than a task's 20 steps, or
/// has a string without its end or past its length.
pub fn history(body: &[u8]) -> Result<TaskHistory> {
    let mut record = Cursor::new(body);
    let index = record.word("history's index")?;
    let count = record.word("history's step count")?;
    ensure!(count <= STEPS, "a history of {count} steps, past {STEPS}");
    let mut steps = Vec::new();
    for _ in 0..count {
        let activity_type = record.word("done step's type")?;
        let target_name = record.string(TARGET_LONGEST, "done step's target")?;
        let item_list = record.string(OPEN_LONGEST, "done step's items")?;
        let goal = record.word("done step's goal")?;
        // Two words EQEmu writes as 0xffffffff.
        record.take(8, "done step's marks")?;
        let zone_id = record.word("done step's zone")?;
        // A word EQEmu writes as 0.
        record.word("done step's last word")?;
        let description = record.string(OPEN_LONGEST, "done step's description")?;
        steps.push(DoneStep {
            activity_type,
            target_name,
            item_list,
            goal,
            zone_id,
            description,
        });
    }
    record.end("task history")?;
    Ok(TaskHistory { index, steps })
}

const fn kind_word(kind: TaskKind) -> u32 {
    match kind {
        TaskKind::Task => 0,
        TaskKind::Quest => 2,
    }
}

fn words(opcode: u16, words: &[u32]) -> EncodedCommand {
    EncodedCommand {
        opcode,
        body: words.iter().flat_map(|word| word.to_le_bytes()).collect(),
    }
}

/// Titanium's acceptance of an offered task: `OP_AcceptNewTask`, 12 bytes,
/// the kind as offered, the task and the giver's spawn.
///
/// # Errors
///
/// Task 0, which `EQEmu` reads as no answer, and spawn 0.
pub fn titanium_accept(kind: TaskKind, task_id: u32, giver: u16) -> Result<EncodedCommand> {
    ensure!(task_id != 0, "an acceptance names no task");
    ensure!(giver != 0, "an acceptance names no giver");
    Ok(words(
        ACCEPT_OPCODE,
        &[kind_word(kind), task_id, u32::from(giver)],
    ))
}

/// Titanium's request to cancel an entry: `OP_CancelTask`, 8 bytes, its
/// slot and kind.
///
/// # Errors
///
/// A slot past its kind's part of the journal, which `EQEmu`'s
/// `RemoveTask` would write past its quests.
pub fn titanium_cancel(at: TaskSlot) -> Result<EncodedCommand> {
    ensure!(
        at.slot < at.kind.slots(),
        "a cancel past the journal's {:?} slots",
        at.kind
    );
    Ok(words(CANCEL_OPCODE, &[at.slot, kind_word(at.kind)]))
}

/// Titanium's request for a finished task's steps:
/// `OP_TaskHistoryRequest`, 4 bytes, its index in the last finished list.
///
/// # Errors
///
/// An index past the 50 `EQEmu` lists.
pub fn titanium_history(index: u32) -> Result<EncodedCommand> {
    let listed = u32::try_from(super::COMPLETED_LISTED).context("finished list bound")?;
    ensure!(index < listed, "a history index past the {listed} listed");
    Ok(words(HISTORY_REQUEST_OPCODE, &[index]))
}

/// Titanium's `/tasktimers`: `OP_TaskTimers` with an empty body, which
/// `EQEmu` doesn't read (the program's body is unread, R49-38).
#[must_use]
pub const fn titanium_timers() -> EncodedCommand {
    EncodedCommand {
        opcode: TIMERS_OPCODE,
        body: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
