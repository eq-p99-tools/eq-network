//! Creation choices in protocol IDs. Functional rules checked against the
//! clients' class/deity/city checks: Titanium 0x416e9c, 0x419e83, 0x4171a1;
//! `EQMac` 0x4bc730, 0x4bc786, 0x4bcbe9. No installed text is bundled here.
use crate::GameDialect;
use serde::Serialize;

/// The creation policy offered with a character list.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Rules {
    /// P99's classic races and classes using Titanium packets.
    Classic,
    /// Titanium's races and classes, including Frogloks and Berserkers.
    Titanium,
    /// `EQMac`'s races and classes through Luclin.
    EqMac,
}

/// Playable race IDs and this library's short labels.
pub const RACES: &[(u32, &str)] = &[
    (1, "Human"),
    (2, "Barbarian"),
    (3, "Erudite"),
    (4, "Wood Elf"),
    (5, "High Elf"),
    (6, "Dark Elf"),
    (7, "Half Elf"),
    (8, "Dwarf"),
    (9, "Troll"),
    (10, "Ogre"),
    (11, "Halfling"),
    (12, "Gnome"),
    (128, "Iksar"),
    (130, "Vah Shir"),
    (330, "Froglok"),
];

/// Class IDs and short labels.
pub const CLASSES: &[(u32, &str)] = &[
    (1, "Warrior"),
    (2, "Cleric"),
    (3, "Paladin"),
    (4, "Ranger"),
    (5, "Shadow Knight"),
    (6, "Druid"),
    (7, "Monk"),
    (8, "Bard"),
    (9, "Rogue"),
    (10, "Shaman"),
    (11, "Necromancer"),
    (12, "Wizard"),
    (13, "Magician"),
    (14, "Enchanter"),
    (15, "Beastlord"),
    (16, "Berserker"),
];

/// Deity IDs in the clients' choice order (`EQMac` 0xc6f00).
pub const DEITIES: &[(u32, &str)] = &[
    (396, "Agnostic"),
    (201, "Bertoxxulous"),
    (205, "Bristlebane"),
    (203, "Cazic-Thule"),
    (206, "Innoruuk"),
    (207, "Karana"),
    (204, "Erollisi Marr"),
    (208, "Mithaniel Marr"),
    (212, "Rodcet Nife"),
    (209, "Prexus"),
    (210, "Quellious"),
    (213, "Solusek Ro"),
    (202, "Brell Serilis"),
    (214, "The Tribunal"),
    (215, "Tunare"),
    (216, "Veeshan"),
    (211, "Rallos Zek"),
];

// Each bit names a class (bit zero is Warrior), in RACES order.
const CLASSES_BY_RACE: [u32; 15] = [
    0x3dff, 0xc301, 0x3c16, 0x01a9, 0x3806, 0x3d13, 0x01ad, 0x8107, 0xc211, 0xc211, 0x012f, 0x3d17,
    0x4651, 0xc381, 0x0f17,
];

// Each bit names a deity in DEITIES order. Rows name race, columns class.
// These represent availability relationships, not copies of client tables.
const WORSHIP: [[u32; 16]; 12] = [
    [
        0x101f3, 0x1f2, 0x1e0, 0x4020, 0x12, 0x4020, 0x401, 0x1ffe5, 0x177, 0, 0x12, 0x9f3, 0x1f3,
        0x1f3, 0, 0,
    ],
    [
        0x12001, 0, 0, 0, 0, 0, 0, 0, 0x2005, 0x2000, 0, 0, 0, 0, 0x2080, 0x12001,
    ],
    [
        0, 0x608, 0x600, 0, 8, 0, 0, 0, 0, 0, 8, 0xe01, 0x601, 0x601, 0, 0,
    ],
    [
        0x14021, 0, 0, 0x4000, 0, 0x4000, 0, 0x1ffe5, 0x4025, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        0, 0x4000, 0x4000, 0, 0, 0, 0, 0, 0, 0, 0, 0x48e1, 0x40e1, 0x40e1, 0, 0,
    ],
    [
        0x10011, 0x10, 0, 0, 0x10, 0, 0, 0, 0x15, 0, 0x10, 0x811, 0x11, 0x11, 0, 0,
    ],
    [
        0x163f3, 0, 0x41e0, 0x4020, 0, 0x4020, 0, 0x1ffe5, 0x4167, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        0x1001, 0x1000, 0x1000, 0, 0, 0, 0, 0, 0x1005, 0, 0, 0, 0, 0, 0, 0x1005,
    ],
    [
        0x10019, 0, 0, 0, 0x18, 0, 0, 0, 0, 0x18, 0, 0, 0, 0, 0x18, 0x10019,
    ],
    [
        0x10009, 0, 0, 0, 0x10008, 0, 0, 0, 0, 0x10000, 0, 0, 0, 0, 0x10000, 0x10009,
    ],
    [
        0x11001, 4, 0x20, 0x20, 0, 0x20, 0, 0, 0x1005, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        0x11003, 0x1006, 0x1000, 0, 2, 0, 0, 0, 0x1007, 0, 2, 0x1803, 0x1003, 0x1003, 0, 0,
    ],
];

/// A starting city and the value this generation sends for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct City {
    /// Short label for the choice.
    pub name: &'static str,
    /// Titanium's city choice, or `EQMac`'s actual start-zone ID.
    pub start_zone: u32,
}

impl Rules {
    /// Whether creation can opt into the tutorial instead of the home city.
    #[must_use]
    pub const fn offers_tutorial(self) -> bool {
        matches!(self, Self::Titanium)
    }

    /// Packet generation used by these choices.
    #[must_use]
    pub const fn dialect(self) -> GameDialect {
        match self {
            Self::Classic | Self::Titanium => GameDialect::Titanium,
            Self::EqMac => GameDialect::EqMac,
        }
    }

    /// Maximum occupied slots supported by these character lists.
    #[must_use]
    pub const fn slots(self) -> usize {
        8
    }

    /// Races available in this policy and the world's expansion mask, when known.
    #[must_use]
    pub fn races(self, expansions: Option<crate::rules::Expansions>) -> Vec<(u32, &'static str)> {
        RACES
            .iter()
            .copied()
            .filter(|(race, _)| {
                let policy = match self {
                    Self::Classic => *race != 130 && *race != 330,
                    Self::EqMac => *race != 330,
                    Self::Titanium => true,
                };
                let expansion = match race {
                    128 => Some(1),
                    130 => Some(3),
                    330 => Some(5),
                    _ => None,
                };
                policy && expansion.is_none_or(|n| expansions.is_none_or(|e| e.includes(n)))
            })
            .collect()
    }

    /// Classes offered for a race, including era and expansion restrictions.
    #[must_use]
    pub fn classes(
        self,
        race: u32,
        expansions: Option<crate::rules::Expansions>,
    ) -> Vec<(u32, &'static str)> {
        let Some(index) = RACES.iter().position(|(id, _)| *id == race) else {
            return Vec::new();
        };
        if !self.races(expansions).iter().any(|(id, _)| *id == race) {
            return Vec::new();
        }
        CLASSES
            .iter()
            .copied()
            .filter(|(class, _)| {
                let era = match self {
                    Self::Classic => {
                        *class <= 14 && !matches!((race, *class), (11, 3 | 4) | (12, 3 | 5))
                    }
                    Self::EqMac => *class <= 15,
                    Self::Titanium => true,
                };
                let expansion = match class {
                    15 => Some(3),
                    16 => Some(7),
                    _ => None,
                };
                era && CLASSES_BY_RACE[index] & (1 << (class - 1)) != 0
                    && expansion.is_none_or(|n| expansions.is_none_or(|e| e.includes(n)))
            })
            .collect()
    }

    /// Deities for this race/class under the world's `PvP` ruleset.
    #[must_use]
    pub fn deities(self, race: u32, class: u32, pvp: u32) -> Vec<(u32, &'static str)> {
        if !self.classes(race, None).iter().any(|(id, _)| *id == class) {
            return Vec::new();
        }
        let mask = match race {
            1..=12 => WORSHIP[(race - 1) as usize][(class - 1) as usize],
            128 => 1 << 3,
            130 => {
                if pvp == 4 {
                    1 << 14
                } else {
                    1
                }
            }
            330 => match class {
                2 => (1 << 4) | (1 << 7),
                5 | 9 | 11 => 1 << 4,
                _ => 1 << 7,
            },
            _ => 0,
        };
        DEITIES
            .iter()
            .copied()
            .enumerate()
            .filter(|(index, _)| {
                if pvp == 2 && race == 7 {
                    return *index == 14;
                }
                if pvp == 4 {
                    if *index == 0 || (*index == 16 && matches!(race, 4 | 11)) {
                        return false;
                    }
                    if (race == 1 && class == 7 && *index == 15)
                        || (race == 2 && matches!(class, 1 | 10) && *index == 7)
                    {
                        return true;
                    }
                }
                mask & (1 << index) != 0
            })
            .map(|(_, deity)| deity)
            .collect()
    }

    /// Cities permitted by the selected race, class and deity.
    #[must_use]
    pub fn cities(self, race: u32, class: u32, deity: u32, pvp: u32) -> Vec<City> {
        if !self
            .deities(race, class, pvp)
            .iter()
            .any(|(id, _)| *id == deity)
        {
            return Vec::new();
        }
        let candidates: &[u32] = match race {
            1 => &[1, 4],
            2 => &[2],
            3 => &[0],
            4 => &[9],
            5 => &[10],
            6 => &[5],
            7 => &[1, 4, 9, 10],
            8 => &[8],
            9 => &[6],
            10 => &[7],
            11 => &[3],
            12 => &[11],
            128 => &[12],
            130 => &[13],
            330 => &[14],
            _ => &[],
        };
        candidates
            .iter()
            .copied()
            .filter(|city| city_allowed(race, class, deity, pvp, *city))
            .map(|city| {
                let name = match city {
                    0 if deity == 203 => "Paineel",
                    0 => "Erudin",
                    1 if matches!(class, 4 | 6) => "Surefall Glade",
                    1 => "Qeynos",
                    2 => "Halas",
                    3 => "Rivervale",
                    4 => "Freeport",
                    5 => "Neriak",
                    6 => "Grobb",
                    7 => "Oggok",
                    8 => "Kaladim",
                    9 => "Kelethin",
                    10 => "Felwithe",
                    11 => "Ak'Anon",
                    12 => "Cabilis",
                    13 => "Shar Vahl",
                    _ => "Gukta",
                };
                City {
                    name,
                    start_zone: if self == Self::EqMac {
                        eqmac_zone(race, class, deity, city)
                    } else {
                        city
                    },
                }
            })
            .collect()
    }
}

fn city_allowed(race: u32, class: u32, deity: u32, pvp: u32, city: u32) -> bool {
    if !matches!(race, 1 | 7) {
        return true;
    }
    if city != 4 && matches!(deity, 204 | 208 | 206) {
        return false;
    }
    if city != 1 && matches!(deity, 201 | 212 | 207) {
        return false;
    }
    if race == 1 {
        return (!matches!(class, 4 | 6) || city == 1)
            && (class != 7
                || (deity != 210 || city == 4)
                    && (deity != 396 && (pvp != 4 || deity != 216) || city == 1));
    }
    if pvp == 2 && !matches!(city, 9 | 10) {
        return false;
    }
    if class == 4 && (!matches!(city, 1 | 9) || city == 9 && deity != 215) {
        return false;
    }
    if deity == 215 {
        if matches!(class, 1 | 9) && city != 9 {
            return false;
        }
        if class == 6 && !matches!(city, 1 | 9) {
            return false;
        }
        if class == 3 && city != 10 {
            return false;
        }
    }
    city != 10 || class == 3 && deity == 215
}

// EQMac InitPlayerStart (0x1d77d0). The city choice
// becomes a zone ID; its server supplies the safe position and bind.
fn eqmac_zone(race: u32, class: u32, deity: u32, city: u32) -> u32 {
    match race {
        2 => 29,
        3 => {
            if deity == 203 {
                75
            } else if class >= 12 {
                23
            } else {
                24
            }
        }
        4 => 54,
        5 => {
            if matches!(class, 2 | 3) {
                61
            } else {
                62
            }
        }
        6 => {
            if matches!(class, 2 | 5 | 9 | 11) {
                42
            } else {
                41
            }
        }
        8 => {
            if class == 1 {
                60
            } else {
                67
            }
        }
        9 => 52,
        10 => 49,
        11 => 19,
        12 => 55,
        128 => {
            if class == 11 {
                82
            } else {
                106
            }
        }
        130 => 155,
        _ => match city {
            9 => 54,
            10 => 61,
            4 => {
                if matches!(class, 3 | 8) {
                    8
                } else if matches!(class, 5 | 9 | 11) || deity == 204 {
                    10
                } else {
                    9
                }
            }
            _ => {
                if matches!(class, 4 | 6) {
                    3
                } else if matches!(class, 7 | 9) || matches!(class, 2 | 3) && deity == 212 {
                    2
                } else {
                    1
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policies_limit_races_classes_and_expansions_before_showing_choices() {
        let races = |rules: Rules| {
            rules
                .races(None)
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>()
        };
        assert!(!races(Rules::Classic).contains(&130));
        assert!(races(Rules::EqMac).contains(&130));
        assert!(!races(Rules::EqMac).contains(&330));
        assert!(races(Rules::Titanium).contains(&330));
        assert_eq!(
            Rules::Titanium.deities(330, 2, 0),
            [(206, "Innoruuk"), (208, "Mithaniel Marr")]
        );
        assert_eq!(Rules::Titanium.deities(330, 5, 0), [(206, "Innoruuk")]);
        assert!(!Rules::Classic
            .classes(12, None)
            .iter()
            .any(|(id, _)| *id == 5));
        assert!(Rules::EqMac
            .classes(12, None)
            .iter()
            .any(|(id, _)| *id == 5));
        assert!(Rules::EqMac
            .classes(2, None)
            .iter()
            .all(|(id, _)| *id != 16));
        let classic = Some(crate::rules::Expansions(0));
        assert_eq!(
            Rules::Titanium.classes(128, classic),
            [] as [(u32, &str); 0]
        );
        assert!(Rules::Titanium
            .classes(2, classic)
            .iter()
            .all(|(id, _)| *id < 15));
    }

    #[test]
    fn every_legal_deity_has_a_home_and_known_multizone_cities_translate() {
        for rules in [Rules::Classic, Rules::Titanium, Rules::EqMac] {
            for pvp in [0, 1, 2, 4] {
                for (race, _) in rules.races(None) {
                    for (class, _) in rules.classes(race, None) {
                        let deities = rules.deities(race, class, pvp);
                        assert!(!deities.is_empty(), "{rules:?}/{race}/{class}/{pvp}");
                        for (deity, _) in deities {
                            assert!(
                                !rules.cities(race, class, deity, pvp).is_empty(),
                                "{rules:?}/{race}/{class}/{deity}/{pvp}"
                            );
                        }
                    }
                }
            }
        }
        let zone = |race, class, deity| Rules::EqMac.cities(race, class, deity, 0)[0].start_zone;
        assert_eq!(zone(1, 2, 212), 2);
        assert_eq!(zone(1, 4, 207), 3);
        assert_eq!(zone(3, 11, 203), 75);
        assert_eq!(zone(5, 2, 215), 61);
        assert_eq!(zone(5, 12, 396), 62);
        assert_eq!(zone(128, 1, 203), 106);
        assert_eq!(zone(128, 11, 203), 82);
        assert_eq!(Rules::Titanium.cities(1, 2, 212, 0)[0].start_zone, 1);
    }
}
