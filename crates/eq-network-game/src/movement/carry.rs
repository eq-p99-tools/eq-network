//! How each client generation carries other spawns between the server's
//! reports of them.
use crate::{
    world::{Position, SpawnKind},
    GameDialect,
};
use std::time::Duration;

/// A spawn's motion as the client carries it between the server's reports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Carried {
    /// Where it is now.
    pub position: Position,
    /// Its motion now, in EQ units per second along X, Y and Z.
    pub velocity: [f32; 3],
    /// Its forward input from the last report, as
    /// [`WorldEvent::Position`](crate::world::WorldEvent::Position) gives it.
    pub forward: f32,
    /// How long since the last report.
    pub since_report: Duration,
    /// What kind of spawn it is, which the `EQMac` step tells apart.
    pub kind: SpawnKind,
}

/// One client generation's step, as its program carries a spawn.
struct Step {
    /// The program's speed unit, units per 50 ms, in a second.
    ticks_per_second: f32,
    /// The most 50 ms spans one call moves a spawn by: the program holds a
    /// frame's time, in those spans (elapsed ms × 0.02), to 3.
    most_spans: f32,
    /// How much of a spawn's speed each call keeps on normal ground.
    kept: f32,
    /// How much of that a moving NPC's horizontal speed keeps: half.
    npc_share: f32,
    /// How much of its forward input each call adds on normal ground.
    added: f32,
    /// How long the program carries a spawn that keeps a forward input
    /// without a report; None where it has no such rule.
    stale: Option<Duration>,
    /// The races whose reported vertical motion the program drops.
    level_races: &'static [u32],
}

/// `EQMac`, from TAKP sitting 2 M1 (line 283), the TAKP speeds read of
/// 10-31 and TAKP read R1 of 10-31: the hold to 3 spans at 0x5e8510 and
/// 0x5e8594, the NPC halving at 0x54e1d7 in the step 0x54d964.
const EQMAC: Step = Step {
    ticks_per_second: 20.0,
    most_spans: 3.0,
    kept: 0.625,
    npc_share: 0.5,
    added: 0.8,
    stale: Some(Duration::from_secs(12)),
    level_races: &[0x48, 0x49],
};

/// Titanium, from Titanium read R2 of 10-31: the step 0x4bfe0f (0.02 at
/// 0x6410b8, 0.8 and 0.625 at 0x6431d8 and 0x643f7c, the NPC halving at
/// 0x4c06f0-0x4c074e by 0x6410bc) and the decoder 0x465304 (the level
/// races). Its ground path takes F and D from the physics block, whose
/// values weren't traced; that they are 0.8 and 0.625, TAKP's, is inferred
/// (checklist 6.320). It names no stale rule but for a marker report, which
/// this client doesn't decode.
const TITANIUM: Step = Step {
    ticks_per_second: 20.0,
    most_spans: 3.0,
    kept: 0.625,
    npc_share: 0.5,
    added: 0.8,
    stale: None,
    level_races: &[0x48, 0x49, 0x194],
};

const fn step(dialect: GameDialect) -> &'static Step {
    match dialect {
        GameDialect::EqMac => &EQMAC,
        GameDialect::Titanium => &TITANIUM,
    }
}

impl Carried {
    /// A spawn as a report puts it, of this kind and race, in this
    /// generation. A report sets the position, heading, input and speed
    /// outright, with no blending (TAKP read R1 (b), Titanium read R2 (a)).
    /// Both programs drop the vertical motion of races 0x48 and 0x49, and
    /// Titanium's of 0x194 too (the TAKP speeds read of 10-31, R2 (a)).
    #[must_use]
    pub fn reported(
        dialect: GameDialect,
        kind: SpawnKind,
        race: u32,
        position: Position,
        velocity: [f32; 3],
        forward: f32,
    ) -> Self {
        let mut velocity = velocity;
        if step(dialect).level_races.contains(&race) {
            velocity[2] = 0.0;
        }
        Self {
            position,
            velocity,
            forward,
            since_report: Duration::ZERO,
            kind,
        }
    }
}

/// Carries a spawn on for one frame of `elapsed`, as the generation's client
/// does between the server's reports. The spawn faces its position's
/// heading, as the last report gave it.
///
/// Both programs run the same step, once a frame for each spawn (TAKP
/// sitting 2 M1, the TAKP speeds read and TAKP read R1 of 10-31; Titanium
/// read R2 of 10-31). Each call keeps 0.625 of the spawn's speed, adds 0.8
/// of its forward input along its heading, and moves it by the result
/// times the frame's time in 50 ms spans, at most 3: the speed update runs
/// once a call and isn't scaled by time; the move is v × min(ms × 0.02,
/// 3.0). The programs' speed is in units per 50 ms, which is what M1's "20
/// a second" names, not a call rate. On the ground a player's spawn settles
/// at 42.67 units a second for each unit of input, whatever the frame rate;
/// only the climb to it after a report goes by frames.
///
/// An NPC keeps only half as much of its horizontal speed each call while
/// that speed isn't zero (TAKP 0x54e1d7, Titanium 0x4c06f0: type 1,
/// D × 0.5), so it settles at 23.27 units a second for each unit of input,
/// the servers' own pace: 2.327 for each unit of TAKP's speed byte, and
/// 0.582 for each unit of Titanium's animation value. Its vertical speed
/// keeps 0.625 as before. That every NPC is halved is inferred: TAKP skips
/// it for an NPC whose physics block links it to something (block+0x198,
/// unread, probably a mount; checklist 6.316), and Titanium for one whose
/// block+0x160 is set, or while its byte 0x923798 is set and a per-spawn
/// test holds (all unread; checklist 6.321). This client tracks none of
/// them.
///
/// On `EQMac`, a spawn whose last report is over 12 s old and that still
/// has an input stays where it is. Left out: the heading rate, whose unit
/// is unread on `EQMac` (Titanium's, 0.05 a 50 ms span, is read but not
/// decoded yet), so the heading stays as reported; gravity, the ground and
/// other media, which the host's floor handles; the reports that keep the
/// old Z, which the host's floor handles too: within 5 units for a spawn
/// whose +0x10a (`EQMac`, checklist 6.317) or +0x28c (Titanium) is 2, and
/// for the level races under a global the reads don't name (checklist
/// 6.318 and 6.322); a report that marks the spawn (0x42 in the input or
/// animation); and an `EQMac` player's spawn whose report the program
/// applies only as a stop, under a flag of its own (R1 (b), block+0x190;
/// checklist 6.319).
pub fn carry(dialect: GameDialect, spawn: &mut Carried, elapsed: Duration) {
    spawn.since_report = spawn.since_report.saturating_add(elapsed);
    let step = step(dialect);
    if step
        .stale
        .is_some_and(|stale| spawn.since_report > stale && spawn.forward != 0.0)
    {
        return;
    }
    let spans = (elapsed.as_secs_f32() * step.ticks_per_second).min(step.most_spans);
    // Heading 0 faces +Y and a quarter turn (128) faces +X.
    let angle = spawn.position.heading / 512.0 * std::f32::consts::TAU;
    let push = spawn.forward * step.added;
    let push = [angle.sin() * push, angle.cos() * push, 0.0];
    // The programs' speed is in units per 50 ms.
    let mut speed = spawn.velocity.map(|value| value / step.ticks_per_second);
    // The programs test the horizontal speed before they keep any.
    #[allow(clippy::float_cmp, reason = "the programs test for exactly zero")]
    let horizontal = if spawn.kind == SpawnKind::Npc && (speed[0] != 0.0 || speed[1] != 0.0) {
        step.kept * step.npc_share
    } else {
        step.kept
    };
    for (index, (value, push)) in speed.iter_mut().zip(push).enumerate() {
        let kept = if index < 2 { horizontal } else { step.kept };
        *value = *value * kept + push;
    }
    spawn.position.x += speed[0] * spans;
    spawn.position.y += speed[1] * spans;
    spawn.position.z += speed[2] * spans;
    spawn.velocity = speed.map(|value| value * step.ticks_per_second);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walker(heading: f32, forward: f32) -> Carried {
        Carried::reported(
            GameDialect::EqMac,
            SpawnKind::Player,
            1,
            Position {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                heading,
            },
            [0.0; 3],
            forward,
        )
    }

    #[test]
    fn eqmac_carries_a_spawn_along_its_heading_at_the_programs_speed() {
        // A speed byte of 7, facing +X, at one frame a tick.
        let mut spawn = walker(128.0, 0.7);
        for _ in 0..200 {
            carry(GameDialect::EqMac, &mut spawn, Duration::from_millis(50));
        }
        assert!((spawn.velocity[0] - 29.87).abs() < 0.05, "{spawn:?}");
        assert!(spawn.velocity[1].abs() < 0.001);
        // 200 ticks at 1.4933 a tick, less the 0.625 / 0.375 ticks the
        // speed takes to settle.
        assert!(
            (spawn.position.x - 1.493_33 * (200.0 - 0.625 / 0.375)).abs() < 0.05,
            "{spawn:?}"
        );
        assert_eq!(spawn.since_report, Duration::from_secs(10));
        // The first tick moves 0.56 units, not the full speed.
        let mut spawn = walker(0.0, 0.7);
        carry(GameDialect::EqMac, &mut spawn, Duration::from_millis(50));
        assert!((spawn.position.y - 0.56).abs() < 0.001, "{spawn:?}");
    }

    #[test]
    fn eqmac_steps_once_a_call_and_moves_by_the_frames_time() {
        // One call updates the speed once and moves by the time it was
        // given: 30 ms is 0.6 of a 50 ms span, so 0.56 × 0.6.
        let mut spawn = walker(0.0, 0.7);
        carry(GameDialect::EqMac, &mut spawn, Duration::from_millis(30));
        assert!((spawn.position.y - 0.336).abs() < 0.001, "{spawn:?}");
        assert!((spawn.velocity[1] - 11.2).abs() < 0.001, "{spawn:?}");
        // The climb goes by calls, but the speed it settles at doesn't
        // depend on the frame rate: 29.87 at 60 frames a second as at 20.
        for _ in 0..600 {
            carry(
                GameDialect::EqMac,
                &mut spawn,
                Duration::from_micros(16_667),
            );
        }
        assert!((spawn.velocity[1] - 29.87).abs() < 0.05, "{spawn:?}");
    }

    #[test]
    fn eqmac_slows_a_reported_motion_and_holds_a_stale_one() {
        let mut spawn = Carried::reported(
            GameDialect::EqMac,
            SpawnKind::Player,
            1,
            Position::default(),
            [20.0, 0.0, -5.0],
            0.0,
        );
        carry(GameDialect::EqMac, &mut spawn, Duration::from_millis(50));
        assert!((spawn.velocity[0] - 12.5).abs() < 0.001, "{spawn:?}");
        assert!((spawn.position.x - 0.625).abs() < 0.001);
        assert!((spawn.velocity[2] + 3.125).abs() < 0.001);
        // A long frame updates the speed once and moves by at most three
        // 50 ms spans of it: 0.56 × 3.
        let mut spawn = walker(0.0, 0.7);
        carry(GameDialect::EqMac, &mut spawn, Duration::from_millis(1020));
        assert!((spawn.position.y - 1.68).abs() < 0.001, "{spawn:?}");
        // Over 12 s without a report, a walker stops where it is.
        let mut spawn = walker(0.0, 0.7);
        spawn.since_report = Duration::from_secs(12);
        carry(GameDialect::EqMac, &mut spawn, Duration::from_millis(50));
        let held = spawn.position;
        carry(GameDialect::EqMac, &mut spawn, Duration::from_millis(50));
        assert_eq!(spawn.position, held);
        // Races 0x48 and 0x49 keep level.
        let boat = Carried::reported(
            GameDialect::EqMac,
            SpawnKind::Npc,
            0x48,
            Position::default(),
            [1.0, 2.0, 3.0],
            0.0,
        );
        assert_eq!(boat.velocity, [1.0, 2.0, 0.0]);
    }

    /// How a server walks a spawn north and reports it: at `speed` units a
    /// second, each `spacing` units, with this velocity and forward input in
    /// each report.
    struct Walk {
        dialect: GameDialect,
        kind: SpawnKind,
        speed: f32,
        spacing: f32,
        velocity: [f32; 3],
        forward: f32,
    }

    /// Walks a spawn as `walk` says, carrying it at `fps` frames a second
    /// between reports, and gives how far the carried spawn is from each
    /// report as it lands.
    fn walk_misses(walk: &Walk, fps: u32) -> Vec<f32> {
        let frame = Duration::from_secs(1) / fps;
        let between = walk.spacing / walk.speed;
        let report = |y: f32| {
            Carried::reported(
                walk.dialect,
                walk.kind,
                1,
                Position {
                    x: 0.0,
                    y,
                    z: 0.0,
                    heading: 0.0,
                },
                walk.velocity,
                walk.forward,
            )
        };
        let mut spawn = report(0.0);
        let mut misses = Vec::new();
        for leg in 1..=12_u8 {
            let next = walk.spacing * f32::from(leg);
            while (spawn.since_report + frame).as_secs_f32() <= between {
                carry(walk.dialect, &mut spawn, frame);
            }
            misses.push(spawn.position.y - next);
            spawn = report(next);
        }
        misses
    }

    /// Walks an NPC as the TAKP server moves one and reports it. The server
    /// moves it 2.32 units a second for each unit of the speed byte
    /// (`EQMacEmu` `zone/mob_movement_manager.cpp`, `MoveToCommand`: frame
    /// time × speed × 0.4 × 1.45, with the byte at a quarter of the speed
    /// in `zone/waypoints.cpp`, `SetRunAnimation`), and reports it each 10
    /// units it moves, with the forward input and no velocity.
    fn misses(kind: SpawnKind, byte: u8, fps: u32) -> Vec<f32> {
        walk_misses(
            &Walk {
                dialect: GameDialect::EqMac,
                kind,
                speed: 2.32 * f32::from(byte),
                spacing: 10.0,
                velocity: [0.0; 3],
                forward: f32::from(byte) / 10.0,
            },
            fps,
        )
    }

    #[test]
    fn eqmac_carries_an_npc_at_the_servers_pace_between_its_reports() {
        for fps in [30, 60, 144] {
            for byte in [4, 5, 7] {
                let misses = misses(SpawnKind::Npc, byte, fps);
                assert!(
                    misses.iter().all(|miss| miss.abs() <= 1.5),
                    "{fps} fps, byte {byte}: {misses:?}"
                );
            }
        }
        // A player's spawn keeps the full damping and runs well ahead of
        // the same reports.
        let misses = misses(SpawnKind::Player, 5, 60);
        assert!(misses.iter().all(|miss| *miss > 5.0), "{misses:?}");
    }

    #[test]
    fn eqmac_settles_an_npc_at_half_a_players_pace_and_restarts_on_a_report() {
        let settle = |kind| {
            let mut spawn = Carried::reported(
                GameDialect::EqMac,
                kind,
                1,
                Position::default(),
                [0.0; 3],
                0.5,
            );
            for _ in 0..100 {
                carry(GameDialect::EqMac, &mut spawn, Duration::from_millis(50));
            }
            spawn.velocity[1] / 5.0
        };
        // Per unit of the speed byte: 2.327 for an NPC, 4.267 for a player.
        assert!((settle(SpawnKind::Npc) - 2.327).abs() < 0.001);
        assert!((settle(SpawnKind::Player) - 4.267).abs() < 0.001);
        // From a report's zero speed, an NPC is at 69, 90 and 97 % of it
        // after one, two and three calls, whatever their length.
        let mut spawn = Carried::reported(
            GameDialect::EqMac,
            SpawnKind::Npc,
            1,
            Position::default(),
            [0.0; 3],
            0.5,
        );
        let steady = 0.5 * 0.8 / (1.0 - 0.3125) * 20.0;
        for (millis, share) in [(16, 0.6875), (7, 0.902_34), (33, 0.969_48)] {
            carry(
                GameDialect::EqMac,
                &mut spawn,
                Duration::from_millis(millis),
            );
            assert!(
                (spawn.velocity[1] / steady - share).abs() < 0.001,
                "{spawn:?}"
            );
        }
        // Only the horizontal speed is damped harder.
        let mut falling = Carried::reported(
            GameDialect::EqMac,
            SpawnKind::Npc,
            1,
            Position::default(),
            [0.0, 0.0, -20.0],
            0.0,
        );
        carry(GameDialect::EqMac, &mut falling, Duration::from_millis(50));
        assert!((falling.velocity[2] + 12.5).abs() < 0.001, "{falling:?}");
        // A report replaces where it is and its speed outright.
        let reported = Carried::reported(
            GameDialect::EqMac,
            SpawnKind::Npc,
            1,
            Position {
                x: 3.0,
                y: 4.0,
                z: 5.0,
                heading: 128.0,
            },
            [1.0, 2.0, 0.0],
            0.4,
        );
        assert_eq!(
            (reported.position.x, reported.velocity, reported.forward),
            (3.0, [1.0, 2.0, 0.0], 0.4)
        );
        assert_eq!(reported.since_report, Duration::ZERO);
    }

    /// A Titanium report's motion for an animation value and deltas in u/s,
    /// as the decoder gives it: the input is the animation over 40.
    fn titanium(kind: SpawnKind, race: u32, velocity: [f32; 3], animation: f32) -> Carried {
        Carried::reported(
            GameDialect::Titanium,
            kind,
            race,
            Position::default(),
            velocity,
            animation / 40.0,
        )
    }

    #[test]
    fn titanium_relaxes_a_p99_npc_from_its_deltas_to_its_animations_pace() {
        // P99 packs an NPC's deltas at about 2.2 times its speed in the
        // program's unit; an animation value of 20 is 11.64 a second.
        let steady = 0.582 * 20.0;
        for millis in [7, 16, 33] {
            let mut spawn = titanium(SpawnKind::Npc, 1, [0.0, 2.2 * steady, 0.0], 20.0);
            for _ in 0..3 {
                carry(
                    GameDialect::Titanium,
                    &mut spawn,
                    Duration::from_millis(millis),
                );
            }
            // Within 4 % after three calls, whatever their length.
            assert!(
                (spawn.velocity[1] / steady - 1.0).abs() < 0.04,
                "{millis} ms: {spawn:?}"
            );
            for _ in 0..30 {
                carry(
                    GameDialect::Titanium,
                    &mut spawn,
                    Duration::from_millis(millis),
                );
            }
            assert!((spawn.velocity[1] - 11.636).abs() < 0.01, "{spawn:?}");
        }
        // Reported as P99 walks it, each 10 units, it lands near each report.
        for fps in [30, 60, 144] {
            let misses = walk_misses(
                &Walk {
                    dialect: GameDialect::Titanium,
                    kind: SpawnKind::Npc,
                    speed: steady,
                    spacing: 10.0,
                    velocity: [0.0, 2.2 * steady, 0.0],
                    forward: 0.5,
                },
                fps,
            );
            assert!(
                misses.iter().all(|miss| miss.abs() <= 1.5),
                "{fps} fps: {misses:?}"
            );
        }
    }

    #[test]
    fn titanium_carries_a_player_at_the_speed_its_deltas_give() {
        // A running player, 42.67 a second for each unit of input: its
        // deltas give its speed, and its input settles there.
        let speed = 42.667 * 1.05;
        for fps in [30, 60, 144] {
            let misses = walk_misses(
                &Walk {
                    dialect: GameDialect::Titanium,
                    kind: SpawnKind::Player,
                    speed,
                    spacing: 30.0,
                    velocity: [0.0, speed, 0.0],
                    forward: 1.05,
                },
                fps,
            );
            assert!(
                misses.iter().all(|miss| miss.abs() <= 1.5),
                "{fps} fps: {misses:?}"
            );
        }
        // A player's speed keeps the full damping: from its deltas alone it
        // slows, to 62.5 % after one call.
        let mut spawn = titanium(SpawnKind::Player, 1, [0.0, 20.0, 0.0], 0.0);
        carry(GameDialect::Titanium, &mut spawn, Duration::from_millis(50));
        assert!((spawn.velocity[1] - 12.5).abs() < 0.001, "{spawn:?}");
    }

    #[test]
    fn titanium_carries_an_eqemu_npc_on_its_animation_alone() {
        // Newer EQEmu sends no deltas; it moves an NPC 0.58 a second for
        // each unit of its animation value and reports it at each path
        // node, here 40 units apart.
        for animation in [10_u8, 20, 28] {
            for fps in [30, 60, 144] {
                let misses = walk_misses(
                    &Walk {
                        dialect: GameDialect::Titanium,
                        kind: SpawnKind::Npc,
                        speed: 0.58 * f32::from(animation),
                        spacing: 40.0,
                        velocity: [0.0; 3],
                        forward: f32::from(animation) / 40.0,
                    },
                    fps,
                );
                assert!(
                    misses.iter().all(|miss| miss.abs() <= 1.5),
                    "animation {animation}, {fps} fps: {misses:?}"
                );
            }
        }
    }

    #[test]
    fn titanium_keeps_level_races_and_has_no_stale_rule() {
        for race in [0x48, 0x49, 0x194] {
            assert_eq!(
                titanium(SpawnKind::Npc, race, [1.0, 2.0, 3.0], 0.0).velocity,
                [1.0, 2.0, 0.0]
            );
        }
        // 0x194 keeps its vertical motion on EQMac.
        let mac = Carried::reported(
            GameDialect::EqMac,
            SpawnKind::Npc,
            0x194,
            Position::default(),
            [1.0, 2.0, 3.0],
            0.0,
        );
        assert_eq!(mac.velocity, [1.0, 2.0, 3.0]);
        // Over 12 s without a report, a Titanium walker keeps walking.
        let mut spawn = titanium(SpawnKind::Npc, 1, [0.0; 3], 20.0);
        spawn.since_report = Duration::from_secs(13);
        carry(GameDialect::Titanium, &mut spawn, Duration::from_millis(50));
        assert!(spawn.position.y > 0.0, "{spawn:?}");
    }
}
