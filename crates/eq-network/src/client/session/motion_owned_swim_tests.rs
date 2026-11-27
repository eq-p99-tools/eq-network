//! The inert ordinary-swim seam through the standard counted position sender.
use super::*;
use crate::client::session::feature::{testing, Sink};
use eq_network_game::movement::{
    titanium_speeds,
    water::{WetInput, WetKind, WetVelocity},
    HeadingUpdate, MovementMode, Pace, SwimControl, SwimControls, SwimInput,
};

fn calibrated() -> MotionCalibration {
    titanium_speeds(&SpeedState {
        run: 0.7,
        walk: 0.3,
        level: 50,
        hit_points: Some((500, 500)),
        rate: 0,
        weight: 0,
        capacity: 100,
        sneaking: false,
        ducking: false,
        zone_id: 1,
        swimming_skill: Some(100),
    })
    .motion
    .unwrap()
}

#[test]
fn regrant_republishes_an_untouched_real_reset_receipt() {
    let start = Instant::now();
    let mut world = World::new(11);
    let mut feature = Motion::new(true, true, None).with_swimming();
    testing::run(|out| {
        world.body.admit(owner(start, out, false));
        feature.publish_swim_state(&mut world, None);
        feature.withdraw_input(&mut world, out).unwrap();
        world
            .body
            .motion
            .as_mut()
            .unwrap()
            .calibrate(calibrated(), start + Duration::from_millis(100))
            .unwrap();
        feature.publish_swim_state(&mut world, None);
    });
    let resets: Vec<_> = world
        .take_news()
        .into_iter()
        .filter_map(|message| match message {
            Message::Event(WorldEvent::SwimState {
                basis: Some(basis), ..
            }) => Some(basis),
            _ => None,
        })
        .collect();
    assert_eq!(resets.len(), 2);
    assert_eq!(
        resets[0], resets[1],
        "no artificial reset or epoch was minted"
    );
    assert!(resets[1].is_reset());
}

fn owner(start: Instant, out: &Out<'_, '_>, gate: bool) -> MotionSession {
    let mut motion = MotionSession::new(11, 7, Position::default(), start)
        .unwrap()
        .with_swimming(true)
        .with_heading_snapshots(true)
        .with_counter(out.wire.update_counter())
        .with_gate(if gate { out.wire.update_gate() } else { None });
    motion.calibrate(calibrated(), start).unwrap();
    motion
}

fn observed(motion: &MotionSession, physical: &mut WetVelocity) -> (MovementRequest, SwimControls) {
    let basis = motion.swim_basis().unwrap();
    let swim = calibrated().swim.unwrap();
    let (across, up) = physical.advance(
        swim.set,
        WetInput {
            horizontal: [0., swim.forward],
            vertical: 0.,
            jump: false,
        },
        0.1,
    );
    let position = motion.position();
    let mut request = MovementRequest::new(
        MovementMode::Swim,
        11,
        Position {
            x: position.x + across[0],
            y: position.y + across[1],
            z: position.z + up,
            heading: 0.,
        },
        basis.started_at() + Duration::from_millis(100),
    );
    request.heading = HeadingUpdate::Snapshot;
    let controls = SwimControls::new(
        basis,
        vec![SwimControl::new(
            Duration::from_millis(100),
            0.,
            WetKind::Normal,
            SwimInput::Forward(Pace::Run),
        )
        .unwrap()],
    )
    .unwrap();
    (request, controls)
}

#[test]
fn typed_swim_uses_normal_owner_sender_and_queues_receipt_after_local_result() {
    let start = Instant::now()
        .checked_sub(Duration::from_millis(200))
        .unwrap();
    let mut world = World::new(11);
    world.admitted = Some(start);
    world.own_spawn = Some(7);
    world.player.admit(testing::player(7));
    let mut feature = Motion::new(true, true, None).with_swimming();
    feature.belongings = Some((
        world.inventory.revision(),
        world.coins.purse,
        world.coins.cursor,
    ));
    let output = testing::run(|out| {
        let motion = owner(start, out, false);
        let (request, controls) = observed(&motion, &mut WetVelocity::default());
        let position = request.position;
        world.body.admit(motion);
        let command = ClientCommand::SwimFrame {
            request,
            controls: Box::new(controls),
        };
        assert!(feature.owns(&command));
        assert_eq!(command.session_id(), Some(11));
        feature.handle(&command, &mut world, out).unwrap();
        assert_eq!(out.log.movement_packets_sent, 1);
        assert_eq!(world.body.position(), Some(position));
        position
    });
    assert_eq!(output.unreliable.len(), 1);
    assert!(matches!(
        output.events.last(),
        Some(ClientEvent::World(WorldEvent::MotionSent {
            refused: None,
            ..
        }))
    ));
    assert!(world.take_news().iter().any(|message| matches!(message,
        Message::Event(WorldEvent::SwimState { position, created: Some(_), basis: Some(_), .. }) if *position == output.result)));
}

#[test]
fn repeated_same_position_corrections_queue_distinct_real_reset_receipts() {
    let start = Instant::now();
    let mut world = World::new(11);
    let mut feature = Motion::new(true, true, None).with_swimming();
    let output = testing::run(|out| {
        world.body.admit(owner(start, out, false));
        feature.publish_swim_state(&mut world, None);
        let first = world.body.motion.as_ref().unwrap().swim_basis();
        for milliseconds in [100, 200] {
            let at = start + Duration::from_millis(milliseconds);
            let motion = world.body.motion.as_mut().unwrap();
            motion.correct(Position::default(), at).unwrap();
            motion.calibrate(calibrated(), at).unwrap();
            feature.publish_swim_state(&mut world, None);
        }
        first
    });
    let receipts: Vec<_> = world
        .take_news()
        .into_iter()
        .filter_map(|message| match message {
            Message::Event(WorldEvent::SwimState {
                basis: Some(basis), ..
            }) => Some(basis),
            _ => None,
        })
        .collect();
    assert_eq!(receipts.len(), 3);
    assert_eq!(Some(receipts[0]), output.result);
    assert!(receipts.iter().all(|basis| basis.is_reset()));
    assert_ne!(receipts[0], receipts[1]);
    assert_ne!(receipts[1], receipts[2]);
}

#[test]
fn owned_acceptance_and_held_samples_do_not_count_as_sent_packets() {
    let output = testing::run(|out| {
        let mut motion = owner(Instant::now(), out, true);
        let mut physical = WetVelocity::default();
        let mut callbacks = 0_u64;
        let mut last_sequence = None;
        for _ in 0..12 {
            let (request, controls) = observed(&motion, &mut physical);
            motion
                .send_swim_sample(&request, Some(&controls), request.created, |sample| {
                    out.request_unreliable(&Request::Position(*sample))?;
                    callbacks += 1;
                    last_sequence = Some(sample.sequence);
                    Ok(())
                })
                .unwrap();
            assert_eq!(out.log.movement_packets_sent, callbacks);
        }
        assert!(callbacks > 0 && callbacks < 12);
        assert!(last_sequence.is_some_and(|sequence| u64::from(sequence) > callbacks));
        let mut refused = MovementRequest::new(
            MovementMode::Swim,
            11,
            Position {
                y: motion.position().y - 0.2,
                ..motion.position()
            },
            motion.swim_basis().unwrap().started_at() + Duration::from_millis(100),
        );
        refused.heading = HeadingUpdate::Snapshot;
        let before = motion.position();
        assert!(motion
            .send_swim_sample(&refused, None, refused.created, |_| panic!(
                "refusal sent a packet"
            ))
            .is_err());
        assert_eq!(motion.position(), before);
        assert_eq!(out.log.movement_packets_sent, callbacks);
        assert!(motion.swim_basis().is_none());
        callbacks
    });
    assert_eq!(
        output.result,
        u64::try_from(output.unreliable.len()).unwrap()
    );
}

struct Broken;
impl Sink for Broken {
    fn send(&mut self, _: &eq_network_game::command::EncodedCommand) -> Result<()> {
        anyhow::bail!("synthetic send failure")
    }
    fn send_unreliable(&mut self, _: &eq_network_game::command::EncodedCommand) -> Result<()> {
        anyhow::bail!("synthetic send failure")
    }
    fn last_received_seconds(&self) -> u64 {
        0
    }
}

#[test]
fn owned_trial_send_failure_does_not_commit_or_increment_the_counter() {
    let output = testing::run(|out| {
        let mut motion = owner(Instant::now(), out, false);
        let mut physical = WetVelocity::default();
        let (request, controls) = observed(&motion, &mut physical);
        let basis = motion.swim_basis();
        let before = motion.position();
        {
            let mut broken = Broken;
            let mut failing = Out {
                sink: &mut broken,
                log: &mut *out.log,
                wire: out.wire,
                sender: out.sender,
            };
            assert!(motion
                .send_swim_sample(&request, Some(&controls), request.created, |sample| {
                    failing.request_unreliable(&Request::Position(*sample))
                })
                .is_err());
            assert_eq!(failing.log.movement_packets_sent, 0);
        }
        assert_eq!(motion.position(), before);
        assert_eq!(motion.swim_basis(), basis);
        motion
            .send_swim_sample(&request, Some(&controls), request.created, |sample| {
                out.request_unreliable(&Request::Position(*sample))
            })
            .unwrap();
        assert_eq!(out.log.movement_packets_sent, 1);
    });
    assert_eq!(output.unreliable.len(), 1);
}
