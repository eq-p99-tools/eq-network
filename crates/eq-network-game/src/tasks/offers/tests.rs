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

    fn string(mut self, text: &str) -> Self {
        self.0.extend_from_slice(text.as_bytes());
        self.0.push(0);
        self
    }
}

fn offered_task(record: Record, task_id: u32, title: &str, shown: u32) -> Record {
    let mut record = record
        .word(task_id)
        .word(0)
        .word(2)
        .string(title)
        .string("Bring the wolf pelts to the tanner.")
        .word(shown);
    for index in 0..shown {
        record = record
            .signed(i32::try_from(index).unwrap())
            .signed(2)
            .signed(0)
            .string("a gray wolf")
            .string("")
            .signed(5)
            .signed(0)
            .signed(0)
            .signed(54)
            .string("Slay gray wolves");
    }
    record
}

/// `SendTaskSelector`'s record: `count` tasks from spawn 77.
fn offer_record(count: u32, kind: u32, title: &str) -> Vec<u8> {
    let mut record = Record::default().word(count).word(kind).word(77);
    for index in 0..count {
        record = offered_task(record, 1000 + index, title, 1);
    }
    record.0
}

#[test]
fn an_offer_reads_every_field_as_sent() {
    let read = offers(&offer_record(2, 2, "The Tanner's Request")).unwrap();
    assert_eq!(read.kind, TaskKind::Quest);
    assert_eq!(read.giver, 77);
    assert_eq!(read.tasks.len(), 2);
    assert_eq!(
        read.tasks[1],
        OfferedTask {
            task_id: 1001,
            duration_seconds: 0,
            duration_code: 2,
            title: "The Tanner's Request".to_owned(),
            description: "Bring the wolf pelts to the tanner.".to_owned(),
            steps: vec![OfferedStep {
                index: 0,
                activity_type: 2,
                target_name: "a gray wolf".to_owned(),
                item_list: String::new(),
                goal: 5,
                skill_id: 0,
                spell_id: 0,
                zone_id: 54,
                description: "Slay gray wolves".to_owned(),
            }],
        }
    );
    assert_eq!(
        offers(&offer_record(1, 0, "A task")).unwrap().kind,
        TaskKind::Task
    );
}

#[test]
fn an_offer_cut_anywhere_or_with_bytes_after_it_is_refused() {
    let record = offer_record(2, 2, "The Tanner's Request");
    for length in 0..record.len() {
        assert!(offers(&record[..length]).is_err(), "cut at {length}");
    }
    let mut longer = record;
    longer.push(0);
    assert!(offers(&longer).is_err());
}

#[test]
fn an_offer_counts_one_to_forty_of_a_titanium_kind_and_twenty_steps() {
    assert!(offers(&offer_record(0, 2, "None")).is_err());
    assert!(offers(&offer_record(40, 2, "Many")).is_ok());
    assert!(offers(&offer_record(41, 2, "Too many")).is_err());
    assert!(offers(&offer_record(1, 1, "Shared")).is_err());
    assert!(offers(&offer_record(1, 3, "Other")).is_err());
    // A count past what follows is a cut record.
    let mut short = offer_record(2, 2, "Two");
    short[..4].copy_from_slice(&3u32.to_le_bytes());
    assert!(offers(&short).is_err());
    let steps =
        |shown: u32| offered_task(Record::default().word(1).word(2).word(77), 9, "S", shown).0;
    assert!(offers(&steps(20)).is_ok());
    assert!(offers(&steps(21)).is_err());
    assert!(offers(&offer_record(1, 2, &"t".repeat(63))).is_ok());
    assert!(offers(&offer_record(1, 2, &"t".repeat(64))).is_err());
}

#[test]
fn an_offered_step_past_twenty_is_refused() {
    let mut record = Record::default().word(1).word(2).word(77);
    record = record
        .word(9)
        .word(0)
        .word(0)
        .string("T")
        .string("D")
        .word(1)
        .signed(20);
    record = record
        .signed(2)
        .signed(0)
        .string("")
        .string("")
        .signed(1)
        .signed(0)
        .signed(0)
        .signed(0)
        .string("");
    assert!(offers(&record.0).is_err());
}

fn history_record(index: u32, count: u32) -> Vec<u8> {
    let mut record = Record::default().word(index).word(count);
    for _ in 0..count {
        record = record
            .word(2)
            .string("a gray wolf")
            .string("")
            .word(5)
            .word(0xffff_ffff)
            .word(0xffff_ffff)
            .word(54)
            .word(0)
            .string("Slay gray wolves");
    }
    record.0
}

#[test]
fn a_history_reads_its_done_steps() {
    let read = history(&history_record(3, 2)).unwrap();
    assert_eq!(read.index, 3);
    assert_eq!(
        read.steps,
        vec![
            DoneStep {
                activity_type: 2,
                target_name: "a gray wolf".to_owned(),
                item_list: String::new(),
                goal: 5,
                zone_id: 54,
                description: "Slay gray wolves".to_owned(),
            };
            2
        ]
    );
    assert_eq!(history(&history_record(0, 0)).unwrap().steps, vec![]);
    assert!(history(&history_record(0, 20)).is_ok());
    assert!(history(&history_record(0, 21)).is_err());
    let record = history_record(3, 2);
    for length in 0..record.len() {
        assert!(history(&record[..length]).is_err(), "cut at {length}");
    }
    let mut longer = record;
    longer.push(0);
    assert!(history(&longer).is_err());
}

#[test]
fn no_refusal_carries_the_offers_words() {
    let title = "Secret Title Words";
    let cases = [
        offer_record(1, 1, title),
        offer_record(1, 2, &format!("{title}{}", "x".repeat(64))),
        offer_record(1, 2, title)[..40].to_vec(),
    ];
    for record in cases {
        let error = format!("{:#}", offers(&record).unwrap_err());
        assert!(
            !error.contains("Secret") && !error.contains("tanner"),
            "{error}"
        );
    }
    let mut record = history_record(0, 1);
    record.truncate(record.len() - 3);
    let error = format!("{:#}", history(&record).unwrap_err());
    assert!(
        !error.contains("wolf") && !error.contains("Slay"),
        "{error}"
    );
}

#[test]
fn the_requests_are_the_sizes_eqemus_handlers_read() {
    let accept = titanium_accept(TaskKind::Quest, 1234, 77).unwrap();
    assert_eq!(accept.opcode, 0x207f);
    assert_eq!(accept.body, [2, 0, 0, 0, 0xd2, 0x04, 0, 0, 77, 0, 0, 0]);
    assert_eq!(
        titanium_accept(TaskKind::Task, 5, 1).unwrap().body[..4],
        [0, 0, 0, 0]
    );
    assert!(titanium_accept(TaskKind::Quest, 0, 77).is_err());
    assert!(titanium_accept(TaskKind::Quest, 5, 0).is_err());

    let cancel = titanium_cancel(TaskSlot {
        kind: TaskKind::Quest,
        slot: 18,
    })
    .unwrap();
    assert_eq!(
        (cancel.opcode, cancel.body),
        (0x3ba8, vec![18, 0, 0, 0, 2, 0, 0, 0])
    );
    assert_eq!(
        titanium_cancel(TaskSlot {
            kind: TaskKind::Task,
            slot: 0
        })
        .unwrap()
        .body,
        [0; 8]
    );
    for (kind, slot) in [(TaskKind::Quest, 19), (TaskKind::Task, 1)] {
        assert!(titanium_cancel(TaskSlot { kind, slot }).is_err());
    }

    let history = titanium_history(49).unwrap();
    assert_eq!((history.opcode, history.body), (0x5df4, vec![49, 0, 0, 0]));
    assert!(titanium_history(50).is_err());

    let timers = titanium_timers();
    assert_eq!((timers.opcode, timers.body), (0x6a1d, vec![]));
}
