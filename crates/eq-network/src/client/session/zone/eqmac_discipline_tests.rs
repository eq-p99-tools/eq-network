//! TAKP's and Quarm's `/disc` and `/shield` through each server's own wire
//! and features (6.603 G1, G2).
use super::*;
use crate::client::{session::feature::testing, ServerProtocol};
use eq_network_game::{food::AutoEat, world::WorldEvent};

const MAC: [ServerProtocol; 2] = [ServerProtocol::Takp, ServerProtocol::Quarm];
const WARRIOR: u32 = 1;
const MONK: u32 = 7;

/// An admitted level-1 player (7) of `class`: too low for any discipline,
/// which the server, not the session, refuses.
fn setup(protocol: ServerProtocol, class: u32) -> (Features, World) {
    let mut features = Features::new(servers::server_type(protocol), "Tester", AutoEat::default());
    let mut world = World::new(5);
    let mut player = testing::player(7);
    player.class = Some(class);
    player.level = 1;
    world.player.admit(player);
    world.own_spawn = Some(7);
    world.admitted = Some(Instant::now());
    testing::run_on(servers::server_type(protocol).wire(), |out| {
        features.admitted(&mut world, out)
    })
    .result
    .unwrap();
    (features, world)
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

fn discipline(spell_id: u32) -> ClientCommand {
    ClientCommand::UseDiscipline {
        session_id: 5,
        spell_id,
        timer: 1,
        target_id: 0,
        timed_self: true,
        created: Instant::now(),
    }
}

fn reuse() -> ClientCommand {
    ClientCommand::AskDisciplineReuse {
        session_id: 5,
        created: Instant::now(),
    }
}

fn sent(outcome: &testing::Outcome<Result<bool>>) -> Vec<(u16, Vec<u8>)> {
    outcome
        .sent
        .iter()
        .map(|packet| (packet.opcode, packet.body.clone()))
        .collect()
}

fn cast_refused(outcome: &testing::Outcome<Result<bool>>, spell: u32) -> bool {
    outcome.events.iter().any(|event| {
        matches!(
            event,
            ClientEvent::World(WorldEvent::CastRejected { spell_id, .. }) if *spell_id == spell
        )
    })
}

#[test]
fn a_mac_discipline_goes_by_its_number_and_only_a_spell_without_one_is_refused() {
    for protocol in MAC {
        // Evasive is number 4; a level-1 Warrior still sends it, and the
        // server's own line answers the level.
        let (mut features, mut world) = setup(protocol, WARRIOR);
        let outcome = command(protocol, &mut features, &mut world, &discipline(4503));
        outcome.result.as_ref().unwrap();
        assert_eq!(sent(&outcome), [(0xe641, vec![4, 0, 0, 0])], "{protocol:?}");
        // Nothing waits for a cast: a spell may go at once.
        assert!(
            features.holds(&world, Instant::now()).is_empty(),
            "{protocol:?}"
        );
        // A second try within the second sends nothing.
        let outcome = command(protocol, &mut features, &mut world, &discipline(4503));
        outcome.result.as_ref().unwrap();
        assert_eq!(sent(&outcome), [], "{protocol:?}");
        assert!(cast_refused(&outcome, 4503), "{protocol:?}");

        // Number 6 is a Warrior's Furious and a Monk's Whirlwind.
        let (mut features, mut world) = setup(protocol, WARRIOR);
        let outcome = command(protocol, &mut features, &mut world, &discipline(4674));
        assert_eq!(sent(&outcome), [(0xe641, vec![6, 0, 0, 0])], "{protocol:?}");
        let (mut features, mut world) = setup(protocol, MONK);
        let outcome = command(protocol, &mut features, &mut world, &discipline(4509));
        assert_eq!(sent(&outcome), [(0xe641, vec![6, 0, 0, 0])], "{protocol:?}");

        // A Monk has no number for Evasive, and a spell no discipline casts
        // has none either: nothing is sent.
        for spell in [4503, 202] {
            let (mut features, mut world) = setup(protocol, MONK);
            let outcome = command(protocol, &mut features, &mut world, &discipline(spell));
            outcome.result.as_ref().unwrap();
            assert_eq!(sent(&outcome), [], "{protocol:?} {spell}");
            assert!(cast_refused(&outcome, spell), "{protocol:?} {spell}");
        }
    }
}

#[test]
fn a_bare_mac_disc_asks_the_server_and_its_ready_answer_is_a_line() {
    for protocol in MAC {
        let (mut features, mut world) = setup(protocol, MONK);
        let outcome = command(protocol, &mut features, &mut world, &reuse());
        outcome.result.as_ref().unwrap();
        assert_eq!(sent(&outcome), [(0xe641, vec![0; 4])], "{protocol:?}");
        // Asked again at once, it waits out the same second.
        let outcome = command(protocol, &mut features, &mut world, &reuse());
        outcome.result.as_ref().unwrap();
        assert_eq!(sent(&outcome), [], "{protocol:?}");
        assert!(
            outcome.events.iter().any(|event| matches!(
                event,
                ClientEvent::World(WorldEvent::SocialRefused { session_id: 5, .. })
            )),
            "{protocol:?}"
        );
        // Quarm's four bytes and TAKP's six both reach the host as string
        // 393 in the server's color, and neither ends a cast.
        let wire = servers::server_type(protocol).wire();
        for body in [&[0x89, 0x01, 15, 0][..], &[0x89, 0x01, 15, 0, 0, 0]] {
            let line = wire.chat(0x3542, body, false).unwrap().unwrap();
            assert_eq!(
                (line.string_id, line.message_type),
                (Some(393), Some(15)),
                "{protocol:?}"
            );
            assert!(
                wire.messages(0x3542, body, servers::server_type(protocol).item_rules())
                    .is_empty(),
                "{protocol:?}"
            );
        }
    }
}

#[test]
fn titanium_takes_no_bare_disc() {
    for protocol in [ServerProtocol::EqEmu, ServerProtocol::Project1999] {
        let mut features =
            Features::new(servers::server_type(protocol), "Tester", AutoEat::default());
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        let outcome = command(protocol, &mut features, &mut world, &reuse());
        assert_eq!(sent(&outcome), [], "{protocol:?}");
    }
}

#[test]
fn a_mac_shield_names_its_target_in_two_bytes() {
    for protocol in MAC {
        let (mut features, mut world) = setup(protocol, WARRIOR);
        let shield = |spawn_id| ClientCommand::Shield {
            session_id: 5,
            spawn_id,
            created: Instant::now(),
        };
        let outcome = command(protocol, &mut features, &mut world, &shield(0));
        assert_eq!(sent(&outcome), [], "{protocol:?}");
        let outcome = command(protocol, &mut features, &mut world, &shield(0x0102));
        outcome.result.as_ref().unwrap();
        assert_eq!(sent(&outcome), [(0x4b42, vec![0x02, 0x01])], "{protocol:?}");
        // Within the 500 ms pace, nothing more goes.
        let outcome = command(protocol, &mut features, &mut world, &shield(0x0102));
        assert_eq!(sent(&outcome), [], "{protocol:?}");
    }
}
