//! `RoF2`'s player profile and its spawn records, as stock `EQEmu` writes
//! them, inferred for `RoF2` (6.502, 6.510).
//!
//! Neither is a struct. Each one is a stream with counted arrays and
//! terminated strings, written field by field (`common/patches/rof2.cpp`:
//! `ENCODE(OP_PlayerProfile)`, and `ENCODE(OP_ZoneSpawns)`, to which
//! `ENCODE(OP_ZoneEntry)` and `ENCODE(OP_NewSpawn)` forward). The readers
//! walk the whole of each stream in the encoder's order. Every count must be
//! the constant the encoder writes, every string must end inside its field,
//! and the walk must end exactly at the body's end. No count sizes an
//! allocation.
//!
//! Stock `EQEmu` sends every spawn a `RoF2` client sees as one record of its
//! own under one number, `OP_ZoneEntry`: the player's own spawn first, then
//! the zone's spawns (`EntityList::SendZoneSpawnsBulk` in `zone/entity.cpp`
//! fills a bulk packet, whose encoder writes one record per spawn) and each
//! later one. One walker reads them all
//! ([`spawn`](crate::rof2_player::spawn)); the own spawn is a player's
//! record ([`player`](crate::rof2_player::player)).
//!
//! What the `RoF2` program reads from each field is not read yet (6.502,
//! 6.510). That includes whether it checks the profile's checksum, gems 9 to
//! 12, the appearance and `/who` fields, a spawn's motion deltas, and what
//! it does with a record the encoder rewrote for a body type of 66 and up.
use crate::world::{
    profile_heading, BaseAttributes, PlayerState, Position, PostureState, SpawnKind, SpawnState,
};
use anyhow::{ensure, Context, Result};

/// `OP_PlayerProfile` (`utils/patches/patch_RoF2.conf`; inferred, 6.502).
pub const PROFILE_OPCODE: u16 = 0x6506;
/// `OP_ZoneEntry` from the zone: every spawn record, the player's own
/// first (`utils/patches/patch_RoF2.conf`; inferred, 6.502 and 6.510).
pub const SPAWN_OPCODE: u16 = 0x5089;
/// The player's own spawn, the first record under [`SPAWN_OPCODE`].
pub const OWN_SPAWN_OPCODE: u16 = SPAWN_OPCODE;

/// The longest string the profile holds, with its NUL: names of 64 bytes
/// (`rof2_structs.h`).
const PROFILE_STRING: usize = 64;
/// A spawn's name field with its NUL (`eq_packet_structs.h`, `Spawn_Struct`).
const SPAWN_NAME: usize = 64;
/// A spawn's last name, title and suffix fields with their NUL (the encoder
/// cuts title and suffix at 32; `Spawn_Struct` gives the last name 32).
const SPAWN_PART: usize = 32;

/// The counts the profile encoder writes, each followed by that many entries
/// (`rof2_structs.h`, `rof2_limits.h`; `MAX_PP_*`, `BUFF_COUNT`,
/// `SPELLBOOK_SIZE`, `SPELL_GEM_COUNT`, `BANDOLIERS_SIZE`, `POTION_BELT_SIZE`,
/// `TRIBUTE_SIZE`).
mod counts {
    pub(super) const BINDS: u32 = 5;
    pub(super) const UNKNOWN_TEN: u32 = 10;
    pub(super) const EQUIPMENT: u32 = 22;
    pub(super) const MATERIALS: u32 = 9;
    pub(super) const AA: u32 = 300;
    pub(super) const SKILLS: u32 = 100;
    pub(super) const INNATE: u32 = 25;
    pub(super) const DISCIPLINES: u32 = 300;
    pub(super) const TIMESTAMPS: u32 = 20;
    pub(super) const RECAST: u32 = 20;
    pub(super) const TIMESTAMPS2: u32 = 100;
    pub(super) const SPELLBOOK: u32 = 720;
    pub(super) const GEMS: u32 = 16;
    pub(super) const REFRESH: u32 = 13;
    pub(super) const BUFFS: u32 = 42;
    pub(super) const AA_TABS: u32 = 5;
    pub(super) const BANDOLIERS: u32 = 20;
    pub(super) const BANDOLIER_ITEMS: usize = 4;
    pub(super) const POTIONS: u32 = 5;
    pub(super) const NAME: u32 = 64;
    pub(super) const LAST_NAME: u32 = 32;
    pub(super) const LANGUAGES: u32 = 32;
    pub(super) const TRIBUTES: u32 = 5;
    pub(super) const GUILD_TRIBUTES: u32 = 10;
    pub(super) const UNKNOWN_STRING: u32 = 64;
    pub(super) const LDON: u32 = 6;
    pub(super) const LEADERSHIP: u32 = 64;
    pub(super) const RECENT_KILLS: u32 = 50;
    pub(super) const TRAILING_BLOCK: u32 = 1024;
}

/// A bounded reader over a body: every read checks what is left.
struct Cursor<'a> {
    body: &'a [u8],
    at: usize,
    what: &'static str,
}

impl<'a> Cursor<'a> {
    const fn new(body: &'a [u8], what: &'static str) -> Self {
        Self { body, at: 0, what }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(length)
            .filter(|end| *end <= self.body.len())
            .with_context(|| format!("truncated RoF2 {}", self.what))?;
        let bytes = &self.body[self.at..end];
        self.at = end;
        Ok(bytes)
    }

    fn skip(&mut self, length: usize) -> Result<()> {
        self.take(length).map(|_| ())
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn short(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn word(&mut self) -> Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn float(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.word()?))
    }

    fn words(&mut self, count: usize) -> Result<Vec<u32>> {
        let bytes = self.take(count.checked_mul(4).context("RoF2 word count")?)?;
        Ok(bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| u32::from_le_bytes(*word))
            .collect())
    }

    /// A count the encoder writes as a constant; any other value is refused.
    fn count(&mut self, expected: u32, of: &str) -> Result<()> {
        let count = self.word()?;
        ensure!(
            count == expected,
            "invalid RoF2 {} {of} count {count}",
            self.what
        );
        Ok(())
    }

    /// A NUL-terminated string that ends, NUL and all, within `field` bytes.
    fn string(&mut self, field: usize) -> Result<&'a [u8]> {
        let rest = &self.body[self.at..];
        let end = rest
            .iter()
            .take(field)
            .position(|byte| *byte == 0)
            .with_context(|| format!("unterminated RoF2 {} string", self.what))?;
        let text = &rest[..end];
        self.at += end + 1;
        Ok(text)
    }

    /// A string in a fixed field of `field` bytes, terminated inside it.
    fn fixed_string(&mut self, field: usize) -> Result<&'a [u8]> {
        let bytes = self.take(field)?;
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .with_context(|| format!("unterminated RoF2 {} field", self.what))?;
        Ok(&bytes[..end])
    }

    fn end(&self) -> Result<()> {
        ensure!(
            self.at == self.body.len(),
            "{} trailing bytes after the RoF2 {}",
            self.body.len() - self.at,
            self.what
        );
        Ok(())
    }
}

/// What the player takes from the profile.
#[derive(Debug, PartialEq)]
struct Profile {
    gender: u8,
    race: u32,
    class: u8,
    level: u8,
    deity: u32,
    practice_points: u32,
    mana: u32,
    attributes: [u32; 7],
    skills: Vec<u32>,
    disciplines: Vec<u32>,
    gems: Vec<u32>,
    refresh: Vec<u32>,
    endurance: u32,
    name: String,
    languages: Vec<u8>,
    /// x, y, z and the heading as saved; the encoder writes y first.
    position: [f32; 4],
}

/// Walks the profile the way `ENCODE(OP_PlayerProfile)` writes it.
///
/// # Errors
/// Refuses a length word that isn't the body's length less 9, any count
/// other than the encoder's, a string that doesn't end in its field, a cut
/// body, and a trailing byte.
#[allow(clippy::too_many_lines)]
fn profile(body: &[u8]) -> Result<Profile> {
    let mut read = Cursor::new(body, "profile");
    // The checksum is not checked: what the RoF2 program does with a wrong
    // one is not read (6.502).
    read.skip(4)?;
    let length = read.word()?;
    ensure!(
        usize::try_from(length).ok() == body.len().checked_sub(9),
        "invalid RoF2 profile length word"
    );
    read.skip(8)?;
    let gender = read.byte()?;
    let race = read.word()?;
    let class = read.byte()?;
    let level = read.byte()?;
    read.skip(1)?;
    read.count(counts::BINDS, "bind")?;
    read.skip(5 * 20)?;
    let deity = read.word()?;
    read.skip(4)?;
    read.count(counts::UNKNOWN_TEN, "first unknown")?;
    read.skip(10 * 4)?;
    read.count(counts::EQUIPMENT, "equipment")?;
    read.skip(22 * 20)?;
    read.count(counts::MATERIALS, "second equipment")?;
    read.skip(9 * 20)?;
    read.count(counts::MATERIALS, "tint")?;
    read.skip(9 * 4)?;
    read.count(counts::MATERIALS, "second tint")?;
    read.skip(9 * 4)?;
    // Colours and features, the Drakkin words, five unknown bytes, the height,
    // three unknown floats, and two unknown words.
    read.skip(11 + 12 + 5 + 16 + 8)?;
    let practice_points = read.word()?;
    let mana = read.word()?;
    read.skip(4)?;
    let mut attributes = [0; 7];
    for attribute in &mut attributes {
        *attribute = read.word()?;
    }
    read.skip(7 * 4)?;
    read.count(counts::AA, "AA")?;
    read.skip(300 * 12)?;
    read.count(counts::SKILLS, "skill")?;
    let skills = read.words(100)?;
    read.count(counts::INNATE, "innate skill")?;
    read.skip(25 * 4)?;
    read.count(counts::DISCIPLINES, "discipline")?;
    let disciplines = read.words(300)?;
    read.count(counts::TIMESTAMPS, "timestamp")?;
    read.skip(20 * 4)?;
    read.count(counts::RECAST, "recast")?;
    read.skip(20 * 4)?;
    read.count(counts::TIMESTAMPS2, "second timestamp")?;
    read.skip(100 * 4)?;
    read.count(counts::SPELLBOOK, "spellbook")?;
    read.skip(720 * 4)?;
    read.count(counts::GEMS, "gem")?;
    let gems = read.words(16)?;
    read.count(counts::REFRESH, "gem refresh")?;
    let refresh = read.words(13)?;
    read.skip(1)?;
    read.count(counts::BUFFS, "buff")?;
    read.skip(42 * 80)?;
    // Coins on the person and the cursor, three unknown words, thirst,
    // hunger, and AA points spent.
    read.skip(14 * 4)?;
    read.count(counts::AA_TABS, "AA tab")?;
    read.skip(5 * 4 + 4 + 2)?;
    read.count(counts::BANDOLIERS, "bandolier")?;
    for _ in 0..counts::BANDOLIERS {
        read.string(PROFILE_STRING)?;
        for _ in 0..counts::BANDOLIER_ITEMS {
            read.string(PROFILE_STRING)?;
            read.skip(8)?;
        }
    }
    read.count(counts::POTIONS, "potion belt")?;
    for _ in 0..counts::POTIONS {
        read.string(PROFILE_STRING)?;
        read.skip(8)?;
    }
    // Four totals, seven base resists and five unknown words, a word of 20
    // and four more unknown.
    read.skip(4 * 4 + 12 * 4 + 4 + 4 * 4)?;
    let endurance = read.word()?;
    read.skip(8)?;
    read.count(counts::NAME, "name")?;
    let name = read.fixed_string(64)?;
    read.count(counts::LAST_NAME, "last name")?;
    read.fixed_string(32)?;
    // Birthday twice, last login, time played, time entitled, expansions.
    read.skip(6 * 4)?;
    read.count(counts::LANGUAGES, "language")?;
    let languages = read.take(32)?.to_vec();
    // The zone and its instance.
    read.short()?;
    read.short()?;
    let y = read.float()?;
    let x = read.float()?;
    let z = read.float()?;
    let heading = read.float()?;
    // PvP, GM, guild and rank, unknowns, experience, eye height, the bank,
    // shared platinum and unknowns, tribute points.
    read.skip(4 + 4 + 1 + 4 + 1 + 4 + 8 + 1 + 16 + 16 + 4 + 8 + 16 + 2)?;
    read.count(counts::TRIBUTES, "tribute")?;
    read.skip(5 * 8)?;
    read.count(counts::GUILD_TRIBUTES, "guild tribute")?;
    read.skip(10 * 8)?;
    read.skip(6 * 4 + 125 + 9 * 4)?;
    read.count(counts::UNKNOWN_STRING, "first unknown string")?;
    read.skip(64 + 3 + 8 + 3 + 4 + 3 + 4 + 1 + 4)?;
    read.count(counts::UNKNOWN_STRING, "second unknown string")?;
    read.skip(64)?;
    read.count(counts::UNKNOWN_STRING, "third unknown string")?;
    read.skip(64 + 4 + 320 + 343)?;
    // The character's number and the leadership switch.
    read.skip(4 + 1)?;
    read.count(counts::LDON, "LDoN")?;
    read.skip(6 * 4 + 4 + 16 + 8)?;
    read.count(counts::LEADERSHIP, "leadership")?;
    read.skip(64 * 4 + 4 + 7 * 4)?;
    // The last kill and the last death.
    for _ in 0..2 {
        read.string(PROFILE_STRING)?;
        read.skip(6 * 4)?;
    }
    read.skip(4)?;
    read.count(counts::RECENT_KILLS, "recent kill")?;
    for _ in 0..counts::RECENT_KILLS {
        read.string(PROFILE_STRING)?;
        read.skip(6 * 4)?;
    }
    // AA experience and unknowns, the consents, the level again, the helm,
    // and the rest timer.
    read.skip(5 * 4 + 4 + 4 + 1 + 4)?;
    read.count(counts::TRAILING_BLOCK, "trailing block")?;
    read.skip(1024 + 8 + 1)?;
    read.end()?;
    Ok(Profile {
        gender,
        race,
        class,
        level,
        deity,
        practice_points,
        mana,
        attributes,
        skills,
        disciplines,
        gems,
        refresh,
        endurance,
        name: text(name)?,
        languages,
        position: [x, y, z, heading],
    })
}

/// A name or name part: printable ASCII.
fn text(bytes: &[u8]) -> Result<String> {
    ensure!(
        bytes
            .iter()
            .all(|byte| byte.is_ascii_graphic() || *byte == b' '),
        "invalid RoF2 name"
    );
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

/// The "other data" bits after the spawn's bitfields
/// (`ENCODE(OP_ZoneSpawns)`): a title and a suffix follow the position when
/// set, and a buyer sets the lowest.
const TITLE: u8 = 0x10;
const SUFFIX: u8 = 0x20;
/// A treasure chest's bit: the encoder sets it for a spawn of the `LDoN`
/// treasure class.
const TREASURE: u8 = 0x04;
/// What a destructible object sets, all of it: 0xe1. Of its bits, 0x80
/// and 0x40 come from nothing else, so either one names a destructible,
/// and a record with one but not the rest of 0xe1 is refused.
const DESTRUCTIBLE: u8 = 0xe1;
const DESTRUCTIBLE_ONLY: u8 = 0x80 | 0x40;
/// The word the encoder writes over a destructible object's bitfields.
const DESTRUCTIBLE_BITFIELDS: u32 = 0x1d60_0000;
/// The races whose records carry nine tints and nine textures even for a
/// non-player: every race to the gnome's (12), the Iksar, the Vah Shir,
/// the Froglok and the Drakkin (`common/races.h`). Other non-players carry
/// three textures.
const DRESSED_RACES: [u32; 4] = [128, 130, 330, 522];
/// The real estate GUID the encoder writes in every record: 16 characters
/// and a NUL. The walker takes any 16 and no other length.
const GUID: &str = "0000000000000000";
/// The record's tail after the title and suffix: two words, the mercenary
/// byte, the GUID, two real estate ids, and 29 bytes.
const TAIL: usize = 8 + 1 + GUID.len() + 1 + 8 + 29;

/// What a spawn record is, by the branches the encoder takes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Object {
    /// A player, creature or corpse.
    Mob,
    /// An `LDoN` treasure chest: three strings and a fixed block after the
    /// emitter words.
    Treasure,
    /// A destructible object: the same strings and block, its bitfields
    /// written over, and no body type.
    Destructible,
}

/// Everything read from one spawn record.
#[derive(Debug, PartialEq)]
struct Record {
    name: String,
    id: u32,
    level: u8,
    npc: u8,
    object: Object,
    /// Gender (bits 0 and 1) and invisibility (bits 10 to 21), from the
    /// bitfields; zero for a destructible object, whose bitfields are the
    /// encoder's constant.
    gender: u8,
    invisibility: u16,
    body_type: Option<u32>,
    hp: u8,
    size: f32,
    walk_speed: f32,
    run_speed: f32,
    race: u32,
    class: u8,
    stand: u8,
    last_name: String,
    pet_owner: u32,
    /// The packed position's five words.
    position: [u32; 5],
    title: String,
    suffix: String,
}

/// Walks one spawn record the way `ENCODE(OP_ZoneSpawns)` writes it
/// (`common/patches/rof2.cpp`), choosing each branch from the record's own
/// bytes as the encoder chose it from the spawn:
///
/// - the "other data" byte names a treasure chest (0x04, the encoder's
///   `class_ == Class::LDoNTreasure`) and a destructible object (0xe1, its
///   `DestructibleObject`). Either one is followed by three strings and a
///   53-byte block. Both set is a destructible object of the treasure
///   class, which the encoder writes once, as a destructible.
/// - The property count is 0 for a destructible object and 1, followed by
///   the body type, for everything else; any other count is refused.
/// - Nine tints and nine textures (216 bytes) follow for a player (NPC
///   byte 0) and for the dressed races; three textures (60 bytes) for
///   every other non-player. The race is the one sent: the encoder's
///   rewrite of body types 66 and up to race 127 comes before its test, and
///   is read as sent.
/// - A title follows the position when its bit is set. A suffix follows
///   when its bit is set, except on a destructible object, whose 0xe1 sets
///   the suffix bit whether it has one or not: there a suffix follows when
///   more bytes are left than the record's fixed tail (inferred, 6.510).
///
/// # Errors
/// Refuses a record whose bits contradict each other (one of 0x80 and
/// 0x40 without the rest of 0xe1, or a destructible object's bitfields not
/// the encoder's constant), a property count the branch doesn't take, an
/// unterminated string, an empty flagged title or suffix, a GUID that
/// isn't 16 characters, a cut record and a trailing byte.
fn record(body: &[u8]) -> Result<Record> {
    let mut read = Cursor::new(body, "spawn");
    let name = text(read.string(SPAWN_NAME)?)?;
    let id = read.word()?;
    let level = read.byte()?;
    // The eye height (a destructible object's melee range).
    read.skip(4)?;
    let npc = read.byte()?;
    let bitfields = read.word()?;
    let other = read.byte()?;
    let object = if other & DESTRUCTIBLE_ONLY != 0 {
        ensure!(
            other & DESTRUCTIBLE == DESTRUCTIBLE && bitfields == DESTRUCTIBLE_BITFIELDS,
            "contradictory RoF2 spawn bits"
        );
        Object::Destructible
    } else if other & TREASURE != 0 {
        Object::Treasure
    } else {
        Object::Mob
    };
    // The emitter radius and its id.
    read.skip(8)?;
    if object != Object::Mob {
        // The model, two names, the appearance, the animation, ten sound
        // ids, the collision byte and the object type.
        for _ in 0..3 {
            read.string(SPAWN_NAME)?;
        }
        read.skip(4 + 4 + 10 * 4 + 1 + 4)?;
    }
    let properties = read.byte()?;
    let body_type = match (object, properties) {
        (Object::Destructible, 0) => None,
        (Object::Mob | Object::Treasure, 1) => Some(read.word()?),
        _ => anyhow::bail!("invalid RoF2 spawn property count"),
    };
    let hp = read.byte()?;
    // Colours and features, the Drakkin words, the illusion and head bytes.
    read.skip(6 + 12 + 4)?;
    let size = read.float()?;
    read.skip(1)?;
    let walk_speed = read.float()?;
    let run_speed = read.float()?;
    let race = read.word()?;
    // Holding, deity, guild and rank.
    read.skip(1 + 4 + 8)?;
    let class = read.byte()?;
    // PvP.
    read.skip(1)?;
    let stand = read.byte()?;
    // Light and fly mode.
    read.skip(2)?;
    let last_name = text(read.string(SPAWN_PART)?)?;
    // AA title, guild shown, temporary pet.
    read.skip(4 + 1 + 1)?;
    let pet_owner = read.word()?;
    // Find bits, player state, three tint indexes and two weapon words.
    read.skip(1 + 4 + 12 + 8)?;
    let dressed = npc == 0 || race <= 12 || DRESSED_RACES.contains(&race);
    read.skip(if dressed { 9 * 4 + 9 * 20 } else { 3 * 20 })?;
    let mut position = [0; 5];
    for word in &mut position {
        *word = read.word()?;
    }
    let title = flagged(&mut read, other & TITLE != 0)?;
    let suffix = if object == Object::Destructible {
        body.len() - read.at > TAIL
    } else {
        other & SUFFIX != 0
    };
    let suffix = flagged(&mut read, suffix)?;
    read.skip(8 + 1)?;
    ensure!(
        read.string(GUID.len() + 1)?.len() == GUID.len(),
        "invalid RoF2 spawn tail"
    );
    read.skip(8 + 29)?;
    read.end()?;
    let (gender, invisibility) = if object == Object::Destructible {
        (0, 0)
    } else {
        (
            u8::try_from(bitfields & 0x3).expect("two bits"),
            u16::try_from((bitfields >> 10) & 0xfff).expect("twelve bits"),
        )
    };
    Ok(Record {
        name,
        id,
        level,
        npc,
        object,
        gender,
        invisibility,
        body_type,
        hp,
        size,
        walk_speed,
        run_speed,
        race,
        class,
        stand,
        last_name,
        pet_owner,
        position,
        title,
        suffix,
    })
}

/// A title or suffix the record flags: present, ended in its field and
/// not empty.
fn flagged(read: &mut Cursor<'_>, present: bool) -> Result<String> {
    if !present {
        return Ok(String::new());
    }
    let part = read.string(SPAWN_PART)?;
    ensure!(!part.is_empty(), "empty flagged RoF2 name part");
    text(part)
}

/// The packed position, from the active `Spawn_Struct_Position` in
/// `common/patches/rof2_structs.h` (the second definition; the first is
/// commented out). Its five words, lowest bit first:
///
/// - word 0: the angle (bits 0 to 11), y (12 to 30), padding (31);
/// - word 1: delta z (0 to 12), delta x (13 to 25), padding (26 to 31);
/// - word 2: x (0 to 18), the heading (19 to 30), padding (31);
/// - word 3: delta heading (0 to 9), z (10 to 28), padding (29 to 31);
/// - word 4: the animation (0 to 9), delta y (10 to 22), padding (23 to 31).
///
/// x, y and z are signed 19-bit values in eighths of a unit, and the
/// heading 12 bits in quarters of the 512-unit turn: `Mob::FillSpawnStruct`
/// (`zone/mob.cpp`) fills them with `FloatToEQ19` and `FloatToEQ12`, whose
/// inverses are `EQ19toFloat` and `EQ12toFloat` (`common/misc_functions.cpp`).
/// The struct declares the heading signed, but `FloatToEQ12` never writes
/// past 2047, so it is read as unsigned 12 bits, as Titanium's is. From
/// integers no value can be non-finite. The angle, the deltas and the
/// animation are not read: stock `EQEmu` sends a spawn's deltas and
/// animation as zero (`EntityList::SendZoneSpawnsBulk` clears the record
/// and `FillSpawnStruct` sets neither), and how the `RoF2` program takes
/// them is not read (6.510).
fn packed_position(words: [u32; 5]) -> Position {
    Position {
        x: crate::world::signed_position(words[2]),
        y: crate::world::signed_position(words[0] >> 12),
        z: crate::world::signed_position(words[3] >> 10),
        heading: f32::from(u16::try_from((words[2] >> 19) & 0xfff).expect("twelve bits")) / 4.0,
    }
}

/// Another spawn from its record: a player, a creature, a corpse, a
/// treasure chest or a destructible object, each walked whole.
///
/// The kind is the NPC byte: 0 a player, 1 an NPC, 2 a player's corpse and
/// 3 an NPC's (`Spawn_Struct`'s note in `common/eq_packet_structs.h`;
/// inferred for `RoF2`, 6.510), any other `Unknown` and walked as the
/// encoder writes a non-player. A nonzero invisibility value is taken as
/// invisible, and a pet owner of 0 as no owner (both inferred, 6.510).
/// Appearance and `/who` listing stay at their defaults, the velocity and
/// forward input at zero: none of their fields is read (6.510).
///
/// # Errors
/// Refuses a record whose bits contradict each other (one of 0x80 and
/// 0x40 without the rest of 0xe1, or a destructible object's bitfields not
/// the encoder's constant), a property count the branch doesn't take, an
/// unterminated string, an empty flagged title or suffix, a GUID that
/// isn't 16 characters, a cut record, a trailing byte, a spawn id that is
/// zero or past 16 bits, and a size that is negative or not finite. A walk
/// or run speed that is negative or not finite costs the spawn only that
/// speed, as a Titanium record's does.
pub fn spawn(body: &[u8]) -> Result<SpawnState> {
    let record = record(body)?;
    let spawn_id = u16::try_from(record.id)
        .ok()
        .filter(|id| *id != 0)
        .context("invalid RoF2 spawn ID")?;
    ensure!(
        record.size.is_finite() && record.size >= 0.0,
        "invalid RoF2 spawn size"
    );
    let speed = |speed: f32| {
        if speed.is_finite() && speed >= 0.0 {
            speed
        } else {
            0.0
        }
    };
    Ok(SpawnState {
        class: Some(record.class),
        spawn_id,
        name: record.name,
        kind: match record.npc {
            0 => SpawnKind::Player,
            1 => SpawnKind::Npc,
            2 => SpawnKind::PlayerCorpse,
            3 => SpawnKind::NpcCorpse,
            other => SpawnKind::Unknown(other),
        },
        race: record.race,
        gender: u32::from(record.gender),
        position: packed_position(record.position),
        velocity: [0.0; 3],
        forward: 0.0,
        walk_speed: speed(record.walk_speed),
        run_speed: speed(record.run_speed),
        size: record.size,
        invisible: record.invisibility != 0,
        appearance: crate::appearance::Appearance::default(),
        level: record.level,
        listing: crate::listing::Listing::default(),
        pet_owner: u16::try_from(record.pet_owner)
            .ok()
            .filter(|owner| *owner != 0),
        hp_percent: (record.hp <= 100).then_some(record.hp),
        posture: Some(PostureState::from(u32::from(record.stand))),
        name_parts: crate::names::NameParts {
            title: record.title,
            last_name: record.last_name,
            suffix: record.suffix,
        },
        body_type: record.body_type,
    })
}

/// The spawn id a record names, when it can be read: what the entry keeps
/// to tell the player's own spawn from the others.
#[must_use]
pub fn spawn_id(body: &[u8]) -> Option<u16> {
    spawn(body).ok().map(|spawn| spawn.spawn_id)
}

/// What the player takes from the own spawn.
#[derive(Debug, PartialEq)]
struct Spawn {
    id: u32,
    size: f32,
    walk_speed: f32,
    run_speed: f32,
    hp: u8,
    stand: u8,
    title: String,
    last_name: String,
    suffix: String,
}

/// The own spawn: a player's record, walked as every record is
/// ([`record`]).
///
/// # Errors
/// Refuses what [`record`] refuses, and a record that isn't a player's: an
/// NPC, a corpse, a treasure chest or a destructible object.
fn own_spawn(body: &[u8]) -> Result<Spawn> {
    let record = record(body)?;
    ensure!(
        record.npc == 0 && record.object == Object::Mob,
        "the RoF2 own spawn is not a player"
    );
    Ok(Spawn {
        id: record.id,
        size: record.size,
        walk_speed: record.walk_speed,
        run_speed: record.run_speed,
        hp: record.hp,
        stand: record.stand,
        title: record.title,
        last_name: record.last_name,
        suffix: record.suffix,
    })
}

/// The player from `RoF2`'s profile and own spawn. `revolution` is the
/// profile's heading scale (see [`profile_heading`]).
///
/// Gems 9 to 12 are not read, because the player holds eight (6.502).
/// Appearance and `/who` listing stay at their defaults, because their
/// fields are not read (6.502).
///
/// # Errors
/// Refuses what either reader refuses, a non-finite position, size or
/// speed, a negative size or speed, and a spawn id that is zero or past
/// 16 bits.
pub fn player(profile_body: &[u8], spawn_body: &[u8], revolution: f32) -> Result<PlayerState> {
    let profile = profile(profile_body)?;
    let spawn = own_spawn(spawn_body)?;
    let [x, y, z, heading] = profile.position;
    ensure!(
        profile.position.iter().all(|value| value.is_finite()),
        "non-finite RoF2 profile position"
    );
    let spawn_id = u16::try_from(spawn.id)
        .ok()
        .filter(|id| *id != 0)
        .context("invalid RoF2 own-spawn ID")?;
    ensure!(
        [spawn.size, spawn.walk_speed, spawn.run_speed]
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0),
        "invalid RoF2 spawn size or speeds"
    );
    ensure!(!profile.name.is_empty(), "empty RoF2 profile name");
    let mut signed = [0; 7];
    for (attribute, value) in signed.iter_mut().zip(profile.attributes) {
        *attribute = i32::try_from(value).context("invalid RoF2 base attribute")?;
    }
    let [strength, stamina, charisma, dexterity, intelligence, agility, wisdom] = signed;
    Ok(PlayerState {
        name: profile.name,
        base_attributes: Some(BaseAttributes {
            strength,
            stamina,
            charisma,
            dexterity,
            intelligence,
            agility,
            wisdom,
        }),
        titanium_environment: None,
        spawn_id,
        race: profile.race,
        class: Some(u32::from(profile.class)).filter(|value| (1..=16).contains(value)),
        deity: Some(profile.deity).filter(|value| *value != 0),
        skills: Some(profile.skills),
        practice_points: Some(profile.practice_points),
        languages: Some(profile.languages),
        disciplines: Some(profile.disciplines),
        gender: u32::from(profile.gender),
        level: profile.level,
        position: Position {
            x,
            y,
            z,
            heading: profile_heading(heading, revolution),
        },
        mana: profile.mana,
        endurance: Some(profile.endurance),
        memorized_spells: std::array::from_fn(|index| {
            let id = profile.gems[index];
            (id != u32::MAX && id != 0xffff && id != 0).then_some(id)
        }),
        spell_refresh_ms: Some(std::array::from_fn(|index| profile.refresh[index])),
        size: spawn.size,
        walk_speed: spawn.walk_speed,
        run_speed: spawn.run_speed,
        hp_percent: (spawn.hp <= 100).then_some(spawn.hp),
        posture: Some(PostureState::from(u32::from(spawn.stand))),
        appearance: crate::appearance::Appearance::default(),
        listing: crate::listing::Listing::default(),
        name_parts: crate::names::NameParts {
            title: spawn.title,
            last_name: spawn.last_name,
            suffix: spawn.suffix,
        },
    })
}

/// Bodies built from the field list, for tests here and in the session.
#[cfg(any(test, feature = "test-bodies"))]
#[doc(hidden)]
pub mod samples;
#[cfg(test)]
mod tests;
