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

/// The `EQMac` program's speed unit, units per 50 ms, in a second.
const EQMAC_TICKS_PER_SECOND: f32 = 20.0;
/// The most 50 ms spans one of its steps moves a spawn by: the program
/// holds a frame's time, in those spans (elapsed ms × 0.02), to 3 (TAKP
/// sitting 2 M1 line 283; TAKP read R1, 0x5e8510 and 0x5e8594).
const EQMAC_MOST_SPANS: f32 = 3.0;
/// How much of a spawn's speed each tick keeps on normal ground.
const EQMAC_KEPT: f32 = 0.625;
/// How much of a moving NPC's horizontal speed each tick keeps: half of
/// [`EQMAC_KEPT`] (TAKP read R1 of 10-31: the step 0x54d964 halves it at
/// 0x54e1d7 for a spawn of type 1 whose horizontal speed isn't zero).
const EQMAC_NPC_KEPT: f32 = EQMAC_KEPT * 0.5;
/// How much of its forward input each tick adds on normal ground.
const EQMAC_ADDED: f32 = 0.8;
/// How long the program carries a spawn that keeps a forward input without
/// a report.
const EQMAC_STALE: Duration = Duration::from_secs(12);
/// The races whose reported vertical motion the `EQMac` program drops.
const EQMAC_LEVEL_RACES: [u32; 2] = [0x48, 0x49];

impl Carried {
    /// A spawn as a report puts it, of this kind and race, in this
    /// generation. A report sets the position, heading, input and speed
    /// outright, with no blending (TAKP read R1 (b)). `EQMac` drops the
    /// vertical motion of races 0x48 and 0x49 (from the program, the TAKP
    /// speeds read of 10-31).
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
        if dialect == GameDialect::EqMac && EQMAC_LEVEL_RACES.contains(&race) {
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
/// - `EQMac`, from the program (TAKP sitting 2 M1, the TAKP speeds read of
///   10-31 and TAKP read R1 of 10-31): the frame loop calls the step once a
///   frame for each spawn (speeds read), and each call keeps 0.625 of the
///   spawn's speed, adds 0.8 of its forward input along its heading, and
///   moves it by the result times the frame's time in 50 ms spans, at most
///   3 (R1: the speed update runs once a call and isn't scaled by time; the
///   move is v × min(ms × 0.02, 3.0)). The program's speed is in units per
///   50 ms, which is what M1's "20 a second" names, not a call rate. On the
///   ground a player's spawn settles at 42.67 units a second for each unit
///   of input, whatever the frame rate; only the climb to it after a report
///   goes by frames.
///
///   An NPC keeps only half as much of its horizontal speed each call
///   while that speed isn't zero (R1, 0x54e1d7: type 1, D × 0.5), so it
///   settles at 23.27 units a second for each unit of input, the server's
///   own pace (2.327 for each unit of the speed byte). Its vertical speed
///   keeps 0.625 as before. That every NPC is halved is inferred: the
///   program skips it for an NPC whose physics block links it to something
///   (block+0x198, unread, probably a mount), which this client doesn't
///   track (checklist 6.316).
///
///   A spawn whose last report is over 12 s old and that still has an
///   input stays where it is. Left out: its heading rate, whose unit is
///   unread, so the heading stays as reported; gravity, the ground and
///   other media, which the host's floor handles; Z held within 5 units of
///   a report for a spawn whose +0x10a is 2 (R1 (b); +0x10a unread,
///   checklist 6.317) and for races 0x48 and 0x49 under a global the read
///   doesn't name (checklist 6.318); a report that marks the spawn (0x42,
///   0x42 and 0x12345678); and a player's spawn whose report the program
///   applies only as a stop, under a flag of its own (R1 (b), block+0x190;
///   checklist 6.319).
/// - Titanium: on at the reported velocity for the whole frame; its client's
///   own step is unread.
pub fn carry(dialect: GameDialect, spawn: &mut Carried, elapsed: Duration) {
    spawn.since_report = spawn.since_report.saturating_add(elapsed);
    if dialect == GameDialect::EqMac {
        eqmac_step(spawn, elapsed);
    } else {
        let seconds = elapsed.as_secs_f32();
        spawn.position.x += spawn.velocity[0] * seconds;
        spawn.position.y += spawn.velocity[1] * seconds;
        spawn.position.z += spawn.velocity[2] * seconds;
    }
}

/// One call of the `EQMac` program's step (see [`carry`]).
fn eqmac_step(spawn: &mut Carried, elapsed: Duration) {
    if spawn.since_report > EQMAC_STALE && spawn.forward != 0.0 {
        return;
    }
    let spans = (elapsed.as_secs_f32() * EQMAC_TICKS_PER_SECOND).min(EQMAC_MOST_SPANS);
    // Heading 0 faces +Y and a quarter turn (128) faces +X.
    let angle = spawn.position.heading / 512.0 * std::f32::consts::TAU;
    let push = spawn.forward * EQMAC_ADDED;
    let push = [angle.sin() * push, angle.cos() * push, 0.0];
    // The program's speed is in units per 50 ms.
    let mut speed = spawn.velocity.map(|value| value / EQMAC_TICKS_PER_SECOND);
    // The program tests the horizontal speed before it keeps any.
    #[allow(clippy::float_cmp, reason = "the program tests for exactly zero")]
    let horizontal = if spawn.kind == SpawnKind::Npc && (speed[0] != 0.0 || speed[1] != 0.0) {
        EQMAC_NPC_KEPT
    } else {
        EQMAC_KEPT
    };
    for (index, (value, push)) in speed.iter_mut().zip(push).enumerate() {
        let kept = if index < 2 { horizontal } else { EQMAC_KEPT };
        *value = *value * kept + push;
    }
    spawn.position.x += speed[0] * spans;
    spawn.position.y += speed[1] * spans;
    spawn.position.z += speed[2] * spans;
    spawn.velocity = speed.map(|value| value * EQMAC_TICKS_PER_SECOND);
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

    /// Walks an NPC as the TAKP server moves one and reports it, carrying
    /// it at `fps` frames a second between reports, and gives how far the
    /// carried spawn is from each report as it lands. The server moves it
    /// 2.32 units a second for each unit of the speed byte (`EQMacEmu`
    /// `zone/mob_movement_manager.cpp`, `MoveToCommand`: frame time ×
    /// speed × 0.4 × 1.45, with the byte at a quarter of the speed in
    /// `zone/waypoints.cpp`, `SetRunAnimation`), and reports it each 10
    /// units it moves, with the forward input and no velocity.
    fn misses(kind: SpawnKind, byte: u8, fps: u32) -> Vec<f32> {
        let speed = 2.32 * f32::from(byte);
        let frame = Duration::from_secs(1) / fps;
        let between = 10.0 / speed;
        let report = |y: f32| {
            Carried::reported(
                GameDialect::EqMac,
                kind,
                1,
                Position {
                    x: 0.0,
                    y,
                    z: 0.0,
                    heading: 0.0,
                },
                [0.0; 3],
                f32::from(byte) / 10.0,
            )
        };
        let mut spawn = report(0.0);
        let mut misses = Vec::new();
        for leg in 1..=12_u8 {
            let next = 10.0 * f32::from(leg);
            while (spawn.since_report + frame).as_secs_f32() <= between {
                carry(GameDialect::EqMac, &mut spawn, frame);
            }
            misses.push(spawn.position.y - next);
            spawn = report(next);
        }
        misses
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

    #[test]
    fn titanium_carries_a_spawn_on_at_its_reported_velocity() {
        let mut spawn = Carried::reported(
            GameDialect::Titanium,
            SpawnKind::Npc,
            0x48,
            Position::default(),
            [10.0, -4.0, 2.0],
            0.0,
        );
        carry(
            GameDialect::Titanium,
            &mut spawn,
            Duration::from_millis(500),
        );
        assert_eq!(
            (spawn.position.x, spawn.position.y, spawn.position.z),
            (5.0, -2.0, 1.0)
        );
        assert_eq!(spawn.velocity, [10.0, -4.0, 2.0]);
    }
}
