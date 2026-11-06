//! Target packets, production feature ordering and target-bound requests.
use super::super::feature::testing;
use super::*;
use crate::client::{ClientConfig, ServerProtocol};
use eq_network_game::{
    abilities::Ability,
    food::AutoEat,
    request::{Request, Sender},
    targets::TargetState,
    world::{SpawnKind, WorldEvent},
};

const SERVERS: [ServerProtocol; 4] = [
    ServerProtocol::Project1999,
    ServerProtocol::EqEmu,
    ServerProtocol::Takp,
    ServerProtocol::Quarm,
];

fn setup(protocol: ServerProtocol) -> (Features, World) {
    setup_world(protocol, World::new(5))
}

fn setup_world(protocol: ServerProtocol, mut world: World) -> (Features, World) {
    let server = servers::server_type(protocol);
    let mut features = Features::new(server, "Tester", AutoEat::default());
    world.own_spawn = Some(7);
    let mut player = testing::player(7);
    player.class = Some(1);
    player.skills = Some(vec![100; 100]);
    world.spawns.insert(testing::spawn(42, SpawnKind::Npc));
    world.spawns.insert(testing::spawn(43, SpawnKind::Npc));
    testing::run_on(server.wire(), |out| {
        admit(
            player,
            &Zone {
                name: "SyntheticZone".into(),
                far_clip: None,
                sky: None,
                description: None,
            },
            &mut features,
            &mut world,
            out,
        )
    })
    .result
    .unwrap();
    (features, world)
}

fn fixture_world(protocol: ServerProtocol, local_only: bool) -> World {
    let mut config = ClientConfig::for_protocol(protocol, "Example", "Example", "Test", "Tester");
    config.local_only = local_only;
    World::for_zone(5, &config)
}

fn drain_depop(
    protocol: ServerProtocol,
    features: &mut Features,
    world: &mut World,
    command: ClientCommand,
) -> testing::Outcome<Result<Option<ZoneExit>>> {
    testing::run_on(servers::server_type(protocol).wire(), |out| {
        apply_commands(&[command], features, world, out, true)
    })
}

#[test]
fn queued_local_depop_refuses_target_changes_and_reused_spawn_ids_at_drain() {
    use eq_network_game::chat::OutboundChat;

    let protocol = ServerProtocol::EqEmu;
    let server = servers::server_type(protocol);
    let (mut features, mut world) = setup_world(protocol, fixture_world(protocol, true));
    receive_target(protocol, &mut features, &mut world, Some(42))
        .result
        .unwrap();
    let original = world.target.state();
    let queued = ClientCommand::DepopLocalNpc {
        session_id: world.session_id,
        target: original,
        created: Instant::now(),
    };

    receive_target(protocol, &mut features, &mut world, Some(43))
        .result
        .unwrap();
    let refused = |output: testing::Outcome<Result<Option<ZoneExit>>>| {
        assert!(output.result.unwrap().is_none());
        assert!(output.sent.is_empty() && output.unreliable.is_empty());
        assert!(output.events.iter().any(|event| matches!(event,
            ClientEvent::Diagnostic(reason) if reason.contains("fixture NPC target has changed"))));
    };
    refused(drain_depop(
        protocol,
        &mut features,
        &mut world,
        queued.clone(),
    ));

    hear_events(
        protocol,
        &mut features,
        &mut world,
        vec![
            Message::Event(WorldEvent::Despawn(42)),
            Message::Event(WorldEvent::Spawns(vec![testing::spawn(42, SpawnKind::Npc)])),
        ],
    )
    .result
    .unwrap();
    receive_target(protocol, &mut features, &mut world, Some(42))
        .result
        .unwrap();
    let replacement = world.target.state();
    assert_eq!(replacement.spawn_id, original.spawn_id);
    assert!(replacement.available && replacement.revision > original.revision);
    refused(drain_depop(protocol, &mut features, &mut world, queued));

    let current = ClientCommand::DepopLocalNpc {
        session_id: world.session_id,
        target: replacement,
        created: Instant::now(),
    };
    let output = drain_depop(protocol, &mut features, &mut world, current);
    assert!(output.result.unwrap().is_none());
    let expected = server
        .wire()
        .encode(
            &Request::Command(ClientCommand::SendChat(OutboundChat::Say("#depop".into()))),
            Sender {
                name: "Tester",
                spawn_id: Some(7),
            },
        )
        .unwrap();
    assert_eq!(output.sent, [expected]);
    assert!(output.events.is_empty());
    assert!(
        world.spawns.visible(42).is_some(),
        "submission cannot remove the NPC"
    );
}

#[test]
fn depop_requires_a_local_eqemu_connection_and_the_current_admission() {
    for (protocol, local_only, session_id, expected) in [
        (ServerProtocol::EqEmu, false, 5, "requires local EQEmu"),
        (ServerProtocol::Project1999, true, 5, "requires local EQEmu"),
        (ServerProtocol::Takp, true, 5, "requires local EQEmu"),
        (ServerProtocol::EqEmu, true, 4, "earlier zone visit"),
    ] {
        let (mut features, mut world) = setup_world(protocol, fixture_world(protocol, local_only));
        receive_target(protocol, &mut features, &mut world, Some(42))
            .result
            .unwrap();
        let queued = ClientCommand::DepopLocalNpc {
            session_id,
            target: world.target.state(),
            created: Instant::now(),
        };
        let output = drain_depop(protocol, &mut features, &mut world, queued);
        assert!(output.result.unwrap().is_none());
        assert!(output.sent.is_empty() && output.unreliable.is_empty());
        assert!(output.events.iter().any(|event| matches!(event,
            ClientEvent::Diagnostic(reason) if reason.contains(expected))));
    }
}

fn hear_events(
    protocol: ServerProtocol,
    features: &mut Features,
    world: &mut World,
    messages: Vec<Message>,
) -> testing::Outcome<Result<Option<ZoneExit>>> {
    testing::run_on(servers::server_type(protocol).wire(), |out| {
        hear(messages, features, world, out, true)
    })
}

fn receive_target(
    protocol: ServerProtocol,
    features: &mut Features,
    world: &mut World,
    id: Option<u16>,
) -> testing::Outcome<Result<Option<ZoneExit>>> {
    let server = servers::server_type(protocol);
    let id = id.unwrap_or(0);
    let (opcode, body) = if protocol.is_titanium() {
        (
            eq_network_game::targets::TARGET_COMMAND_OPCODE,
            u32::from(id).to_le_bytes().to_vec(),
        )
    } else {
        (
            eq_network_game::targets::EQMAC_TARGET_COMMAND_OPCODE,
            id.to_le_bytes().to_vec(),
        )
    };
    hear_events(
        protocol,
        features,
        world,
        server.wire().messages(opcode, &body, server.item_rules()),
    )
}

fn ability(
    protocol: ServerProtocol,
    features: &mut Features,
    world: &mut World,
    session_id: u64,
    target: Option<TargetState>,
) -> testing::Outcome<Result<bool>> {
    testing::run_on(servers::server_type(protocol).wire(), |out| {
        features.handle(
            &ClientCommand::UseAbility {
                session_id,
                target,
                ability: Ability::Taunt,
                created: Instant::now(),
            },
            world,
            out,
        )
    })
}

fn refused(output: testing::Outcome<Result<bool>>) {
    assert!(output.result.unwrap());
    assert!(
        output.sent.is_empty(),
        "unexpected sends: {:?}",
        output.sent
    );
    assert!(
        output.unreliable.is_empty(),
        "unexpected unreliable sends: {:?}",
        output.unreliable
    );
    assert!(output
        .events
        .iter()
        .any(|event| matches!(event, ClientEvent::World(WorldEvent::AbilityRefused { .. }))));
    assert!(!output.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::AbilityUsed { .. } | WorldEvent::Posture { .. })
    )));
}

#[test]
fn server_changes_clear_and_same_id_return_refuse_old_ability_intent_without_echo() {
    for protocol in SERVERS {
        let (mut features, mut world) = setup(protocol);
        for id in [Some(42), Some(43), Some(42), None] {
            let old = world.target.state();
            let output = receive_target(protocol, &mut features, &mut world, id);
            output.result.unwrap();
            assert!(
                output.sent.is_empty(),
                "target receive echoed: {:?}",
                output.sent
            );
            assert!(
                output.unreliable.is_empty(),
                "unexpected unreliable sends: {:?}",
                output.unreliable
            );
            assert!(output.events.iter().any(|event| matches!(event,
                ClientEvent::World(WorldEvent::TargetChanged { session_id: 5, request_id: 0, state })
                if state.spawn_id == id && state.revision > old.revision)));
            refused(ability(protocol, &mut features, &mut world, 5, Some(old)));
        }
        receive_target(protocol, &mut features, &mut world, Some(43))
            .result
            .unwrap();
        let current = world.target.state();
        refused(ability(protocol, &mut features, &mut world, 5, None));
        let used = ability(protocol, &mut features, &mut world, 5, Some(current));
        assert!(used.result.unwrap());
        let expected = servers::server_type(protocol)
            .wire()
            .encode(
                &Request::Ability {
                    ability: Ability::Taunt,
                    target: Some(43),
                },
                Sender {
                    name: "Tester",
                    spawn_id: Some(7),
                },
            )
            .unwrap();
        assert_eq!(
            used.sent,
            [expected],
            "fresh displayed identity selects its exact recipient"
        );
    }
}

#[test]
fn unknown_target_arrival_is_ordered_but_a_replacement_cannot_reuse_its_identity() {
    for protocol in SERVERS {
        let (mut features, mut world) = setup(protocol);
        receive_target(protocol, &mut features, &mut world, Some(44))
            .result
            .unwrap();
        let unresolved = world.target.state();
        assert_eq!(unresolved.spawn_id, Some(44));
        assert!(!unresolved.available);
        refused(ability(
            protocol,
            &mut features,
            &mut world,
            5,
            Some(unresolved),
        ));
        let appears = || {
            vec![Message::Event(WorldEvent::Spawns(vec![testing::spawn(
                44,
                SpawnKind::Npc,
            )]))]
        };
        let output = hear_events(protocol, &mut features, &mut world, appears());
        output.result.unwrap();
        assert!(
            output.sent.is_empty(),
            "unexpected sends: {:?}",
            output.sent
        );
        let spawn_at = output
            .events
            .iter()
            .position(|event| matches!(event, ClientEvent::World(WorldEvent::Spawns(_))))
            .unwrap();
        let target_at = output
            .events
            .iter()
            .position(|event| {
                matches!(event,
            ClientEvent::World(WorldEvent::TargetChanged { state, .. }) if state.available)
            })
            .unwrap();
        assert!(
            spawn_at < target_at,
            "host must bind the arriving incarnation first"
        );
        let resolved = world.target.state();
        assert!(resolved.available);
        hear_events(protocol, &mut features, &mut world, appears())
            .result
            .unwrap();
        assert!(!world.target.state().available);
        refused(ability(
            protocol,
            &mut features,
            &mut world,
            5,
            Some(resolved),
        ));
        hear_events(protocol, &mut features, &mut world, appears())
            .result
            .unwrap();
        assert!(
            !world.target.state().available,
            "replacement never silently rebinds"
        );
        receive_target(protocol, &mut features, &mut world, Some(44))
            .result
            .unwrap();
        let fresh = world.target.state();
        assert!(fresh.available && fresh.revision > resolved.revision);
        let used = ability(protocol, &mut features, &mut world, 5, Some(fresh));
        assert!(used.result.unwrap());
        assert_eq!(used.sent.len(), 1);
    }
}

#[test]
fn local_refusal_keeps_authority_and_old_selection_sequences_cannot_replay() {
    let protocol = ServerProtocol::Project1999;
    let (mut features, mut world) = setup(protocol);
    let select = |request_id, spawn_id| ClientCommand::SelectTarget {
        session_id: 5,
        request_id,
        spawn_id,
    };
    let sent = testing::run(|out| features.handle(&select(1, Some(42)), &mut world, out));
    assert!(sent.result.unwrap());
    assert_eq!(sent.sent.len(), 1);
    let accepted = world.target.state();
    let rejected = testing::run(|out| features.handle(&select(2, Some(99)), &mut world, out));
    assert!(rejected.result.unwrap());
    assert!(
        rejected.sent.is_empty(),
        "unexpected sends: {:?}",
        rejected.sent
    );
    assert!(rejected.events.iter().any(|event| matches!(event,
        ClientEvent::World(WorldEvent::TargetRejected { request_id: 2, state: Some(state), .. }) if *state == accepted)));
    assert_eq!(world.target.state(), accepted);
    let replay = testing::run(|out| features.handle(&select(1, Some(43)), &mut world, out));
    assert!(replay.result.unwrap());
    assert!(
        replay.sent.is_empty(),
        "unexpected sends: {:?}",
        replay.sent
    );
    assert_eq!(world.target.state(), accepted);
    receive_target(protocol, &mut features, &mut world, None)
        .result
        .unwrap();
    refused(ability(
        protocol,
        &mut features,
        &mut world,
        5,
        Some(accepted),
    ));
}

#[test]
fn target_before_admission_is_staged_and_an_old_admission_cannot_send_an_ability() {
    let protocol = ServerProtocol::Project1999;
    let (mut features, mut world) = setup(protocol);
    world.admitted = None;
    let staged = receive_target(protocol, &mut features, &mut world, Some(42));
    staged.result.unwrap();
    assert!(
        staged.sent.is_empty(),
        "unexpected sends: {:?}",
        staged.sent
    );
    assert!(!staged
        .events
        .iter()
        .any(|event| matches!(event, ClientEvent::World(_))));
    let announced = testing::run(|out| features.admitted(&mut world, out));
    announced.result.unwrap();
    assert!(announced.events.iter().any(|event| matches!(event,
        ClientEvent::World(WorldEvent::TargetChanged { session_id: 5, state, .. })
        if state.spawn_id == Some(42))));
    let old = world.target.state();
    world.session_id = 6;
    refused(ability(protocol, &mut features, &mut world, 5, Some(old)));
}
