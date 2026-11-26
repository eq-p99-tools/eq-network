use super::*;

fn input(pitch: f32) -> WetInput {
    WetInput {
        horizontal: [1.7, 0.0],
        vertical: 1.7 * pitch.to_radians().sin(),
        jump: false,
    }
}

#[test]
fn water_pieces_preserve_legacy_travel_velocity_and_phase_bits() {
    for set in [LIQUID_5, LIQUID_6_TO_8, LIQUID_5_POEARTHB] {
        for pitch in [0.0, 15.0, 60.0, 85.0, -60.0] {
            let mut legacy = WetVelocity::from_units_per_second([3.0, -2.0], -4.0);
            let mut pieces = legacy;
            for seconds in [0.1, 0.117, 0.167, 0.2, 0.25, 0.001] {
                let expected = legacy.advance(set, input(pitch), seconds);
                let mut actual = ([0.0_f32; 2], 0.0_f32);
                let mut left = seconds;
                while left > 0.0 {
                    let piece = pieces
                        .advance_piece(WetFrameMedium::Water(set), input(pitch), left)
                        .unwrap();
                    assert!(piece.seconds > 0.0 && piece.seconds <= left);
                    assert!(piece.seconds <= WET_FRAME_SECONDS + BOUNDARY);
                    actual.0[0] += piece.horizontal[0];
                    actual.0[1] += piece.horizontal[1];
                    actual.1 += piece.vertical;
                    left -= piece.seconds;
                }
                assert_eq!(actual.0.map(f32::to_bits), expected.0.map(f32::to_bits));
                assert_eq!(actual.1.to_bits(), expected.1.to_bits());
                assert_eq!(pieces, legacy);
            }
        }
    }
}

#[test]
fn a_request_boundary_cannot_apply_an_extra_update_or_new_medium() {
    let mut velocity = WetVelocity::default();
    let first = velocity
        .advance_piece(WetFrameMedium::Water(LIQUID_5), input(15.0), 0.015_833_333)
        .unwrap();
    assert!(first.starts_frame && !first.completes_frame);
    assert!(-0.1 + first.vertical > 0.0, "origin crossed inside frame");
    let saved = velocity.units_per_second();
    let remaining = velocity.frame_phase().unwrap().remaining_seconds;
    let rest = velocity
        .advance_piece(WetFrameMedium::Air, WetInput::default(), remaining)
        .unwrap();
    assert!(!rest.starts_frame && rest.completes_frame);
    assert_eq!(saved, velocity.units_per_second());
    let air = velocity
        .advance_piece(WetFrameMedium::Air, input(85.0), WET_FRAME_SECONDS)
        .unwrap();
    assert!(air.starts_frame && air.completes_frame);
    assert!(velocity.units_per_second().0[0] < saved.0[0]);
}

#[test]
fn dry_surface_frame_matches_read_and_ignores_new_input() {
    let mut a = WetVelocity::from_units_per_second([3.0, 4.0], 5.0);
    let mut b = a;
    let expected_horizontal = a.horizontal.map(|v| v * 0.99);
    let expected_vertical = (a.vertical - WET_GRAVITY * WET_FRAME_TICKS) * 0.99;
    let first = a
        .advance_piece(WetFrameMedium::Air, WetInput::default(), WET_FRAME_SECONDS)
        .unwrap();
    let other = b
        .advance_piece(
            WetFrameMedium::Air,
            WetInput {
                jump: true,
                ..input(85.0)
            },
            WET_FRAME_SECONDS,
        )
        .unwrap();
    assert_eq!(first, other);
    assert_eq!(a, b);
    assert_eq!(
        a.horizontal.map(f32::to_bits),
        expected_horizontal.map(f32::to_bits)
    );
    assert_eq!(a.vertical.to_bits(), expected_vertical.to_bits());
}

#[test]
fn unknown_phase_and_invalid_time_decline_without_mutation() {
    let mut normal = WetVelocity::default();
    for seconds in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let before = normal;
        assert!(normal
            .advance_piece(WetFrameMedium::Air, input(15.0), seconds)
            .is_none());
        assert_eq!(normal, before);
    }
    let mut overdue = WetVelocity::default();
    for _ in 0..4 {
        overdue.advance(LIQUID_5, input(15.0), BOUNDARY * 0.5);
    }
    assert!(overdue.frame_phase().is_none());
    let before = overdue;
    assert!(overdue
        .advance_piece(WetFrameMedium::Air, input(15.0), 0.1)
        .is_none());
    assert_eq!(overdue, before);
}

#[test]
fn sub_window_remainder_matches_legacy_without_a_frame_event() {
    let mut a = WetVelocity::from_units_per_second([3.0, 4.0], 5.0);
    let mut b = a;
    let expected = b.advance(LIQUID_5, input(15.0), BOUNDARY * 0.5);
    let piece = a
        .advance_piece(WetFrameMedium::Water(LIQUID_5), input(15.0), BOUNDARY * 0.5)
        .unwrap();
    assert!(!piece.starts_frame && !piece.completes_frame);
    assert_eq!(
        piece.horizontal.map(f32::to_bits),
        expected.0.map(f32::to_bits)
    );
    assert_eq!(piece.vertical.to_bits(), expected.1.to_bits());
    assert_eq!(a, b);
}

#[test]
fn snapshots_clamp_and_stop_preserve_the_existing_phase_contract() {
    let mut velocity = WetVelocity::default();
    velocity.advance(LIQUID_5, input(15.0), 0.117);
    let phase = velocity.frame_phase().unwrap();
    assert!(!phase.at_boundary);
    let snapshot = velocity;
    velocity.clamp(WetCaps::new(LIQUID_5, 0.7, 0.7));
    assert_eq!(velocity.frame_phase(), Some(phase));
    let mut restored = snapshot;
    let a = restored.advance_piece(WetFrameMedium::Water(LIQUID_5), input(15.0), 0.1);
    let mut duplicate = snapshot;
    assert_eq!(
        a,
        duplicate.advance_piece(WetFrameMedium::Water(LIQUID_5), input(15.0), 0.1)
    );
    assert_eq!(restored, duplicate);
    velocity.stop();
    assert_eq!(velocity, WetVelocity::default());
    assert!(velocity.frame_phase().unwrap().at_boundary);
}

#[derive(Clone, Copy)]
struct Surface {
    velocity: WetVelocity,
    wet: bool,
    x: f64,
    z: f64,
    updates: u32,
    refreshes: u32,
}
impl Default for Surface {
    fn default() -> Self {
        Self {
            velocity: WetVelocity::default(),
            wet: true,
            x: 0.0,
            z: -0.1,
            updates: 0,
            refreshes: 0,
        }
    }
}
impl Surface {
    fn advance(&mut self, seconds: f32, pitch: f32) {
        let mut left = seconds;
        while left > 0.0 {
            let medium = if self.wet {
                WetFrameMedium::Water(LIQUID_5)
            } else {
                WetFrameMedium::Air
            };
            let piece = self
                .velocity
                .advance_piece(medium, input(pitch), left)
                .unwrap();
            self.updates += u32::from(piece.starts_frame);
            self.x += f64::from(piece.horizontal[0]);
            self.z += f64::from(piece.vertical);
            if piece.completes_frame {
                self.wet = self.z < 0.0;
                self.refreshes += 1;
            }
            left -= piece.seconds;
        }
    }
}

#[test]
fn continuous_surface_clock_has_no_384ms_excess_and_600_ten_second_updates() {
    for pitch in [15.0, 60.0, 85.0] {
        let mut split = Surface::default();
        let mut elapsed_ms = 0_u64;
        for ms in [100_u64, 117, 167, 200, 250].into_iter().cycle() {
            let ms = ms.min(10_000 - elapsed_ms);
            split.advance(Duration::from_millis(ms).as_secs_f32(), pitch);
            elapsed_ms += ms;
            let whole = elapsed_ms * 60 / 1000;
            let partial_units = elapsed_ms * 60 % 1000;
            let mut continuous = Surface::default();
            for _ in 0..whole {
                continuous.advance(WET_FRAME_SECONDS, pitch);
            }
            if partial_units > 0 {
                continuous.advance(
                    Duration::from_nanos(partial_units * 1_000_000_000 / 60_000).as_secs_f32(),
                    pitch,
                );
            }
            assert_eq!(split.updates, continuous.updates);
            assert_eq!(split.refreshes, continuous.refreshes);
            assert_eq!(split.wet, continuous.wet);
            let error = (split.x - continuous.x)
                .abs()
                .max((split.z - continuous.z).abs());
            assert!(
                error < 0.000_01,
                "pitch{pitch},elapsed{elapsed_ms}: {error}"
            );
            if elapsed_ms == 384 {
                assert_eq!(split.updates, 24);
            }
            if elapsed_ms == 10_000 {
                assert_eq!(split.updates, 600);
                assert_eq!(split.refreshes, 600);
                assert!(split.velocity.frame_phase().unwrap().at_boundary);
                break;
            }
        }
    }
}
