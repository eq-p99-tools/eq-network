//! Character-select lists contain fixed name slots, not searchable packet text.
use crate::appearance::{Appearance, TextureSlot};
use crate::GameDialect;
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// One occupied server-side character slot.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CharacterChoice {
    /// Slot in the server list; empty slots retain their indexes.
    pub slot: u8,
    /// Exact server spelling used for world entry.
    pub name: String,
    /// Level when supplied by a verified dialect layout.
    pub level: Option<u8>,
    /// Current zone identifier when supplied by a verified layout.
    pub zone_id: Option<u32>,
    /// How the character looks, where the server type's list is checked to
    /// carry it ([`CharacterChoice::without_look`] removes it elsewhere).
    pub look: Option<CharacterLook>,
}

/// How a listed character looks, and what its list record says besides,
/// as both client generations' character lists carry it (`EQEmu`'s
/// Titanium `CharacterSelect_Struct`, `EQMacEmu`'s `CharacterSelect_Struct`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct CharacterLook {
    /// Race/model identifier.
    pub race: u32,
    /// Model gender identifier.
    pub gender: u8,
    /// Class identifier.
    pub class: u8,
    /// Deity identifier.
    pub deity: u32,
    /// Worn gear and features, as a spawn record carries them: armour
    /// materials and tints per slot, and the held items' model numbers.
    pub appearance: Appearance,
    /// The two eye colors.
    pub eye_colors: [u8; 2],
    /// Whether the world offers to send the character home (Titanium's list
    /// only; false on `EQMac`, whose list has no such field).
    pub go_home: bool,
    /// Whether the world offers the tutorial (as `go_home`).
    pub tutorial: bool,
}

impl CharacterChoice {
    /// The entry without what a list says beyond its names where the
    /// server type's list isn't checked to carry it: the look, and on
    /// `EQMac` the level and zone too.
    #[must_use]
    pub fn without_look(self, dialect: GameDialect) -> Self {
        let verified = match dialect {
            GameDialect::Titanium => true,
            // RoF2's list is read from `EQEmu`'s source, not checked against
            // the client (6.497).
            GameDialect::EqMac | GameDialect::Rof2 => false,
        };
        Self {
            level: self.level.filter(|_| verified),
            zone_id: self.zone_id.filter(|_| verified),
            look: None,
            ..self
        }
    }
}

/// Decodes occupied slots and rejects malformed names or ambiguous duplicates.
///
/// # Errors
/// Rejects truncated records, unterminated/non-ASCII names and duplicate names.
pub fn decode(dialect: GameDialect, body: &[u8]) -> Result<Vec<CharacterChoice>> {
    let layout = match dialect {
        GameDialect::Titanium => {
            ensure!(body.len() == 1704, "invalid Titanium character list length");
            &TITANIUM
        }
        GameDialect::EqMac => {
            ensure!(body.len() >= 1620, "truncated EQMac character list");
            &EQMAC
        }
        GameDialect::Rof2 => return rof2(body),
    };
    let mut choices: Vec<CharacterChoice> = Vec::new();
    for (index, field) in body[layout.name..layout.name + 640]
        .as_chunks::<64>()
        .0
        .iter()
        .enumerate()
    {
        let end = field
            .iter()
            .position(|byte| *byte == 0)
            .context("unterminated character name")?;
        if end == 0 {
            continue;
        }
        let name = &field[..end];
        if name == b"<none>" {
            continue;
        }
        ensure!(
            name.iter().all(u8::is_ascii_alphabetic),
            "invalid character name"
        );
        let name = std::str::from_utf8(name)?.to_owned();
        ensure!(
            !choices
                .iter()
                .any(|choice| choice.name.eq_ignore_ascii_case(&name)),
            "duplicate character name"
        );
        let record = Record {
            body,
            layout,
            index,
        };
        choices.push(CharacterChoice {
            slot: u8::try_from(index)?,
            name,
            level: Some(record.byte(layout.level)),
            zone_id: Some(record.word(layout.zone)),
            look: Some(record.look()),
        });
    }
    Ok(choices)
}

/// The opcode of `RoF2`'s character list (`EQEmu`'s
/// `utils/patches/patch_RoF2.conf`, `OP_SendCharInfo`; inferred, 6.497).
pub const ROF2_OPCODE: u16 = 0x00d2;

/// The fixed part of a `RoF2` list entry, after its name and the name's NUL.
const ROF2_ENTRY: usize = 274;

/// Where each field of a `RoF2` entry's fixed part sits.
mod rof2_at {
    pub(super) const CLASS: usize = 0;
    pub(super) const RACE: usize = 1;
    pub(super) const LEVEL: usize = 5;
    pub(super) const ZONE: usize = 11;
    pub(super) const GENDER: usize = 15;
    pub(super) const FACE: usize = 16;
    /// Nine 24-byte entries: the material at +0, the tint at +20.
    pub(super) const EQUIPMENT: usize = 17;
    pub(super) const DEITY: usize = 243;
    pub(super) const PRIMARY: usize = 247;
    pub(super) const SECONDARY: usize = 251;
    pub(super) const HAIR_COLOR: usize = 255;
    pub(super) const BEARD_COLOR: usize = 256;
    pub(super) const EYES: [usize; 2] = [257, 258];
    pub(super) const HAIR_STYLE: usize = 259;
    pub(super) const BEARD: usize = 260;
    pub(super) const GO_HOME: usize = 261;
    pub(super) const TUTORIAL: usize = 262;
}

/// `RoF2`'s character list, as `EQEmu` writes it (`common/patches/rof2.cpp`,
/// `ENCODE(OP_SendCharInfo)`; `common/patches/rof2_structs.h`,
/// `CharacterSelect_Struct` and `CharacterSelectEntry_Struct`), inferred for
/// `RoF2` (6.497): a 32-bit count, then that many entries, each a
/// NUL-terminated name and [`ROF2_ENTRY`] fixed bytes. The list holds only
/// existing characters, so an entry's slot is its place in the list.
fn rof2(body: &[u8]) -> Result<Vec<CharacterChoice>> {
    let count = body.get(..4).context("truncated RoF2 character list")?;
    let count = usize::try_from(u32::from_le_bytes(count.try_into()?))?;
    // Each entry takes at least its name's NUL and its fixed part, so a
    // count the body can't hold is refused before anything is kept.
    ensure!(
        count <= (body.len() - 4) / (1 + ROF2_ENTRY),
        "RoF2 character list counts more entries than it holds"
    );
    let mut choices: Vec<CharacterChoice> = Vec::with_capacity(count);
    let mut at = 4;
    for index in 0..count {
        let rest = &body[at..];
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .context("unterminated character name")?;
        let name = &rest[..end];
        ensure!(
            !name.is_empty() && name.iter().all(u8::is_ascii_alphabetic),
            "invalid character name"
        );
        let name = std::str::from_utf8(name)?.to_owned();
        ensure!(
            !choices
                .iter()
                .any(|choice| choice.name.eq_ignore_ascii_case(&name)),
            "duplicate character name"
        );
        let fixed = rest
            .get(end + 1..end + 1 + ROF2_ENTRY)
            .context("truncated RoF2 character entry")?;
        let word = |offset: usize| {
            u32::from_le_bytes(fixed[offset..offset + 4].try_into().unwrap_or_default())
        };
        let equipment = |slot: usize, field: usize| word(rof2_at::EQUIPMENT + slot * 24 + field);
        let materials = std::array::from_fn(|slot| match TextureSlot::from_index(slot) {
            // The held items' model numbers, as spawn records carry them.
            Some(TextureSlot::Primary) => word(rof2_at::PRIMARY),
            Some(TextureSlot::Secondary) => word(rof2_at::SECONDARY),
            _ => equipment(slot, 0),
        });
        let tints = std::array::from_fn(|slot| {
            let [blue, green, red, used] = equipment(slot, 20).to_le_bytes();
            (used != 0).then_some([red, green, blue])
        });
        choices.push(CharacterChoice {
            slot: u8::try_from(index)?,
            name,
            level: Some(fixed[rof2_at::LEVEL]),
            zone_id: Some(u32::from(u16::from_le_bytes([
                fixed[rof2_at::ZONE],
                fixed[rof2_at::ZONE + 1],
            ]))),
            look: Some(CharacterLook {
                race: word(rof2_at::RACE),
                gender: fixed[rof2_at::GENDER],
                class: fixed[rof2_at::CLASS],
                deity: word(rof2_at::DEITY),
                appearance: Appearance {
                    materials,
                    tints,
                    face: fixed[rof2_at::FACE],
                    hair_style: fixed[rof2_at::HAIR_STYLE],
                    hair_color: fixed[rof2_at::HAIR_COLOR],
                    beard: fixed[rof2_at::BEARD],
                    beard_color: fixed[rof2_at::BEARD_COLOR],
                    show_helm: true,
                    ..Appearance::default()
                },
                eye_colors: rof2_at::EYES.map(|offset| fixed[offset]),
                go_home: fixed[rof2_at::GO_HOME] != 0,
                tutorial: fixed[rof2_at::TUTORIAL] != 0,
            }),
        });
        at += end + 1 + ROF2_ENTRY;
    }
    ensure!(
        at == body.len(),
        "trailing bytes after the RoF2 character list"
    );
    Ok(choices)
}

/// Where each array of a character list starts: every array holds ten
/// entries, one per slot, of the width its reader takes.
struct Layout {
    name: usize,
    level: usize,
    zone: usize,
    race: usize,
    /// Width of a race entry: Titanium's is four bytes, `EQMac`'s two.
    race_width: usize,
    gender: usize,
    class: usize,
    deity: usize,
    deity_width: usize,
    face: usize,
    textures: usize,
    tints: usize,
    primary: usize,
    secondary: usize,
    hair_color: usize,
    hair_style: usize,
    beard: usize,
    beard_color: usize,
    eyes: [usize; 2],
    go_home: Option<usize>,
    tutorial: Option<usize>,
}

/// `EQEmu`'s `common/patches/titanium_structs.h`, `CharacterSelect_Struct`.
const TITANIUM: Layout = Layout {
    name: 1024,
    level: 1694,
    zone: 964,
    race: 0,
    race_width: 4,
    gender: 1664,
    class: 1004,
    deity: 832,
    deity_width: 4,
    face: 1014,
    textures: 420,
    tints: 40,
    primary: 912,
    secondary: 780,
    hair_color: 952,
    hair_style: 410,
    beard: 892,
    beard_color: 400,
    eyes: [1674, 1684],
    go_home: Some(872),
    tutorial: Some(882),
};

/// `EQMacEmu`'s `common/eq_packet_structs.h`, `CharacterSelect_Struct`
/// (Quarm's source has the same).
const EQMAC: Layout = Layout {
    name: 0,
    level: 640,
    zone: 680,
    race: 660,
    race_width: 2,
    gender: 720,
    class: 650,
    deity: 1460,
    deity_width: 2,
    face: 730,
    textures: 740,
    tints: 1100,
    primary: 1480,
    secondary: 1520,
    hair_color: 1560,
    hair_style: 1600,
    beard: 1610,
    beard_color: 1570,
    eyes: [1580, 1590],
    go_home: None,
    tutorial: None,
};

/// One slot's entries in a list's arrays.
struct Record<'a> {
    body: &'a [u8],
    layout: &'a Layout,
    index: usize,
}

impl Record<'_> {
    fn byte(&self, array: usize) -> u8 {
        self.body[array + self.index]
    }

    fn word(&self, array: usize) -> u32 {
        self.at(array + self.index * 4, 4)
    }

    /// A little-endian value of `width` bytes at `offset`.
    fn at(&self, offset: usize, width: usize) -> u32 {
        self.body[offset..offset + width]
            .iter()
            .rev()
            .fold(0, |value, byte| value << 8 | u32::from(*byte))
    }

    /// Entry `slot` of this character's nine-entry array of words.
    fn nine(&self, array: usize, slot: usize) -> u32 {
        self.at(array + (self.index * 9 + slot) * 4, 4)
    }

    fn look(&self) -> CharacterLook {
        let layout = self.layout;
        let materials = std::array::from_fn(|slot| {
            match TextureSlot::from_index(slot) {
                // The held items' model numbers, as spawn records carry them.
                Some(TextureSlot::Primary) => self.word(layout.primary),
                Some(TextureSlot::Secondary) => self.word(layout.secondary),
                _ => self.nine(layout.textures, slot),
            }
        });
        let tints = std::array::from_fn(|slot| {
            let [blue, green, red, used] = self.nine(layout.tints, slot).to_le_bytes();
            (used != 0).then_some([red, green, blue])
        });
        CharacterLook {
            race: self.at(
                layout.race + self.index * layout.race_width,
                layout.race_width,
            ),
            gender: self.byte(layout.gender),
            class: self.byte(layout.class),
            deity: self.at(
                layout.deity + self.index * layout.deity_width,
                layout.deity_width,
            ),
            appearance: Appearance {
                materials,
                tints,
                face: self.byte(layout.face),
                hair_style: self.byte(layout.hair_style),
                hair_color: self.byte(layout.hair_color),
                beard: self.byte(layout.beard),
                beard_color: self.byte(layout.beard_color),
                show_helm: true,
                ..Appearance::default()
            },
            eye_colors: layout.eyes.map(|array| self.byte(array)),
            go_home: layout.go_home.is_some_and(|array| self.byte(array) != 0),
            tutorial: layout.tutorial.is_some_and(|array| self.byte(array) != 0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `RoF2` entry as `EQEmu` writes it, with every field this reads set
    /// to its own value and every other byte 0xee.
    fn rof2_entry(name: &str, level: u8) -> Vec<u8> {
        let mut entry = name.as_bytes().to_vec();
        entry.push(0);
        let mut fixed = vec![0xee; ROF2_ENTRY];
        let mut put = |offset: usize, bytes: &[u8]| {
            fixed[offset..offset + bytes.len()].copy_from_slice(bytes);
        };
        put(rof2_at::CLASS, &[5]);
        put(rof2_at::RACE, &330u32.to_le_bytes());
        put(rof2_at::LEVEL, &[level]);
        put(rof2_at::ZONE, &202u16.to_le_bytes());
        put(rof2_at::GENDER, &[1]);
        put(rof2_at::FACE, &[3]);
        for slot in 0..9u8 {
            let at = rof2_at::EQUIPMENT + usize::from(slot) * 24;
            put(at, &u32::from(10 + slot).to_le_bytes());
            // Blue, green, red, then whether the tint is used: slot 4 has none.
            let used = if slot == 4 { 0 } else { 0xff };
            put(at + 20, &[slot, 2, 3, used]);
        }
        put(rof2_at::DEITY, &211u32.to_le_bytes());
        put(rof2_at::PRIMARY, &27u32.to_le_bytes());
        put(rof2_at::SECONDARY, &28u32.to_le_bytes());
        put(rof2_at::HAIR_COLOR, &[6]);
        put(rof2_at::BEARD_COLOR, &[7]);
        put(rof2_at::EYES[0], &[8]);
        put(rof2_at::EYES[1], &[9]);
        put(rof2_at::HAIR_STYLE, &[2]);
        put(rof2_at::BEARD, &[4]);
        put(rof2_at::GO_HOME, &[1]);
        put(rof2_at::TUTORIAL, &[0]);
        entry.extend(fixed);
        entry
    }

    fn rof2_list(entries: &[Vec<u8>]) -> Vec<u8> {
        let mut body = u32::try_from(entries.len()).unwrap().to_le_bytes().to_vec();
        for entry in entries {
            body.extend(entry);
        }
        body
    }

    #[test]
    fn rof2s_list_reads_every_entry_as_eqemu_writes_it() {
        assert_eq!(decode(GameDialect::Rof2, &rof2_list(&[])).unwrap(), []);
        let body = rof2_list(&[
            rof2_entry("Example", 30),
            rof2_entry("Ab", 1),
            rof2_entry("Another", 65),
        ]);
        let entries = decode(GameDialect::Rof2, &body).unwrap();
        let read: Vec<_> = entries
            .iter()
            .map(|entry| (entry.slot, entry.name.as_str(), entry.level, entry.zone_id))
            .collect();
        assert_eq!(
            read,
            [
                (0, "Example", Some(30), Some(202)),
                (1, "Ab", Some(1), Some(202)),
                (2, "Another", Some(65), Some(202)),
            ]
        );
        let look = entries[1].look.unwrap();
        assert_eq!(
            (look.race, look.gender, look.class, look.deity),
            (330, 1, 5, 211)
        );
        assert_eq!(
            look.appearance.materials,
            [10, 11, 12, 13, 14, 15, 16, 27, 28]
        );
        assert_eq!(look.appearance.tints[0], Some([3, 2, 0]));
        assert_eq!(look.appearance.tints[4], None);
        assert_eq!(look.appearance.tints[8], Some([3, 2, 8]));
        assert_eq!(
            (
                look.appearance.face,
                look.appearance.hair_style,
                look.appearance.hair_color,
                look.appearance.beard,
                look.appearance.beard_color,
            ),
            (3, 2, 6, 4, 7)
        );
        assert_eq!(look.eye_colors, [8, 9]);
        assert!(look.go_home && !look.tutorial);
        // Unchecked against the client, the session keeps only the names.
        let kept = entries[0].clone().without_look(GameDialect::Rof2);
        assert_eq!((kept.level, kept.zone_id, kept.look), (None, None, None));
    }

    #[test]
    fn rof2s_list_refuses_every_length_it_cannot_trust() {
        let one = rof2_list(&[rof2_entry("Example", 30)]);
        for cut in [0, 3, 4, 8, one.len() - 1] {
            assert!(decode(GameDialect::Rof2, &one[..cut]).is_err(), "{cut}");
        }
        let mut trailing = one.clone();
        trailing.push(0);
        assert!(decode(GameDialect::Rof2, &trailing).is_err());
        // A count past what the body holds, however large, is refused
        // before anything is kept.
        for count in [2, u32::MAX] {
            let mut body = one.clone();
            body[..4].copy_from_slice(&count.to_le_bytes());
            assert!(decode(GameDialect::Rof2, &body).is_err(), "{count}");
        }
        let duplicate = rof2_list(&[rof2_entry("Example", 30), rof2_entry("EXAMPLE", 1)]);
        assert!(decode(GameDialect::Rof2, &duplicate).is_err());
        for name in ["", "Ex4mple", "Ex ample"] {
            let body = rof2_list(&[rof2_entry(name, 1)]);
            assert!(decode(GameDialect::Rof2, &body).is_err(), "{name:?}");
        }
        // A name with no NUL anywhere after it.
        let mut unterminated = 1u32.to_le_bytes().to_vec();
        unterminated.extend(vec![b'A'; 1 + ROF2_ENTRY]);
        assert!(decode(GameDialect::Rof2, &unterminated).is_err());
    }
    #[test]
    fn names_only_come_from_occupied_slots_and_preserve_server_spelling() {
        let mut body = vec![0; 1704];
        body[10..15].copy_from_slice(b"Decoy");
        body[1024 + 192..1024 + 199].copy_from_slice(b"Example");
        body[1697] = 12;
        body[976..980].copy_from_slice(&22u32.to_le_bytes());
        let entries = decode(GameDialect::Titanium, &body).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            (
                entries[0].slot,
                entries[0].name.as_str(),
                entries[0].level,
                entries[0].zone_id
            ),
            (3, "Example", Some(12), Some(22))
        );
        assert!(decode(GameDialect::Titanium, &body[..1703]).is_err());
        body[1024..1031].copy_from_slice(b"example");
        assert!(decode(GameDialect::Titanium, &body).is_err());
        let mut mac = vec![0; 1620];
        mac[64..71].copy_from_slice(b"Example");
        let entries = decode(GameDialect::EqMac, &mac).unwrap();
        assert_eq!(entries[0].slot, 1);
        assert!(decode(GameDialect::EqMac, &mac[..1619]).is_err());
        mac[..64].fill(b'A');
        assert!(decode(GameDialect::EqMac, &mac).is_err());
    }

    /// Writes slot 2's every field at its own offset, as `layout` places it.
    fn record(len: usize, layout: &Layout) -> Vec<u8> {
        const SLOT: usize = 2;
        let mut body = vec![0; len];
        let mut put = |offset: usize, bytes: &[u8]| {
            body[offset..offset + bytes.len()].copy_from_slice(bytes);
        };
        put(layout.name + SLOT * 64, b"Example\0");
        put(layout.level + SLOT, &[30]);
        put(layout.zone + SLOT * 4, &22u32.to_le_bytes());
        put(
            layout.race + SLOT * layout.race_width,
            &330u32.to_le_bytes()[..layout.race_width],
        );
        put(layout.gender + SLOT, &[1]);
        put(layout.class + SLOT, &[5]);
        put(
            layout.deity + SLOT * layout.deity_width,
            &211u32.to_le_bytes()[..layout.deity_width],
        );
        put(layout.face + SLOT, &[3]);
        // Chain on the chest (texture slot 1), dyed red; a texture value in
        // the held slots that the model numbers below override.
        put(layout.textures + (SLOT * 9 + 1) * 4, &2u32.to_le_bytes());
        put(layout.textures + (SLOT * 9 + 7) * 4, &99u32.to_le_bytes());
        put(layout.tints + (SLOT * 9 + 1) * 4, &[0x10, 0x20, 0xc0, 0xff]);
        put(layout.primary + SLOT * 4, &10u32.to_le_bytes());
        put(layout.secondary + SLOT * 4, &200u32.to_le_bytes());
        put(layout.hair_color + SLOT, &[4]);
        put(layout.hair_style + SLOT, &[5]);
        put(layout.beard + SLOT, &[6]);
        put(layout.beard_color + SLOT, &[7]);
        put(layout.eyes[0] + SLOT, &[8]);
        put(layout.eyes[1] + SLOT, &[9]);
        if let (Some(home), Some(tutorial)) = (layout.go_home, layout.tutorial) {
            put(home + SLOT, &[1]);
            put(tutorial + SLOT, &[1]);
        }
        body
    }

    #[test]
    fn both_generations_lists_carry_each_characters_look() {
        for (dialect, len, layout) in [
            (GameDialect::Titanium, 1704, &TITANIUM),
            (GameDialect::EqMac, 1620, &EQMAC),
        ] {
            let entries = decode(dialect, &record(len, layout)).unwrap();
            let [entry] = &entries[..] else {
                panic!("{dialect:?}: one character");
            };
            assert_eq!(
                (entry.slot, entry.name.as_str(), entry.level, entry.zone_id),
                (2, "Example", Some(30), Some(22)),
                "{dialect:?}"
            );
            let look = entry.look.unwrap();
            let titanium = dialect == GameDialect::Titanium;
            assert_eq!(
                (look.race, look.gender, look.class, look.deity),
                (330, 1, 5, 211),
                "{dialect:?}"
            );
            let appearance = look.appearance;
            assert_eq!(appearance.material(TextureSlot::Chest), 2);
            assert_eq!(
                appearance.tint(TextureSlot::Chest),
                Some([0xc0, 0x20, 0x10])
            );
            assert_eq!(appearance.tint(TextureSlot::Head), None);
            assert_eq!(appearance.material(TextureSlot::Primary), 10);
            assert_eq!(appearance.material(TextureSlot::Secondary), 200);
            assert_eq!(
                (
                    appearance.face,
                    appearance.hair_color,
                    appearance.hair_style,
                    appearance.beard,
                    appearance.beard_color
                ),
                (3, 4, 5, 6, 7),
                "{dialect:?}"
            );
            assert_eq!(look.eye_colors, [8, 9]);
            assert_eq!((look.go_home, look.tutorial), (titanium, titanium));
        }
    }

    #[test]
    fn a_list_not_checked_for_looks_keeps_only_what_it_did() {
        let titanium = decode(GameDialect::Titanium, &record(1704, &TITANIUM)).unwrap();
        let entry = titanium[0].clone().without_look(GameDialect::Titanium);
        assert_eq!(
            (entry.level, entry.zone_id, entry.look),
            (Some(30), Some(22), None)
        );
        let mac = decode(GameDialect::EqMac, &record(1620, &EQMAC)).unwrap();
        let entry = mac[0].clone().without_look(GameDialect::EqMac);
        assert_eq!((entry.level, entry.zone_id, entry.look), (None, None, None));
    }
}
