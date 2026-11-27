//! Numerical criteria for Titanium's wet collision wrapper.
//!
//! These helpers are not wired into movement or replay admission. A future
//! owner must retain the actual physical-step boundary, cache-valid move lifecycle,
//! exception state and surrounding native branches. They never establish an
//! anchor, certify caller state, alter velocity, or grant movement. In particular,
//! a network report batch or a tiny integrator tail is not a native move.

mod cache;
pub use cache::{wet_post_sweep, WetMoveCache, WetPostSweep};

/// Every axis must be strictly below this distance to discard a target.
pub const STATIONARY_AXIS_DISTANCE: f32 = 0.01;
/// Height limit of the tiny swept-move and blocked-sink numerical criteria.
pub const NEGLIGIBLE_HEIGHT: f32 = 0.001;
/// A tiny swept move must have strictly less squared travel than this.
pub const NEGLIGIBLE_SQUARED_TRAVEL: f32 = 1e-6;
/// Minimum consecutive move count with valid floor/headroom cache state.
/// The collision wrapper increments it before testing; queries can reset it
/// on that same move. This is neither elapsed frames nor the wire counter.
pub const MATURE_CACHE_MOVES: u32 = 3;
/// Native fast-sink boundary, in world units per program tick, not per second.
pub const BLOCKED_SINK_VELOCITY: f32 = -1.0;

/// Tests the whole-target, pre-sweep stationary criterion.
///
/// `delta` is the actual target minus last placed position after the preceding
/// native snap, in one consistent world frame. It is not the replay error.
/// `cache_valid_moves` must be observed after the native pre-test increment.
/// Continuous XY changes can reset it every move during the later floor query.
/// `bypass_stationary` is the observed native exception predicate, not a guessed
/// mapping from a general movement option. The whole target is selected only
/// when every axis qualifies; individual small axes are never rounded away.
///
/// Returns `None` for missing observations or non-finite deltas. Neither
/// `Some(true)` nor `Some(false)` is a movement-admission decision.
#[must_use]
pub fn stationary_target(
    delta: [f32; 3],
    cache_valid_moves: Option<u32>,
    bypass_stationary: Option<bool>,
) -> Option<bool> {
    let moves = cache_valid_moves?;
    let bypass = bypass_stationary?;
    delta.into_iter().all(f32::is_finite).then(|| {
        moves >= MATURE_CACHE_MOVES
            && !bypass
            && delta
                .into_iter()
                .all(|axis| axis.abs() < STATIONARY_AXIS_DISTANCE)
    })
}

/// Tests the numeric/cache part of the separate post-sweep tiny-move rule.
///
/// The caller must independently establish the ordinary native branch and its
/// carrier/refusal conditions. This does not copy the pre-sweep exception gate,
/// choose the saved-good position, revert a pose or zero velocity. `achieved`
/// uses world XYZ with Z up, after the real sweep, not a report batch.
///
/// Returns `None` for unknown cache state or non-finite/overflowed geometry.
#[must_use]
pub fn tiny_swept_move(achieved: [f32; 3], cache_valid_moves: Option<u32>) -> Option<bool> {
    let moves = cache_valid_moves?;
    if !achieved.into_iter().all(f32::is_finite) {
        return None;
    }
    let squared = achieved.into_iter().map(|axis| axis * axis).sum::<f32>();
    squared.is_finite().then_some(
        moves >= MATURE_CACHE_MOVES
            && achieved[2].abs() <= NEGLIGIBLE_HEIGHT
            && squared < NEGLIGIBLE_SQUARED_TRAVEL,
    )
}

/// Tests only the achieved-height and native-velocity part of the sink clamp.
///
/// The actual caller still needs the native not-refused/non-carrier and
/// stationary/cache branch gates. A selected rule clamps vertical velocity
/// only; it never discards horizontal carry or resets the integrator phase.
/// Returns `None` for non-finite observations.
#[must_use]
pub fn tiny_height_fast_sink(achieved_height: f32, vertical_velocity: f32) -> Option<bool> {
    (achieved_height.is_finite() && vertical_velocity.is_finite()).then_some(
        achieved_height.abs() < NEGLIGIBLE_HEIGHT && vertical_velocity < BLOCKED_SINK_VELOCITY,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_requires_all_three_strict_axis_bounds() {
        let below = f32::from_bits(STATIONARY_AXIS_DISTANCE.to_bits() - 1);
        let above = f32::from_bits(STATIONARY_AXIS_DISTANCE.to_bits() + 1);
        for sign in [-1.0, 1.0] {
            assert_eq!(
                stationary_target([sign * below; 3], Some(3), Some(false)),
                Some(true)
            );
            for axis in 0..3 {
                for boundary in [STATIONARY_AXIS_DISTANCE, above] {
                    let mut delta = [0.0; 3];
                    delta[axis] = sign * boundary;
                    assert_eq!(stationary_target(delta, Some(3), Some(false)), Some(false));
                }
            }
        }
        // This diagonal is longer than0.01, but each axis qualifies.
        assert_eq!(
            stationary_target([0.0099; 3], Some(3), Some(false)),
            Some(true)
        );
        // A normal stroke must not be erased because one axis is small.
        assert_eq!(
            stationary_target([0.07, 0.08, -0.001], Some(3), Some(false)),
            Some(false)
        );
    }

    #[test]
    fn target_requires_known_cache_and_exception_observations() {
        assert_eq!(
            stationary_target([0.0; 3], Some(2), Some(false)),
            Some(false)
        );
        assert_eq!(
            stationary_target([0.0; 3], Some(3), Some(false)),
            Some(true)
        );
        assert_eq!(
            stationary_target([0.0; 3], Some(3), Some(true)),
            Some(false)
        );
        assert_eq!(stationary_target([0.0; 3], None, Some(false)), None);
        assert_eq!(stationary_target([0.0; 3], Some(3), None), None);
        // With a genuinely observed immature cache, continuous small XY
        // motion does not qualify for either stationary numerical rule.
        assert_eq!(
            stationary_target([0.0005, 0.0, 0.0], Some(1), Some(false)),
            Some(false)
        );
        assert_eq!(tiny_swept_move([0.0005, 0.0, 0.0], Some(1)), Some(false));
    }

    #[test]
    fn swept_rule_keeps_its_own_limits_and_cache_gate() {
        assert_eq!(tiny_swept_move([0.0005; 3], Some(3)), Some(true));
        assert_eq!(tiny_swept_move([0.0005; 3], Some(2)), Some(false));
        assert_eq!(tiny_swept_move([0.0; 3], None), None);
        let below = f32::from_bits(NEGLIGIBLE_HEIGHT.to_bits() - 1);
        let above = f32::from_bits(NEGLIGIBLE_HEIGHT.to_bits() + 1);
        for sign in [-1.0, 1.0] {
            for axis in 0..3 {
                let mut delta = [0.0; 3];
                delta[axis] = sign * below;
                assert_eq!(tiny_swept_move(delta, Some(3)), Some(true));
                // At the representable0.001 boundary the squared cutoff
                // is reached or exceeded, so strict squared travel fails.
                for boundary in [NEGLIGIBLE_HEIGHT, above] {
                    delta[axis] = sign * boundary;
                    assert_eq!(tiny_swept_move(delta, Some(3)), Some(false));
                }
            }
        }
    }

    #[test]
    fn sink_criterion_uses_absolute_height_and_strict_velocity() {
        for sign in [-1.0, 1.0] {
            assert_eq!(tiny_height_fast_sink(sign * 0.0009, -2.0), Some(true));
            assert_eq!(
                tiny_height_fast_sink(sign * NEGLIGIBLE_HEIGHT, -2.0),
                Some(false)
            );
            assert_eq!(tiny_height_fast_sink(sign * 0.002, -2.0), Some(false));
        }
        assert_eq!(
            tiny_height_fast_sink(0.0, BLOCKED_SINK_VELOCITY),
            Some(false)
        );
        assert_eq!(
            tiny_height_fast_sink(0.0, f32::from_bits((-1.0_f32).to_bits() + 1)),
            Some(true)
        );
        assert_eq!(tiny_height_fast_sink(0.0, 2.0), Some(false));
    }

    #[test]
    fn nonfinite_or_overflowed_observations_are_unknown() {
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for axis in 0..3 {
                let mut delta = [0.0; 3];
                delta[axis] = invalid;
                assert_eq!(stationary_target(delta, Some(3), Some(false)), None);
                assert_eq!(tiny_swept_move(delta, Some(3)), None);
            }
            assert_eq!(tiny_height_fast_sink(invalid, -2.0), None);
            assert_eq!(tiny_height_fast_sink(0.0, invalid), None);
        }
        assert_eq!(tiny_swept_move([f32::MAX; 3], Some(3)), None);
    }
}
