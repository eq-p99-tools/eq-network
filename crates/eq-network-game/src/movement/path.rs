//! Bounded, observed positions within one ordinary movement proposal.
use crate::world::Position;
use anyhow::{ensure, Result};
use std::time::Duration;

/// One observed position, relative to the start of its movement sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovementPoint {
    /// Time within the sample; this does not schedule a packet.
    pub elapsed: Duration,
    /// Actor origin in the same world frame as the request endpoint.
    pub position: Position,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_path_rejects_ambiguous_or_unbounded_vertices() {
        let point = |ms, z| MovementPoint {
            elapsed: Duration::from_millis(ms),
            position: Position {
                z,
                ..Position::default()
            },
        };
        for points in [
            vec![],
            vec![point(0, 0.0)],
            vec![point(1, 0.0), point(100, 1.0)],
            vec![point(0, 0.0), point(0, 1.0)],
            vec![point(0, 0.0), point(251, 1.0)],
            vec![point(0, 0.0), point(100, f32::NAN)],
        ] {
            assert!(MovementPath::new(points).is_err());
        }
        let path = MovementPath::new(vec![point(0, 0.0), point(50, 2.0), point(100, 0.0)]).unwrap();
        assert_eq!(path.at(Duration::from_millis(50)).z, 2.0);
        assert_eq!(path.at(Duration::from_millis(25)).z, 1.0);
        assert_eq!(path.at(Duration::from_secs(1)).z, 0.0);
    }
}

/// Collision-resolved vertices supplied by the host for one ordinary sample.
///
/// This is recording data, not permission to move or a replacement integrator.
/// The session checks it only after the ordinary endpoint passes its guard.
#[derive(Clone, Debug, PartialEq)]
pub struct MovementPath(Vec<MovementPoint>);

impl MovementPath {
    /// Retains two to eight finite, strictly ordered vertices over at most 250 ms.
    ///
    /// # Errors
    /// Rejects missing endpoints, duplicate times, non-finite coordinates or
    /// a span outside an ordinary proposal's existing maximum interval.
    pub fn new(points: Vec<MovementPoint>) -> Result<Self> {
        ensure!(
            (2..=8).contains(&points.len()),
            "invalid movement path length"
        );
        ensure!(points[0].elapsed.is_zero(), "movement path has no start");
        ensure!(
            points
                .iter()
                .all(|point| super::valid_position(point.position)),
            "non-finite movement path"
        );
        ensure!(
            points
                .windows(2)
                .all(|pair| pair[0].elapsed < pair[1].elapsed),
            "unordered movement path"
        );
        ensure!(
            points
                .last()
                .is_some_and(|point| point.elapsed <= Duration::from_millis(250)),
            "movement path exceeds proposal span"
        );
        Ok(Self(points))
    }

    /// The measured vertices, including both endpoints.
    pub fn points(&self) -> &[MovementPoint] {
        &self.0
    }

    /// The original sample's duration, never an extra movement allowance.
    pub fn duration(&self) -> Duration {
        self.0.last().expect("validated endpoints").elapsed
    }

    /// Interpolates only between retained vertices, holding the endpoints
    /// outside their interval. No new arc or minimum airborne hold is fitted.
    pub fn at(&self, elapsed: Duration) -> Position {
        for pair in self.0.windows(2) {
            if elapsed <= pair[1].elapsed {
                let share = elapsed.saturating_sub(pair[0].elapsed).as_secs_f32()
                    / (pair[1].elapsed - pair[0].elapsed).as_secs_f32();
                let (a, b) = (pair[0].position, pair[1].position);
                return Position {
                    x: a.x + (b.x - a.x) * share,
                    y: a.y + (b.y - a.y) * share,
                    z: a.z + (b.z - a.z) * share,
                    heading: b.heading,
                };
            }
        }
        self.0.last().expect("validated endpoints").position
    }
}
