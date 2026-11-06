use super::{super::feature::testing, *};
use eq_network_game::{
    request::Request,
    world::{Position, WorldEvent},
    zoning::ZoneOffer,
};

#[derive(Clone, Copy)]
enum Transition {
    Death,
    Departure,
    Exit,
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
