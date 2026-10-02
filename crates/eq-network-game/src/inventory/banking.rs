//! Conservative client policy for classic personal banking; not a server authorization.
//! Reference: <https://github.com/EQEmu/Server/blob/master/zone/entity.cpp>
//! (`GetClosestBanker`); client proximity is intentionally stricter than that server check.
use crate::world::{Position, SpawnKind, SpawnState};

/// Whether a visible banker is within twenty EQ units in three dimensions.
///
/// `EQEmu` identifies bankers by NPC class 40. Its server distance limit is larger;
/// this deliberately tighter client policy is not a claim about P99's exact limit.
/// Callers must re-evaluate against current admission state before every bank move.
#[must_use]
pub fn banker_in_range(position: Position, spawn: &SpawnState) -> bool {
    if spawn.kind != SpawnKind::Npc || spawn.class != Some(40) || spawn.invisible {
        return false;
    }
    let distance = (position.x - spawn.position.x)
        .hypot(position.y - spawn.position.y)
        .hypot(position.z - spawn.position.z);
    distance.is_finite() && distance <= 20.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_identity_distance_and_visibility_are_all_required() {
        let mut banker = SpawnState {
            class: Some(40),
            spawn_id: 9,
            name: "Synthetic banker".into(),
            kind: SpawnKind::Npc,
            race: 1,
            gender: 0,
            position: Position::default(),
            velocity: [0.0; 3],
            size: 6.0,
            invisible: false,
            appearance: crate::appearance::Appearance::default(),
            level: 0,
            listing: crate::listing::Listing::default(),
            pet_owner: None,
        };
        let origin = Position::default();
        assert!(banker_in_range(origin, &banker));
        banker.position.x = 20.0;
        assert!(banker_in_range(origin, &banker));
        banker.position.z = 1.0;
        assert!(!banker_in_range(origin, &banker));
        banker.position = origin;
        for class in [None, Some(1), Some(66), Some(255)] {
            banker.class = class;
            assert!(!banker_in_range(origin, &banker));
        }
        banker.class = Some(40);
        banker.kind = SpawnKind::Player;
        assert!(!banker_in_range(origin, &banker));
        banker.kind = SpawnKind::Npc;
        banker.invisible = true;
        assert!(!banker_in_range(origin, &banker));
        banker.invisible = false;
        banker.position.x = f32::NAN;
        assert!(!banker_in_range(origin, &banker));
    }
}
