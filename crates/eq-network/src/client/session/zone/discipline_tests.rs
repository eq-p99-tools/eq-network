//! Admission-scoped discipline state and the cancellation slot revision.
use super::super::feature::testing;
use super::*;
use crate::client::ServerProtocol;
use eq_network_game::{
    buffs::{cancel::Request, Buff, BuffUpdate},
    disciplines::{Active, TITANIUM_BUFF_SLOT},
    food::AutoEat,
};

fn staged_discipline_input(faded: bool) -> Vec<Message> {
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
            replaces_slot: false,
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
    input
}

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

        let admission = testing::run(|out| -> Result<()> {
            let input = staged_discipline_input(faded);
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
        assert_eq!(admission.sent, []);
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

#[test]
fn an_exact_empty_discipline_record_releases_the_slot_but_preserves_received_reuse() {
    use crate::client::ClientConfig;
    use eq_network_game::disciplines::{self, Discipline};
    use eq_network_game::vision::SightCatalog;

    #[derive(Debug)]
    struct Catalog;
    impl SightCatalog for Catalog {
        fn sees_invisible(&self, spell_id: u32, _level: u16, landed: bool) -> bool {
            spell_id == 4499 && !landed
        }
    }

    let mut config = ClientConfig::for_protocol(
        ServerProtocol::EqEmu,
        "Example",
        "Example",
        "Test",
        "Tester",
    );
    config.sight_catalog = Some(std::sync::Arc::new(Catalog));
    let server = servers::server_type(config.protocol);
    let wire = server.wire();
    let mut features = Features::new(server, "Constructed", AutoEat::default());
    let mut world = World::for_zone(5, &config);
    let mut player = testing::player(7);
    player.class = Some(1);
    player.level = 55;
    player.disciplines = Some(vec![4499, 4503]);
    world.own_spawn = Some(7);
    world.player.admit(player);
    world.admitted = Some(Instant::now());
    let slot = |actor: u32, spell, effect, mode| {
        wire.messages(
            0x6a53,
            &[actor, effect, spell, 30, 0, 7, TITANIUM_BUFF_SLOT, mode]
                .map(u32::to_le_bytes)
                .concat(),
            server.item_rules(),
        )
    };
    let timer = |seconds| {
        wire.messages(
            disciplines::TIMER_OPCODE,
            &[1, seconds, 0].map(u32::to_le_bytes).concat(),
            server.item_rules(),
        )
    };
    let use_evasive = || ClientCommand::UseDiscipline {
        session_id: 5,
        discipline: Discipline::Evasive,
        created: Instant::now(),
    };
    let output = testing::run_on(wire, |out| -> Result<()> {
        for messages in [
            slot(7, 4499, 2, 0),
            slot(7, 4503, 2, 1),
            slot(7, u32::MAX, 2, 1),
            slot(8, u32::MAX, 0, 3),
        ] {
            hear(messages, &mut features, &mut world, out, true)?;
        }
        assert!(world.sight.sees_invisible());
        apply_commands(&[use_evasive()], &mut features, &mut world, out, true)?;
        hear(timer(30), &mut features, &mut world, out, true)?;
        hear(
            slot(7, u32::MAX, 0, 3),
            &mut features,
            &mut world,
            out,
            true,
        )?;
        assert!(!world.sight.sees_invisible());
        apply_commands(&[use_evasive()], &mut features, &mut world, out, true)?;
        hear(timer(0), &mut features, &mut world, out, true)?;
        apply_commands(&[use_evasive()], &mut features, &mut world, out, true)?;
        Ok(())
    });
    output.result.unwrap();
    assert_eq!(
        output.sent,
        [disciplines::titanium_cast(Discipline::Evasive, 7).unwrap()]
    );
    let reasons: Vec<_> = output
        .events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::World(WorldEvent::CastRejected { reason, .. }) => Some(reason.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        reasons,
        [
            "A discipline is already active",
            "The shared discipline timer is still running"
        ]
    );
}
