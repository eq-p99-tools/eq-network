//! How far each client generation's program lets the player reach what
//! they use: doors, ground items, world containers, merchants and the one
//! they bandage. A reach the program's read leaves unknown keeps today's.
//!
//! Sources: Titanium sitting 8a (T6 doors, T8 Bind Wound) and the TAKP read
//! of 10-30 (T6 targeting and reach, T8 Bind Wound). Neither generation's
//! door reach counts its two size terms here (checklist 6.311), so a door
//! is reached no farther than the program's own reach.
use crate::world::Position;
use std::time::Duration;

/// A reach: how far, whether height counts, and whether the distance
/// itself is still within.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reach {
    /// The distance, in world units.
    pub distance: f32,
    /// Whether height is left out (a 2-D distance).
    pub flat: bool,
    /// Whether exactly `distance` is still within.
    pub inclusive: bool,
}

impl Reach {
    /// Whether `to` is within reach of `from`.
    #[must_use]
    pub fn admits(self, from: Position, to: Position) -> bool {
        let flat = (to.x - from.x).hypot(to.y - from.y);
        let distance = if self.flat {
            flat
        } else {
            flat.hypot(to.z - from.z)
        };
        distance.is_finite()
            && if self.inclusive {
                distance <= self.distance
            } else {
                distance < self.distance
            }
    }
}

/// A generation's reaches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reaches {
    /// Clicking a door.
    pub doors: Reach,
    /// Picking an item up or opening a world container.
    pub objects: Reach,
    /// Opening a merchant's window, where it is read.
    pub merchants: Option<Reach>,
    /// How far an open world container stays open, checked every
    /// [`CONTAINER_CHECK`], where it is read.
    pub containers: Option<Reach>,
    /// Bandaging another player.
    pub bandage: Reach,
    /// The program's string for a player too far to bandage.
    pub too_far_to_bandage: u32,
}

/// How often the TAKP program checks that an open world container is still
/// within reach: more than 500 ms apart (0x41647a; TAKP read T6).
pub const CONTAINER_CHECK: Duration = Duration::from_millis(500);

/// The Titanium program's reaches.
pub const TITANIUM: Reaches = Reaches {
    // 20 (0x644368) plus two size terms, at most (s8a T6); the distance
    // routine's dimension is unread, so height counts as before.
    doors: Reach {
        distance: 20.0,
        flat: false,
        inclusive: true,
    },
    // Unread: today's.
    objects: Reach {
        distance: 20.0,
        flat: false,
        inclusive: true,
    },
    // Unread (checklist 6.312): no reach of the client's own.
    merchants: None,
    containers: None,
    // Refused when greater than 15 (0x640e28; s8a T8), with string 12418;
    // the distance routine's dimension is unread, so height counts.
    bandage: Reach {
        distance: 15.0,
        flat: false,
        inclusive: true,
    },
    too_far_to_bandage: 12418,
};

/// The TAKP program's reaches (TAKP read T6 and T8).
pub const EQMAC: Reaches = Reaches {
    // A 2-D distance of at most 20 (0x5e3840) plus two size terms.
    doors: Reach {
        distance: 20.0,
        flat: true,
        inclusive: true,
    },
    // A 3-D distance under 20, for ground items and world containers alike.
    objects: Reach {
        distance: 20.0,
        flat: false,
        inclusive: false,
    },
    // A 3-D distance of at most 20, for merchants, bankers, guildmasters
    // and trader shops alike.
    merchants: Some(Reach {
        distance: 20.0,
        flat: false,
        inclusive: true,
    }),
    // Closed when beyond 20, in 3-D.
    containers: Some(Reach {
        distance: 20.0,
        flat: false,
        inclusive: true,
    }),
    // A 2-D distance under 15 (0x5e44d4), with string 420.
    bandage: Reach {
        distance: 15.0,
        flat: true,
        inclusive: false,
    },
    too_far_to_bandage: 420,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, z: f32) -> Position {
        Position {
            x,
            z,
            ..Position::default()
        }
    }

    #[test]
    fn a_reach_measures_as_its_program_does() {
        let here = Position::default();
        // Height counts on Titanium's doors, not on TAKP's.
        assert!(!TITANIUM.doors.admits(here, at(15.0, 15.0)));
        assert!(EQMAC.doors.admits(here, at(15.0, 15.0)));
        assert!(EQMAC.doors.admits(here, at(20.0, 0.0)));
        // TAKP's ground items are reached under 20, not at it.
        assert!(!EQMAC.objects.admits(here, at(20.0, 0.0)));
        assert!(TITANIUM.objects.admits(here, at(20.0, 0.0)));
        // Bandaging reaches 15 on both.
        assert!(TITANIUM.bandage.admits(here, at(15.0, 0.0)));
        assert!(!EQMAC.bandage.admits(here, at(15.0, 0.0)));
        assert!(!TITANIUM.bandage.admits(here, at(15.1, 0.0)));
        assert!(!EQMAC.objects.admits(here, at(f32::NAN, 0.0)));
    }
}
