//! Constructed cancellation through actual selected policy, admission and gate.
use super::super::feature::testing;
use super::*;
use crate::client::ServerProtocol;
use eq_network_game::{
    buffs::{
        cancel::{Progress, Request},
        Buff,
    },
    food::AutoEat,
    world::WorldEvent,
};

#[test]
fn cancellation_policy_gate_keeps_admission_age_and_lifecycle_boundaries() {
    for (protocol, sends) in [(ServerProtocol::EqEmu, 1), (ServerProtocol::Project1999, 2)] {
        let mut features = Features::new(
            servers::server_type(protocol),
            "Constructed",
            AutoEat::default(),
        );
        let mut world = World::new(5);
        let buff = Buff {
            spell_id: 42,
            caster_level: 12,
            effect_type: 2,
            bard_modifier: 10,
            duration_ticks: 30,
            counters: 0,
            caster_id: 7,
        };
        features
            .admit(
                &Message::Event(WorldEvent::BuffSnapshot(vec![None, Some(buff)])),
                &mut world,
            )
            .unwrap();
        world.player.admit(testing::player(7));
        world.own_spawn = Some(7);
        let now = Instant::now();
        world.admitted = Some(now);
        testing::run(|out| features.admitted(&mut world, out))
            .result
            .unwrap();
        let make = |session_id, created| {
            ClientCommand::CancelBuff(Request {
                session_id,
                revision: 1,
                slot: 1,
                spell_id: 42,
                beneficial: Some(true),
                cancellable: Some(true),
                created,
            })
        };
        assert!(features
            .capabilities()
            .contains(&crate::world::Capability::CancellingBuffs));
        assert!(features.refusal(&make(6, now), &world, now).is_some());
        assert!(features
            .refusal(&make(5, now), &world, now + Duration::from_secs(1))
            .is_some());
        assert!(features
            .refusal(&make(5, now + Duration::from_millis(1)), &world, now)
            .is_some());
        assert!(features.refusal(&make(5, now), &world, now).is_none());
        let outcome = testing::run(|out| features.handle(&make(5, now), &mut world, out));
        assert!(outcome.result.unwrap());
        assert_eq!(outcome.sent.len(), sends);
        if protocol == ServerProtocol::Project1999 {
            assert_eq!(
                outcome.sent,
                eq_network_game::buffs::cancel::project1999_packets(7, 1, 42).unwrap(),
            );
        }
        assert!(outcome.events.iter().any(|event| matches!(event,
            ClientEvent::World(WorldEvent::BuffCancellation(update)) if update.progress == Progress::Submitted)));
        world.died();
        assert!(features.refusal(&make(5, now), &world, now).is_some());
    }
    for protocol in [ServerProtocol::Quarm, ServerProtocol::Takp] {
        let features = Features::new(
            servers::server_type(protocol),
            "Constructed",
            AutoEat::default(),
        );
        assert!(!features
            .capabilities()
            .contains(&crate::world::Capability::CancellingBuffs));
    }
}
