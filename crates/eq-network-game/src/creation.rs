//! Titanium character creation: name approval, then the creation request.
//!
//! Stat rules mirror `EQEmu`'s `CheckCharCreateInfoTitanium`: every stat starts at
//! its race plus class base, and the class's bonus points must all be spent.
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// `OP_ApproveName`: the requested name, race, class and deity; answered with one byte.
pub const APPROVE_NAME_OPCODE: u16 = 0x3ea6;
/// `OP_CharacterCreate`: the 80-byte creation request for the approved name.
pub const CREATE_OPCODE: u16 = 0x10b2;

/// A character to create; stats are in STR, STA, AGI, DEX, WIS, INT, CHA order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NewCharacter {
    /// Capitalized name of 4 to 15 letters.
    pub name: String,
    /// Player race ID (1 human ... 12 gnome, 128 iksar, 130 vah shir, 330 froglok).
    pub race: u32,
    /// Class ID, 1 (warrior) through 16 (berserker).
    pub class: u32,
    /// 0 male, 1 female.
    pub gender: u32,
    /// Deity ID (for example 207 Karana, 212 Rodcet Nife, 396 agnostic).
    pub deity: u32,
    /// Titanium start-zone choice (1 Qeynos, 4 Freeport, ...).
    pub start_zone: u32,
    /// Final stats including the spent bonus points.
    pub stats: [u32; 7],
}

const BASE_RACE: [[u32; 7]; 16] = [
    [75, 75, 75, 75, 75, 75, 75],
    [103, 95, 82, 70, 70, 60, 55],
    [60, 70, 70, 70, 83, 107, 70],
    [65, 65, 95, 80, 80, 75, 75],
    [55, 65, 85, 70, 95, 92, 80],
    [60, 65, 90, 75, 83, 99, 60],
    [70, 70, 90, 85, 60, 75, 75],
    [90, 90, 70, 90, 83, 60, 45],
    [108, 109, 83, 75, 60, 52, 40],
    [130, 122, 70, 70, 67, 60, 37],
    [70, 75, 95, 90, 80, 67, 50],
    [60, 70, 85, 85, 67, 98, 60],
    [70, 70, 90, 85, 80, 75, 55],
    [90, 75, 90, 70, 70, 65, 65],
    [70, 80, 100, 100, 75, 75, 50],
    [70, 80, 85, 75, 80, 85, 75],
];

/// Class stat bonuses followed by the class's free points.
const BASE_CLASS: [[u32; 8]; 16] = [
    [10, 10, 5, 0, 0, 0, 0, 25],
    [5, 5, 0, 0, 10, 0, 0, 30],
    [10, 5, 0, 0, 5, 0, 10, 20],
    [5, 10, 10, 0, 5, 0, 0, 20],
    [10, 5, 0, 0, 0, 10, 5, 20],
    [0, 10, 0, 0, 10, 0, 0, 30],
    [5, 5, 10, 10, 0, 0, 0, 20],
    [5, 0, 0, 10, 0, 0, 10, 25],
    [0, 0, 10, 10, 0, 0, 0, 30],
    [0, 5, 0, 0, 10, 0, 5, 30],
    [0, 0, 0, 10, 0, 10, 0, 30],
    [0, 10, 0, 0, 0, 10, 0, 30],
    [0, 10, 0, 0, 0, 10, 0, 30],
    [0, 0, 0, 0, 0, 10, 10, 30],
    [0, 10, 5, 0, 10, 0, 5, 20],
    [10, 5, 0, 10, 0, 0, 0, 25],
];

fn race_index(race: u32) -> Option<usize> {
    match race {
        1..=12 => usize::try_from(race - 1).ok(),
        128 => Some(12),
        130 => Some(13),
        330 => Some(14),
        522 => Some(15),
        _ => None,
    }
}

/// Base stats for a race and class, and the free points still to spend.
#[must_use]
pub fn base_stats(race: u32, class: u32) -> Option<([u32; 7], u32)> {
    let race = BASE_RACE.get(race_index(race)?)?;
    let class = BASE_CLASS.get(usize::try_from(class.checked_sub(1)?).ok()?)?;
    let mut stats = [0; 7];
    for (index, stat) in stats.iter_mut().enumerate() {
        *stat = race[index] + class[index];
    }
    Some((stats, class[7]))
}

impl NewCharacter {
    /// Builds a character that spends every free point on `primary` (0 = STR ... 6 = CHA).
    ///
    /// # Errors
    /// Rejects unknown races, classes or stat indexes.
    pub fn with_points_in(
        name: impl Into<String>,
        (race, class, gender): (u32, u32, u32),
        (deity, start_zone): (u32, u32),
        primary: usize,
    ) -> Result<Self> {
        let (mut stats, points) =
            base_stats(race, class).context("unknown race or class for creation")?;
        *stats.get_mut(primary).context("unknown stat")? += points;
        Ok(Self {
            name: name.into(),
            race,
            class,
            gender,
            deity,
            start_zone,
            stats,
        })
    }

    /// Checks the name and stat rules the server enforces.
    ///
    /// # Errors
    /// Rejects names or stats the server would refuse.
    pub fn validate(&self) -> Result<()> {
        let bytes = self.name.as_bytes();
        ensure!(
            (4..=15).contains(&bytes.len())
                && bytes[0].is_ascii_uppercase()
                && bytes[1..].iter().all(u8::is_ascii_lowercase),
            "names are 4 to 15 letters, capitalized, with no other capitals"
        );
        let (base, points) = base_stats(self.race, self.class).context("unknown race or class")?;
        ensure!(
            self.stats
                .iter()
                .zip(base)
                .all(|(stat, base)| (base..=base + points).contains(stat))
                && self.stats.iter().sum::<u32>() == base.iter().sum::<u32>() + points,
            "stats must spend exactly the class's free points"
        );
        Ok(())
    }

    /// Encodes the 76-byte name approval request.
    ///
    /// # Errors
    /// Rejects invalid characters.
    pub fn name_approval(&self) -> Result<[u8; 76]> {
        self.validate()?;
        let mut body = [0; 76];
        body[..self.name.len()].copy_from_slice(self.name.as_bytes());
        body[64..68].copy_from_slice(&self.race.to_le_bytes());
        body[68..72].copy_from_slice(&self.class.to_le_bytes());
        body[72..76].copy_from_slice(&self.deity.to_le_bytes());
        Ok(body)
    }

    /// Encodes the 80-byte creation request; appearance uses the default face and hair.
    ///
    /// # Errors
    /// Rejects invalid characters.
    pub fn create_request(&self) -> Result<[u8; 80]> {
        self.validate()?;
        let mut body = [0; 80];
        let mut put = |offset: usize, value: u32| {
            body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        put(0, self.class);
        put(16, self.gender);
        put(20, self.race);
        put(24, self.start_zone);
        put(32, self.deity);
        for (index, stat) in self.stats.iter().enumerate() {
            put(36 + index * 4, *stat);
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_cleric_spends_every_point_and_encodes_titanium_layouts() {
        let cleric = NewCharacter::with_points_in("Testcleric", (1, 2, 0), (212, 1), 4).unwrap();
        assert_eq!(cleric.stats, [80, 80, 75, 75, 115, 75, 75]);
        let approval = cleric.name_approval().unwrap();
        assert_eq!(&approval[..10], b"Testcleric");
        assert_eq!(&approval[64..76], &[1, 0, 0, 0, 2, 0, 0, 0, 212, 0, 0, 0]);
        let create = cleric.create_request().unwrap();
        assert_eq!(&create[..4], &[2, 0, 0, 0]);
        assert_eq!(&create[20..28], &[1, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(&create[52..56], &115u32.to_le_bytes());
        for bad in ["test", "TestCleric", "Tst", "Testclericwithlongname"] {
            let mut invalid = cleric.clone();
            invalid.name = bad.into();
            assert!(invalid.name_approval().is_err(), "{bad}");
        }
        let mut cheat = cleric;
        cheat.stats[0] += 1;
        assert!(cheat.create_request().is_err());
        assert!(base_stats(99, 2).is_none() && base_stats(1, 17).is_none());
        assert_eq!(base_stats(128, 1).unwrap().1, 25);
    }
}
