//! P99 progression authority through the real zone event and command loops.
use super::super::feature::testing;
use super::*;
use crate::client::ServerProtocol;
use eq_network_game::{
    confirmations::{ConsentClosure, ConsentId, ConsentKind, TRANSLOCATE_OPCODE},
    resurrection::OFFER_OPCODE as RESURRECTION_OPCODE,
    training::{self, TrainingRequest, TrainingUpdate},
    world::{Position, SpawnKind},
    zoning::{ZoneAnswer, ZoneOffer},
};

struct Visit {
    features: Features,
    world: World,
}

impl Visit {
    fn new() -> Self {
        let server = servers::server_type(ServerProtocol::Project1999);
        let mut features =
            Features::new(server, "Tester", eq_network_game::food::AutoEat::default());
        let mut world = World::new(5);
        let mut player = testing::player(7);
        player.name = "Tester".into();
        player.class = Some(1);
        player.practice_points = Some(3);
        player.skills = Some(vec![0; 100]);
        player.skills.as_mut().unwrap()[30] = 12;
        world.player.admit(player);
        world.own_spawn = Some(7);
        world.zone = (4, 0);
        world.admitted = Some(Instant::now());
        let admission = testing::run(|out| {
            features.admitted(&mut world, out)?;
            settle(&mut features, &mut world, out, true)
        });
        assert!(admission.result.unwrap().is_none());
        Self { features, world }
    }

    fn news(&mut self, messages: Vec<Message>) -> testing::Outcome<Result<Option<ZoneExit>>> {
        testing::run(|out| hear(messages, &mut self.features, &mut self.world, out, true))
    }

    fn packet(&mut self, opcode: u16, body: &[u8]) -> testing::Outcome<Result<Option<ZoneExit>>> {
        self.news(eq_network_game::message::titanium(opcode, body))
    }

    fn command(&mut self, command: ClientCommand) -> testing::Outcome<Result<Option<ZoneExit>>> {
        testing::run(|out| {
            apply_commands(&[command], &mut self.features, &mut self.world, out, true)
        })
    }

    fn answer(
        &mut self,
        id: ConsentId,
        accept: bool,
    ) -> testing::Outcome<Result<Option<ZoneExit>>> {
        self.command(match id.kind {
            ConsentKind::Resurrection => ClientCommand::AnswerResurrection {
                session_id: id.session_id,
                offer_id: id,
                accept,
            },
            ConsentKind::Translocate => ClientCommand::AnswerTranslocate {
                session_id: id.session_id,
                offer_id: id,
                accept,
            },
        })
    }
}

fn prompt(kind: ConsentKind, x: f32) -> (u16, Vec<u8>) {
    match kind {
        ConsentKind::Resurrection => {
            let mut body = vec![0; 228];
            body[4..6].copy_from_slice(&4u16.to_le_bytes());
            body[12..16].copy_from_slice(&x.to_le_bytes());
            body[24..30].copy_from_slice(b"Tester");
            body[92..98].copy_from_slice(b"Helper");
            body[156..160].copy_from_slice(&392u32.to_le_bytes());
            body[160..173].copy_from_slice(b"Tester's body");
            (RESURRECTION_OPCODE, body)
        }
        ConsentKind::Translocate => {
            let mut body = vec![0; 92];
            body[..4].copy_from_slice(&4u32.to_le_bytes());
            body[4..8].copy_from_slice(&2000u32.to_le_bytes());
            body[12..18].copy_from_slice(b"Helper");
            body[80..84].copy_from_slice(&x.to_le_bytes());
            (TRANSLOCATE_OPCODE, body)
        }
    }
}

fn issued(visit: &mut Visit, kind: ConsentKind, x: f32) -> (ConsentId, Instant) {
    let (opcode, body) = prompt(kind, x);
    let result = visit.packet(opcode, &body);
    assert!(result.result.unwrap().is_none());
    assert_eq!(result.sent, []);
    match result.events.as_slice() {
        [ClientEvent::World(WorldEvent::Resurrection(question))] => {
            (question.id, question.expires_at)
        }
        [ClientEvent::World(WorldEvent::TranslocateOffer(question))] => {
            (question.id, question.expires_at)
        }
        _ => panic!("expected one consent prompt"),
    }
}

fn assert_replay_withheld(visit: &mut Visit, kind: ConsentKind, x: f32) {
    let (opcode, body) = prompt(kind, x);
    let replay = visit.packet(opcode, &body);
    assert!(replay.result.unwrap().is_none());
    assert!(replay.sent.is_empty() && replay.events.is_empty());
}

#[test]
fn p99_retired_consent_stays_withheld_after_replacement_answer_and_expiry() {
    for kind in [ConsentKind::Resurrection, ConsentKind::Translocate] {
        let mut visit = Visit::new();
        let (a, _) = issued(&mut visit, kind, 10.0);
        let (b, _) = issued(&mut visit, kind, 20.0);
        assert_ne!(a, b);
        assert_replay_withheld(&mut visit, kind, 10.0);
        assert_eq!(visit.answer(a, true).sent, []);
        assert_eq!(visit.answer(b, false).sent.len(), 1);
        assert_replay_withheld(&mut visit, kind, 20.0);

        let (c, expiry) = issued(&mut visit, kind, 30.0);
        let expired = testing::run(|out| visit.features.tick(expiry, &mut visit.world, out));
        expired.result.unwrap();
        assert_eq!(expired.sent.len(), 1);
        assert!(expired.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::ConsentClosed { id, closure: ConsentClosure::Expired }) if *id == c
        )));
        assert_replay_withheld(&mut visit, kind, 30.0);
        let (d, _) = issued(&mut visit, kind, 40.0);
        assert_eq!(visit.answer(d, false).sent.len(), 1);
    }
}

#[test]
fn p99_interrupted_consent_stays_retired_when_the_same_zone_resumes() {
    for kind in [ConsentKind::Resurrection, ConsentKind::Translocate] {
        let mut visit = Visit::new();
        let (a, _) = issued(&mut visit, kind, 10.0);
        let transfer = visit.news(vec![Message::ZoneOffer(ZoneOffer {
            zone_id: 8,
            instance_id: 0,
            position: Position::default(),
            reason: 0,
            to_bind: false,
            solicited: true,
        })]);
        assert!(transfer.result.unwrap().is_none());
        assert!(transfer.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::ConsentClosed { id, closure: ConsentClosure::Interrupted }) if *id == a
        )));
        let refused = visit.news(vec![Message::ZoneAnswer(ZoneAnswer {
            character: "Tester".into(),
            zone_id: 8,
            instance_id: 0,
            position: None,
            success: -2,
        })]);
        assert!(refused.result.unwrap().is_none());
        assert!(!visit.world.lifecycle.blocks_motion());
        assert_replay_withheld(&mut visit, kind, 10.0);
        assert_eq!(visit.answer(a, true).sent, []);
        let (next, _) = issued(&mut visit, kind, 20.0);
        assert_eq!(visit.answer(next, false).sent.len(), 1);
    }
}

#[test]
fn p99_completed_translocate_cannot_reopen_or_move_through_an_old_echo() {
    let mut visit = Visit::new();
    let kind = ConsentKind::Translocate;
    let complete = |visit: &mut Visit, x| {
        let (opcode, mut body) = prompt(kind, x);
        body[88..].copy_from_slice(&1u32.to_le_bytes());
        let result = visit.packet(opcode, &body);
        assert!(result.result.unwrap().is_none());
        assert_eq!(result.sent, []);
        result.events.into_iter().any(|event| {
            matches!(
                event,
                ClientEvent::World(WorldEvent::SameZoneRelocated(arrival))
                    if arrival.session_id == 5 && arrival.spawn_id == 7
            )
        })
    };
    let (first, _) = issued(&mut visit, kind, 10.0);
    assert_eq!(visit.answer(first, true).sent.len(), 1);
    assert!(complete(&mut visit, 10.0));
    assert_eq!(
        visit.world.player.as_ref().unwrap().position.x.to_bits(),
        10.0f32.to_bits()
    );
    assert_replay_withheld(&mut visit, kind, 10.0);
    assert_eq!(visit.answer(first, true).sent, []);
    assert!(!complete(&mut visit, 10.0));

    let (next, _) = issued(&mut visit, kind, 20.0);
    assert_eq!(visit.answer(next, true).sent.len(), 1);
    assert!(!complete(&mut visit, 10.0));
    assert!(complete(&mut visit, 20.0));
    assert_eq!(
        visit.world.player.as_ref().unwrap().position.x.to_bits(),
        20.0f32.to_bits()
    );
}

#[test]
fn p99_skill_update_remains_authoritative_without_releasing_practice() {
    let mut visit = Visit::new();
    let mut trainer = testing::spawn(42, SpawnKind::Npc);
    trainer.class = Some(20);
    visit.world.spawns.insert(trainer);
    let request = |request| ClientCommand::Training {
        session_id: 5,
        request,
        created: Instant::now(),
    };
    let sent = visit.command(request(TrainingRequest::Open { trainer: 42 }));
    assert_eq!(sent.sent, [training::titanium_open(42, 7)]);
    let mut reply = sent.sent[0].body.clone();
    reply[412] = 1;
    reply[8 + 30 * 4] = 200;
    let received = visit.packet(training::OFFER_OPCODE, &reply);
    assert!(received.result.unwrap().is_none());
    assert!(received.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::Training(TrainingUpdate::Offered(_)))
    )));

    let train = TrainingRequest::TrainAt {
        trainer: 42,
        skill: 30,
    };
    assert_eq!(
        visit.command(request(train)).sent,
        [training::titanium_train(42, 30).unwrap()]
    );
    let body = [30u32.to_le_bytes(), 13u32.to_le_bytes()].concat();
    let received = visit.packet(0x6a93, &body);
    assert!(received.result.unwrap().is_none());
    assert!(received.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::Skill {
            skill_id: 30,
            value: 13
        })
    )));
    let player = visit.world.player.as_ref().unwrap();
    assert_eq!(player.skills.as_ref().unwrap()[30], 13);
    assert_eq!(player.practice_points, Some(3));
    assert_eq!(visit.command(request(train)).sent, []);
}
