//! Independent ground movement components and their collision endpoint budget.
use super::MovementMode;
use crate::world::Position;

/// The requested longitudinal mode while taking a sidestep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagonalAlong {
    /// Running forward.
    Run,
    /// Walking forward.
    Walk,
    /// Backing up. The run/walk toggle still selects the lateral rate.
    Backward {
        /// Whether walking is selected.
        walking: bool,
    },
}

impl DiagonalAlong {
    /// Whether the walk toggle selects the lateral rate.
    #[must_use]
    pub const fn walking(self) -> bool {
        match self {
            Self::Run => false,
            Self::Walk => true,
            Self::Backward { walking } => walking,
        }
    }

    pub(super) const fn mode(self) -> MovementMode {
        match self {
            Self::Run => MovementMode::Forward,
            Self::Walk => MovementMode::Walk,
            Self::Backward { .. } => MovementMode::Backward,
        }
    }
}

/// Which side the player holds while moving forward or back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrafeSide {
    /// Sidestep left.
    Left,
    /// Sidestep right.
    Right,
}

/// A combined ground input, tied to the previously accepted body heading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroundedDiagonal {
    /// The longitudinal input that supplies the speed and animation.
    pub along: DiagonalAlong,
    /// The independent lateral input.
    pub side: StrafeSide,
}

impl GroundedDiagonal {
    /// Validates the collision-resolved position against both signed budgets.
    /// A slide can shorten either component. It cannot use one axis's budget
    /// to move faster on the other, reverse an axis, or change the heading.
    #[must_use]
    #[allow(clippy::float_cmp)] // This mode keeps the previously accepted heading exactly.
    pub fn admits(
        self,
        from: Position,
        to: Position,
        along: f32,
        lateral: f32,
        seconds: f32,
    ) -> bool {
        const ROUNDING: f32 = 0.01;
        if to.heading != from.heading {
            return false;
        }
        let (sin, cos) = (from.heading / 512.0 * std::f32::consts::TAU).sin_cos();
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        let forward = dx * sin + dy * cos;
        let right = -dx * cos + dy * sin;
        let advance = if matches!(self.along, DiagonalAlong::Backward { .. }) {
            -forward
        } else {
            forward
        };
        let sidestep = match self.side {
            StrafeSide::Left => -right,
            StrafeSide::Right => right,
        };
        (-ROUNDING..=along * seconds + ROUNDING).contains(&advance)
            && (-ROUNDING..=lateral * seconds + ROUNDING).contains(&sidestep)
    }
}

/// Lateral world speeds while longitudinal input is active. Independent
/// of the grant for strafing by itself. None leaves that mode uncalibrated.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiagonalRates {
    /// Lateral speed with running selected.
    pub run: Option<f32>,
    /// Lateral speed with walking selected.
    pub walk: Option<f32>,
}

impl DiagonalRates {
    pub(crate) const fn is_unavailable(&self) -> bool {
        self.run.is_none() && self.walk.is_none()
    }

    /// The lateral calibration matching the request's run/walk toggle.
    #[must_use]
    pub const fn for_mode(self, diagonal: GroundedDiagonal) -> Option<f32> {
        if diagonal.along.walking() {
            self.walk
        } else {
            self.run
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotated_body_budget_allows_a_shortened_slide_without_axis_credit_transfer() {
        let diagonal = GroundedDiagonal {
            along: DiagonalAlong::Run,
            side: StrafeSide::Right,
        };
        let from = Position {
            heading: 128.0,
            ..Position::default()
        };
        let slide = Position {
            x: 0.1,
            y: 0.3,
            ..from
        };
        assert!(diagonal.admits(from, slide, 6.0, 3.0, 0.1));
        let shifted = Position {
            x: 0.1,
            y: 0.4,
            ..from
        };
        assert!(!diagonal.admits(from, shifted, 6.0, 3.0, 0.1));
        let reverse = Position {
            x: -0.2,
            y: 0.1,
            ..from
        };
        assert!(!diagonal.admits(from, reverse, 6.0, 3.0, 0.1));
    }
}
