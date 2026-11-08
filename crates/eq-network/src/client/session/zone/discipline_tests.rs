//! Admission-scoped discipline state and the cancellation slot revision.
use super::super::feature::testing;
use super::*;
use crate::client::ServerProtocol;
use eq_network_game::{
    buffs::{cancel::Request, Buff, BuffUpdate},
    disciplines::{Active, TITANIUM_BUFF_SLOT},
    food::AutoEat,
};

#[test]
fn staged_discipline_presence_or_fade_keeps_cancellation_revision_aligned() {
    for faded in [false, true] {
        let mut features = Features::new(
            servers::server_type(ServerProtocol::EqEmu),
            "Constructed",
            AutoEat::default(),
        );
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let ordinary = Buff {
            spell_id: 42,
            caster_level: 12,
            effect_type: 2,
            bard_modifier: 10,
            duration_ticks: 30,
            counters: 0,
            caster_id: 7,
        };
        let discipline = Buff {
            spell_id: 4499,
            ..ordinary.clone()
        };
        let slot = |buff| {
            Message::Event(WorldEvent::Buff(BuffUpdate {
                entity_id: 7,
                slot: TITANIUM_BUFF_SLOT,
                spell_id: 4499,
                buff,
            }))
        };
        let mut input = vec![
            Message::Event(WorldEvent::BuffSnapshot(vec![None, Some(ordinary)])),
            slot(Some(discipline)),
        ];
        if faded {
            input.push(slot(None));
        }

        let admission = testing::run(|out| -> Result<()> {
            assert!(hear(input, &mut features, &mut world, out, true)?.is_none());
            let zone = Zone {
                name: "constructed".into(),
                far_clip: None,
                sky: None,
                description: None,
            };
            admit(testing::player(7), &zone, &mut features, &mut world, out)
        });
        admission.result.unwrap();
        assert!(admission.sent.is_empty());
        let expected = if faded {
            Active::Inactive
        } else {
            Active::Titanium(4499)
        };
        let entered = admission
            .events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    ClientEvent::World(WorldEvent::Entered { session_id: 5, .. })
                )
            })
            .unwrap();
        let staged = admission
            .events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    ClientEvent::World(WorldEvent::DisciplineAdmission { session_id: 5, state })
                        if *state == expected
                )
            })
            .unwrap();
        assert!(entered < staged);

        // Every own-slot event the host sees advances the cancellation
        // revision. The staged discipline fact must not add one.
        let revision = admission
            .events
            .iter()
            .fold(0, |revision, event| match event {
                ClientEvent::World(WorldEvent::BuffSnapshot(_)) => revision + 1,
                ClientEvent::World(WorldEvent::Buff(update)) if update.entity_id == 7 => {
                    revision + 1
                }
                _ => revision,
            });
        assert_eq!(revision, 1);
        let command = ClientCommand::CancelBuff(Request {
            session_id: 5,
            revision,
            slot: 1,
            spell_id: 42,
            beneficial: Some(true),
            cancellable: Some(true),
            created: Instant::now(),
        });
        let cancelled = testing::run(|out| features.handle(&command, &mut world, out));
        assert!(cancelled.result.unwrap());
        let [packet] = cancelled.sent.as_slice() else {
            panic!("expected one EQEmu cancellation record");
        };
        assert_eq!(packet.opcode, 0x6a53);
        assert_eq!(packet.body.len(), 32);
        assert_eq!(&packet.body[8..12], &42u32.to_le_bytes());
        assert_eq!(&packet.body[24..28], &1u32.to_le_bytes());
    }
}
