//! What the Titanium program does when a bind spell lands on the player
//! (reads R49-22i and R49-22j, checklist 6.615 slice 2).
//!
//! The spell-landing routine 0x414913 takes the branch for SPA 25 (Bind
//! Affinity) on the spawn the spell lands on, only for a player's spawn and
//! only for the program's own character. It decides ([`crate::binding::titanium_landing`]):
//!
//! 1. the zone's table entry has flag 0x20 ([`crate::binding::titanium_no_bind_zone`]):
//!    line 0x69, nothing bound;
//! 2. the caster is the player, or the caster's spawn has its GM byte set
//!    (spawn +0x256): it binds and saves ([`crate::binding::Landing::Saves`]) when the
//!    caster is a GM, or the "zone permits" term is 0, or the zone's time
//!    type is 2, 3 or 4, or the player stands in one of 0x40dd2e's areas
//!    ([`crate::binding::titanium_bind_area`]); otherwise line 0x103;
//! 3. anyone else's bind: it binds without saving when the time type is 2,
//!    3 or 4 or the player stands in an area; otherwise line 0x103.
//!
//! The "zone permits" term (the program's iVar9) is 1 when the server's
//! `[0x90047c]` is set or `[0x900478]` is 4 or 6 (`OP_LogServer` words +0 and
//! +4), else 0 when the time type is 1, else the zone's era
//! ([`crate::binding::titanium_later_era`]). Both lines are in colour 0x121 with no
//! arguments.
//!
//! What the bind writes, the profile's bind table, changes nothing on the
//! wire: the server keeps the bind and sends it. The session doesn't model it
//! (6.615 B3).

/// The no-bind line, for a zone whose table entry has flag 0x20 (0x415f4c).
pub const NO_BIND_HERE: u32 = 0x69;

/// The refusal line for a bind the zone doesn't allow (0x4152bd).
pub const CANNOT_BIND_HERE: u32 = 0x103;

/// The colour both lines are printed in (0x4649b1's third argument).
pub const LINE_COLOUR: u32 = 0x121;

/// The zones whose entry in the program's static zone table (built at
/// 0x5f7000, flags at +0x198) has bit 0x20, none below 126 (read R49-22j
/// part 2, item 1). The read names these ids, which are 50, and counts 48;
/// the ids are kept as named until a re-read settles the two.
const NO_BIND_ZONES: [std::ops::RangeInclusive<u16>; 15] = [
    126..=126,
    171..=171,
    186..=186,
    200..=212,
    214..=223,
    225..=228,
    280..=284,
    291..=291,
    293..=293,
    295..=295,
    297..=297,
    316..=316,
    334..=336,
    338..=343,
    345..=345,
];

/// Whether the zone's table entry has the no-bind flag 0x20 (0x5f6c00).
#[must_use]
pub fn titanium_no_bind_zone(zone: u16) -> bool {
    NO_BIND_ZONES.iter().any(|ids| ids.contains(&zone))
}

/// The zones whose table entry has a non-zero era (+8, the zone builder's
/// first argument at 0x5f7000; read R49-22j part 2, item 2). Every other zone
/// is era 0.
///
/// Two rows of the read say "minus a few" (229-276 and 347-368) without
/// naming the few, and the read places 77 both in era 0 and in era 6. All of
/// those are kept as a later era here (inferred): the term then refuses a
/// self bind the program might allow, never the reverse.
const LATER_ERA_ZONES: [std::ops::RangeInclusive<u16>; 15] = [
    77..=79,
    81..=97,
    102..=121,
    123..=129,
    150..=150,
    152..=176,
    179..=179,
    200..=228,
    229..=276,
    277..=277,
    279..=299,
    300..=309,
    316..=336,
    337..=345,
    347..=368,
];

/// Whether the zone's era (its table entry's +8, 0x5f6b90) is not 0: a zone
/// from after the original game, where a self bind needs another term.
#[must_use]
pub fn titanium_later_era(zone: u16) -> bool {
    LATER_ERA_ZONES.iter().any(|ids| ids.contains(&zone))
}

/// One bound of an area's range: inclusive or exclusive, as the program's
/// compare form makes it (read R49-22j part 2, item 3).
#[derive(Clone, Copy)]
enum Range {
    /// `[low, high]`.
    Closed(f32, f32),
    /// `(low, high)`.
    Open(f32, f32),
    /// No test on this axis.
    Any,
}

impl Range {
    fn holds(self, value: f32) -> bool {
        match self {
            Self::Closed(low, high) => (low..=high).contains(&value),
            Self::Open(low, high) => low < value && value < high,
            Self::Any => true,
        }
    }
}

/// One of 0x40dd2e's boxes, in the program's spawn order: +0x30, +0x34 and
/// +0x38, which is the save body's order, so the session's y, x and z.
#[derive(Clone, Copy)]
struct Area {
    first: Range,
    second: Range,
    height: Range,
}

impl Area {
    const fn flat(first: Range, second: Range) -> Self {
        Self {
            first,
            second,
            height: Range::Any,
        }
    }

    fn holds(self, position: crate::world::Position) -> bool {
        self.first.holds(position.y)
            && self.second.holds(position.x)
            && self.height.holds(position.z)
    }
}

/// The areas 0x40dd2e tests by zone (read R49-22j part 2, item 3, which
/// corrects R49-22i: a zone not listed returns 0). Zone 84 passes anywhere;
/// 113 and 114 have two boxes each.
fn areas(zone: u16) -> Option<&'static [Area]> {
    use Range::{Any, Closed, Open};
    const fn closed(first: (f32, f32), second: (f32, f32)) -> Area {
        Area::flat(Closed(first.0, first.1), Closed(second.0, second.1))
    }
    const fn open(first: (f32, f32), second: (f32, f32)) -> Area {
        Area::flat(Open(first.0, first.1), Open(second.0, second.1))
    }
    const Z13: [Area; 1] = [open((-720.0, -550.0), (-260.0, -73.0))];
    const Z22: [Area; 1] = [closed((-1610.0, -1310.0), (-300.0, -100.0))];
    const Z30: [Area; 1] = [closed((1675.0, 1875.0), (-5140.0, -4940.0))];
    const Z34: [Area; 1] = [closed((2650.0, 2850.0), (790.0, 990.0))];
    const Z35: [Area; 1] = [closed((-1520.0, -1320.0), (900.0, 1100.0))];
    const Z68: [Area; 1] = [closed((-1220.0, -1020.0), (-2625.0, -2425.0))];
    const Z78: [Area; 1] = [open((-2250.0, -1200.0), (-550.0, 275.0))];
    const Z84: [Area; 1] = [Area::flat(Any, Any)];
    const Z92: [Area; 1] = [open((-2250.0, -2100.0), (1180.0, 1560.0))];
    const Z93: [Area; 1] = [open((2344.0, 3538.0), (2011.0, 3852.0))];
    const Z110: [Area; 1] = [open((5270.0, 5365.0), (325.0, 510.0))];
    const Z113: [Area; 2] = [
        closed((-176.0, 17.0), (-657.0, -603.0)),
        closed((-287.0, -102.0), (3139.0, 3196.0)),
    ];
    const Z114: [Area; 2] = [
        Area {
            first: Closed(-320.0, 535.0),
            second: Closed(-930.0, -200.0),
            height: Closed(-10.0, 60.0),
        },
        Area {
            first: Closed(535.0, 1000.0),
            second: Closed(-800.0, -200.0),
            height: Closed(-10.0, 120.0),
        },
    ];
    const Z224: [Area; 1] = [Area {
        first: Closed(1230.0, 1740.0),
        second: Closed(-450.0, 230.0),
        height: Closed(-10.0, 66.0),
    }];
    Some(match zone {
        13 => &Z13,
        22 => &Z22,
        30 => &Z30,
        34 => &Z34,
        35 => &Z35,
        68 => &Z68,
        78 => &Z78,
        84 => &Z84,
        92 => &Z92,
        93 => &Z93,
        110 => &Z110,
        113 => &Z113,
        114 => &Z114,
        224 => &Z224,
        _ => return None,
    })
}

/// 0x40dd2e: whether the player, standing at `position` in `zone`, is in
/// one of the zone's bind areas. A zone without areas returns false.
#[must_use]
pub fn titanium_bind_area(zone: u16, position: crate::world::Position) -> bool {
    areas(zone).is_some_and(|areas| areas.iter().any(|area| area.holds(position)))
}

/// What the program reads to decide a bind landing on the player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bind {
    /// The zone's ID (character +0x104).
    pub zone: u16,
    /// Where the player stands as the spell lands.
    pub position: crate::world::Position,
    /// Whether the caster is the player.
    pub self_cast: bool,
    /// Whether the caster's spawn has its GM byte set (+0x256, from the spawn
    /// record's byte 1; inferred to be the GM flag, read R49-22j item 6).
    pub caster_gm: bool,
    /// The zone header's time type (`OP_NewZone` +0x1a8, `[0x923b88]`; read
    /// 10f), 1 by the program's default until a header comes.
    pub time_type: u8,
    /// Whether the server's `OP_LogServer` words make the "zone permits"
    /// term 1: `[0x90047c]` set, or `[0x900478]` 4 or 6.
    pub server_permits: bool,
}

/// What the program does when the bind lands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Landing {
    /// It binds the player and saves with reason 0x0e (0x4a1350(0, 1, 0x0e)).
    Saves,
    /// It binds the player without saving: someone else's bind.
    Binds,
    /// It binds nothing and prints this line ([`NO_BIND_HERE`] or
    /// [`CANNOT_BIND_HERE`]) in [`LINE_COLOUR`].
    Refused(u32),
}

/// The Titanium program's decision in 0x414913 for SPA 25 landing on the
/// player, in its order (reads R49-22i, R49-22j).
#[must_use]
pub fn titanium_landing(bind: &Bind) -> Landing {
    if titanium_no_bind_zone(bind.zone) {
        return Landing::Refused(NO_BIND_HERE);
    }
    let special_time = matches!(bind.time_type, 2..=4);
    let area = || titanium_bind_area(bind.zone, bind.position);
    if bind.self_cast || bind.caster_gm {
        // Read R49-22i [R]: the server's words set the term to 1, which
        // fails it. Read 22j part 2's summary reads them as letting a self
        // bind through; the reviewer's question stands (the plan's note).
        let zone_permits =
            !bind.server_permits && (bind.time_type == 1 || !titanium_later_era(bind.zone));
        if bind.caster_gm || zone_permits || special_time || area() {
            Landing::Saves
        } else {
            Landing::Refused(CANNOT_BIND_HERE)
        }
    } else if special_time || area() {
        Landing::Binds
    } else {
        Landing::Refused(CANNOT_BIND_HERE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Position;

    fn at(x: f32, y: f32, z: f32) -> Position {
        Position {
            x,
            y,
            z,
            heading: 0.0,
        }
    }

    fn own(zone: u16) -> Bind {
        Bind {
            zone,
            position: at(0.0, 0.0, 0.0),
            self_cast: true,
            caster_gm: false,
            time_type: 0,
            server_permits: false,
        }
    }

    #[test]
    fn the_no_bind_flag_covers_the_named_zones() {
        let flagged = (0..=u16::MAX)
            .filter(|&zone| titanium_no_bind_zone(zone))
            .count();
        assert_eq!(flagged, 50);
        assert!(!titanium_no_bind_zone(213));
        assert!(!titanium_no_bind_zone(125));
        assert!(titanium_no_bind_zone(345));
        assert!(!titanium_no_bind_zone(0));
    }

    #[test]
    fn a_flagged_zone_refuses_with_line_0x69_whoever_casts() {
        let mut bind = own(200);
        assert_eq!(titanium_landing(&bind), Landing::Refused(NO_BIND_HERE));
        bind.caster_gm = true;
        bind.time_type = 2;
        assert_eq!(titanium_landing(&bind), Landing::Refused(NO_BIND_HERE));
    }

    #[test]
    fn a_self_bind_in_an_original_zone_saves() {
        assert_eq!(titanium_landing(&own(2)), Landing::Saves);
    }

    #[test]
    fn a_self_bind_in_a_later_zone_needs_another_term() {
        let mut bind = own(110);
        assert_eq!(titanium_landing(&bind), Landing::Refused(CANNOT_BIND_HERE));
        bind.time_type = 1;
        assert_eq!(titanium_landing(&bind), Landing::Saves);
        bind.time_type = 3;
        assert_eq!(titanium_landing(&bind), Landing::Saves);
        bind.time_type = 5;
        assert_eq!(titanium_landing(&bind), Landing::Refused(CANNOT_BIND_HERE));
        // Iceclad's area, in the program's order: its first range is the
        // session's y.
        bind.position = at(400.0, 5300.0, 0.0);
        assert_eq!(titanium_landing(&bind), Landing::Saves);
        bind.position = at(5300.0, 400.0, 0.0);
        assert_eq!(titanium_landing(&bind), Landing::Refused(CANNOT_BIND_HERE));
    }

    #[test]
    fn the_server_words_fail_the_zone_term_as_read_r49_22i_has_it() {
        let mut bind = own(2);
        bind.server_permits = true;
        assert_eq!(titanium_landing(&bind), Landing::Refused(CANNOT_BIND_HERE));
        bind.time_type = 4;
        assert_eq!(titanium_landing(&bind), Landing::Saves);
    }

    #[test]
    fn a_gm_casters_bind_saves_anywhere_but_a_flagged_zone() {
        let mut bind = own(110);
        bind.self_cast = false;
        bind.caster_gm = true;
        assert_eq!(titanium_landing(&bind), Landing::Saves);
    }

    #[test]
    fn someone_elses_bind_binds_without_saving_only_in_an_area_or_time_type() {
        let mut bind = own(2);
        bind.self_cast = false;
        assert_eq!(titanium_landing(&bind), Landing::Refused(CANNOT_BIND_HERE));
        bind.time_type = 2;
        assert_eq!(titanium_landing(&bind), Landing::Binds);
        bind.time_type = 0;
        bind.zone = 84;
        assert_eq!(titanium_landing(&bind), Landing::Binds);
    }

    #[test]
    fn the_areas_keep_each_bounds_inclusiveness() {
        // Zone 22's ranges are closed, zone 13's open.
        assert!(titanium_bind_area(22, at(-300.0, -1610.0, 0.0)));
        assert!(!titanium_bind_area(13, at(-260.0, -720.0, 0.0)));
        assert!(titanium_bind_area(13, at(-259.0, -719.0, 0.0)));
        // A zone without areas returns 0, the part 2 correction.
        assert!(!titanium_bind_area(2, at(0.0, 0.0, 0.0)));
        // Zone 114's second box tests the height up to 120.
        assert!(titanium_bind_area(114, at(-500.0, 600.0, 100.0)));
        assert!(!titanium_bind_area(114, at(-500.0, 600.0, 121.0)));
        // Zone 113's boxes are flat.
        assert!(titanium_bind_area(113, at(3150.0, -200.0, 5000.0)));
    }
}
