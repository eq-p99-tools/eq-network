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
    /// The player's level.
    pub level: u8,
    /// The player's hit points and their maximum, as the server last
    /// reported them, once it has.
    pub hit_points: Option<(i32, i32)>,
    /// The movement rate the player's buffs add up to, in percent, as the
    /// host reports it; -1000 or less roots the player (see [`eqmac`]).
    pub rate: i16,
    /// What the player carries, in whole units, as the host reports it.
    pub weight: u16,
    /// What the player can carry before it slows them, as the host reports
    /// it. 0 is only the guard before the host has the player's profile, and
    /// counts no one as carrying too much; a 0 after the profile is in is a
    /// bug in the host, not a weightless player.
    pub capacity: u32,
    /// Whether the player is sneaking, as the server last said.
    pub sneaking: bool,
    /// The zone the player is in.
    pub zone_id: u16,
}

/// What a generation's client makes of the player's state: the speeds it
/// lets them move at, and whether it counts them as carrying too much.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Speeds {
    /// The speeds, or None while the player can't move.
    pub motion: Option<MotionCalibration>,
    /// Whether the player carries more than they can, and the official
    /// client's line for the state, an `eqstr_us.txt` string ID, which it
    /// prints once as the player enters it; None where the generation's
    /// rule for it is unread.
    pub encumbrance: Option<(bool, u32)>,
}

/// Forward input on normal ground per unit of steady speed, in units per
/// second: the program adds 0.8 of the input each tick and keeps 0.625 of
/// the speed, so the speed settles at 0.8 / 0.375 a tick, 20 ticks a second
/// (from the program, TAKP sitting 2 M1).
pub(crate) const EQMAC_UNITS_PER_INPUT: f32 = 0.8 / 0.375 * 20.0;
/// World units per second to the program's velocity, in units a tick.
const EQMAC_VELOCITY_SCALE: f32 = 1.0 / 20.0;
/// The most the speed modifier adds, without the run-speed ability's ranks.
const EQMAC_MODIFIER_CAP: f32 = 1.092_006;
/// The movement rate at or below which the player is rooted (see
/// [`eqmac`]).
const EQMAC_ROOTED: i16 = -1000;
/// The least input a slowed player keeps.
const EQMAC_SLOWEST: f32 = 0.01;
/// The zone where the program never slows the player.
const EQMAC_UNSLOWED_ZONE: u16 = 151;
/// The official client's lines as the player starts and stops carrying too
/// much; both generations' programs print the same two.
const EQMAC_ENCUMBERED: u32 = 0x3068;
const EQMAC_UNENCUMBERED: u32 = 0x3067;

/// Where the generations' speed rules differ; the rest of [`eqmac`]'s rule
/// is the same in both programs.
#[derive(Clone, Copy, Debug)]
struct Generation {
    /// The least movement rate the modifier takes, above the root: Titanium
    /// raises a slow to -65% (the Titanium speeds read of 10-31, part 3);
    /// TAKP's writer has no floor.
    rate_floor: Option<i16>,
    /// The generation's velocity on the wire per unit a second.
    velocity_scale: f32,
}

/// `EQMac`'s side of the table.
const EQMAC: Generation = Generation {
    rate_floor: None,
    velocity_scale: EQMAC_VELOCITY_SCALE,
};

/// `EQMac`'s speeds, from the program (TAKP sitting 2 M1, "Speeds", and the
/// TAKP speeds read of 10-31): the forward input is the profile's run speed,
/// or its walk speed walking, times the movement multiplier, plus the buffs'
/// speed modifier, and the player settles at [`EQMAC_UNITS_PER_INPUT`] units
/// a second for each unit of it; 0.7 runs at 29.9 and 0.46 walks at 19.6.
///
/// - The modifier is the buffs' movement rate in percent of the run speed,
///   held to 1.092006; a slowed input under 0 moves at 0.01, and a rate of
///   -1000 or less roots the player. Two places in the program name the
///   root: the input writer at 0x4caaba zeroes the input at -1000 or less,
///   and the speed routine (M1 line 333) roots below -1000. They agree but
///   at exactly -1000, where the writer's zeroed input decides, so -1000
///   roots. That the rate the program reads is the buffs' total of spell
///   effect 3 is read from its routine's entry only.
/// - The multiplier is 0.5 below a fifth of the maximum hit points and 50,
///   or 0.75 below 0.33 of it and 100, above level 3 (the program also
///   compares with 0 under a character flag it sets elsewhere, unread); then
///   less half of what the player carries beyond their capacity, in parts of
///   it, never below 0; always 1 in zone 151.
/// - Sneaking, the input is half the walk speed, whether running or not.
///   Backing up takes the walk speed, half sneaking, less the modifier.
///   Strafing alone takes half the walk speed times twice the multiplier,
///   halved again sneaking or slowed; the modifier doesn't add to it.
/// - The speed byte is the forward input in tenths, so strafing alone sends
///   0. An input of 0, from carrying twice the capacity or a slow that
///   takes it all, holds the player: None.
///
/// Titanium's program reads the same (the Titanium speeds read of 10-31):
/// the same 42.667 units a second per unit of input on the ground, the same
/// root, floors, multiplier, encumbrance and lines, with backing up always on
/// the walk speed and strafing alone at the base times the multiplier with
/// no modifier. Its strafe rate is halved when the modifier is below 0 or
/// the player ducks, or under a flag the read doesn't name (the read
/// corrects sitting 7's reading of that test). It differs in three places:
/// it raises a slow to -65% before working out the modifier; with the rate
/// at 0, running adds 0.01 per point of the base movement speed effect (271)
/// instead, which is 0 until the session decodes the player's alternate
/// advancement; and the cap rises 0.01 per point of effect 290, also 0 until
/// then. The hit-point pair is read as a current and maximum that fits hit
/// points, as TAKP names it. Not taken from Titanium's read: ducking, the
/// flag at +0x1b1 and its value at +0xec4, mounts, water (the session
/// doesn't swim), the halved decay and model 141. Its values on the wire,
/// the animation and the velocity's scale, are unread, so Titanium has no
/// rule of its own yet.
///
/// What the session doesn't know, it takes the slower side of: the
/// modifier's cap is the base one, as the run-speed ability's ranks aren't
/// tracked (nor the small modifier the ability gives with no buffs), and
/// strafing takes the walk speed, where the program takes the run speed
/// while running, since a strafe doesn't say which it is. Crouching,
/// swimming and riding are left out: the session stands the player up to
/// move, and doesn't take swimming or mounts.
#[must_use]
pub fn eqmac(state: &SpeedState) -> Speeds {
    speeds(EQMAC, state)
}

/// A generation's speeds (see [`eqmac`]).
fn speeds(generation: Generation, state: &SpeedState) -> Speeds {
    let encumbered = encumbered(state);
    Speeds {
        motion: motion(generation, state),
        encumbrance: Some(if encumbered {
            (true, EQMAC_ENCUMBERED)
        } else {
            (false, EQMAC_UNENCUMBERED)
        }),
    }
}

fn motion(generation: Generation, state: &SpeedState) -> Option<MotionCalibration> {
    if state.rate <= EQMAC_ROOTED {
        return None;
    }
    let multiplier = multiplier(state);
    let rate = generation
        .rate_floor
        .map_or(state.rate, |floor| state.rate.max(floor));
    let modifier = (f32::from(rate) / 100.0 * state.run).min(EQMAC_MODIFIER_CAP);
    let slowed = modifier < 0.0;
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
        velocity_scale: generation.velocity_scale,
        animation: byte(running),
        walk: (walking > 0.0).then_some(WalkCalibration {
            units_per_second: speed(walking),
            velocity_scale: generation.velocity_scale,
            animation: byte(walking),
        }),
        backward: (backing < 0.0).then_some(BackwardCalibration {
            units_per_second: speed(backing),
            velocity_scale: generation.velocity_scale,
            animation: byte(backing),
        }),
        strafe: (strafing > 0.0).then_some(StrafeCalibration {
            units_per_second: speed(strafing),
            velocity_scale: generation.velocity_scale,
            animation: 0,
        }),
    };
    calibration.validate().ok().map(|()| calibration)
}

/// Whether the player carries more than their capacity.
fn encumbered(state: &SpeedState) -> bool {
    state.capacity > 0 && u32::from(state.weight) > state.capacity
}

/// The program's movement multiplier (see [`eqmac`]).
#[allow(clippy::cast_precision_loss)] // Hit points and weights well within f32.
fn multiplier(state: &SpeedState) -> f32 {
    if state.zone_id == EQMAC_UNSLOWED_ZONE {
        return 1.0;
    }
    let mut multiplier = match state.hit_points {
        Some((current, maximum)) if current < maximum && maximum > 0 && state.level > 3 => {
            let ratio = current as f32 / maximum as f32;
            if ratio < 0.2 && current < 50 {
                0.5
            } else if ratio < 0.33 && current < 100 {
                0.75
            } else {
                1.0
            }
        }
        _ => 1.0,
    };
    if encumbered(state) {
        let (weight, capacity) = (f32::from(state.weight), state.capacity as f32);
        multiplier = (multiplier - (weight - capacity) / (2.0 * capacity)).max(0.0);
    }
    multiplier
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player() -> SpeedState {
        SpeedState {
            run: 0.7,
            walk: 0.46,
            level: 50,
            hit_points: Some((500, 500)),
            rate: 0,
            weight: 50,
            capacity: 100,
            sneaking: false,
            zone_id: 1,
        }
    }

    fn close(left: f32, right: f32) -> bool {
        (left - right).abs() < 0.05
    }

    fn running(state: SpeedState) -> Option<f32> {
        eqmac(&state).motion.map(|speeds| speeds.units_per_second)
    }

    #[test]
    fn eqmac_runs_and_walks_as_fast_as_the_program() {
        let speeds = eqmac(&player());
        assert_eq!(speeds.encumbrance, Some((false, 0x3067)));
        let speeds = speeds.motion.unwrap();
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
    fn eqmac_slows_a_hurt_player_above_level_3() {
        let at = |hit_points, level| {
            running(SpeedState {
                hit_points: Some(hit_points),
                level,
                ..player()
            })
            .unwrap()
        };
        assert!(close(at((49, 500), 50), 29.9 / 2.0));
        assert!(close(at((99, 500), 50), 29.9 * 0.75));
        assert!(close(at((99, 310), 50), 29.9 * 0.75));
        // 0.33 of the maximum or more, or 100 and up, is unhurt.
        assert!(close(at((99, 290), 50), 29.9));
        assert!(close(at((100, 1000), 50), 29.9));
        // So is anyone up to level 3.
        assert!(close(at((49, 500), 3), 29.9));
        assert!(close(at((49, 500), 4), 29.9 / 2.0));
    }

    #[test]
    fn eqmac_speeds_follow_the_buffs_movement_rate() {
        let rate = |rate| SpeedState { rate, ..player() };
        // A snare takes 35% of the run speed off every input.
        let snared = eqmac(&rate(-50)).motion.unwrap();
        assert!(close(snared.units_per_second, 14.9));
        assert!(close(snared.walk.unwrap().units_per_second, 4.7));
        assert!(close(snared.backward.unwrap().units_per_second, 4.7));
        assert!(close(snared.strafe.unwrap().units_per_second, 9.8));
        // One past the walk speed leaves walking at its least.
        let crawling = eqmac(&rate(-90)).motion.unwrap();
        assert!(close(crawling.walk.unwrap().units_per_second, 0.43));
        assert_eq!(crawling.walk.unwrap().animation, 0);
        // A buff adds its rate, up to the cap.
        assert!(close(running(rate(50)).unwrap(), 44.8));
        assert!(close(running(rate(300)).unwrap(), (0.7 + 1.092) * 42.667));
        // A root holds the player.
        assert_eq!(running(rate(-1000)), None);
        assert!(running(rate(-999)).is_some());
    }

    #[test]
    fn a_generation_with_a_floor_slows_no_more_than_it() {
        let floored = Generation {
            rate_floor: Some(-65),
            ..EQMAC
        };
        let at = |rate| {
            speeds(floored, &SpeedState { rate, ..player() })
                .motion
                .map(|speeds| speeds.units_per_second)
        };
        // -65% of the run speed off the run speed leaves 0.245.
        assert!(close(at(-65).unwrap(), 0.245 * 42.667));
        assert!(close(at(-90).unwrap(), 0.245 * 42.667));
        assert!(close(at(-999).unwrap(), 0.245 * 42.667));
        // The root comes before the floor.
        assert_eq!(at(-1000), None);
    }

    #[test]
    fn eqmac_slows_a_sneaking_or_overloaded_player_except_in_zone_151() {
        let sneaking = eqmac(&SpeedState {
            sneaking: true,
            ..player()
        })
        .motion
        .unwrap();
        assert!(close(sneaking.units_per_second, 9.8));
        assert!(close(sneaking.walk.unwrap().units_per_second, 9.8));
        assert!(close(sneaking.backward.unwrap().units_per_second, 9.8));
        assert!(close(sneaking.strafe.unwrap().units_per_second, 9.8));
        let loaded = |weight| SpeedState { weight, ..player() };
        let speeds = eqmac(&loaded(150));
        assert_eq!(speeds.encumbrance, Some((true, 0x3068)));
        assert!(close(speeds.motion.unwrap().units_per_second, 29.9 * 0.75));
        assert_eq!(eqmac(&loaded(100)).encumbrance, Some((false, 0x3067)));
        // Twice the capacity or more holds the player.
        assert_eq!(running(loaded(300)), None);
        // An unknown capacity slows no one.
        assert!(close(
            running(SpeedState {
                capacity: 0,
                ..loaded(300)
            })
            .unwrap(),
            29.9
        ));
        let hurt_and_loaded = SpeedState {
            hit_points: Some((40, 500)),
            ..loaded(150)
        };
        assert!(close(running(hurt_and_loaded).unwrap(), 29.9 * 0.25));
        assert!(close(
            running(SpeedState {
                zone_id: 151,
                ..hurt_and_loaded
            })
            .unwrap(),
            29.9
        ));
    }
}
