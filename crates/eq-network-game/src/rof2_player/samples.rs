//! `RoF2` profile and spawn record bodies built from the field list, for
//! tests here and in the session: our own writer, with filler in every field the
//! readers skip, so a reader that slips by a byte reads filler.
#![allow(missing_docs, clippy::missing_panics_doc, clippy::must_use_candidate)]
use super::{DESTRUCTIBLE, DESTRUCTIBLE_BITFIELDS, SUFFIX, TITLE, TREASURE};

/// What fills every field the readers skip.
pub const FILLER: u8 = 0x5a;

/// Builds a body field by field, remembering where each count went.
#[derive(Default)]
pub struct Body {
    /// The body so far.
    pub bytes: Vec<u8>,
    /// Where each count went.
    pub counts: Vec<usize>,
}

impl Body {
    fn byte(&mut self, value: u8) -> &mut Self {
        self.bytes.push(value);
        self
    }

    fn short(&mut self, value: u16) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn word(&mut self, value: u32) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn float(&mut self, value: f32) -> &mut Self {
        self.word(value.to_bits())
    }

    fn count(&mut self, value: u32) -> &mut Self {
        self.counts.push(self.bytes.len());
        self.word(value)
    }

    fn fill(&mut self, length: usize) -> &mut Self {
        self.bytes.extend(std::iter::repeat_n(FILLER, length));
        self
    }

    /// A string and its NUL.
    fn text(&mut self, value: &str) -> &mut Self {
        self.bytes.extend_from_slice(value.as_bytes());
        self.byte(0)
    }

    /// A string in a field of `length` bytes, zero after its end.
    fn field(&mut self, value: &str, length: usize) -> &mut Self {
        let start = self.bytes.len();
        self.text(value);
        self.bytes.resize(start + length, 0);
        self
    }
}

/// The values a sample player's bodies carry.
pub struct Sample {
    pub name: &'static str,
    pub gems: [u32; 16],
    pub position: [f32; 4],
    pub spawn_id: u32,
    pub npc: u8,
    pub other: u8,
    pub properties: u8,
    pub title: &'static str,
    pub suffix: &'static str,
    pub speeds: [f32; 2],
    pub strength: u32,
}

impl Default for Sample {
    fn default() -> Self {
        let mut gems = [u32::MAX; 16];
        gems[..12].copy_from_slice(&[11, 0, 13, 0xffff, 15, 16, 17, 18, 19, 20, 21, 22]);
        Self {
            name: "Tester",
            gems,
            // x, y, z, heading.
            position: [10.5, -20.25, 3.0, 128.0],
            spawn_id: 4321,
            npc: 0,
            other: TITLE | SUFFIX,
            properties: 1,
            title: "Sir",
            suffix: "the Bold",
            speeds: [0.46, 0.7],
            strength: 75,
        }
    }
}

/// A profile as the field list in `ENCODE(OP_PlayerProfile)` lays it out.
#[allow(clippy::too_many_lines)]
pub fn profile_body(sample: &Sample) -> Body {
    let mut body = Body::default();
    // Checksum, then the length word, set at the end; two unknown words.
    body.word(0x1234_5678).word(0).fill(8);
    // Gender, race, class, level twice.
    body.byte(1).word(6).byte(12).byte(35).byte(35);
    body.count(5).fill(5 * 20);
    body.word(201).fill(4);
    body.count(10).fill(10 * 4);
    body.count(22).fill(22 * 20);
    body.count(9).fill(9 * 20);
    body.count(9).fill(9 * 4);
    body.count(9).fill(9 * 4);
    // Hair and beard colour, a word, eyes, hair style, beard, face; the
    // Drakkin heritage, tattoo and details; five bytes; height; three
    // floats; primary and secondary.
    body.fill(2 + 4 + 5)
        .fill(3 * 4)
        .fill(5)
        .fill(4)
        .fill(3 * 4)
        .fill(2 * 4);
    // Practice points, mana, hit points.
    body.word(7).word(900).fill(4);
    body.word(sample.strength);
    for attribute in [80, 85, 90, 95, 100, 105] {
        body.word(attribute);
    }
    body.fill(7 * 4);
    body.count(300).fill(300 * 3 * 4);
    body.count(100);
    for skill in 0..100 {
        body.word(skill * 3);
    }
    body.count(25).fill(25 * 4);
    body.count(300);
    for slot in 0..300 {
        body.word(if slot < 3 { 5000 + slot } else { 0 });
    }
    body.count(20).fill(20 * 4);
    body.count(20).fill(20 * 4);
    body.count(100).fill(100 * 4);
    body.count(720).fill(720 * 4);
    body.count(16);
    for gem in sample.gems {
        body.word(gem);
    }
    body.count(13);
    for refresh in 0..13 {
        body.word(1000 + refresh);
    }
    body.fill(1);
    body.count(42).fill(42 * 80);
    // Coins on the person and the cursor, three words, thirst, hunger, AA
    // points spent.
    body.fill(8 * 4 + 3 * 4 + 3 * 4);
    // The AA tabs, the unspent points, two bytes.
    body.count(5).fill(5 * 4).fill(4).fill(2);
    body.count(20);
    for set in 0..20 {
        body.text(if set == 2 { "Melee" } else { "" });
        for item in 0..4 {
            body.text(if set == 2 && item == 0 {
                "Short Sword"
            } else {
                ""
            })
            .fill(8);
        }
    }
    body.count(5);
    for slot in 0..5 {
        body.text(if slot == 1 { "Healing Potion" } else { "" })
            .fill(8);
    }
    // Four totals, seven resists, five words, a word, four words.
    body.fill(4 * 4).fill(7 * 4).fill(5 * 4).fill(4).fill(4 * 4);
    body.word(450).fill(2 * 4);
    body.count(64).field(sample.name, 64);
    body.count(32).field("Smith", 32);
    // Birthday, account start, last login, time played, time entitled,
    // expansions.
    body.fill(6 * 4);
    body.count(32);
    for language in 0..32 {
        body.byte(if language < 28 { 100 - language } else { 0 });
    }
    let [x, y, z, heading] = sample.position;
    body.short(2)
        .short(0)
        .float(y)
        .float(x)
        .float(z)
        .float(heading);
    // Four bytes, the guild, its rank, a word, a byte, a word, experience,
    // a byte, the bank, shared platinum, three words, a word, two words,
    // career tribute, a word, tribute, a word, two bytes.
    body.fill(4)
        .fill(4)
        .fill(1)
        .fill(4)
        .fill(1)
        .fill(4)
        .fill(8)
        .fill(1);
    body.fill(4 * 4)
        .fill(4 * 4)
        .fill(4)
        .fill(2 * 4)
        .fill(4 * 4)
        .fill(2);
    body.count(5).fill(5 * 8);
    body.count(10).fill(10 * 8);
    // Six words, 125 bytes, two words, the crystals, three words.
    body.fill(6 * 4)
        .fill(125)
        .fill(2 * 4)
        .fill(4 * 4)
        .fill(3 * 4);
    body.count(64).fill(64);
    body.fill(3)
        .fill(2 * 4)
        .fill(3)
        .fill(4)
        .fill(3)
        .fill(4)
        .fill(1)
        .fill(4);
    body.count(64).fill(64);
    body.count(64).fill(64);
    body.fill(4).fill(320).fill(343);
    // The character's number, the leadership switch.
    body.fill(4).fill(1);
    body.count(6).fill(6 * 4);
    // Points available, two leadership experiences, two point counts.
    body.fill(4).fill(2 * 8).fill(2 * 4);
    body.count(64).fill(64 * 4);
    // Air, seven PvP numbers.
    body.fill(4).fill(7 * 4);
    body.text("").fill(6 * 4);
    body.text("Victim").fill(6 * 4);
    body.fill(4);
    body.count(50);
    for kill in 0..50 {
        body.text(if kill == 0 { "Victim" } else { "" }).fill(6 * 4);
    }
    // AA experience, four words, four consents, the level, the helm, the
    // rest timer.
    body.fill(4).fill(4 * 4).fill(4).fill(4).fill(1).fill(4);
    body.count(1024).fill(1024).fill(2 * 4).fill(1);
    set_length(&mut body.bytes);
    body
}

/// The length word: the body's length less 9.
pub fn set_length(bytes: &mut [u8]) {
    let length = u32::try_from(bytes.len() - 9).unwrap();
    bytes[4..8].copy_from_slice(&length.to_le_bytes());
}

/// A player's own spawn as the field list in `ENCODE(OP_ZoneSpawns)` lays
/// it out.
pub fn spawn_body(sample: &Sample) -> Vec<u8> {
    let mut body = Body::default();
    body.text(sample.name).word(sample.spawn_id);
    // Level, eye height.
    body.fill(1).fill(4);
    body.byte(sample.npc).fill(4).byte(sample.other);
    // Emitter radius and id.
    body.fill(8);
    body.byte(sample.properties);
    if sample.properties == 1 {
        body.fill(4);
    }
    body.byte(100);
    // Six colour and feature bytes, the Drakkin words, four bytes.
    body.fill(6).fill(12).fill(4);
    body.float(2.5)
        .fill(1)
        .float(sample.speeds[0])
        .float(sample.speeds[1]);
    // Race, holding, deity, guild, rank, class, PvP.
    body.fill(4).fill(1).fill(4).fill(4).fill(4).fill(1).fill(1);
    body.byte(110).fill(2);
    body.text("Smith");
    // AA title, guild shown, temporary pet, pet owner, find bits, player
    // state, three tint indexes, two weapon words.
    body.fill(4)
        .fill(1)
        .fill(1)
        .fill(4)
        .fill(1)
        .fill(4)
        .fill(3 * 4)
        .fill(2 * 4);
    // Nine tints, nine textures of five words, the packed position.
    body.fill(9 * 4).fill(9 * 5 * 4).fill(5 * 4);
    if sample.other & TITLE != 0 {
        body.text(sample.title);
    }
    if sample.other & SUFFIX != 0 {
        body.text(sample.suffix);
    }
    body.fill(2 * 4)
        .fill(1)
        .text("0000000000000000")
        .fill(2 * 4)
        .fill(29);
    body.bytes
}

/// The values one spawn record carries, for any branch.
#[derive(Clone)]
pub struct Record {
    pub name: &'static str,
    pub spawn_id: u32,
    pub level: u8,
    pub npc: u8,
    pub bitfields: u32,
    pub other: u8,
    /// The model, the second name and the string a treasure chest or a
    /// destructible object carries; written when `object` is set.
    pub object: Option<[&'static str; 3]>,
    pub properties: u8,
    pub body_type: u32,
    pub hp: u8,
    pub size: f32,
    pub speeds: [f32; 2],
    pub race: u32,
    pub class: u8,
    pub stand: u8,
    pub last_name: &'static str,
    pub pet_owner: u32,
    /// Nine tints and nine textures (true) or three textures.
    pub dressed: bool,
    pub position: [u32; 5],
    /// Written when not empty, whatever `other` says.
    pub title: &'static str,
    pub suffix: &'static str,
}

impl Record {
    /// A creature of a race that is not dressed: three textures.
    pub fn creature() -> Self {
        Self {
            name: "a_rat01",
            spawn_id: 55,
            level: 3,
            npc: 1,
            // Female (1), and an invisibility of 5.
            bitfields: 1 | (5 << 10),
            other: 0,
            object: None,
            properties: 1,
            body_type: 21,
            hp: 80,
            size: 2.0,
            speeds: [0.3, 0.9],
            race: 36,
            class: 1,
            stand: 100,
            last_name: "",
            pet_owner: 0,
            dressed: false,
            position: pack(&Packed {
                x: 80,
                y: -160,
                z: 24,
                heading: 1024,
                ..Packed::default()
            }),
            title: "",
            suffix: "",
        }
    }

    /// Another player, with a title and a suffix: dressed.
    pub fn player() -> Self {
        Self {
            name: "Other",
            spawn_id: 77,
            level: 20,
            npc: 0,
            bitfields: 0,
            other: TITLE | SUFFIX,
            race: 1,
            last_name: "Doe",
            class: 3,
            stand: 110,
            dressed: true,
            title: "Lady",
            suffix: "the Kind",
            ..Self::creature()
        }
    }

    /// An NPC of a dressed race.
    pub fn dressed_npc() -> Self {
        Self {
            name: "Guard_Tester00",
            race: 128,
            dressed: true,
            ..Self::creature()
        }
    }

    /// An `LDoN` treasure chest.
    pub fn treasure() -> Self {
        Self {
            name: "a_chest00",
            other: TREASURE,
            class: 62,
            object: Some(["chest_model", "", "click me"]),
            ..Self::creature()
        }
    }

    /// A destructible object, with the encoder's bits.
    pub fn destructible() -> Self {
        Self {
            name: "a_barrel00",
            bitfields: DESTRUCTIBLE_BITFIELDS,
            other: DESTRUCTIBLE,
            object: Some(["barrel_model", "barrel", ""]),
            properties: 0,
            ..Self::creature()
        }
    }

    /// The length `ENCODE(OP_ZoneSpawns)` sizes this record at.
    pub fn encoder_length(&self) -> usize {
        let mut length = 206 + self.name.len() + self.last_name.len();
        for part in [self.title, self.suffix] {
            if !part.is_empty() {
                length += part.len() + 1;
            }
        }
        if let Some(strings) = self.object {
            if self.properties == 0 {
                length -= 4;
            }
            length += 53 + strings.iter().map(|string| string.len() + 1).sum::<usize>();
        }
        length + if self.dressed { 216 } else { 60 }
    }
}

/// A spawn record as the field list in `ENCODE(OP_ZoneSpawns)` lays it out,
/// with filler in every field the walker skips.
pub fn record_body(record: &Record) -> Vec<u8> {
    let mut body = Body::default();
    body.text(record.name)
        .word(record.spawn_id)
        .byte(record.level);
    // Eye height.
    body.fill(4);
    body.byte(record.npc)
        .word(record.bitfields)
        .byte(record.other);
    // Emitter radius and id.
    body.fill(8);
    if let Some(strings) = record.object {
        for string in strings {
            body.text(string);
        }
        // Appearance, animation, ten sound ids, collision byte, type.
        body.fill(4 + 4 + 10 * 4 + 1 + 4);
    }
    body.byte(record.properties);
    if record.properties == 1 {
        body.word(record.body_type);
    }
    body.byte(record.hp);
    // Six colour and feature bytes, the Drakkin words, four bytes.
    body.fill(6).fill(12).fill(4);
    body.float(record.size)
        .fill(1)
        .float(record.speeds[0])
        .float(record.speeds[1])
        .word(record.race);
    // Holding, deity, guild, rank.
    body.fill(1).fill(4).fill(8);
    body.byte(record.class).fill(1).byte(record.stand).fill(2);
    body.text(record.last_name);
    // AA title, guild shown, temporary pet.
    body.fill(4 + 1 + 1).word(record.pet_owner);
    // Find bits, player state, three tint indexes, two weapon words.
    body.fill(1 + 4 + 12 + 8);
    body.fill(if record.dressed {
        9 * 4 + 9 * 20
    } else {
        3 * 20
    });
    for word in record.position {
        body.word(word);
    }
    for part in [record.title, record.suffix] {
        if !part.is_empty() {
            body.text(part);
        }
    }
    body.fill(8)
        .fill(1)
        .text("0000000000000000")
        .fill(8)
        .fill(29);
    body.bytes
}

/// The packed position's fields, as raw bit values.
#[derive(Clone, Copy, Default)]
pub struct Packed {
    /// Signed 19-bit values, in eighths of a unit.
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// 12 bits, in quarters of the 512-unit turn.
    pub heading: u32,
    pub angle: u32,
    pub delta_x: i32,
    pub delta_y: i32,
    pub delta_z: i32,
    pub delta_heading: i32,
    pub animation: i32,
}

/// Packs the fields into five words, the bit positions written out here as
/// our own table of the active `Spawn_Struct_Position`: (word, lowest bit,
/// width) for each field.
pub fn pack(packed: &Packed) -> [u32; 5] {
    let mut words = [0u32; 5];
    #[allow(clippy::cast_sign_loss)]
    let fields: [(i64, usize, u32, u32); 10] = [
        (i64::from(packed.angle), 0, 0, 12),
        (i64::from(packed.y), 0, 12, 19),
        (i64::from(packed.delta_z), 1, 0, 13),
        (i64::from(packed.delta_x), 1, 13, 13),
        (i64::from(packed.x), 2, 0, 19),
        (i64::from(packed.heading), 2, 19, 12),
        (i64::from(packed.delta_heading), 3, 0, 10),
        (i64::from(packed.z), 3, 10, 19),
        (i64::from(packed.animation), 4, 0, 10),
        (i64::from(packed.delta_y), 4, 10, 13),
    ];
    for (value, word, shift, width) in fields {
        let mask = (1u64 << width) - 1;
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let bits = (value as u64 & mask) as u32;
        words[word] |= bits << shift;
    }
    words
}
