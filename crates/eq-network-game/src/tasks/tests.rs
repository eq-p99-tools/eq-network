use super::*;

/// Builds records field by field, as `EQEmu`'s senders write them.
#[derive(Default)]
struct Record(Vec<u8>);

impl Record {
    fn word(mut self, value: u32) -> Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn signed(self, value: i32) -> Self {
        self.word(u32::from_le_bytes(value.to_le_bytes()))
    }

    fn byte(mut self, value: u8) -> Self {
        self.0.push(value);
        self
    }

    fn string(mut self, text: &str) -> Self {
        self.0.extend_from_slice(text.as_bytes());
        self.0.push(0);
        self
    }

    fn done(self) -> Vec<u8> {
        self.0
    }
}

/// `SendActiveTaskDescription`'s record for quest slot 3.
fn description_record(kind: u32, slot: u32, title: &str) -> Vec<u8> {
    Record::default()
        .word(slot)
        .word(1234)
        .byte(1)
        .word(kind)
        .word(0)
        .string(title)
        .word(3600)
        .word(2)
        .word(1_700_000_000)
        .string("Bring the wolf pelts to the tanner.")
        .byte(1)
        .word(250)
        .word(1)
        .word(0)
        .string("A tanned cloak")
        .string("\u{12}0012345 A tanned cloak\u{12}")
        .word(0)
        .byte(0)
        .done()
}

fn long_activity(activity_id: u32) -> Vec<u8> {
    Record::default()
        .word(3)
        .word(2)
        .word(1234)
        .word(activity_id)
        .word(1)
        .signed(2)
        .word(1)
        .word(0)
        .string("a gray wolf")
        .string("")
        .signed(5)
        .signed(0)
        .signed(0)
        .signed(54)
        .word(0)
        .string("Slay gray wolves")
        .signed(2)
        .byte(1)
        .done()
}

fn hidden_activity(optional: u8) -> Vec<u8> {
    Record::default()
        .word(3)
        .word(2)
        .word(1234)
        .word(4)
        .word(0)
        .word(0xffff_ffff)
        .byte(optional)
        .done()
}

#[test]
fn a_description_reads_every_field_as_sent() {
    let read = description(&description_record(2, 3, "The Tanner's Request")).unwrap();
    assert_eq!(
        read,
        TaskDescription {
            at: TaskSlot {
                kind: TaskKind::Quest,
                slot: 3
            },
            task_id: 1234,
            open_journal: true,
            title: "The Tanner's Request".to_owned(),
            duration_seconds: 3600,
            duration_code: 2,
            started_at: 1_700_000_000,
            description: "Bring the wolf pelts to the tanner.".to_owned(),
            rewards: Rewards {
                listed: true,
                coin_copper: 250,
                experience: true,
                faction: false,
                text: "A tanned cloak".to_owned(),
                item_link: "\u{12}0012345 A tanned cloak\u{12}".to_owned(),
            },
        }
    );
    let task = description(&description_record(0, 0, "A task")).unwrap();
    assert_eq!(task.at.kind, TaskKind::Task);
}

#[test]
fn a_description_cut_short_anywhere_or_with_bytes_after_it_is_refused() {
    let record = description_record(2, 3, "The Tanner's Request");
    for length in 0..record.len() {
        assert!(description(&record[..length]).is_err(), "cut at {length}");
    }
    let mut longer = record;
    longer.push(0);
    assert!(description(&longer).is_err());
}

#[test]
fn a_shared_task_a_slot_past_the_journal_and_a_title_too_long_are_refused() {
    assert!(description(&description_record(1, 0, "Shared")).is_err());
    assert!(description(&description_record(3, 0, "Other")).is_err());
    assert!(description(&description_record(0, 1, "Second task")).is_err());
    assert!(description(&description_record(2, 19, "Twentieth quest")).is_err());
    assert!(description(&description_record(2, 18, "Nineteenth quest")).is_ok());
    assert!(description(&description_record(2, 0, &"t".repeat(63))).is_ok());
    assert!(description(&description_record(2, 0, &"t".repeat(64))).is_err());
}

#[test]
fn no_refusal_carries_the_tasks_words() {
    let title = "Secret Title Words";
    let cases = [
        description_record(1, 0, title),
        description_record(2, 0, &format!("{title}{}", "x".repeat(64))),
        description_record(2, 0, title)[..40].to_vec(),
    ];
    for record in cases {
        let error = format!("{:#}", description(&record).unwrap_err());
        assert!(!error.contains("Secret"), "{error}");
        assert!(!error.contains("tanner"), "{error}");
    }
    let mut activity_record = long_activity(0);
    activity_record.truncate(activity_record.len() - 3);
    let error = format!("{:#}", activity(&activity_record).unwrap_err());
    assert!(
        !error.contains("wolf") && !error.contains("Slay"),
        "{error}"
    );
}

#[test]
fn a_shown_step_reads_its_objective() {
    let read = activity(&long_activity(2)).unwrap();
    assert_eq!(
        read,
        TaskActivity {
            at: TaskSlot {
                kind: TaskKind::Quest,
                slot: 3
            },
            task_id: 1234,
            activity_id: 2,
            group: 1,
            step: Step::Shown(Objective {
                activity_type: 2,
                optional: true,
                target_name: "a gray wolf".to_owned(),
                item_list: String::new(),
                goal: 5,
                skill_id: 0,
                spell_id: 0,
                zone_id: 54,
                description: "Slay gray wolves".to_owned(),
                done: 2,
            }),
        }
    );
}

#[test]
fn a_hidden_step_is_its_header_the_mark_and_its_optional_byte() {
    let read = activity(&hidden_activity(1)).unwrap();
    assert_eq!(read.activity_id, 4);
    assert_eq!(read.step, Step::Hidden { optional: true });
    assert_eq!(
        activity(&hidden_activity(0)).unwrap().step,
        Step::Hidden { optional: false }
    );
    assert!(activity(&hidden_activity(2)).is_err());
    let mut longer = hidden_activity(0);
    longer.push(0);
    assert!(activity(&longer).is_err());
}

#[test]
fn a_step_cut_short_anywhere_or_numbered_past_twenty_is_refused() {
    let record = long_activity(0);
    for length in 0..record.len() {
        assert!(activity(&record[..length]).is_err(), "cut at {length}");
    }
    let mut longer = record;
    longer.push(0);
    assert!(activity(&longer).is_err());
    assert!(activity(&long_activity(19)).is_ok());
    assert!(activity(&long_activity(20)).is_err());
}

#[test]
fn a_string_without_its_end_is_refused_not_cut() {
    // A finished task's title running to the record's end.
    let mut record = Record::default().word(1).word(7).done();
    record.extend_from_slice(b"Done task");
    assert!(completed_list(&record).is_err());
    // A step's target name longer than its 64 with the end: refused, where
    // cutting it would read the rest of the name as the next fields.
    let named = |name: &str| {
        let mut record = long_activity(0);
        let start = 32;
        let end = start + "a gray wolf".len();
        record.splice(start..end, name.bytes());
        record
    };
    assert!(activity(&named(&"w".repeat(63))).is_ok());
    assert!(activity(&named(&"w".repeat(64))).is_err());
}

fn completion_record(
    slot: u32,
    kind: u32,
    activity_id: u32,
    completed: u32,
    stage: u32,
) -> Vec<u8> {
    Record::default()
        .word(slot)
        .word(kind)
        .word(1234)
        .word(activity_id)
        .word(completed)
        .word(stage)
        .done()
}

#[test]
fn a_completion_says_a_step_a_task_or_a_failure() {
    let step = completion(&completion_record(3, 2, 1, 1, 1)).unwrap();
    assert_eq!(step.outcome, Outcome::Completed);
    assert!(step.stage_completed);
    assert_eq!(step.activity_id, 1);
    let whole = completion(&completion_record(0, 0, 2, 1, 0)).unwrap();
    assert_eq!(
        (whole.at.kind, whole.outcome, whole.stage_completed),
        (TaskKind::Task, Outcome::Completed, false)
    );
    let failed = completion(&completion_record(3, 2, 0, 0, 0)).unwrap();
    assert_eq!(failed.outcome, Outcome::Failed);
    assert!(completion(&completion_record(3, 2, 0, 0, 1)).is_err());
    assert!(completion(&completion_record(3, 2, 0, 2, 0)).is_err());
    assert!(completion(&completion_record(3, 2, 20, 1, 0)).is_err());
    assert!(completion(&completion_record(3, 1, 0, 1, 0)).is_err());
    let record = completion_record(3, 2, 1, 1, 1);
    assert!(completion(&record[..23]).is_err());
}

fn completed_record(count: u32, entries: usize) -> Vec<u8> {
    let mut record = Record::default().word(count);
    for index in 0..entries {
        record = record
            .word(u32::try_from(index).unwrap() + 1)
            .string("Done task")
            .word(1_700_000_000);
    }
    record.done()
}

#[test]
fn the_finished_list_reads_up_to_fifty_and_its_count_must_match() {
    let list = completed_list(&completed_record(2, 2)).unwrap();
    assert_eq!(
        list,
        vec![
            CompletedTask {
                task_id: 1,
                title: "Done task".to_owned(),
                completed_at: 1_700_000_000
            },
            CompletedTask {
                task_id: 2,
                title: "Done task".to_owned(),
                completed_at: 1_700_000_000
            },
        ]
    );
    assert_eq!(completed_list(&completed_record(0, 0)).unwrap(), vec![]);
    assert_eq!(completed_list(&completed_record(50, 50)).unwrap().len(), 50);
    assert!(completed_list(&completed_record(51, 51)).is_err());
    assert!(completed_list(&completed_record(3, 2)).is_err());
    assert!(completed_list(&completed_record(1, 2)).is_err());
    assert!(completed_list(&[]).is_err());
}

#[test]
fn a_removal_names_its_slot() {
    let removed = removal(&Record::default().word(3).word(2).done()).unwrap();
    assert_eq!(
        removed,
        TaskSlot {
            kind: TaskKind::Quest,
            slot: 3
        }
    );
    assert!(removal(&Record::default().word(0).word(1).done()).is_err());
    assert!(removal(&Record::default().word(0).done()).is_err());
}
