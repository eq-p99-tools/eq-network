//! Constructed stun packets through the selected policy and receive path.
use super::super::feature::testing;
use super::*;
use crate::client::ServerProtocol;
use eq_network_game::{control::StunState, food::AutoEat, world::WorldEvent};

fn setup(protocol: ServerProtocol) -> (Features, World) {
    let mut features = Features::new(
        servers::server_type(protocol),
        "Synthetic",
        AutoEat::default(),
    );
    let mut world = World::new(5);
    world.player.admit(testing::player(7));
    world.own_spawn = Some(7);
    world.admitted = Some(Instant::now());
    testing::run_on(servers::server_type(protocol).wire(), |out| {
        features.admitted(&mut world, out)
    })
    .result
    .unwrap();
    (features, world)
}

fn receive(
    protocol: ServerProtocol,
    features: &mut Features,
    world: &mut World,
    opcode: u16,
    body: &[u8],
) -> testing::Outcome<Result<Option<ZoneExit>>> {
    let server = servers::server_type(protocol);
    testing::run_on(server.wire(), |out| {
        hear(
            read_messages(server.wire(), opcode, body, server.item_rules()),
            features,
            world,
            out,
            true,
        )
    })
}

fn own_stuns(events: &[ClientEvent]) -> Vec<(u64, u64, StunState)> {
    events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::World(WorldEvent::OwnStun {
                session_id,
                revision,
                state,
            }) => Some((*session_id, *revision, *state)),
            _ => None,
        })
        .collect()
}

#[test]
fn titanium_wire_stun_reaches_eqemu_and_p99_control_and_expires() {
    for protocol in [ServerProtocol::EqEmu, ServerProtocol::Project1999] {
        let (mut features, mut world) = setup(protocol);
        let output = receive(
            protocol,
            &mut features,
            &mut world,
            0x1e51,
            &[0x40, 0x1f, 0, 0],
        );
        assert!(output.result.unwrap().is_none());
        assert!(world.control.stunned(), "{protocol:?}");
        assert_eq!(own_stuns(&output.events), [(5, 1, StunState::Stunned)]);
        assert!(output.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Stun(Some(duration)))
                if *duration == Duration::from_secs(8)
        )));
        assert!(output.sent.is_empty() && output.unreliable.is_empty());

        let output = testing::run(|out| {
            features.tick(Instant::now() + Duration::from_secs(9), &mut world, out)
        });
        output.result.unwrap();
        assert!(!world.control.stunned());
        assert_eq!(own_stuns(&output.events), [(5, 2, StunState::Clear)]);

        let mut next = World::new(6);
        let mut features = Features::new(
            servers::server_type(protocol),
            "Synthetic",
            AutoEat::default(),
        );
        let output = testing::run(|out| features.admitted(&mut next, out));
        output.result.unwrap();
        assert!(!next.control.stunned());
        assert_eq!(own_stuns(&output.events), [(6, 0, StunState::Clear)]);
    }
}

#[test]
fn eqemu_zero_and_overlapping_wire_stuns_retain_titanium_semantics() {
    let protocol = ServerProtocol::EqEmu;
    let (mut features, mut world) = setup(protocol);
    let output = receive(
        protocol,
        &mut features,
        &mut world,
        0x1e51,
        &8000u32.to_le_bytes(),
    );
    output.result.unwrap();
    assert_eq!(own_stuns(&output.events), [(5, 1, StunState::Stunned)]);
    for milliseconds in [0u32, 1, 60000] {
        let output = receive(
            protocol,
            &mut features,
            &mut world,
            0x1e51,
            &milliseconds.to_le_bytes(),
        );
        output.result.unwrap();
        assert!(world.control.stunned());
        assert_eq!(own_stuns(&output.events), []);
        assert!(output.sent.is_empty() && output.unreliable.is_empty());
    }
    let output =
        testing::run(|out| features.tick(Instant::now() + Duration::from_secs(9), &mut world, out));
    output.result.unwrap();
    assert_eq!(own_stuns(&output.events), [(5, 2, StunState::Clear)]);

    let output = receive(protocol, &mut features, &mut world, 0x1e51, &[0; 4]);
    output.result.unwrap();
    assert!(world.control.stunned());
    assert_eq!(own_stuns(&output.events), [(5, 3, StunState::Stunned)]);
}

#[test]
fn malformed_eqemu_wire_stuns_never_establish_a_timer() {
    let protocol = ServerProtocol::EqEmu;
    let (mut features, mut world) = setup(protocol);
    for body in [&[][..], &[0; 3], &[0; 5]] {
        let output = receive(protocol, &mut features, &mut world, 0x1e51, body);
        assert!(output.result.unwrap().is_none());
        assert!(!world.control.stunned());
        assert_eq!(own_stuns(&output.events), []);
        assert!(output
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::Diagnostic(message)
                if message.contains("invalid stun length (opcode=0x1e51, payload_len="))));
        assert!(output.sent.is_empty() && output.unreliable.is_empty());
    }
}

#[test]
fn eqmac_wire_stuns_remain_raw_without_titanium_control_policy() {
    for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
        let (mut features, mut world) = setup(protocol);
        let output = receive(
            protocol,
            &mut features,
            &mut world,
            0x5b41,
            &8000u32.to_le_bytes(),
        );
        assert!(output.result.unwrap().is_none());
        assert!(!world.control.stunned());
        assert_eq!(own_stuns(&output.events), []);
        assert!(output.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Stun(Some(duration)))
                if *duration == Duration::from_secs(8)
        )));
        assert!(output.sent.is_empty() && output.unreliable.is_empty());
    }
}
