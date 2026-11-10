//! How fast each client generation lets the player move, from what the
//! session knows of them.
use super::session::{BackwardCalibration, MotionCalibration, StrafeCalibration, WalkCalibration};
use super::DiagonalRates;

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
    /// Effective carrying capacity. Zero means no encumbrance in this formula,
    /// never unavailable input; availability is handled by `SpeedInputState`.
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
    /// The share of the jump's starting speed the player has: the same
    /// health and weight multiplier that slows running. The Titanium program
    /// launches both its jumps at this factor times the jump speed, for the
    /// player's own character (0x4193c5, read at 0x4aca9f and 0x4acc18); for
    /// `EQMac` that its jump takes the factor too is inferred (recording
    /// checklist 6.414).
    pub jump_factor: f32,
}

/// Forward input on normal ground per unit of steady speed, in units per
/// second: the program adds 0.8 of the input each tick and keeps 0.625 of
/// the speed, so the speed settles at 0.8 / 0.375 a tick, 20 ticks a second
/// (from the program, TAKP sitting 2 M1).
pub(crate) const EQMAC_UNITS_PER_INPUT: f32 = 0.8 / 0.375 * 20.0;
/// World units per second to the programs' velocity, in units a tick; both
/// generations send it so (the Titanium client update read of 10-31).
const VELOCITY_SCALE: f32 = 1.0 / 20.0;
/// The least and most forward input the programs put on the wire: Titanium
/// holds its record's input to these before it multiplies (the Titanium
/// client update read of 10-31), and `EQMac`'s byte in tenths spans them.
const WIRE_INPUTS: (f32, f32) = (-12.8, 12.7);
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
    /// The animation field on the wire per unit of forward input: `EQMac`'s
    /// speed byte is in tenths, Titanium's 10-bit field in fortieths.
    animation_per_input: f32,
    /// A zone where the program never slows the player: 151 on `EQMac`.
    /// Titanium's two tests that hold the multiplier at 1 are unread, so it
    /// slows the player everywhere, the slower side.
    unslowed_zone: Option<u16>,
    /// Grounded combined longitudinal and lateral input is calibrated.
    grounded_diagonal: bool,
    /// Strafing alone takes whichever base speed is selected, the run
    /// speed running (the Titanium speeds read; the R5 audit's S1). The
    /// `EQMac` rule reads it as the walk speed, so `EQMac` keeps that.
    strafes_on_selected_base: bool,
    /// Backing up in water, unless sneaking, takes only the buffs' speed
    /// modifier (Titanium 0x4a2bd8-0x4a2c06; read R13 b). `EQMac`'s is
    /// unread, so it keeps backing up in water as any swim stroke.
    swim_backs_on_modifier: bool,
}

/// `EQMac`'s side of the table.
const EQMAC: Generation = Generation {
    rate_floor: None,
    animation_per_input: 10.0,
    unslowed_zone: Some(EQMAC_UNSLOWED_ZONE),
    grounded_diagonal: false,
    strafes_on_selected_base: false,
    swim_backs_on_modifier: false,
};

/// Titanium's side of the table, from the Titanium speeds read and the
/// Titanium client update read of 10-31.
const TITANIUM: Generation = Generation {
    rate_floor: Some(-65),
    animation_per_input: 40.0,
    unslowed_zone: None,
    grounded_diagonal: true,
    strafes_on_selected_base: true,
    swim_backs_on_modifier: true,
};

/// `EQMac`'s speeds, from the program (TAKP sitting 2 M1, "Speeds", and the
/// TAKP speeds read of 10-31): the forward input is the profile's run speed,
/// or its walk speed walking, times the movement multiplier, plus the buffs'
/// speed modifier, and the player settles at `EQMAC_UNITS_PER_INPUT` units
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
/// then. Its two tests that hold the multiplier at 1 are unread, so zone
/// 151's exception is `EQMac`'s only. The hit-point pair is read as a current and maximum that fits hit
/// points, as TAKP names it. Not taken from Titanium's read: ducking, the
/// flag at +0x1b1 and its value at +0xec4, mounts, the native swim rate,
/// the halved decay and model 141. Sneaking takes `EQMac`'s rule, the slower
/// side. Its values on the wire are [`titanium`]'s.
///
/// Titanium strafes on whichever base speed is selected: the run speed
/// running, the walk speed walking. A strafe doesn't say which is selected,
/// so the session bounds it by the running rate and gives the walking rate
/// beside it ([`StrafeCalibration::walk_units_per_second`]) for the player
/// to keep to; `EQMac` strafes on the walk speed. Ducking halves the strafe
/// on the player's side, which the session doesn't track. That a buff's
/// speed modifier adds nothing to a strafe is read from the routine's shape
/// only; if it does add, ours is the slower side.
///
/// What the session doesn't know, it takes the slower side of: the
/// modifier's cap is the base one, as the run-speed ability's ranks aren't
/// tracked (nor the small modifier the ability gives with no buffs).
/// Riding and native swim skill tuning are left out. A Titanium swim
/// admission can apply the conservative bound in [`super::swim_speed`].
#[must_use]
pub fn eqmac(state: &SpeedState) -> Speeds {
    speeds(EQMAC, state)
}

/// Titanium's speeds: [`eqmac`]'s rule with Titanium's three differences,
/// and its own wire values. Its client update (the Titanium client update
/// read of 10-31, writer 0x465244) carries the forward input times 40 in
/// its 10-bit animation field, after holding the input to -12.8..12.7, so
/// the defaults send 28 running, 18 walking, -18 backing and 0 strafing
/// alone; the field moves with the multiplier and the modifier. Its deltas
/// are the velocity in units a 50 ms tick. The heading delta, the turn rate
/// in heading units a second, doesn't depend on the mode, and isn't part of
/// this rule.
#[must_use]
pub fn titanium(state: &SpeedState) -> Speeds {
    speeds(TITANIUM, state)
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
        jump_factor: multiplier(generation, state),
    }
}

fn motion(generation: Generation, state: &SpeedState) -> Option<MotionCalibration> {
    if state.rate <= EQMAC_ROOTED {
        return None;
    }
    let multiplier = multiplier(generation, state);
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
    // In water, not sneaking, the base is 0: the player backs up only by
    // the modifier, as slowly as a slow lets them when it would go the
    // other way. Sneaking keeps the land rule. Ducking is the host's, and
    // keeps the land rule there too.
    let swim_backing = (generation.swim_backs_on_modifier && !state.sneaking).then(|| {
        let input = -modifier;
        if input > 0.0 {
            -EQMAC_SLOWEST
        } else {
            input
        }
    });
    let strafe_share = if state.sneaking || slowed { 0.5 } else { 1.0 };
    let strafing_walking = state.walk * 0.5 * 2.0 * multiplier * strafe_share;
    // Sneaking takes `EQMac`'s rule on both generations, the slower side.
    let strafing = if generation.strafes_on_selected_base && !state.sneaking {
        state.run * multiplier * strafe_share
    } else {
        strafing_walking
    };
    let speed = |input: f32| input.abs() * EQMAC_UNITS_PER_INPUT;
    let diagonal = if generation.grounded_diagonal && !state.sneaking {
        // Titanium eqgame.exe (0x4bfe0f, 0x4c00d7–0x4c01c2) takes half
        // the active raw run/walk base for this lateral input (0x6410bc =
        // 0.5; base fields loaded at 0x48170c–0x48171f). With longitudinal
        // input, the standalone strafe factor and additive spell modifier
        // do not enter this branch. We apply the steady-state scale.
        let lateral = |base| (base > 0.0).then_some(speed(base * 0.5));
        DiagonalRates {
            run: lateral(state.run),
            walk: lateral(state.walk),
        }
    } else {
        DiagonalRates::default()
    };
    // The programs cut the field toward zero. Whether Titanium's conversion
    // truncates is read from its routine's shape only (0x60e98c), and the
    // product is taken in single precision.
    #[allow(clippy::cast_possible_truncation)] // Held to the field's range first.
    let byte = |input: f32| {
        (input.clamp(WIRE_INPUTS.0, WIRE_INPUTS.1) * generation.animation_per_input) as i16
    };
    let calibration = MotionCalibration {
        swim_backward_units_per_second: generation
            .swim_backs_on_modifier
            .then(|| swim_backing.map_or_else(|| speed(backing), speed)),
        diagonal,
        units_per_second: speed(running),
        velocity_scale: VELOCITY_SCALE,
        animation: byte(running),
        walk: (walking > 0.0).then_some(WalkCalibration {
            units_per_second: speed(walking),
            velocity_scale: VELOCITY_SCALE,
            animation: byte(walking),
        }),
        backward: (backing < 0.0).then_some(BackwardCalibration {
            units_per_second: speed(backing),
            velocity_scale: VELOCITY_SCALE,
            animation: byte(backing),
        }),
        strafe: (strafing > 0.0).then_some(StrafeCalibration {
            units_per_second: speed(strafing),
            velocity_scale: VELOCITY_SCALE,
            animation: 0,
            walk_units_per_second: (strafing_walking > 0.0 && strafing_walking < strafing)
                .then(|| speed(strafing_walking)),
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
fn multiplier(generation: Generation, state: &SpeedState) -> f32 {
    if generation.unslowed_zone == Some(state.zone_id) {
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
        assert_eq!(speeds.diagonal, DiagonalRates::default());
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
    fn titanium_strafes_on_the_selected_base_bounded_by_the_running_rate() {
        // R5 audit S1: running strafes at the run speed, walking at the walk
        // speed, times the multiplier, with no modifier.
        let strafe = titanium(&player()).motion.unwrap().strafe.unwrap();
        assert!(close(strafe.units_per_second, 29.9), "{strafe:?}");
        assert!(close(strafe.walk_units_per_second.unwrap(), 19.6));
        assert_eq!(strafe.animation, 0);
        // A slow halves both, without its modifier.
        let slowed = titanium(&SpeedState {
            rate: -40,
            ..player()
        })
        .motion
        .unwrap()
        .strafe
        .unwrap();
        assert!(close(slowed.units_per_second, 29.9 / 2.0), "{slowed:?}");
        assert!(close(slowed.walk_units_per_second.unwrap(), 19.6 / 2.0));
        // Hurt, the multiplier slows both alike.
        let hurt = titanium(&SpeedState {
            hit_points: Some((49, 500)),
            level: 50,
            ..player()
        })
        .motion
        .unwrap()
        .strafe
        .unwrap();
        assert!(close(hurt.units_per_second, 29.9 / 2.0), "{hurt:?}");
        // Sneaking keeps `EQMac`'s rule, one rate for both.
        let sneaking = titanium(&SpeedState {
            sneaking: true,
            ..player()
        })
        .motion
        .unwrap()
        .strafe
        .unwrap();
        assert!(close(sneaking.units_per_second, 9.8), "{sneaking:?}");
        assert_eq!(sneaking.walk_units_per_second, None);
        // `EQMac` strafes on the walk speed only.
        let mac = eqmac(&player()).motion.unwrap().strafe.unwrap();
        assert_eq!(mac.walk_units_per_second, None);
    }

    #[test]
    fn titanium_diagonal_uses_half_the_selected_base_and_preserves_backward_calibration() {
        let state = player();
        let straight = titanium(&state).motion.unwrap();
        let slowed = titanium(&SpeedState { rate: -40, ..state }).motion.unwrap();
        let encumbered = titanium(&SpeedState {
            weight: 150,
            ..state
        })
        .motion
        .unwrap();
        for calibration in [straight, slowed, encumbered] {
            assert!(close(
                calibration.diagonal.run.unwrap(),
                0.35 * EQMAC_UNITS_PER_INPUT
            ));
            assert!(close(
                calibration.diagonal.walk.unwrap(),
                0.23 * EQMAC_UNITS_PER_INPUT
            ));
        }
        assert!(close(
            straight.backward.unwrap().units_per_second,
            0.46 * EQMAC_UNITS_PER_INPUT
        ));
        assert!(slowed.units_per_second < straight.units_per_second);
        assert!(
            slowed.strafe.unwrap().units_per_second < straight.strafe.unwrap().units_per_second
        );
        assert!(titanium(&SpeedState {
            rate: -1000,
            ..state
        })
        .motion
        .is_none());
        let sneaking = titanium(&SpeedState {
            sneaking: true,
            ..state
        })
        .motion
        .unwrap();
        assert_eq!(sneaking.diagonal, DiagonalRates::default());
    }

    #[test]
    fn titanium_backs_up_in_water_only_by_the_modifier() {
        let water = |state: SpeedState| {
            titanium(&state)
                .motion
                .unwrap()
                .swim_backward_units_per_second
        };
        // Unbuffed, not at all.
        assert_eq!(water(player()), Some(0.0));
        // A run buff of 50% of the run speed backs up at that modifier.
        let buffed = water(SpeedState {
            rate: 50,
            ..player()
        })
        .unwrap();
        assert!(close(buffed, 0.35 * 42.667), "{buffed}");
        // A slow, which would go the other way, creeps at 0.01.
        let slowed = water(SpeedState {
            rate: -30,
            ..player()
        })
        .unwrap();
        assert!(close(slowed, 0.01 * 42.667), "{slowed}");
        // Sneaking keeps the land rule: half the walk speed.
        let sneaking = SpeedState {
            sneaking: true,
            ..player()
        };
        let land = titanium(&sneaking).motion.unwrap().backward.unwrap();
        assert!(close(water(sneaking).unwrap(), land.units_per_second));
        assert!(close(land.units_per_second, 0.23 * 42.667));
        // `EQMac`'s rule is unread: it backs up in water as any stroke.
        assert_eq!(
            eqmac(&player())
                .motion
                .unwrap()
                .swim_backward_units_per_second,
            None
        );
    }

    #[test]
    fn a_jump_starts_at_the_health_and_weight_factor() {
        let factor = |state: SpeedState| titanium(&state).jump_factor;
        assert!(close(factor(player()), 1.0));
        let hurt = |current| SpeedState {
            hit_points: Some((current, 500)),
            ..player()
        };
        assert!(close(factor(hurt(49)), 0.5));
        assert!(close(factor(hurt(99)), 0.75));
        // Carrying 150 of 100 takes a quarter more; twice the capacity
        // takes all of it.
        assert!(close(
            factor(SpeedState {
                weight: 150,
                ..player()
            }),
            0.75
        ));
        assert!(close(
            factor(SpeedState {
                weight: 250,
                ..hurt(99)
            }),
            0.0
        ));
        assert!(close(eqmac(&hurt(49)).jump_factor, 0.5));
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
    fn titanium_sends_forty_times_the_input_and_slows_everywhere() {
        let speeds = |state: SpeedState| titanium(&state).motion.unwrap();
        let normal = speeds(player());
        assert!(close(normal.units_per_second, 29.9));
        assert_eq!(
            (
                normal.animation,
                normal.walk.unwrap().animation,
                normal.backward.unwrap().animation,
                normal.strafe.unwrap().animation
            ),
            (28, 18, -18, 0)
        );
        // The field moves with the modifier: 0.7 + 0.28 sends 39.
        assert_eq!(
            speeds(SpeedState {
                rate: 40,
                ..player()
            })
            .animation,
            39
        );
        // A slow to the least input sends 0.
        let crawling = speeds(SpeedState {
            rate: -65,
            ..player()
        })
        .walk
        .unwrap();
        assert_eq!(crawling.animation, 0);
        // Zone 151 slows a hurt player on Titanium.
        let hurt = SpeedState {
            hit_points: Some((40, 500)),
            zone_id: 151,
            ..player()
        };
        assert!(close(speeds(hurt).units_per_second, 29.9 / 2.0));
        assert!(close(running(hurt).unwrap(), 29.9));
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
