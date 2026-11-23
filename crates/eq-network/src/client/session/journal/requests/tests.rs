use super::super::super::feature::{testing, Feature, World};
use super::super::{tests::Record, Journal};
use super::*;
use crate::client::ClientEvent;
use eq_network_game::{
    message::Message,
    tasks::{self, TaskKind},
    world::WorldEvent,
};

const QUEST_3: TaskSlot = TaskSlot {
    kind: TaskKind::Quest,
    slot: 3,
};

fn message(opcode: u16, body: &[u8]) -> Message {
    let mut messages = eq_network_game::message::titanium(opcode, body);
    assert_eq!(messages.len(), 1, "{messages:?}");
    messages.remove(0)
}

/// `SendTaskSelector`'s offer of `tasks` from spawn 77.
fn offer(tasks: &[u32]) -> Message {
    let mut record = Record::default()
        .word(u32::try_from(tasks.len()).unwrap())
        .word(2)
        .word(77);
    for task_id in tasks {
        record = record
            .word(*task_id)
            .word(0)
            .word(2)
            .string("Private Offer Words")
            .string("Private offer description")
            .word(0);
    }
    message(tasks::offers::OFFERS_OPCODE, &record.0)
}

fn described(slot: u32, task_id: u32) -> Message {
    let body = Record::default()
        .word(slot)
        .word(task_id)
        .byte(0)
        .word(2)
        .word(0)
        .string("Title")
        .word(0)
        .word(3)
        .word(1_700_000_000)
        .string("Words")
        .byte(1)
        .word(10)
        .word(1)
        .word(1)
        .string("")
        .string("")
        .word(0)
        .byte(0)
        .0;
    message(tasks::DESCRIPTION_OPCODE, &body)
}

fn removed(slot: u32) -> Message {
    message(
        tasks::REMOVAL_OPCODE,
        &Record::default().word(slot).word(2).0,
    )
}

fn completed(count: u32) -> Message {
    let mut record = Record::default().word(count);
    for task_id in 0..count {
        record = record.word(task_id + 1).string("Done").word(5);
    }
    message(tasks::COMPLETED_OPCODE, &record.0)
}

fn accept(task_id: u32) -> ClientCommand {
    ClientCommand::AcceptTask {
        session_id: 5,
        task_id,
        created: Instant::now(),
    }
}

fn cancel(at: TaskSlot) -> ClientCommand {
    ClientCommand::CancelTask {
        session_id: 5,
        at,
        created: Instant::now(),
    }
}

fn history(index: u32) -> ClientCommand {
    ClientCommand::AskTaskHistory {
        session_id: 5,
        index,
        created: Instant::now(),
    }
}

fn timers() -> ClientCommand {
    ClientCommand::ListTaskTimers {
        session_id: 5,
        created: Instant::now(),
    }
}

/// What one command did: the packets sent, and the refusal told, if any.
fn run(
    journal: &mut Journal,
    world: &mut World,
    command: &ClientCommand,
) -> (Vec<(u16, Vec<u8>)>, Option<String>) {
    assert!(journal.owns(command), "{command:?}");
    let told = testing::run(|out| journal.handle(command, world, out));
    told.result.unwrap();
    let refusal = told.events.iter().find_map(|event| match event {
        ClientEvent::World(WorldEvent::TaskRefused {
            session_id: 5,
            reason,
        }) => Some(reason.clone()),
        _ => None,
    });
    let sent = told
        .sent
        .into_iter()
        .map(|packet| (packet.opcode, packet.body))
        .collect();
    (sent, refusal)
}

fn hear(journal: &mut Journal, world: &mut World, news: &Message) -> Vec<ClientEvent> {
    let told = testing::run(|out| journal.observe(news, world, out));
    told.result.unwrap();
    told.events
}

fn sent_nothing(outcome: &(Vec<(u16, Vec<u8>)>, Option<String>)) -> bool {
    outcome.0.is_empty() && outcome.1.is_some()
}

#[test]
fn an_accept_goes_only_for_an_offered_task_while_its_npc_giver_is_held() {
    let mut journal = Journal::with_requests();
    let mut world = World::new(5);
    assert!(sent_nothing(&run(&mut journal, &mut world, &accept(1000))));

    let events = hear(&mut journal, &mut world, &offer(&[1000, 1001]));
    assert!(matches!(
        &events[..],
        [ClientEvent::World(WorldEvent::TasksOffered { session_id: 5, offers })]
            if offers.giver == 77 && offers.tasks.len() == 2
    ));
    // Not offered, and task 0, are refused; the offer stands.
    assert!(sent_nothing(&run(&mut journal, &mut world, &accept(999))));
    assert!(sent_nothing(&run(&mut journal, &mut world, &accept(0))));
    // The giver isn't a spawn the session holds: refused, and the offer goes.
    let gone = run(&mut journal, &mut world, &accept(1000));
    assert!(sent_nothing(&gone), "{gone:?}");
    assert!(journal.requests.as_ref().unwrap().offer.is_none());

    // A corpse is no giver either.
    hear(&mut journal, &mut world, &offer(&[1000, 1001]));
    world
        .spawns
        .insert(testing::spawn(77, SpawnKind::NpcCorpse));
    assert!(sent_nothing(&run(&mut journal, &mut world, &accept(1000))));

    hear(&mut journal, &mut world, &offer(&[1000, 1001]));
    world.spawns.insert(testing::spawn(77, SpawnKind::Npc));
    let (sent, refusal) = run(&mut journal, &mut world, &accept(1001));
    assert_eq!(refusal, None);
    assert_eq!(
        sent,
        [(0x207f, vec![2, 0, 0, 0, 0xe9, 0x03, 0, 0, 77, 0, 0, 0])]
    );
    // One send drops the offer, as EQEmu clears its own.
    assert!(sent_nothing(&run(&mut journal, &mut world, &accept(1000))));
}

#[test]
fn a_task_already_held_is_not_accepted_again() {
    let mut journal = Journal::with_requests();
    let mut world = World::new(5);
    world.spawns.insert(testing::spawn(77, SpawnKind::Npc));
    hear(&mut journal, &mut world, &described(3, 1000));
    hear(&mut journal, &mut world, &offer(&[1000]));
    assert!(sent_nothing(&run(&mut journal, &mut world, &accept(1000))));
    assert!(journal.requests.as_ref().unwrap().offer.is_some());
}

#[test]
fn an_offer_before_the_admission_is_refused_and_not_kept() {
    let mut journal = Journal::with_requests();
    let mut world = World::new(5);
    journal.admit(&offer(&[1000]), &mut world).unwrap();
    assert!(journal.requests.as_ref().unwrap().offer.is_none());
    let told = testing::run(|out| journal.admitted(&mut world, out));
    told.result.unwrap();
    let gaps: Vec<_> = told
        .events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::Gap(gap) => Some(gap),
            _ => None,
        })
        .collect();
    assert_eq!(gaps.len(), 1);
    assert_eq!(
        (gaps[0].opcode, gaps[0].sample.as_ref()),
        (tasks::offers::OFFERS_OPCODE, None)
    );
    let said = format!("{:?}", told.events);
    assert!(!said.contains("Private"), "{said}");
}

#[test]
fn a_decline_sends_nothing_and_ends_the_offer() {
    let mut journal = Journal::with_requests();
    let mut world = World::new(5);
    let decline = ClientCommand::DeclineTasks { session_id: 5 };
    assert!(sent_nothing(&run(&mut journal, &mut world, &decline)));
    hear(&mut journal, &mut world, &offer(&[1000]));
    assert_eq!(run(&mut journal, &mut world, &decline), (vec![], None));
    world.spawns.insert(testing::spawn(77, SpawnKind::Npc));
    assert!(sent_nothing(&run(&mut journal, &mut world, &accept(1000))));
}

#[test]
fn a_cancel_needs_a_held_entry_and_goes_once_until_the_removal() {
    let mut journal = Journal::with_requests();
    let mut world = World::new(5);
    assert!(sent_nothing(&run(
        &mut journal,
        &mut world,
        &cancel(QUEST_3)
    )));
    for at in [
        TaskSlot {
            kind: TaskKind::Quest,
            slot: 19,
        },
        TaskSlot {
            kind: TaskKind::Task,
            slot: 1,
        },
    ] {
        assert!(sent_nothing(&run(&mut journal, &mut world, &cancel(at))));
    }
    hear(&mut journal, &mut world, &described(3, 1000));
    assert_eq!(
        run(&mut journal, &mut world, &cancel(QUEST_3)),
        (vec![(0x3ba8, vec![3, 0, 0, 0, 2, 0, 0, 0])], None)
    );
    // A second waits for the server's removal.
    assert!(sent_nothing(&run(
        &mut journal,
        &mut world,
        &cancel(QUEST_3)
    )));
    hear(&mut journal, &mut world, &removed(3));
    assert!(sent_nothing(&run(
        &mut journal,
        &mut world,
        &cancel(QUEST_3)
    )));
    hear(&mut journal, &mut world, &described(3, 1001));
    assert_eq!(run(&mut journal, &mut world, &cancel(QUEST_3)).0.len(), 1);

    // A new zone holds no cancel and no offer, and still takes requests.
    hear(&mut journal, &mut world, &offer(&[1000]));
    journal.connection_ended(&mut world);
    let requests = journal.requests.as_ref().unwrap();
    assert!(requests.cancelling.is_empty() && requests.offer.is_none());
}

#[test]
fn a_history_names_a_listed_task_and_is_paced() {
    let mut journal = Journal::with_requests();
    let mut world = World::new(5);
    assert!(sent_nothing(&run(&mut journal, &mut world, &history(0))));
    hear(&mut journal, &mut world, &completed(2));
    assert!(sent_nothing(&run(&mut journal, &mut world, &history(2))));
    assert_eq!(
        run(&mut journal, &mut world, &history(1)),
        (vec![(0x5df4, vec![1, 0, 0, 0])], None)
    );
    assert!(sent_nothing(&run(&mut journal, &mut world, &history(0))));
    journal.requests.as_mut().unwrap().history_sent = Instant::now().checked_sub(PACE);
    assert_eq!(run(&mut journal, &mut world, &history(0)).0.len(), 1);

    let reply = Record::default().word(1).word(0).0;
    let events = hear(
        &mut journal,
        &mut world,
        &message(tasks::offers::HISTORY_OPCODE, &reply),
    );
    assert!(matches!(
        &events[..],
        [ClientEvent::World(WorldEvent::TaskHistory { session_id: 5, history })]
            if history.index == 1
    ));
}

#[test]
fn task_timers_are_paced() {
    let mut journal = Journal::with_requests();
    let mut world = World::new(5);
    assert_eq!(
        run(&mut journal, &mut world, &timers()),
        (vec![(0x6a1d, vec![])], None)
    );
    assert!(sent_nothing(&run(&mut journal, &mut world, &timers())));
    journal.requests.as_mut().unwrap().timers_sent = Instant::now().checked_sub(PACE);
    assert_eq!(run(&mut journal, &mut world, &timers()).0.len(), 1);
}

#[test]
fn a_journal_without_requests_owns_none_and_still_tells_offers() {
    let mut journal = Journal::default();
    let mut world = World::new(5);
    assert_eq!(journal.capabilities(), vec![]);
    for command in [
        accept(1),
        ClientCommand::DeclineTasks { session_id: 5 },
        cancel(QUEST_3),
        history(0),
        timers(),
    ] {
        assert!(!journal.owns(&command), "{command:?}");
    }
    let events = hear(&mut journal, &mut world, &offer(&[1000]));
    assert!(matches!(
        &events[..],
        [ClientEvent::World(WorldEvent::TasksOffered { .. })]
    ));
    assert_eq!(
        Journal::with_requests().capabilities(),
        vec![crate::world::Capability::Tasks]
    );
}
