use super::{super::feature::testing, *};
use eq_network_game::{
    command::EncodedCommand,
    movement::{MotionSession, MovementMode, MovementRequest},
    request::Request,
    world::{Position, WorldEvent},
    zoning::ZoneOffer,
};

#[derive(Default)]
struct FailSecondMove(usize);

impl super::super::feature::Sink for FailSecondMove {
    fn send(&mut self, _: &EncodedCommand) -> Result<()> {
        panic!("this move batch has no reliable sends");
    }

    fn send_unreliable(&mut self, _: &EncodedCommand) -> Result<()> {
        self.0 += 1;
        ensure!(self.0 == 1, "scripted movement send failure");
        Ok(())
    }

    fn last_received_seconds(&self) -> u64 {
        0
    }
}

#[test]
fn every_dequeued_move_gets_one_stamped_result_even_when_a_send_ends_the_batch() {
    use super::super::motion::Motion;

    let now = Instant::now();
    let at = |millis| now.checked_sub(Duration::from_millis(millis)).unwrap();
    let mut world = World::new(5);
    world.admitted = Some(at(1000));
    world.own_spawn = Some(7);
    world.player.admit(testing::player(7));
    let mut features = Features {
        provided: vec![Box::new(Motion::new(true, false, None))],
        choices: Vec::new(),
        spell_landing_flag: None,
        answers_list_polls: false,
        gameplay_departures: false,
    };
    let original = Position::default();
    let request = |mode, session_id, created| {
        ClientCommand::Move(MovementRequest::new(mode, session_id, original, created))
    };
    let commands = [
        request(MovementMode::Fall, 4, at(120)),
        request(MovementMode::Forward, 5, at(115)),
        request(MovementMode::Fall, 5, at(100)),
        request(MovementMode::Fall, 5, at(20)),
        request(MovementMode::Fall, 4, at(10)),
        request(MovementMode::Fall, 5, now),
    ];
    let mut sink = FailSecondMove::default();
    let outcome = testing::run(|out| {
        features.admitted(&mut world, out)?;
        // Replace only the pacing gate so both checked samples reach the fake
        // transport during this single production command batch.
        world
            .body
            .admit(MotionSession::new(5, 7, original, at(1000))?.with_falls(true));
        apply_commands(
            &commands,
            &mut features,
            &mut world,
            &mut Out {
                sink: &mut sink,
                log: &mut *out.log,
                wire: out.wire,
                sender: out.sender,
            },
            true,
        )
    });
    assert_eq!(sink.0, 2);
    let Err(error) = outcome.result else {
        panic!("the scripted send failure should end the batch");
    };
    assert_eq!(error.to_string(), "scripted movement send failure");
    assert_eq!(world.body.position(), Some(original));
    let results: Vec<_> = outcome
        .events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::World(WorldEvent::MotionSent {
                session_id,
                created,
                position,
                refused,
            }) => {
                assert_eq!(*position, original);
                Some((*session_id, *created, refused.as_deref()))
            }
            ClientEvent::World(WorldEvent::MotionRefused {
                session_id,
                created,
                reason,
            }) => Some((*session_id, *created, Some(reason.as_str()))),
            _ => None,
        })
        .collect();
    assert_eq!(
        results,
        [
            (4, at(120), Some("That was meant for an earlier zone visit")),
            (
                5,
                at(115),
                Some("player input is withdrawn; only vertical descent is allowed")
            ),
            (5, at(100), None),
            (5, at(20), Some("The zone connection ended")),
            (4, at(10), Some("The zone connection ended")),
            (5, now, Some("The zone connection ended")),
        ]
    );
}

#[derive(Clone, Copy)]
enum Transition {
    Death,
    Departure,
    Exit,
    SettleError,
}

/// Only the first command should reach this owner. Its outcome changes
/// whether subsequent commands may run, as a real feature's can.
impl Feature for Transition {
    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::ClickDoor { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::ClickDoor { door_id, .. } = *command else {
            unreachable!("only door clicks are owned");
        };
        out.request(&Request::ClickDoor(door_id))?;
        match self {
            Self::Death => world.lifecycle.mark_dead(),
            Self::Departure => {
                let now = Instant::now();
                world.lifecycle.offer(
                    ZoneOffer {
                        zone_id: 9,
                        instance_id: 0,
                        position: Position::default(),
                        reason: 0,
                        to_bind: false,
                        solicited: true,
                    },
                    now,
                )?;
                world.lifecycle.finish(true, now)?;
            }
            Self::Exit => world.end(ZoneExit::CharacterSelect),
            Self::SettleError => world.happened(Message::LoggedOut),
        }
        Ok(())
    }
}

#[test]
fn a_transition_refuses_each_command_already_drained_behind_it() {
    for (transition, expected) in [
        (Transition::Death, "The player is dead"),
        (Transition::Departure, "The player is between zones"),
        (Transition::Exit, "The zone connection ended"),
    ] {
        let mut features = Features {
            provided: vec![Box::new(transition)],
            choices: Vec::new(),
            spell_landing_flag: None,
            answers_list_polls: false,
            gameplay_departures: true,
        };
        let mut world = World::new(5);
        world.admitted = Some(Instant::now());
        world.player.admit(testing::player(7));
        let click = |door_id| ClientCommand::ClickDoor {
            session_id: 5,
            door_id,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| -> Result<()> {
            let exit = apply_commands(
                &[click(1), click(2), click(3)],
                &mut features,
                &mut world,
                out,
                true,
            )?;
            if matches!(transition, Transition::Exit) {
                assert!(matches!(exit, Some(ZoneExit::CharacterSelect)));
            } else {
                assert!(exit.is_none());
                // The next batch starts under the same hold.
                assert!(
                    apply_commands(&[click(4)], &mut features, &mut world, out, true)?.is_none()
                );
            }
            Ok(())
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1, "{expected}");
        let refusals: Vec<_> = outcome
            .events
            .iter()
            .filter_map(|event| {
                if matches!(event, ClientEvent::Diagnostic(_)) {
                    return None;
                }
                let ClientEvent::World(WorldEvent::DoorAction {
                    session_id: 5,
                    door_id,
                    error: Some(reason),
                    ..
                }) = event
                else {
                    panic!("expected a refusal scoped to the original admission");
                };
                assert_eq!(reason, expected);
                Some(*door_id)
            })
            .collect();
        let doors = if matches!(transition, Transition::Exit) {
            &[2, 3][..]
        } else {
            &[2, 3, 4][..]
        };
        assert_eq!(refusals, doors);
    }
}

#[test]
fn settlement_failure_refuses_every_move_behind_the_completed_command() {
    let mut features = Features {
        provided: vec![Box::new(Transition::SettleError)],
        choices: vec![],
        spell_landing_flag: None,
        answers_list_polls: false,
        gameplay_departures: false,
    };
    let mut world = World::new(5);
    let created = Instant::now();
    world.admitted = Some(created);
    world.own_spawn = Some(7);
    world.player.admit(testing::player(7));
    let movement = |session_id| {
        ClientCommand::Move(MovementRequest::new(
            MovementMode::Forward,
            session_id,
            Position::default(),
            created,
        ))
    };
    let commands = [
        ClientCommand::ClickDoor {
            session_id: 5,
            door_id: 1,
            created,
        },
        movement(4),
        movement(5),
    ];
    let outcome =
        testing::run(|out| apply_commands(&commands, &mut features, &mut world, out, true));
    assert_eq!(outcome.sent.len(), 1);
    assert!(outcome
        .result
        .is_err_and(|error| error.to_string() == "server logged the character out"));
    let refused: Vec<_> = outcome
        .events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::World(WorldEvent::MotionRefused {
                session_id,
                created,
                reason,
            }) => Some((*session_id, *created, reason.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        refused,
        [
            (4, created, "The zone connection ended"),
            (5, created, "The zone connection ended"),
        ]
    );
}

#[test]
fn a_delivery_error_does_not_attempt_to_publish_remaining_move_results() {
    let config = crate::client::ClientConfig::new("Tester", "Fixture", "Local", "Tester");
    let mut deliveries = 0;
    let mut handler = |_| {
        deliveries += 1;
        anyhow::bail!("scripted host delivery failure");
    };
    let mut log = Events::new(&config, &mut handler);
    let created = Instant::now();
    let command = ClientCommand::Move(MovementRequest::new(
        MovementMode::Forward,
        5,
        Position::default(),
        created,
    ));
    let delivery = log
        .refuse_move(&command, "trigger delivery failure")
        .unwrap_err();
    let error = abort_commands(std::iter::once(&command), delivery, &mut log);
    assert!(error.is::<crate::client::EventDeliveryError>());
    assert_eq!(deliveries, 1);
}
