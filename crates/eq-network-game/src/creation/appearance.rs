//! Creation-only appearance choices. Race tables describe available controls;
//! each client generation owns its ranges and wire face composition.
use super::{NewCharacter, Rules};
use serde::Serialize;

/// Cosmetic values independent of the base face.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Cosmetics {
    /// Hair style, or erudite head detail.
    pub hair_style: u8,
    /// Hair palette index.
    pub hair_color: u8,
    /// Beard style.
    pub beard: u8,
    /// Beard palette index.
    pub beard_color: u8,
    /// Left eye palette index.
    pub eye_color_1: u8,
    /// Right eye palette index.
    pub eye_color_2: u8,
    /// Barbarian facial tattoo.
    pub woad: u8,
}

/// An independently selectable feature.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Feature {
    /// Base face.
    Face,
    /// Hair style or erudite head detail.
    Hair,
    /// Hair color.
    HairColor,
    /// Beard style.
    Beard,
    /// Beard color.
    BeardColor,
    /// Left eye color.
    LeftEye,
    /// Right eye color.
    RightEye,
    /// Barbarian facial tattoo.
    Woad,
}

impl Feature {
    /// Stable field order for editors and validation.
    pub const ALL: [Self; 8] = [
        Self::Face,
        Self::Hair,
        Self::HairColor,
        Self::Beard,
        Self::BeardColor,
        Self::LeftEye,
        Self::RightEye,
        Self::Woad,
    ];
    /// Label for a creation control.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Face => "Face",
            Self::Hair => "Hair / head detail",
            Self::HairColor => "Hair color",
            Self::Beard => "Beard",
            Self::BeardColor => "Beard color",
            Self::LeftEye => "Left eye",
            Self::RightEye => "Right eye",
            Self::Woad => "Facial tattoo",
        }
    }
    /// Current value.
    #[must_use]
    pub const fn get(self, c: &NewCharacter) -> u8 {
        match self {
            Self::Face => c.face,
            Self::Hair => c.cosmetics.hair_style,
            Self::HairColor => c.cosmetics.hair_color,
            Self::Beard => c.cosmetics.beard,
            Self::BeardColor => c.cosmetics.beard_color,
            Self::LeftEye => c.cosmetics.eye_color_1,
            Self::RightEye => c.cosmetics.eye_color_2,
            Self::Woad => c.cosmetics.woad,
        }
    }
    /// Changes the value; callers validate against their session's rules.
    pub fn set(self, c: &mut NewCharacter, value: u8) {
        *match self {
            Self::Face => &mut c.face,
            Self::Hair => &mut c.cosmetics.hair_style,
            Self::HairColor => &mut c.cosmetics.hair_color,
            Self::Beard => &mut c.cosmetics.beard,
            Self::BeardColor => &mut c.cosmetics.beard_color,
            Self::LeftEye => &mut c.cosmetics.eye_color_1,
            Self::RightEye => &mut c.cosmetics.eye_color_2,
            Self::Woad => &mut c.cosmetics.woad,
        } = value;
    }
}

/// Inclusive bounds for each control. Absent controls keep zero on the wire.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppearanceRules {
    ranges: [Option<(u8, u8)>; 8],
    none: [bool; 8],
}

impl AppearanceRules {
    /// Choices offered by this race, gender and client generation. 255 is the
    /// protocol's absent-hair or absent-beard choice, never an ordinal.
    pub fn fields(self) -> impl Iterator<Item = (Feature, Vec<u8>)> {
        Feature::ALL
            .into_iter()
            .zip(self.ranges)
            .zip(self.none)
            .filter_map(|((f, range), none)| {
                range.map(|(lo, hi)| (f, (lo..=hi).chain(none.then_some(255)).collect()))
            })
    }
    /// Keeps valid choices and resets unavailable ones after a race/gender change.
    pub fn reconcile(self, c: &mut NewCharacter) {
        for ((f, range), none) in Feature::ALL.into_iter().zip(self.ranges).zip(self.none) {
            let (lo, hi) = range.unwrap_or((0, 0));
            let value = f.get(c);
            if !((lo..=hi).contains(&value) || none && value == 255) {
                f.set(c, lo);
            }
        }
    }
    /// Whether every submitted feature belongs to this policy.
    #[must_use]
    pub fn accepts(self, c: &NewCharacter) -> bool {
        // Zeroed cosmetics are the classic/default appearance accepted by both
        // wire families, including callers that do not use the appearance editor.
        if c.cosmetics == Cosmetics::default() {
            return self.ranges[0].is_some_and(|(lo, hi)| (lo..=hi).contains(&c.face));
        }
        Feature::ALL
            .into_iter()
            .zip(self.ranges)
            .zip(self.none)
            .all(|((f, range), none)| {
                let (lo, hi) = range.unwrap_or((0, 0));
                (lo..=hi).contains(&f.get(c)) || none && f.get(c) == 255
            })
    }
    /// Randomizes only controls offered by this policy, retaining identity and stats.
    pub fn randomize(self, c: &mut NewCharacter) {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        for (feature, values) in self.fields() {
            feature.set(c, values[rng.gen_range(0..values.len())]);
        }
        self.reconcile(c);
    }

    /// Steps a control, including an absent choice only where the client allows it.
    pub fn step(self, c: &mut NewCharacter, feature: Feature, next: bool) {
        if let Some((_, values)) = self.fields().find(|(f, _)| *f == feature) {
            let index = values
                .iter()
                .position(|value| *value == feature.get(c))
                .unwrap_or(0);
            let index = if next {
                (index + 1) % values.len()
            } else {
                (index + values.len() - 1) % values.len()
            };
            feature.set(c, values[index]);
        }
    }
}

// Columns: hair max, beard max; -1 denotes an absent control. Rows are race,
// male, female. EQMac 0x23c610/0x23c690 and Titanium 0x47a523/0x50032f.
const FEATURES: [(u32, [i8; 2], [i8; 2]); 15] = [
    (1, [2, 2], [4, -1]),
    (2, [2, 2], [4, -1]),
    (3, [5, 8], [4, -1]),
    (4, [2, 2], [-1, -1]),
    (5, [2, 2], [3, -1]),
    (6, [2, 2], [3, -1]),
    (7, [2, 2], [3, -1]),
    (8, [2, 2], [4, 1]),
    (9, [-1, 2], [-1, -1]),
    (10, [-1, 2], [-1, -1]),
    (11, [2, 2], [4, -1]),
    (12, [2, 2], [4, -1]),
    (128, [-1, -1], [-1, -1]),
    (130, [-1, -1], [-1, -1]),
    (330, [-1, -1], [-1, -1]),
];

impl Rules {
    /// Appearance controls for this session. Classic models carry their hair
    /// and eyes in face textures. Luclin has separate attachments and palettes.
    #[must_use]
    pub fn appearance(self, race: u32, gender: u32) -> AppearanceRules {
        match self {
            Self::Classic => AppearanceRules {
                ranges: [Some((0, 7)), None, None, None, None, None, None, None],
                none: [false; 8],
            },
            Self::Titanium | Self::EqMac => luclin(race, gender),
        }
    }
    /// Whether this policy uses classic face textures in its creation preview.
    #[must_use]
    pub const fn classic_appearance(self) -> bool {
        matches!(self, Self::Classic)
    }
}

fn attached(race: u32, gender: u32) -> (Option<u8>, Option<u8>) {
    let Some((_, hair, beard)) = FEATURES.iter().find(|(r, ..)| *r == race) else {
        return (None, None);
    };
    let index = usize::try_from(gender).unwrap_or(2);
    (
        hair.get(index).and_then(|n| u8::try_from(*n).ok()),
        beard.get(index).and_then(|n| u8::try_from(*n).ok()),
    )
}

fn luclin(race: u32, gender: u32) -> AppearanceRules {
    let (hair, beard) = attached(race, gender);
    // Official Titanium 0x47a561 and EQMac 0x23c710 use the same palettes.
    let color = match race {
        5 => (4, 17),
        6 => (11, 19),
        9 | 10 => (0, 23),
        12 => (0, 22),
        _ => (0, 18),
    };
    let separate_hair = hair.is_some() && race != 3;
    let separate_beard = beard.is_some() && !matches!((race, gender), (5..=7, 0) | (8, 1));
    let mut none = [false; 8];
    none[1] = separate_hair && (gender == 0 || matches!(race, 9 | 10));
    none[3] = separate_beard && race != 8;
    AppearanceRules {
        ranges: [
            Some((0, if race == 330 { 9 } else { 7 })),
            hair.map(|n| (0, n)),
            separate_hair.then_some(color),
            beard.map(|n| (0, n)),
            beard.map(|_| color),
            Some(if race == 330 { (0, 9) } else { (1, 10) }),
            Some(if race == 330 { (0, 9) } else { (1, 10) }),
            (race == 2).then_some((0, 8)),
        ],
        none,
    }
}

/// Luclin detail in the face's tens digit (`EQMac` 0x23c570 and 0x2ad3c0;
/// Titanium 0x47a491 and 0x50032f). The base face stays independently editable.
#[must_use]
pub fn wire_face(c: &NewCharacter) -> u8 {
    let detail = match (c.race, c.gender) {
        (2, _) => c.cosmetics.woad,
        (3, _) => c.cosmetics.hair_style,
        (5..=7, 0) | (8, 1) => c.cosmetics.beard,
        _ => 0,
    };
    c.face.saturating_add(detail.saturating_mul(10))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_offered_cosmetic_encodes_in_its_client_layout() {
        for policy in [Rules::Classic, Rules::Titanium, Rules::EqMac] {
            for (race, _) in policy.races(None) {
                for gender in 0..=1 {
                    let class = policy.classes(race, None)[0].0;
                    let mut c = NewCharacter::with_points_in(
                        "Testhero",
                        (race, class, gender),
                        (396, 1),
                        0,
                    )
                    .unwrap();
                    let rules = policy.appearance(race, gender);
                    rules.reconcile(&mut c);
                    for (feature, values) in rules.fields() {
                        for value in values {
                            feature.set(&mut c, value);
                            assert!(
                                rules.accepts(&c),
                                "{policy:?}/{race}/{gender}/{feature:?}/{value}"
                            );
                            let bytes = c.create_request().unwrap();
                            let word = |offset| {
                                u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
                            };
                            assert_eq!(
                                [word(4), word(8), word(12), word(28), word(68), word(72)],
                                [
                                    c.cosmetics.hair_style,
                                    c.cosmetics.beard,
                                    c.cosmetics.beard_color,
                                    c.cosmetics.hair_color,
                                    c.cosmetics.eye_color_1,
                                    c.cosmetics.eye_color_2
                                ]
                                .map(u32::from)
                            );
                            assert_eq!(word(64), u32::from(wire_face(&c)));
                            if policy == Rules::EqMac {
                                let mac = c.eqmac_create_request().unwrap();
                                assert_eq!(mac[174], c.face);
                                assert_eq!(
                                    &mac[5422..5429],
                                    &[
                                        c.cosmetics.hair_color,
                                        c.cosmetics.beard_color,
                                        c.cosmetics.eye_color_1,
                                        c.cosmetics.eye_color_2,
                                        c.cosmetics.hair_style,
                                        c.cosmetics.beard,
                                        wire_face(&c)
                                    ]
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn face_details_are_independent_and_none_never_enters_the_face_digit() {
        for (race, gender, feature, detail, expected) in [
            (2, 0, Feature::Woad, 8, 87),
            (3, 1, Feature::Hair, 8, 87),
            (5, 0, Feature::Beard, 3, 37),
            (8, 1, Feature::Beard, 1, 17),
            (1, 0, Feature::Beard, 255, 7),
        ] {
            let mut c =
                NewCharacter::with_points_in("Testhero", (race, 1, gender), (396, 1), 0).unwrap();
            Rules::Titanium.appearance(race, gender).reconcile(&mut c);
            c.face = 7;
            feature.set(&mut c, detail);
            assert_eq!(wire_face(&c), expected);
        }
        let mut c = NewCharacter::with_points_in("Testhero", (1, 1, 0), (396, 1), 0).unwrap();
        let rules = Rules::Titanium.appearance(1, 0);
        rules.reconcile(&mut c);
        Feature::Hair.set(&mut c, 255);
        rules.step(&mut c, Feature::Hair, true);
        assert_eq!(c.cosmetics.hair_style, 0);
        rules.step(&mut c, Feature::Hair, false);
        assert_eq!(c.cosmetics.hair_style, 255);
        c.gender = 1;
        Rules::Titanium.appearance(1, 1).reconcile(&mut c);
        assert_eq!(c.cosmetics.hair_style, 0);
        Rules::Classic.appearance(1, 1).reconcile(&mut c);
        assert_eq!(c.cosmetics, Cosmetics::default());
    }
}
