//! Dispatches only session-scoped local movement commands.
use super::{ClientCommand, ClientEvent, Events, Session};
use anyhow::Result;
use eq_network_game::{movement::MotionSession, world::WorldEvent};
use std::time::Instant;

/// Applies an authoritative correction consistently to movement, profile and fallback heartbeat.
pub(super) fn correct_own(
    player: &mut eq_network_game::world::PlayerState,
    motion: Option<&mut MotionSession>,
    stationary: &mut [u8; 36],
    sequence: u16,
    position: eq_network_game::world::Position,
    now: Instant,
) -> Result<()> {
    let packet = eq_network_game::movement::PositionPacket {
        spawn_id: player.spawn_id,
        sequence,
        position,
        delta: [0.0; 3],
        animation: 0,
        delta_heading: 0,
    }
    .encode()?;
    if let Some(motion) = motion {
        motion.correct(position, now)?;
    }
    player.position = position;
    *stationary = packet;
    Ok(())
}

pub(super) fn handle(
    motion: &mut MotionSession,
    session_id: u64,
    command: &ClientCommand,
    session: &mut Session,
    log: &mut Events<'_>,
) -> Result<bool> {
    let now = Instant::now();
    let result = match command {
        ClientCommand::Jump {
            session_id: requested,
            created,
        } => {
            let allowed = motion.jump().and_then(|()| {
                anyhow::ensure!(
                    *requested == session_id
                        && now.saturating_duration_since(*created).as_secs() < 1,
                    "stale jump"
                );
                Ok(())
            });
            match allowed {
                // Transport failures end the admission like any other send.
                Ok(()) => session.send(eq_network_game::movement::JUMP_OPCODE, &[])?,
                Err(error) => log.diagnostic(format!("Rejected jump: {error}"))?,
            }
            return Ok(true);
        }
        ClientCommand::ConfigureMotion {
            session_id: requested,
            calibration,
            created,
        } => {
            if *requested != session_id {
                log.diagnostic("Rejected movement calibration from an old session".into())?;
                return Ok(true);
            }
            motion
                .calibrate_fresh(*calibration, *created, now)
                .map(|()| WorldEvent::MotionState {
                    session_id,
                    units_per_second: Some(calibration.units_per_second),
                    walk_units_per_second: calibration.walk.map(|value| value.units_per_second),
                    strafe_units_per_second: calibration.strafe.map(|value| value.units_per_second),
                    backward_units_per_second: calibration
                        .backward
                        .map(|value| value.units_per_second),
                    falls: motion.falls(),
                })
        }
        ClientCommand::Move(request) => {
            let mut transport_failed = false;
            let result = motion.send_move(request, now, |body| {
                let sent = session.send_unreliable(0x14cb, body);
                transport_failed = sent.is_err();
                sent
            });
            if transport_failed {
                result?;
            } else if let Err(error) = result {
                log.diagnostic(format!("Rejected movement proposal: {error}"))?;
                // Undo local prediction, never publish rejected coordinates.
                log.send(ClientEvent::World(WorldEvent::MotionSent {
                    session_id,
                    position: motion.position(),
                    refused: Some(error.to_string()),
                }))?;
                return Ok(true);
            }
            Ok(WorldEvent::MotionSent {
                session_id,
                position: motion.position(),
                refused: None,
            })
        }
        _ => return Ok(false),
    };
    match result {
        Ok(event) => log.send(ClientEvent::World(event))?,
        Err(error) => log.diagnostic(format!("Rejected movement configuration: {error}"))?,
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "Exact equality verifies unchanged or explicitly assigned state"
    )]
    fn correction_updates_all_position_owners_and_rejects_invalid_data_atomically() {
        let mut spawn = vec![0; 385];
        spawn[340..344].copy_from_slice(&7u32.to_le_bytes());
        let mut player = eq_network_game::world::titanium_player(&vec![0; 19592], &spawn).unwrap();
        let now = Instant::now();
        let mut motion = MotionSession::new(12, 7, player.position, now).unwrap();
        let mut stationary = [0; 36];
        let position = eq_network_game::world::Position {
            x: -123.0,
            y: 456.0,
            z: 7.0,
            heading: 256.0,
        };
        correct_own(
            &mut player,
            Some(&mut motion),
            &mut stationary,
            99,
            position,
            now,
        )
        .unwrap();
        assert_eq!(player.position, position);
        assert_eq!(motion.position(), position);
        assert_eq!(&stationary[..4], &[7, 0, 99, 0]);
        for (offset, expected) in [(4, position.y), (24, position.x), (28, position.z)] {
            assert_eq!(
                f32::from_le_bytes(stationary[offset..offset + 4].try_into().unwrap()),
                expected
            );
        }
        let before = stationary;
        let invalid = eq_network_game::world::Position {
            x: f32::NAN,
            ..position
        };
        assert!(correct_own(
            &mut player,
            Some(&mut motion),
            &mut stationary,
            100,
            invalid,
            now
        )
        .is_err());
        assert_eq!(player.position, position);
        assert_eq!(motion.position(), position);
        assert_eq!(stationary, before);
        let fallback = eq_network_game::world::Position {
            x: 10.0,
            ..position
        };
        correct_own(&mut player, None, &mut stationary, 100, fallback, now).unwrap();
        assert_eq!(player.position, fallback);
        assert_eq!(
            f32::from_le_bytes(stationary[24..28].try_into().unwrap()),
            10.0
        );
    }
}
