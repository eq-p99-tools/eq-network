use super::*;
use crate::client::session::{control::Stuns, feature::testing};

fn stun(world: &mut World, now: Instant) -> Message {
    let message = Message::Event(WorldEvent::Stun(None));
    testing::run(|out| Stuns::observe_at(&message, now, world, out))
        .result
        .unwrap();
    message
}

#[test]
fn stun_cancels_preparation_and_deferred_stop_once_without_a_standing_packet() {
    let start = Instant::now();
    for phase in [
        Phase::Preparing(start),
        Phase::WaitingForStop {
            session_id: 12,
            admitted: Some(start),
            motion_context: (12, start),
            stop: None,
        },
    ] {
        let mut world = World::new(12);
        world.player.admit(testing::player(7));
        world.admitted = Some(start);
        let mut camp = Camp { phase, said: None };
        let message = stun(&mut world, start + Duration::from_millis(100));
        let output = testing::run(|out| camp.observe(&message, &mut world, out));
        output.result.unwrap();
        assert!(output.sent.is_empty() && output.unreliable.is_empty());
        assert!(matches!(
            output.events[..],
            [ClientEvent::World(WorldEvent::Camp(
                CampStatus::Abandoned {
                    by: CampEnd::Stunned
                },
            ))]
        ));
        let output = testing::run(|out| {
            camp.observe(&message, &mut world, out)?;
            camp.tick(start + Duration::from_secs(40), &mut world, out)
        });
        output.result.unwrap();
        assert!(output.sent.is_empty() && output.events.is_empty());
        assert!(!camp.active());
    }
}

#[test]
fn a_stun_never_rewinds_an_already_submitted_logout() {
    let start = Instant::now();
    let mut world = World::new(12);
    let mut camp = Camp::default();
    camp.start(start);
    camp.logout_sent(start);
    let message = stun(&mut world, start + Duration::from_millis(100));
    let output = testing::run(|out| camp.observe(&message, &mut world, out));
    output.result.unwrap();
    assert!(output.sent.is_empty() && output.events.is_empty());
    assert!(camp.logging_out());
    let output = testing::run(|out| camp.tick(start + Duration::from_secs(4), &mut world, out));
    output.result.unwrap();
    assert!(matches!(world.exit(), Some(ZoneExit::CharacterSelect)));
    assert!(matches!(
        output.events[..],
        [ClientEvent::World(WorldEvent::Camp(CampStatus::Camped))]
    ));
    assert_eq!(
        output.sent,
        Vec::<eq_network_game::command::EncodedCommand>::new()
    );
}
