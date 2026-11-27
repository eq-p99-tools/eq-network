use super::*;
use crate::client::{session::camp::transport_tests::logout_peer, ClientConfig, ServerProtocol};
use eq_network_game::{command::Posture, request::Sender, world::CampStatus};

#[test]
fn only_a_p99_peer_close_after_submitted_logout_finishes_the_local_camp() {
    for protocol in [ServerProtocol::Project1999, ServerProtocol::EqEmu] {
        // Idle, preparing, abandoned, and committed departure.
        for phase in 0..4 {
            let (mut connection, worker) = logout_peer(false);
            // Complete the synthetic peer's expected requests independently
            // of the local feature state being tested.
            for packet in [
                eq_network_game::command::titanium_camp(),
                eq_network_game::command::titanium_logout(),
            ] {
                connection.send(packet.opcode, &packet.body).unwrap();
            }
            let error = connection.receive().unwrap_err();
            let server = servers::server_type(protocol);
            let (mut features, mut world) = camp_phase(protocol, phase);
            let config = ClientConfig::new("EXAMPLE", "EXAMPLE", "Test Server", "Tester");
            let mut events = Vec::new();
            let mut handler = |event| {
                events.push(event);
                Ok(())
            };
            let mut log = Events::new(&config, &mut handler);
            {
                let mut out = Out {
                    sink: &mut connection,
                    log: &mut log,
                    wire: server.wire(),
                    sender: Sender {
                        name: "Tester",
                        spawn_id: Some(7),
                    },
                };
                // An unrelated failure cannot borrow the transport's terminal fact.
                for unrelated in [
                    anyhow::anyhow!("handler failed"),
                    std::io::Error::from(std::io::ErrorKind::TimedOut).into(),
                    eq_network_transport::modern::SessionTermination::OutOfSession.into(),
                    eq_network_transport::modern::SessionTermination::PeerDisconnected {
                        reason: Some(7),
                    }
                    .into(),
                ] {
                    features
                        .receive_failed(&unrelated, protocol, &mut world, &mut out)
                        .unwrap();
                    assert!(world.exit().is_none());
                }
                features
                    .receive_failed(&error, protocol, &mut world, &mut out)
                    .unwrap();
            }
            drop(log);
            let completed = protocol == ServerProtocol::Project1999 && phase == 3;
            assert_eq!(
                matches!(world.take_exit(), Some(ZoneExit::CharacterSelect)),
                completed
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        ClientEvent::World(WorldEvent::Camp(CampStatus::ServerClosedWithoutReply))
                    ))
                    .count(),
                usize::from(completed)
            );
            assert!(!events.iter().any(|event| matches!(
                event,
                ClientEvent::World(WorldEvent::Camp(CampStatus::Camped))
            )));
            worker.join().unwrap();
        }
    }
}

fn camp_phase(protocol: ServerProtocol, phase: u8) -> (Features, World) {
    use super::super::feature::testing;
    let server = servers::server_type(protocol);
    let mut features = Features::new(server, "Tester", eq_network_game::food::AutoEat::default());
    let mut world = World::new(5);
    world.player.admit(testing::player(7));
    world.own_spawn = Some(7);
    let now = Instant::now();
    let output = testing::run(|out| -> Result<()> {
        if phase > 0 {
            features.handle(
                &ClientCommand::Camp {
                    session_id: 5,
                    created: now,
                },
                &mut world,
                out,
            )?;
        }
        if phase == 2 {
            features.handle(
                &ClientCommand::SetPosture {
                    session_id: 5,
                    spawn_id: 7,
                    posture: Posture::Standing,
                    created: now,
                },
                &mut world,
                out,
            )?;
        }
        if phase == 3 {
            features.tick(now + Duration::from_secs(31), &mut world, out)?;
        }
        Ok(())
    });
    output.result.unwrap();
    (features, world)
}
