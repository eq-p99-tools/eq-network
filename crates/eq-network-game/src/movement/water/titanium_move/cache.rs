//! Cache lifecycle and the ordinary wet wrapper's post-sweep branch.
//! No geometry, input authority or movement admission is established here.
use super::{tiny_height_fast_sink, tiny_swept_move};

/// One collision owner's floor-query cache lifetime, in world XYZ (Z up).
///
/// This counter advances at wrapper entry, not at a report or an integrator
/// fragment. Its floor query can reset it later in the same move. The owner
/// must also invalidate it on actual placement, relevant object motion and
/// headroom invalidation. A local refusal/unchanged endpoint never creates it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WetMoveCache {
    moves: u32,
    query: Option<[f32; 3]>,
}

impl WetMoveCache {
    /// Starts exactly one actual collision-wrapper call. The native counter
    /// is a wrapping unsigned word. Neither incrementing nor wrapping establishes
    /// admission, rest or a velocity reset.
    pub fn begin_move(&mut self) {
        self.moves = self.moves.wrapping_add(1);
    }

    /// Current count, including this move's increment and any later reset.
    #[must_use]
    pub const fn valid_moves(self) -> u32 {
        self.moves
    }

    /// An independently observed cache-invalidating event. Does not manufacture
    /// a replacement query or alter velocity, floor height or physical phase.
    pub fn invalidate(&mut self) {
        self.moves = 0;
    }

    /// Invalidates the query as well, e.g. when its immutable scene is retired.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Observes the actual floor-query point, after sweep/headroom processing.
    /// Returns whether the previous query remains valid. The caller supplies
    /// the independently evaluated ceiling/boat invalidation, not a movement
    /// label. Unknown state invalidates the count and returns `None`.
    ///
    /// Native comparisons are exact floating-point comparisons (signed zeros
    /// compare equal), not a displacement tolerance. The stored Z is refreshed
    /// only when the query invalidates. Thus repeated small descent eventually
    /// invalidates once it exceeds 0.001 from the stored query.
    #[allow(
        clippy::float_cmp,
        reason = "Native cache uses exact float comparisons"
    )]
    pub fn floor_query(
        &mut self,
        point: [f32; 3],
        ceiling_or_boat_invalidated: Option<bool>,
    ) -> Option<bool> {
        if !point.into_iter().all(f32::is_finite) {
            self.clear();
            return None;
        }
        let Some(external) = ceiling_or_boat_invalidated else {
            self.clear();
            return None;
        };
        let retained = !external
            && self.query.is_some_and(|old| {
                point[0] == old[0] && point[1] == old[1] && point[2] + 0.001 >= old[2]
            });
        if !retained {
            self.invalidate();
            self.query = Some(point);
        }
        Some(retained)
    }
}

/// Action after an ordinary, non-carrier wet sweep and the refusal gate.
/// This is a branch decision, never evidence that a requested move is legal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WetPostSweep {
    /// Restore the native saved-good pose, zero VZ only and skip floor refresh.
    Revert,
    /// Refresh the floor at the actual new XY, then apply the Z-only floor
    /// clamp and its vertical refusal probes. The fast-sink clamp precedes it.
    RefreshFloor {
        /// Clamp VZ to -1 native unit/tick, retaining horizontal carry and phase.
        clamp_fast_sink: bool,
    },
}

/// Models only the ordinary wet post-sweep join. Call after the real sweep and
/// headroom checks, with the then-current count; outer extents, carrier, mount
/// and sweep eligibility must already be resolved. The pre-test exception does
/// not bypass this separate branch. `achieved` is new minus start in XYZ.
#[must_use]
pub fn wet_post_sweep(
    achieved: [f32; 3],
    cache_valid_moves: Option<u32>,
    refused: Option<bool>,
    vertical_velocity: f32,
) -> Option<WetPostSweep> {
    if refused? {
        return Some(WetPostSweep::Revert);
    }
    if tiny_swept_move(achieved, cache_valid_moves)? {
        Some(WetPostSweep::Revert)
    } else {
        Some(WetPostSweep::RefreshFloor {
            clamp_fast_sink: tiny_height_fast_sink(achieved[2], vertical_velocity)?,
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "Native exact-coordinate regression controls"
)]
mod tests {
    use super::*;
    use crate::movement::water::{WetInput, WetVelocity, LIQUID_5, WET_FRAME_SECONDS};

    #[test]
    fn moving_queries_never_mature_even_below_axis_threshold() {
        for axis in [0, 1] {
            let mut cache = WetMoveCache::default();
            for step in 0_u16..100 {
                cache.begin_move();
                assert_eq!(cache.valid_moves(), 1);
                let mut point = [0.0; 3];
                point[axis] = f32::from(step) * 0.0005;
                assert_eq!(cache.floor_query(point, Some(false)), Some(false));
                assert_eq!(cache.valid_moves(), 0);
            }
        }
    }

    #[test]
    fn stable_query_matures_then_a_real_move_resets_on_same_pass() {
        let mut cache = WetMoveCache::default();
        assert_eq!(cache.floor_query([0.0; 3], Some(false)), Some(false));
        for count in 1..=3 {
            cache.begin_move();
            assert_eq!(cache.floor_query([0.0; 3], Some(false)), Some(true));
            assert_eq!(cache.valid_moves(), count);
        }
        cache.begin_move();
        assert_eq!(
            cache.floor_query([0.02, 0.0, 0.0], Some(false)),
            Some(false)
        );
        assert_eq!(cache.valid_moves(), 0);
    }

    #[test]
    fn descent_compares_to_stored_query_and_unknown_cannot_retain_cache() {
        let mut cache = WetMoveCache::default();
        cache.floor_query([0.0; 3], Some(false));
        cache.begin_move();
        assert_eq!(
            cache.floor_query([0.0, 0.0, -0.0006], Some(false)),
            Some(true)
        );
        cache.begin_move();
        assert_eq!(
            cache.floor_query([0.0, 0.0, -0.0012], Some(false)),
            Some(false)
        );
        assert_eq!(cache.valid_moves(), 0);
        cache.begin_move();
        assert_eq!(cache.floor_query([0.0; 3], None), None);
        assert_eq!(cache.valid_moves(), 0);
        cache.begin_move();
        assert_eq!(cache.floor_query([0.0; 3], Some(false)), Some(false));
        cache.begin_move();
        assert_eq!(cache.floor_query([0.0; 3], Some(true)), Some(false));
        assert_eq!(cache.valid_moves(), 0);
    }

    #[test]
    fn mature_zero_sweep_reverts_but_young_cache_refreshes_floor() {
        assert_eq!(
            wet_post_sweep([0.0; 3], Some(3), Some(false), -2.0),
            Some(WetPostSweep::Revert)
        );
        assert_eq!(
            wet_post_sweep([0.0; 3], Some(2), Some(false), -2.0),
            Some(WetPostSweep::RefreshFloor {
                clamp_fast_sink: true
            })
        );
        assert_eq!(
            wet_post_sweep([0.1, 0.0, -0.002], Some(3), Some(false), -2.0),
            Some(WetPostSweep::RefreshFloor {
                clamp_fast_sink: false
            })
        );
        assert_eq!(
            wet_post_sweep([0.1, 0.0, 0.002], Some(3), Some(false), -2.0),
            Some(WetPostSweep::RefreshFloor {
                clamp_fast_sink: false
            })
        );
        assert_eq!(
            wet_post_sweep([1.0; 3], None, Some(true), -2.0),
            Some(WetPostSweep::Revert)
        );
        assert_eq!(wet_post_sweep([0.0; 3], Some(3), None, -2.0), None);
    }

    #[test]
    fn vertical_zero_preserves_horizontal_carry_and_fractional_clock() {
        let mut wet = WetVelocity::from_units_per_second([2.0, -3.0], -8.0);
        wet.advance(LIQUID_5, WetInput::default(), WET_FRAME_SECONDS * 0.4);
        let before = wet;
        wet.zero_vertical();
        assert_eq!(wet.units_per_second().0, before.units_per_second().0);
        assert_eq!(wet.units_per_second().1.to_bits(), 0.0_f32.to_bits());
        assert_eq!(wet.frame_phase(), before.frame_phase());
        // No new gravity/damping step before the already-started frame ends.
        let (horizontal, vertical) =
            wet.advance(LIQUID_5, WetInput::default(), WET_FRAME_SECONDS * 0.2);
        assert!(horizontal[0] > 0.0 && horizontal[1] < 0.0);
        assert_eq!(vertical.to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn sink_clamp_uses_absolute_height_and_retains_clock_and_carry() {
        for achieved in [-0.002, 0.002] {
            let mut wet = WetVelocity::from_units_per_second([2.0, 3.0], -50.0);
            let before = wet;
            wet.clamp_blocked_sink(achieved);
            assert_eq!(wet, before);
        }
        let mut wet = WetVelocity::from_units_per_second([2.0, 3.0], -50.0);
        let before = wet;
        wet.clamp_blocked_sink(0.0005);
        assert_eq!(wet.units_per_second().0, before.units_per_second().0);
        assert_eq!(wet.frame_phase(), before.frame_phase());
        assert_eq!(wet.units_per_second().1.to_bits(), (-20.0_f32).to_bits());
    }
}
