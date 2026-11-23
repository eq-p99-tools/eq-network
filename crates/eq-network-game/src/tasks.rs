//! The task journal stock `EQEmu` keeps for a Titanium player: the task and
//! the quests they have taken, each with its steps, and the tasks they have
//! finished.
//!
//! `EQEmu` sends the whole journal as a zone admits the player
//! (`Client::LoadClientTaskState`: `TaskManager::SendActiveTasksToClient`,
//! then `SendCompletedTasksToClient`), each entry again as it is taken or
//! changes, a step's count as it moves, and a step's or a task's end. A
//! task (kind 0) and up to 19 quests (kind 2) are held at once
//! (`MAXACTIVEQUESTS`), each with up to 20 steps (`MAXACTIVITIESPERTASK`).
//! Shared tasks (kind 1) come after Titanium and are refused here.
//!
//! Layout reference: `EQEmu`'s `TaskDescriptionHeader_Struct`,
//! `TaskDescriptionData1_Struct`, `TaskDescriptionData2_Struct`,
//! `TaskDescriptionTrailer_Struct`, `TaskActivityComplete_Struct` and
//! `CancelTask_Struct` (`common/eq_packet_structs.h`, packed), built by
//! `TaskManager::SendActiveTaskDescription`, `SendTaskActivityShort`,
//! `SendTaskActivityLong` (with `ActivityInformation::SerializeObjective`,
//! `common/tasks.h`, Titanium's branch) and `SendCompletedTasksToClient`
//! (`zone/task_manager.cpp`), `Client::SendTaskActivityComplete` and
//! `SendTaskFailed` (`zone/tasks.cpp`) and `ClientTaskState::CancelTask`
//! (`zone/task_client_state.cpp`). Titanium's encoder rewrites only the
//! description's item link. What the official program does with each field
//! is unread (read R49-38): the names here are `EQEmu`'s.

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;

/// Titanium's `OP_TaskDescription`: one journal entry's description.
pub const DESCRIPTION_OPCODE: u16 = 0x5ef7;
/// Titanium's `OP_TaskActivity`: one step of a journal entry.
pub const ACTIVITY_OPCODE: u16 = 0x682d;
/// Titanium's `OP_TaskActivityComplete`: a step or a whole entry ended.
pub const COMPLETION_OPCODE: u16 = 0x54eb;
/// Titanium's `OP_CompletedTasks`: the tasks the player has finished.
pub const COMPLETED_OPCODE: u16 = 0x76a2;
/// Titanium's `OP_CancelTask`: the server took an entry out of the journal.
pub const REMOVAL_OPCODE: u16 = 0x3ba8;

/// The most quests held at once (`MAXACTIVEQUESTS`, Titanium's 19).
pub const QUEST_SLOTS: u32 = 19;
/// The most steps an entry has (`MAXACTIVITIESPERTASK`).
pub const STEPS: u32 = 20;
/// The most finished tasks `EQEmu` lists (the 50 most recent).
pub const COMPLETED_LISTED: usize = 50;

/// The longest title with its end (`SerializeSelector`: max 64 with null).
const TITLE_LONGEST: usize = 63;
/// The longest description (max 4000 with null).
const DESCRIPTION_LONGEST: usize = 3999;
/// The longest step target name (max 64).
const TARGET_LONGEST: usize = 63;
/// The longest of the other strings, the reward text, the item link and a
/// step's item list and description: bounded only by the record.
const OPEN_LONGEST: usize = usize::MAX;

/// The description's header: slot, task, the open-journal byte, kind and
/// reward type.
const DESCRIPTION_HEADER: usize = 17;
/// Duration, duration code and start time.
const DESCRIPTION_TIMES: usize = 12;
/// The has-rewards byte, coin, experience and faction.
const DESCRIPTION_REWARDS: usize = 13;
/// The trailer: points and the reward-selection byte.
const DESCRIPTION_TRAILER: usize = 5;
/// A step's common start: slot, kind, task, step and group.
const ACTIVITY_HEADER: usize = 20;
/// A hidden step's record (`SendTaskActivityShort`).
const HIDDEN_ACTIVITY: usize = 25;
/// The marker a hidden step carries after its header.
const HIDDEN_MARK: u32 = 0xffff_ffff;
/// `TaskActivityComplete_Struct`.
const COMPLETION: usize = 24;
/// `CancelTask_Struct`.
const REMOVAL: usize = 8;

/// Which part of the journal an entry fills.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
pub enum TaskKind {
    /// The one task (`TaskType::Task`, 0).
    Task,
    /// One of the quests (`TaskType::Quest`, 2).
    Quest,
}

impl TaskKind {
    fn of(value: u32) -> Result<Self> {
        match value {
            0 => Ok(Self::Task),
            2 => Ok(Self::Quest),
            1 => bail!("a shared task, which Titanium has no part for"),
            other => bail!("task kind {other}"),
        }
    }

    /// The slots this kind has: one task, or the quests.
    #[must_use]
    pub const fn slots(self) -> u32 {
        match self {
            Self::Task => 1,
            Self::Quest => QUEST_SLOTS,
        }
    }
}

/// Where an entry sits: its kind and its slot (`SequenceNumber`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct TaskSlot {
    /// The part of the journal.
    pub kind: TaskKind,
    /// The slot in that part, from 0.
    pub slot: u32,
}

impl TaskSlot {
    fn new(kind: u32, slot: u32) -> Result<Self> {
        let kind = TaskKind::of(kind)?;
        ensure!(slot < kind.slots(), "{kind:?} slot {slot} past the journal");
        Ok(Self { kind, slot })
    }
}

/// What an entry's reward is, as `EQEmu` describes it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Rewards {
    /// Whether the entry has rewards at all (`EQEmu` always says so).
    pub listed: bool,
    /// Coin, in copper.
    pub coin_copper: u32,
    /// Whether it gives experience.
    pub experience: bool,
    /// Whether it changes faction.
    pub faction: bool,
    /// The reward's words.
    pub text: String,
    /// The reward item's link, in Titanium's form, or empty.
    pub item_link: String,
}

/// An entry's description (`OP_TaskDescription`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskDescription {
    /// Where it sits.
    pub at: TaskSlot,
    /// The task's number.
    pub task_id: u32,
    /// Whether the server asks for the journal to open (on taking it).
    pub open_journal: bool,
    /// Its title.
    pub title: String,
    /// How long it may take, in seconds; 0 when `duration_code` says.
    pub duration_seconds: u32,
    /// 1 short, 2 medium, 3 long, else unlimited, used when the duration
    /// is 0.
    pub duration_code: u32,
    /// When it was taken (seconds since 1970).
    pub started_at: u32,
    /// Its description.
    pub description: String,
    /// Its reward.
    pub rewards: Rewards,
}

/// One step's objective, as the journal shows it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Objective {
    /// `EQEmu`'s activity type: 1 deliver, 2 kill, 3 loot, 4 speak with,
    /// 5 explore, 6 tradeskill, 7 fish, 8 forage, 9 cast on, 10 skill on,
    /// 11 touch, 13 collect, 100 give coin; others as sent.
    pub activity_type: i32,
    /// Whether it may be left undone.
    pub optional: bool,
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
    /// How many are done.
    pub done: i32,
}

/// A step: hidden until earlier steps are done, or shown.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum Step {
    /// Not yet open ("???").
    Hidden {
        /// Whether it may be left undone.
        optional: bool,
    },
    /// Open, with its objective.
    Shown(Objective),
}

/// One step of an entry (`OP_TaskActivity`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskActivity {
    /// The entry it belongs to.
    pub at: TaskSlot,
    /// The entry's task.
    pub task_id: u32,
    /// The step's number in the task, from 0.
    pub activity_id: u32,
    /// `EQEmu`'s `list_group`, as sent.
    pub group: u32,
    /// The step.
    pub step: Step,
}

/// How a step or an entry ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Outcome {
    /// Done.
    Completed,
    /// Failed (the whole entry).
    Failed,
}

/// A step's or an entry's end (`OP_TaskActivityComplete`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct TaskCompletion {
    /// The entry.
    pub at: TaskSlot,
    /// Its task.
    pub task_id: u32,
    /// The step that ended (0 for a failure).
    pub activity_id: u32,
    /// How.
    pub outcome: Outcome,
    /// `EQEmu`'s `stage_complete`: set when a step ended and the task goes
    /// on, clear when the whole task completed or failed.
    pub stage_completed: bool,
}

/// A task the player has finished (`OP_CompletedTasks`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompletedTask {
    /// Its number.
    pub task_id: u32,
    /// Its title.
    pub title: String,
    /// When it was finished (seconds since 1970).
    pub completed_at: u32,
}

/// Reads one record from its start, refusing anything past its end.
struct Cursor<'a> {
    body: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    const fn new(body: &'a [u8]) -> Self {
        Self { body, at: 0 }
    }

    fn take(&mut self, length: usize, what: &str) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(length)
            .filter(|end| *end <= self.body.len())
            .with_context(|| format!("{what} past the record's end"))?;
        let bytes = &self.body[self.at..end];
        self.at = end;
        Ok(bytes)
    }

    fn byte(&mut self, what: &str) -> Result<u8> {
        Ok(self.take(1, what)?[0])
    }

    fn word(&mut self, what: &str) -> Result<u32> {
        let bytes = self.take(4, what)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn signed(&mut self, what: &str) -> Result<i32> {
        Ok(i32::from_le_bytes(self.word(what)?.to_le_bytes()))
    }

    fn flag(&mut self, what: &str) -> Result<bool> {
        match self.word(what)? {
            0 => Ok(false),
            1 => Ok(true),
            other => bail!("{what} {other}, neither 0 nor 1"),
        }
    }

    fn string(&mut self, longest: usize, what: &str) -> Result<String> {
        let rest = &self.body[self.at..];
        let end = rest
            .iter()
            .take(longest.saturating_add(1))
            .position(|byte| *byte == 0)
            .with_context(|| format!("{what} without its end within its length"))?;
        self.at += end + 1;
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    }

    fn end(&self, what: &str) -> Result<()> {
        ensure!(self.at == self.body.len(), "bytes after the {what}");
        Ok(())
    }
}

/// Reads `OP_TaskDescription`.
///
/// # Errors
///
/// A record that is short, carries bytes past its trailer, names a shared
/// task or a slot past the journal, or has a title, description or flag
/// out of range.
pub fn description(body: &[u8]) -> Result<TaskDescription> {
    ensure!(
        body.len()
            >= DESCRIPTION_HEADER + DESCRIPTION_TIMES + DESCRIPTION_REWARDS + DESCRIPTION_TRAILER,
        "task description shorter than its fixed parts"
    );
    let mut record = Cursor::new(body);
    let slot = record.word("task slot")?;
    let task_id = record.word("task number")?;
    let open_journal = match record.byte("open-journal byte")? {
        0 => false,
        1 => true,
        other => bail!("open-journal byte {other}"),
    };
    let at = TaskSlot::new(record.word("task kind")?, slot)?;
    // `reward_type`: DoN crystals, after Titanium; read past.
    record.word("reward type")?;
    let title = record.string(TITLE_LONGEST, "task title")?;
    let duration_seconds = record.word("task duration")?;
    let duration_code = record.word("task duration code")?;
    let started_at = record.word("task start time")?;
    let description = record.string(DESCRIPTION_LONGEST, "task description")?;
    let listed = match record.byte("has-rewards byte")? {
        0 => false,
        1 => true,
        other => bail!("has-rewards byte {other}"),
    };
    let coin_copper = record.word("coin reward")?;
    let experience = record.flag("experience reward")?;
    let faction = record.flag("faction reward")?;
    let text = record.string(OPEN_LONGEST, "reward text")?;
    let item_link = record.string(OPEN_LONGEST, "reward item link")?;
    // The trailer's points and reward-selection byte: after Titanium.
    record.take(DESCRIPTION_TRAILER, "description trailer")?;
    record.end("task description")?;
    Ok(TaskDescription {
        at,
        task_id,
        open_journal,
        title,
        duration_seconds,
        duration_code,
        started_at,
        description,
        rewards: Rewards {
            listed,
            coin_copper,
            experience,
            faction,
            text,
            item_link,
        },
    })
}

/// Reads `OP_TaskActivity`, hidden or shown.
///
/// # Errors
///
/// A record that is short or long, names a shared task, a slot past the
/// journal or a step past the task's 20, or has a flag or string out of
/// range.
pub fn activity(body: &[u8]) -> Result<TaskActivity> {
    let mut record = Cursor::new(body);
    let slot = record.word("step's task slot")?;
    let at = TaskSlot::new(record.word("step's task kind")?, slot)?;
    let task_id = record.word("step's task number")?;
    let activity_id = record.word("step number")?;
    ensure!(
        activity_id < STEPS,
        "step {activity_id} past a task's {STEPS}"
    );
    let group = record.word("step group")?;
    debug_assert_eq!(record.at, ACTIVITY_HEADER);
    if body.len() == HIDDEN_ACTIVITY
        && body[ACTIVITY_HEADER..ACTIVITY_HEADER + 4] == HIDDEN_MARK.to_le_bytes()
    {
        record.word("hidden step's mark")?;
        let optional = match record.byte("hidden step's optional byte")? {
            0 => false,
            1 => true,
            other => bail!("hidden step's optional byte {other}"),
        };
        record.end("hidden step")?;
        return Ok(TaskActivity {
            at,
            task_id,
            activity_id,
            group,
            step: Step::Hidden { optional },
        });
    }
    let activity_type = record.signed("activity type")?;
    let optional = record.flag("step's optional word")?;
    // The request type, "no longer in live": EQEmu writes 0.
    record.word("step's request type")?;
    let target_name = record.string(TARGET_LONGEST, "step target")?;
    let item_list = record.string(OPEN_LONGEST, "step items")?;
    let goal = record.signed("step goal")?;
    let skill_id = record.signed("step skill")?;
    let spell_id = record.signed("step spell")?;
    let zone_id = record.signed("step zone")?;
    // The dynamic zone switch: after Titanium.
    record.word("step's zone switch")?;
    let description = record.string(OPEN_LONGEST, "step description")?;
    let done = record.signed("step count done")?;
    // A byte `EQEmu` always writes as 1.
    record.byte("step's last byte")?;
    record.end("step")?;
    Ok(TaskActivity {
        at,
        task_id,
        activity_id,
        group,
        step: Step::Shown(Objective {
            activity_type,
            optional,
            target_name,
            item_list,
            goal,
            skill_id,
            spell_id,
            zone_id,
            description,
            done,
        }),
    })
}

/// Reads `OP_TaskActivityComplete`.
///
/// # Errors
///
/// A record of another length, a shared task, a slot or step past the
/// journal, or a flag out of range.
pub fn completion(body: &[u8]) -> Result<TaskCompletion> {
    ensure!(
        body.len() == COMPLETION,
        "task completion of {} bytes",
        body.len()
    );
    let mut record = Cursor::new(body);
    let slot = record.word("ended task's slot")?;
    let at = TaskSlot::new(record.word("ended task's kind")?, slot)?;
    let task_id = record.word("ended task's number")?;
    let activity_id = record.word("ended step")?;
    ensure!(
        activity_id < STEPS,
        "ended step {activity_id} past a task's {STEPS}"
    );
    let outcome = if record.flag("task completed word")? {
        Outcome::Completed
    } else {
        Outcome::Failed
    };
    let stage_completed = record.flag("stage completed word")?;
    ensure!(
        !(outcome == Outcome::Failed && stage_completed),
        "a failure that completes a stage"
    );
    Ok(TaskCompletion {
        at,
        task_id,
        activity_id,
        outcome,
        stage_completed,
    })
}

/// Reads `OP_CompletedTasks`.
///
/// # Errors
///
/// A list that is short, long, counts more than `EQEmu` sends, or has a
/// title out of range.
pub fn completed_list(body: &[u8]) -> Result<Vec<CompletedTask>> {
    let mut record = Cursor::new(body);
    let count = usize::try_from(record.word("finished task count")?)
        .context("finished task count out of range")?;
    ensure!(
        count <= COMPLETED_LISTED,
        "{count} finished tasks, past the {COMPLETED_LISTED} listed"
    );
    let mut list = Vec::with_capacity(count);
    for _ in 0..count {
        let task_id = record.word("finished task's number")?;
        let title = record.string(TITLE_LONGEST, "finished task's title")?;
        let completed_at = record.word("finished task's time")?;
        list.push(CompletedTask {
            task_id,
            title,
            completed_at,
        });
    }
    record.end("finished task list")?;
    Ok(list)
}

/// Reads the server's `OP_CancelTask`: the entry it took out.
///
/// # Errors
///
/// A record of another length, a shared task, or a slot past the journal.
pub fn removal(body: &[u8]) -> Result<TaskSlot> {
    ensure!(
        body.len() == REMOVAL,
        "task removal of {} bytes",
        body.len()
    );
    let mut record = Cursor::new(body);
    let slot = record.word("removed task's slot")?;
    TaskSlot::new(record.word("removed task's kind")?, slot)
}

/// What one Titanium journal message says, for the session's journal to
/// check against what it holds before the host hears of it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum News {
    /// An entry's description.
    Described(TaskDescription),
    /// One of an entry's steps.
    Step(TaskActivity),
    /// A step's or an entry's end.
    Concluded(TaskCompletion),
    /// The tasks the player has finished.
    Completed(Vec<CompletedTask>),
    /// The server took an entry out.
    Removed(TaskSlot),
}

/// Reads a Titanium journal message, or None for another opcode.
///
/// # Errors
///
/// The message's own reader's refusal: by its fields and lengths only,
/// never its words.
pub fn titanium(opcode: u16, body: &[u8]) -> Option<Result<News>> {
    Some(match opcode {
        DESCRIPTION_OPCODE => description(body).map(News::Described),
        ACTIVITY_OPCODE => activity(body).map(News::Step),
        COMPLETION_OPCODE => completion(body).map(News::Concluded),
        COMPLETED_OPCODE => completed_list(body).map(News::Completed),
        REMOVAL_OPCODE => removal(body).map(News::Removed),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
