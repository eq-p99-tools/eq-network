//! `RoF2` profile and own-spawn bodies built from the field list, for tests
//! here and in the session: our own writer, with filler in every field the
//! readers skip, so a reader that slips by a byte reads filler.
#![allow(missing_docs, clippy::missing_panics_doc, clippy::must_use_candidate)]
use super::{SUFFIX, TITLE};

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
