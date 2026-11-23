//! TAKP's and Quarm's book edits through each server's own wire and
//! features (6.604 E4). Both servers delete the spell in their database and
//! echo the 8 bytes, but keep the live profile as it was until the next load;
//! the local book changes only on an echo that says the delete happened.
use super::*;
use crate::client::{session::feature::testing, ServerProtocol};
use eq_network_game::{
    food::AutoEat,
    spells::{BookActionStatus, SpellBook},
    world::WorldEvent,
};

const MAC: [ServerProtocol; 2] = [ServerProtocol::Takp, ServerProtocol::Quarm];

/// An admitted player (7) whose book holds spell 202 in slot 0.
fn setup(protocol: ServerProtocol) -> (Features, World) {
    let server = servers::server_type(protocol);
    let mut features = Features::new(server, "Tester", AutoEat::default());
    let mut world = World::new(5);
    let mut player = testing::player(7);
    player.class = Some(2);
    world.player.admit(player);
    world.own_spawn = Some(7);
    world.admitted = Some(Instant::now());
    testing::run_on(server.wire(), |out| features.admitted(&mut world, out))
        .result
        .unwrap();
    let mut profile = vec![0; 8460];
    profile[1846..1848].copy_from_slice(&202i16.to_le_bytes());
    features
        .admit(
            &Message::Event(WorldEvent::SpellBook(
                SpellBook::eqmac_profile(&profile).unwrap(),
            )),
            &mut world,
        )
        .unwrap();
    (features, world)
}

fn delete() -> ClientCommand {
    ClientCommand::DeleteSpell {
        session_id: 5,
        slot: 0,
        spell_id: 202,
        created: Instant::now(),
    }
}

fn command(
    protocol: ServerProtocol,
    features: &mut Features,
    world: &mut World,
    command: &ClientCommand,
) -> testing::Outcome<Result<bool>> {
    testing::run_on(servers::server_type(protocol).wire(), |out| {
        features.handle(command, world, out)
    })
}

/// The server's echo, and the book statuses it produced.
fn echo(
    protocol: ServerProtocol,
    features: &mut Features,
    world: &mut World,
    opcode: u16,
    body: &[u8],
) -> Vec<BookActionStatus> {
    let server = servers::server_type(protocol);
    let wire = server.wire();
    let outcome = testing::run_on(wire, |out| {
        hear(
            wire.messages(opcode, body, server.item_rules()),
            features,
            world,
            out,
            true,
        )
    });
    assert!(outcome.result.unwrap().is_none());
    assert_eq!(outcome.sent.len(), 0);
    outcome
        .events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::World(WorldEvent::BookAction(status)) => Some(status.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_mac_delete_is_eight_bytes_and_only_an_echo_of_success_empties_the_slot() {
    for protocol in MAC {
        let (mut features, mut world) = setup(protocol);
        let outcome = command(protocol, &mut features, &mut world, &delete());
        assert!(outcome.result.as_ref().unwrap(), "{protocol:?}");
        assert_eq!(outcome.sent.len(), 1, "{protocol:?}");
        assert_eq!(outcome.sent[0].opcode, 0x4a42, "{protocol:?}");
        assert_eq!(outcome.sent[0].body, [0; 8], "{protocol:?}");
        // An echo of `success` 0 (the server found the slot empty) leaves
        // the book as it was: the same delete is taken again.
        let statuses = echo(protocol, &mut features, &mut world, 0x4a42, &[0; 8]);
        assert!(
            matches!(statuses.last(), Some(BookActionStatus::Rejected(_))),
            "{protocol:?}: {statuses:?}"
        );
        let outcome = command(protocol, &mut features, &mut world, &delete());
        assert_eq!(outcome.sent.len(), 1, "{protocol:?}");
        // An echo of `success` 1 empties the slot, so the same delete is
        // refused before anything is sent.
        let statuses = echo(
            protocol,
            &mut features,
            &mut world,
            0x4a42,
            &[0, 0, 0, 0, 1, 0, 0, 0],
        );
        assert_eq!(
            statuses.last(),
            Some(&BookActionStatus::Confirmed),
            "{protocol:?}"
        );
        let outcome = command(protocol, &mut features, &mut world, &delete());
        assert!(outcome.sent.is_empty(), "{protocol:?}");
    }
}

#[test]
fn a_mac_swap_is_two_slots_and_waits_for_its_echo() {
    for protocol in MAC {
        let (mut features, mut world) = setup(protocol);
        let swap = ClientCommand::SwapSpell {
            session_id: 5,
            from: 0,
            to: 1,
            from_spell: 202,
            to_spell: None,
            created: Instant::now(),
        };
        let outcome = command(protocol, &mut features, &mut world, &swap);
        assert!(outcome.result.as_ref().unwrap(), "{protocol:?}");
        assert_eq!(outcome.sent.len(), 1, "{protocol:?}");
        assert_eq!(outcome.sent[0].opcode, 0xce41, "{protocol:?}");
        assert_eq!(
            outcome.sent[0].body,
            [0, 0, 0, 0, 1, 0, 0, 0],
            "{protocol:?}"
        );
        let statuses = echo(
            protocol,
            &mut features,
            &mut world,
            0xce41,
            &[0, 0, 0, 0, 1, 0, 0, 0],
        );
        assert_eq!(
            statuses.last(),
            Some(&BookActionStatus::Confirmed),
            "{protocol:?}"
        );
    }
}
