//! How fast each client generation lets the player move, from what the
//! session knows of them.
use super::session::{BackwardCalibration, MotionCalibration, StrafeCalibration, WalkCalibration};

/// What the official client's speeds depend on, as far as the session knows
/// it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeedState {
    /// The profile's run speed, a server protocol value.
    pub run: f32,
    /// The profile's walk speed, as `run`.
    pub walk: f32,
    /// The player's hit points and their maximum, as the server last
    /// reported them, once it has.
    pub hit_points: Option<(i32, i32)>,
    /// The speed modifier the player's buffs give, as the host reports it.
    pub modifier: f32,
    /// Whether the player is sneaking, as the server last said.
    pub sneaking: bool,
}

/// Forward input on normal ground per unit of steady speed, in units per
/// second: the program adds 0.8 of the input each tick and keeps 0.625 of
/// the speed, so the speed settles at 0.8 / 0.375 a tick, 20 ticks a second
/// (from the program, TAKP sitting 2 M1).
const EQMAC_UNITS_PER_INPUT: f32 = 0.8 / 0.375 * 20.0;
/// World units per second to the program's velocity, in units a tick.
const EQMAC_VELOCITY_SCALE: f32 = 1.0 / 20.0;
/// The most the speed modifier adds, without the run-speed ability's ranks.
const EQMAC_MODIFIER_CAP: f32 = 1.092_006;
/// Below this modifier the player is rooted.
const EQMAC_ROOTED: f32 = -1000.0;
/// The least input a slowed player keeps.
const EQMAC_SLOWEST: f32 = 0.01;

/// `EQMac`'s speeds, from the program (TAKP sitting 2 M1, "Speeds"): the
/// forward input is the profile's run speed, or its walk speed walking, times
/// the movement multiplier, plus the buffs' speed modifier, and the player
/// settles at [`EQMAC_UNITS_PER_INPUT`] units a second for each unit of it;
/// 0.7 runs at 29.9 and 0.46 walks at 19.6. A slowed input under 0 moves at
/// 0.01, and a modifier below -1000 roots the player: None. Sneaking, the
/// input is half the walk speed, whether running or not. Backing up takes the
/// walk speed, half sneaking, less the modifier. Strafing alone takes half
/// the walk speed times twice the multiplier, halved again sneaking or
/// slowed; the modifier doesn't add to it. The speed byte is the forward
/// input in tenths, so strafing alone sends 0.
///
/// What the session doesn't know, it takes the slower side of:
/// - the movement multiplier's hit points drop (0.5 under a fifth of the
///   maximum and 50, 0.75 under a third and 100) applies at every level,
///   where the program applies it only above a level it compares with; its
///   "33%" is taken as a third. Its weight term, which the program takes
///   off when the player carries more than they can, is pending: the
///   capacity it compares the weight with is unread, so an over-burdened
///   player isn't slowed yet;
/// - the modifier's cap is the base one, as the run-speed ability's ranks
///   aren't tracked;
/// - strafing takes the walk speed, where the program takes the run speed
///   while running, since a strafe doesn't say which it is.
///
/// Crouching, swimming and riding are left out: the session stands the
/// player up to move, and doesn't take swimming or mounts.
#[must_use]
pub fn eqmac(state: &SpeedState) -> Option<MotionCalibration> {
    if state.modifier < EQMAC_ROOTED {
        return None;
    }
    let multiplier = multiplier(state.hit_points);
    let modifier = state.modifier.min(EQMAC_MODIFIER_CAP);
    let slowed = state.modifier < 0.0;
    let walk = if state.sneaking {
        state.walk * 0.5
    } else {
        state.walk
    };
    let run = if state.sneaking { walk } else { state.run };
    let forward = |base: f32| {
        let input = base * multiplier + modifier;
        if input < 0.0 {
            EQMAC_SLOWEST
        } else {
            input
        }
    };
    let (running, walking) = (forward(run), forward(walk));
    let backing = {
        let input = -(walk * multiplier) - modifier;
        if input > 0.0 {
            -EQMAC_SLOWEST
        } else {
            input
        }
    };
    let strafing =
        state.walk * 0.5 * 2.0 * multiplier * if state.sneaking || slowed { 0.5 } else { 1.0 };
    let speed = |input: f32| input.abs() * EQMAC_UNITS_PER_INPUT;
    // The program keeps the byte in tenths, cut toward zero.
    #[allow(clippy::cast_possible_truncation)] // Held to a byte's range first.
    let byte = |input: f32| (input * 10.0).clamp(-128.0, 127.0) as i16;
    let calibration = MotionCalibration {
        units_per_second: speed(running),
        velocity_scale: EQMAC_VELOCITY_SCALE,
        animation: byte(running),
        walk: Some(WalkCalibration {
            units_per_second: speed(walking),
            velocity_scale: EQMAC_VELOCITY_SCALE,
            animation: byte(walking),
        }),
        backward: Some(BackwardCalibration {
            units_per_second: speed(backing),
            velocity_scale: EQMAC_VELOCITY_SCALE,
            animation: byte(backing),
        }),
        strafe: (strafing > 0.0).then_some(StrafeCalibration {
            units_per_second: speed(strafing),
            velocity_scale: EQMAC_VELOCITY_SCALE,
            animation: 0,
        }),
    };
    calibration.validate().ok().map(|()| calibration)
}

/// The program's movement multiplier from the player's hit points, at every
/// level (see [`eqmac`]).
fn multiplier(hit_points: Option<(i32, i32)>) -> f32 {
    let Some((current, maximum)) = hit_points.filter(|(_, maximum)| *maximum > 0) else {
        return 1.0;
    };
    let (current, maximum) = (i64::from(current), i64::from(maximum));
    if current * 5 < maximum && current < 50 {
        0.5
    } else if current * 3 < maximum && current < 100 {
        0.75
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player() -> SpeedState {
        SpeedState {
            run: 0.7,
            walk: 0.46,
            hit_points: Some((500, 500)),
            modifier: 0.0,
            sneaking: false,
        }
    }

    fn close(left: f32, right: f32) -> bool {
        (left - right).abs() < 0.05
    }

    #[test]
    fn eqmac_runs_and_walks_as_fast_as_the_program() {
        let speeds = eqmac(&player()).unwrap();
        assert!(close(speeds.units_per_second, 29.9), "{speeds:?}");
        assert_eq!(speeds.animation, 7);
        assert!((speeds.velocity_scale - 0.05).abs() < f32::EPSILON);
        let walk = speeds.walk.unwrap();
        assert!(close(walk.units_per_second, 19.6) && walk.animation == 4);
        let back = speeds.backward.unwrap();
        assert!(close(back.units_per_second, 19.6) && back.animation == -4);
        // Strafing takes the walk speed, the slower of the two.
        let strafe = speeds.strafe.unwrap();
        assert!(close(strafe.units_per_second, 19.6) && strafe.animation == 0);
    }

    #[test]
    fn eqmac_slows_a_hurt_snared_or_sneaking_player_and_roots_a_rooted_one() {
        let at = |hit_points| {
            eqmac(&SpeedState {
                hit_points: Some(hit_points),
                ..player()
            })
            .unwrap()
            .units_per_second
        };
        assert!(close(at((49, 500)), 29.9 / 2.0));
        assert!(close(at((99, 500)), 29.9 * 0.75));
        // A third or more, or 100 and up, is unhurt.
        assert!(close(at((167, 500)), 29.9));
        assert!(close(at((100, 1000)), 29.9));
        let snared = eqmac(&SpeedState {
            modifier: -5.0,
            ..player()
        })
        .unwrap();
        assert!(close(snared.units_per_second, 0.43) && snared.animation == 0);
        assert!(close(snared.backward.unwrap().units_per_second, 0.43));
        assert!(close(snared.strafe.unwrap().units_per_second, 9.8));
        // A buff adds up to its cap.
        let quick = eqmac(&SpeedState {
            modifier: 5.0,
            ..player()
        })
        .unwrap();
        assert!(close(quick.units_per_second, (0.7 + 1.092) * 42.667));
        let sneaking = eqmac(&SpeedState {
            sneaking: true,
            ..player()
        })
        .unwrap();
        assert!(close(sneaking.units_per_second, 9.8));
        assert!(close(sneaking.walk.unwrap().units_per_second, 9.8));
        assert!(close(sneaking.backward.unwrap().units_per_second, 9.8));
        assert!(close(sneaking.strafe.unwrap().units_per_second, 9.8));
        assert_eq!(
            eqmac(&SpeedState {
                modifier: -1001.0,
                ..player()
            }),
            None
        );
    }
}
