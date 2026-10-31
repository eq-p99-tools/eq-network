//! How each client generation carries other spawns between the server's
//! reports of them.
use crate::{world::Position, GameDialect};
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
    /// Time carried over from earlier frames that hasn't yet made a whole
    /// tick of the generation's step; zero where its step has no ticks.
    pub pending: Duration,
}

/// One of the `EQMac` program's ticks.
const EQMAC_TICK: Duration = Duration::from_millis(50);
/// Ticks in a second.
const EQMAC_TICKS_PER_SECOND: f32 = 20.0;
/// The most ticks one of its frames moves a spawn: the program holds a
/// frame's time in ticks to 3 (TAKP sitting 2 M1 line 283).
const EQMAC_MOST_TICKS: u32 = 3;
/// How much of a spawn's speed each tick keeps on normal ground.
const EQMAC_KEPT: f32 = 0.625;
/// How much of its forward input each tick adds on normal ground.
const EQMAC_ADDED: f32 = 0.8;
/// How long the program carries a spawn that keeps a forward input without
/// a report.
const EQMAC_STALE: Duration = Duration::from_secs(12);
/// The races whose reported vertical motion the `EQMac` program drops.
const EQMAC_LEVEL_RACES: [u32; 2] = [0x48, 0x49];

impl Carried {
    /// A spawn as a report puts it, of this race, in this generation.
    /// `EQMac` drops the vertical motion of races 0x48 and 0x49 (from the
    /// program, the TAKP speeds read of 10-31).
    #[must_use]
    pub fn reported(
        dialect: GameDialect,
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
            pending: Duration::ZERO,
        }
    }
}

/// Carries a spawn on for one frame of `elapsed`, as the generation's client
/// does between the server's reports. The spawn faces its position's
/// heading, as the last report gave it.
///
/// - `EQMac`, from the program (TAKP sitting 2 M1 and the TAKP speeds read of
///   10-31): each tick of 50 ms keeps 0.625 of the spawn's speed, adds 0.8
///   of its forward input along its heading and moves it by the result; on
///   the ground it settles at 42.67 units a second for each unit of input.
///   A frame steps the whole ticks its time makes, at most 3 (M1 line 283),
///   and leaves the rest of a tick for the next frame; time past the third
///   tick is dropped, as the program drops it. The step runs per whole tick
///   since keeping 0.625 a tick doesn't scale with a fraction of one. A
///   spawn whose last report is over 12 s old and that still has an input
///   stays where it is. Left out and unread: its heading rate, whose unit is
///   unread, so the heading stays as reported; gravity, the ground and other
///   media, which the host's floor handles; Z held for a report under a
///   global the read doesn't name; a report that marks the spawn (0x42,
///   0x42 and 0x12345678); and a player's spawn the program stops under a
///   flag of its own.
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

/// One frame of the `EQMac` program's step (see [`carry`]).
fn eqmac_step(spawn: &mut Carried, elapsed: Duration) {
    let time = spawn.pending.saturating_add(elapsed);
    let tick = EQMAC_TICK.as_nanos();
    spawn.pending =
        Duration::from_nanos(u64::try_from(time.as_nanos() % tick).expect("less than a tick"));
    let ticks = u32::try_from(time.as_nanos() / tick)
        .unwrap_or(u32::MAX)
        .min(EQMAC_MOST_TICKS);
    if ticks == 0 || (spawn.since_report > EQMAC_STALE && spawn.forward != 0.0) {
        return;
    }
    // Heading 0 faces +Y and a quarter turn (128) faces +X.
    let angle = spawn.position.heading / 512.0 * std::f32::consts::TAU;
    let push = spawn.forward * EQMAC_ADDED;
    let push = [angle.sin() * push, angle.cos() * push, 0.0];
    // The program's speed is in units a tick.
    let mut speed = spawn.velocity.map(|value| value / EQMAC_TICKS_PER_SECOND);
    for _ in 0..ticks {
        for (value, push) in speed.iter_mut().zip(push) {
            *value = *value * EQMAC_KEPT + push;
        }
        spawn.position.x += speed[0];
        spawn.position.y += speed[1];
        spawn.position.z += speed[2];
    }
    spawn.velocity = speed.map(|value| value * EQMAC_TICKS_PER_SECOND);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walker(heading: f32, forward: f32) -> Carried {
        Carried::reported(
            GameDialect::EqMac,
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
    fn eqmac_steps_whole_ticks_and_keeps_the_rest_for_the_next_frame() {
        let mut spawn = walker(0.0, 0.7);
        carry(GameDialect::EqMac, &mut spawn, Duration::from_millis(30));
        assert!(spawn.position.y.abs() < f32::EPSILON, "{spawn:?}");
        assert_eq!(spawn.pending, Duration::from_millis(30));
        carry(GameDialect::EqMac, &mut spawn, Duration::from_millis(30));
        assert!((spawn.position.y - 0.56).abs() < 0.001, "{spawn:?}");
        assert_eq!(spawn.pending, Duration::from_millis(10));
        // At 60 frames a second the spawn settles at the same speed as at
        // 20.
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
            1,
            Position::default(),
            [20.0, 0.0, -5.0],
            0.0,
        );
        carry(GameDialect::EqMac, &mut spawn, Duration::from_millis(50));
        assert!((spawn.velocity[0] - 12.5).abs() < 0.001, "{spawn:?}");
        assert!((spawn.position.x - 0.625).abs() < 0.001);
        assert!((spawn.velocity[2] + 3.125).abs() < 0.001);
        // A long frame moves at most three ticks, and drops the rest:
        // 0.56, then 0.91, then 1.12875.
        let mut spawn = walker(0.0, 0.7);
        carry(GameDialect::EqMac, &mut spawn, Duration::from_millis(1020));
        assert!((spawn.position.y - 2.598_75).abs() < 0.001, "{spawn:?}");
        assert_eq!(spawn.pending, Duration::from_millis(20));
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
            0x48,
            Position::default(),
            [1.0, 2.0, 3.0],
            0.0,
        );
        assert_eq!(boat.velocity, [1.0, 2.0, 0.0]);
    }

    #[test]
    fn titanium_carries_a_spawn_on_at_its_reported_velocity() {
        let mut spawn = Carried::reported(
            GameDialect::Titanium,
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
