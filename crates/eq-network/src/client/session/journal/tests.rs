use super::*;
use crate::client::session::feature::testing;
use eq_network_game::tasks::{Step, TaskKind};

/// Builds records field by field, as `EQEmu`'s senders write them.
#[derive(Default)]
pub(super) struct Record(pub(super) Vec<u8>);

impl Record {
    pub(super) fn word(mut self, value: u32) -> Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub(super) fn byte(mut self, value: u8) -> Self {
        self.0.push(value);
        self
    }

    pub(super) fn string(mut self, text: &str) -> Self {
        self.0.extend_from_slice(text.as_bytes());
        self.0.push(0);
        self
    }
}

const TITLE: &str = "Private Quest Words";

fn description(kind: u32, slot: u32, task_id: u32) -> Vec<u8> {
    Record::default()
        .word(slot)
        .word(task_id)
        .byte(0)
        .word(kind)
        .word(0)
        .string(TITLE)
        .word(0)
        .word(3)
        .word(1_700_000_000)
        .string("Private description words")
        .byte(1)
        .word(10)
        .word(1)
        .word(1)
        .string("")
        .string("")
        .word(0)
        .byte(0)
        .0
}

fn step(kind: u32, slot: u32, task_id: u32, activity_id: u32, done: u32) -> Vec<u8> {
    Record::default()
        .word(slot)
        .word(kind)
        .word(task_id)
        .word(activity_id)
        .word(0)
        .word(2)
        .word(0)
        .word(0)
        .string("a private target")
        .string("")
        .word(5)
        .word(0)
        .word(0)
        .word(0)
        .word(0)
        .string("")
        .word(done)
        .byte(1)
        .0
}

fn message(opcode: u16, body: &[u8]) -> Message {
    let mut messages = eq_network_game::message::titanium(opcode, body);
    assert_eq!(messages.len(), 1, "{messages:?}");
    messages.remove(0)
}

fn described(kind: u32, slot: u32, task_id: u32) -> Message {
    message(tasks::DESCRIPTION_OPCODE, &description(kind, slot, task_id))
}

fn stepped(kind: u32, slot: u32, task_id: u32, activity_id: u32, done: u32) -> Message {
    message(
        tasks::ACTIVITY_OPCODE,
        &step(kind, slot, task_id, activity_id, done),
    )
}

fn removed(kind: u32, slot: u32) -> Message {
    message(
        tasks::REMOVAL_OPCODE,
        &Record::default().word(slot).word(kind).0,
    )
}

fn world_events(events: &[ClientEvent]) -> Vec<&WorldEvent> {
    events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::World(event) => Some(event),
            _ => None,
        })
        .collect()
}

fn gaps(events: &[ClientEvent]) -> Vec<&crate::client::Gap> {
    events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::Gap(gap) => Some(&**gap),
            _ => None,
        })
        .collect()
}

#[test]
fn the_journal_sent_before_admission_is_told_at_admission_task_first() {
    let mut journal = Journal::default();
    let mut world = World::new(5);
    for slot in (0..tasks::QUEST_SLOTS).rev() {
        journal
            .admit(&described(2, slot, 100 + slot), &mut world)
            .unwrap();
        journal
            .admit(&stepped(2, slot, 100 + slot, 0, 1), &mut world)
            .unwrap();
    }
    journal.admit(&described(0, 0, 99), &mut world).unwrap();
    journal.admit(&stepped(0, 0, 99, 0, 0), &mut world).unwrap();
    journal
        .admit(
            &message(
                tasks::COMPLETED_OPCODE,
                &Record::default().word(1).word(7).string("Done").word(5).0,
            ),
            &mut world,
        )
        .unwrap();
    let told = testing::run(|out| journal.admitted(&mut world, out));
    told.result.unwrap();
    let events = world_events(&told.events);
    // One task and 19 quests, each described then stepped, then the list.
    assert_eq!(events.len(), 2 * 20 + 1);
    assert!(matches!(
        events[0],
        WorldEvent::TaskDescribed { session_id: 5, description } if description.task_id == 99
    ));
    assert!(matches!(
        events[1],
        WorldEvent::TaskStepUpdated { activity, .. } if activity.task_id == 99
    ));
    assert!(matches!(
        events[2],
        WorldEvent::TaskDescribed { description, .. }
            if description.at == TaskSlot { kind: TaskKind::Quest, slot: 0 }
    ));
    assert!(matches!(
        events[40],
        WorldEvent::CompletedTasks { list, .. } if list.len() == 1
    ));
    assert_eq!(gaps(&told.events).len(), 0);
}

#[test]
fn a_step_for_no_entry_of_its_task_is_a_gap_by_opcode_and_length_alone() {
    let mut journal = Journal::default();
    let mut world = World::new(5);
    let body = step(2, 3, 100, 0, 1);
    let told = testing::run(|out| journal.observe(&stepped(2, 3, 100, 0, 1), &mut world, out));
    told.result.unwrap();
    assert_eq!(world_events(&told.events).len(), 0);
    let gap = gaps(&told.events);
    assert_eq!(gap.len(), 1);
    assert_eq!(
        (
            gap[0].opcode,
            gap[0].name,
            gap[0].min_length,
            &gap[0].sample
        ),
        (
            tasks::ACTIVITY_OPCODE,
            Some("OP_TaskActivity"),
            body.len(),
            &None
        )
    );
    let said = format!("{:?}", told.events);
    assert!(!said.contains("private"), "{said}");

    // Described for another task: still stale.
    let _ = testing::run(|out| journal.observe(&described(2, 3, 101), &mut world, out));
    let told = testing::run(|out| journal.observe(&stepped(2, 3, 100, 0, 1), &mut world, out));
    assert_eq!(gaps(&told.events).len(), 1);
    // Its own task's step goes through.
    let told = testing::run(|out| journal.observe(&stepped(2, 3, 101, 0, 1), &mut world, out));
    assert!(matches!(
        world_events(&told.events)[..],
        [WorldEvent::TaskStepUpdated { .. }]
    ));
    assert_eq!(gaps(&told.events).len(), 0);
}

#[test]
fn a_stale_step_before_admission_is_told_at_admission_without_its_words() {
    let mut journal = Journal::default();
    let mut world = World::new(5);
    journal
        .admit(&stepped(2, 1, 100, 0, 1), &mut world)
        .unwrap();
    let told = testing::run(|out| journal.admitted(&mut world, out));
    let gap = gaps(&told.events);
    assert_eq!(gap.len(), 1);
    assert_eq!(gap[0].sample, None);
    assert_eq!(world_events(&told.events).len(), 0);
}

#[test]
fn refusals_before_admission_are_kept_up_to_the_bound_then_only_counted() {
    for extra in [0, 5] {
        let mut journal = Journal::default();
        let mut world = World::new(5);
        for _ in 0..MAX_REFUSED + extra {
            journal
                .admit(&stepped(2, 1, 100, 0, 1), &mut world)
                .unwrap();
        }
        assert_eq!(journal.refused.len(), MAX_REFUSED);
        assert_eq!(journal.dropped, extra);
        let told = testing::run(|out| journal.admitted(&mut world, out));
        told.result.unwrap();
        let diagnostics: Vec<_> = told
            .events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::Diagnostic(said) => Some(said.as_str()),
                _ => None,
            })
            .collect();
        if extra == 0 {
            assert_eq!(diagnostics.len(), 0, "{diagnostics:?}");
        } else {
            assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
            assert!(diagnostics[0].contains("dropped 5 "), "{diagnostics:?}");
            assert!(!diagnostics[0].contains("private"), "{diagnostics:?}");
        }
        assert!(journal.refused.is_empty() && journal.dropped == 0);
    }
}

#[test]
fn a_new_task_in_a_slot_forgets_the_old_tasks_steps_and_the_same_task_keeps_them() {
    let mut journal = Journal::default();
    let mut world = World::new(5);
    let at = TaskSlot {
        kind: TaskKind::Quest,
        slot: 2,
    };
    for news in [
        described(2, 2, 100),
        stepped(2, 2, 100, 0, 1),
        stepped(2, 2, 100, 1, 0),
    ] {
        journal.admit(&news, &mut world).unwrap();
    }
    assert_eq!(journal.entries[&at].steps.len(), 2);
    journal.admit(&described(2, 2, 100), &mut world).unwrap();
    assert_eq!(journal.entries[&at].steps.len(), 2);
    journal
        .admit(&stepped(2, 2, 100, 0, 3), &mut world)
        .unwrap();
    assert!(matches!(
        &journal.entries[&at].steps[&0].step,
        Step::Shown(objective) if objective.done == 3
    ));
    journal.admit(&described(2, 2, 200), &mut world).unwrap();
    assert_eq!(journal.entries[&at].steps.len(), 0);
}

#[test]
fn the_removal_empties_its_slot_and_a_new_zone_starts_empty() {
    let mut journal = Journal::default();
    let mut world = World::new(5);
    journal.admit(&described(2, 4, 100), &mut world).unwrap();
    let told = testing::run(|out| journal.observe(&removed(2, 4), &mut world, out));
    assert!(matches!(
        world_events(&told.events)[..],
        [WorldEvent::TaskRemoved { session_id: 5, at }]
            if *at == TaskSlot { kind: TaskKind::Quest, slot: 4 }
    ));
    assert!(journal.entries.is_empty());
    let told = testing::run(|out| journal.observe(&stepped(2, 4, 100, 0, 1), &mut world, out));
    assert_eq!(gaps(&told.events).len(), 1);

    journal.admit(&described(0, 0, 9), &mut world).unwrap();
    journal.connection_ended(&mut world);
    assert!(journal.entries.is_empty() && journal.completed.is_none());
    let told = testing::run(|out| journal.admitted(&mut world, out));
    assert_eq!(told.events.len(), 0);
}

#[test]
fn an_end_is_told_only_for_an_entry_of_its_task() {
    let mut journal = Journal::default();
    let mut world = World::new(5);
    let end = |task_id: u32| {
        message(
            tasks::COMPLETION_OPCODE,
            &Record::default()
                .word(0)
                .word(0)
                .word(task_id)
                .word(1)
                .word(1)
                .word(1)
                .0,
        )
    };
    let told = testing::run(|out| journal.observe(&end(9), &mut world, out));
    assert_eq!(gaps(&told.events).len(), 1);
    journal.admit(&described(0, 0, 9), &mut world).unwrap();
    let told = testing::run(|out| journal.observe(&end(9), &mut world, out));
    assert!(matches!(
        world_events(&told.events)[..],
        [WorldEvent::TaskConcluded { completion, .. }] if completion.stage_completed
    ));
    // The end leaves the entry; EQEmu's removal takes it out.
    assert_eq!(journal.entries.len(), 1);
}

#[test]
fn a_journal_message_is_never_sampled_read_partly_or_unreadable() {
    use super::super::{
        coverage::{Stage, Unheard},
        wire::Titanium,
    };
    use crate::client::GapStage;
    let mut refused = description(1, 0, 9);
    let unreadable = eq_network_game::message::titanium(tasks::DESCRIPTION_OPCODE, &refused);
    let Message::Unreadable { error, .. } = &unreadable[0] else {
        panic!("{unreadable:?}");
    };
    assert!(!error.contains("Private"), "{error}");
    refused.truncate(30);
    let read = description(2, 0, 9);
    let told = testing::run(|out| {
        out.log.begin_stage(GapStage::Zone)?;
        let mut unheard = Unheard::new(Stage::Zone);
        // Read all but its crystal parts: a partly read gap.
        unheard.tell(&Titanium, tasks::DESCRIPTION_OPCODE, &read, out.log)?;
        unheard.sift(
            &Titanium,
            (tasks::DESCRIPTION_OPCODE, &refused),
            eq_network_game::message::titanium(tasks::DESCRIPTION_OPCODE, &refused),
            out.log,
        )?;
        out.log.end_stage()
    });
    told.result.unwrap();
    let gap = gaps(&told.events);
    assert!(gap.len() >= 2, "{gap:?}");
    for gap in gap {
        assert_eq!(gap.sample, None, "{gap:?}");
    }
    let said = format!("{:?}", told.events);
    assert!(
        !said.contains("Private") && !said.contains("50726976"),
        "{said}"
    );
}
