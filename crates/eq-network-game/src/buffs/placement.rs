//! The slot the Titanium program gives a buff landing on the player.
//!
//! A server need not say where a landing buff went: `EQEmu` sends a Titanium
//! client no slot when a buff lands (`Mob::AddBuff` sends its buffs packet
//! to `SoD` and later only), so the program works the slot out itself. Its
//! slot finder 0x414071, called from the action handler 0x414913, is read
//! in R3 (program reads R1-R4), R20 and R49-22e (checklist 6.612). For a
//! player spawn the table holds 38 slots, and the finder runs:
//! - step 2, the blocker (effect 148), only partly read: a new landing
//!   while a held spell carries it has no slot here;
//! - step 3: a spell already held keeps its first slot;
//! - step 4: a held buff it conflicts with, effect by effect, is replaced
//!   in place, or the landing is refused;
//! - step 5: the first empty slot of the spell's band, a song with no free
//!   song slot falling back to the normal band;
//! - step 6: with no slot, a spell without +0x17d evicts the first held
//!   spell with it.
//!
//! The normal band's top is 15 plus the effect-327 total, at most 10 more
//! (0x410db0). The total sums the buffs, worn items and alternate
//! abilities. Only the held buffs are known here, and only those whose
//! value can be worked out, so their total is a lower bound: the slots
//! below it are named, and past it the top is unknown unless the caller
//! says the other sources add nothing.
//!
//! A replace or an eviction is this client's prediction of what the
//! program removes. The server stays the authority: its next buff state
//! wins, and the caller logs any disagreement with the [`Rule`] that fired.
//! Any fact the finder lacks leaves the landing unslotted, never guessed.
//!
//! The finder is Titanium's; reach it through
//! [`GameDialect::buff_placement`](crate::GameDialect::buff_placement),
//! which gives none for the Mac-era generation, whose program is unread.
use std::{collections::BTreeMap, ops::Range};

/// How many slots the program's table holds for a player spawn (0x40d933).
pub const TITANIUM_PLAYER_SLOTS: u32 = 38;
/// The song band, count-13 to count-2 for a player (0x40db4e, 0x410db0),
/// for a spell with +0x17d and +0x19b set (0x40d89f; R20 corrects R3).
pub const TITANIUM_SONG_SLOTS: Range<u32> = 25..37;
/// The last slot, which has a band of its own, for a spell with +0x17d and
/// +0x1c4 set (0x41477a-0x414790; R20).
pub const TITANIUM_LAST_SLOT: u32 = 37;
/// The normal band's top without effect 327 (0x410db0, R20 (c)).
pub const TITANIUM_NORMAL_TOP: u32 = 15;
/// The most slots effect 327 adds to the normal band (0x410db0, R49-22e 3).
pub const TITANIUM_MOST_ADDED: u32 = 10;

/// The effect that adds buff slots.
const ADDS_SLOTS: u32 = 327;
/// A blank effect position.
const BLANK: u32 = 254;

/// The spell fields that choose a landing's band (R20 (a), from the spells
/// file parser 0x44dbf0; fields counted from 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Band {
    /// Spell +0x17d, field 83: the column the book reads as beneficial, and
    /// the flag read R49-22e calls "song" in steps 4 and 6.
    pub beneficial: bool,
    /// Spell +0x19b, field 154: with +0x17d, the song band.
    pub short: bool,
    /// Spell +0x1c4, field 168: with +0x17d, the last slot.
    pub last: bool,
}

/// One effect position of a spell, as the spells file holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effect {
    /// The effect's number; 254 is blank.
    pub id: u32,
    /// Its base value (+0x30 + slot × 4).
    pub base: i32,
    /// Its formula (+0xf0 + slot × 4).
    pub formula: u32,
}

impl Default for Effect {
    fn default() -> Self {
        Self {
            id: BLANK,
            base: 0,
            formula: 100,
        }
    }
}

/// What the finder knows of a spell. A field the installed data lacks is
/// `None`, and a step that needs it leaves the landing unslotted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spell {
    /// The spell's id.
    pub id: u32,
    /// Its band fields.
    pub band: Option<Band>,
    /// Whether it has a Bard level (class field 8 not 255, 0x44e755(8)).
    pub bard: Option<bool>,
    /// Its twelve effect positions.
    pub effects: Option<[Effect; 12]>,
}

impl Spell {
    /// A spell known by its id alone.
    #[must_use]
    pub const fn id(id: u32) -> Self {
        Self {
            id,
            band: None,
            bard: None,
            effects: None,
        }
    }
}

/// A landing on the player, and what the player holds.
#[derive(Clone, Copy, Debug)]
pub struct Landing<'a> {
    /// Every slot the player holds, and the spell held there.
    pub held: &'a BTreeMap<u32, Spell>,
    /// The landing spell.
    pub spell: Spell,
    /// Whether a held spell carries effect 148, which refuses only a spell
    /// not held (step 2, partly read).
    pub blocker: bool,
    /// Whether the worn items and alternate abilities add no effect 327.
    /// Never set until those are known (6.612 T3): no caller knows them.
    pub others_add_no_slots: bool,
}

/// The rule that chose a slot other than step 3's or step 5's, or refused
/// the landing, for the caller's log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Rule {
    /// Step 4 replaced a conflicting held buff in place.
    Replaced,
    /// Step 4 found a conflict with a song on either side: the held slot is
    /// only nominated, and nothing is removed.
    SongNominated,
    /// Step 6 evicted the first held spell with +0x17d.
    Evicted,
    /// Step 4: a new effect 0x2c or 0x2d.
    NewEffectRefused,
    /// Step 4: a landing with +0x17d over a held one without it, at an
    /// effect other than 3.
    OverUnflagged,
    /// Step 4: both have +0x17d, and the new one carries effect 0x3a or
    /// 0x9c.
    FlaggedPair,
    /// Step 4: the held effect is 0x65, or the held spell is one the
    /// program never replaces.
    HeldKept,
    /// Step 4: a value compare refused.
    Weaker,
}

/// Where a landing goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    /// The slot.
    pub slot: u32,
    /// The slot whose buff the program removes first, predicted (T1).
    pub removed: Option<u32>,
    /// The rule that chose it, where step 3 or 5 did not.
    pub rule: Option<Rule>,
}

impl Placed {
    const fn at(slot: u32) -> Self {
        Self {
            slot,
            removed: None,
            rule: None,
        }
    }
}

/// Why a landing was left without a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Unplaced {
    /// The installed spell data lacks a field a step needs.
    SpellFieldsUnknown,
    /// The band has no empty slot, and no eviction applies.
    BandFull,
    /// The known slots of the normal band are full, and whether effect 327
    /// opens more is unknown.
    TopUnknown,
    /// A held spell carries effect 148, whose blocking test (step 2) is only
    /// partly read.
    Blocked,
    /// A step-4 value uses a formula whose evaluation is unread (Q1).
    ValueUnknown,
    /// Step 4 refused the landing: the program writes nothing.
    Refused(Rule),
}

/// A generation's slot finder for a buff landing on the player, as
/// [`titanium_player`].
pub type Finder = fn(&Landing<'_>) -> Result<Placed, Unplaced>;

/// Effects that never conflict with themselves (the list 0x44cf50, R49-22e 1).
fn never_conflicts(effect: u32) -> bool {
    matches!(
        effect,
        0x0d | 0x23 | 0x24 | 0x39 | 0x41 | 0x42 | 0x4f | 0x74 | 0x7c
            ..=0x90
                | 0x94
                | 0x95
                | 0xa7
                | 0xeb
                | 0xfe
                | 0x11e
                | 0x11f
                | 0x128
                | 0x129
                | 0x12e
                | 0x12f
                | 0x136
                | 0x137
                | 0x14f
                | 0x15c
    )
}

/// Held spells step 4 never replaces (R49-22e 1, rule 9).
fn never_replaced(spell: u32) -> bool {
    matches!(
        spell,
        0x307..=0x311
            | 0x4b0..=0x4e2
            | 0x76c..=0x784
            | 0x81f
            | 0xabf
            | 0x2f4
            | 0x1481
            | 0x2f5
            | 0x344
    )
}

/// The spells whose own value counts as -1 (rule 10).
const MINUS_ONE: [u32; 3] = [0x654, 0x718, 0x341];
/// The spell that, held, also makes the new value -1 (rule 10).
const MINUS_ONE_HELD: u32 = 0x716;
/// The one spell the refusals of the value compare let through.
const LET_THROUGH: u32 = 0x344;

/// An effect's value as 0x40f339 works it out, where this client can: for
/// formulas 0 and 100, the base itself, with its sign [I] (6.612 Q1 (a)).
/// The other formulas' evaluation is unread.
const fn value(effect: Effect) -> Option<i64> {
    match effect.formula {
        0 | 100 => Some(effect.base as i64),
        _ => None,
    }
}

/// The normal band's top this client can name, and whether it is the
/// program's exactly.
fn normal_top(landing: &Landing<'_>) -> (u32, bool) {
    // A held effect-327 value that can't be worked out leaves only the
    // fixed 15 named.
    let mut total = Some(0_i64);
    for spell in landing.held.values() {
        let Some(effects) = spell.effects else {
            total = None;
            break;
        };
        for effect in effects.iter().filter(|effect| effect.id == ADDS_SLOTS) {
            total = total
                .zip(value(*effect))
                .map(|(total, value)| total + value);
        }
    }
    let added = total.map_or(0, |total| {
        u32::try_from(total.unsigned_abs())
            .unwrap_or(u32::MAX)
            .min(TITANIUM_MOST_ADDED)
    });
    let exact = added == TITANIUM_MOST_ADDED || (total.is_some() && landing.others_add_no_slots);
    (TITANIUM_NORMAL_TOP + added, exact)
}

/// A band's slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Normal,
    /// The song band; true when the spell also has +0x1c4.
    Song(bool),
    Last,
}

impl Kind {
    /// The band a spell's fields choose (0x40d89f; R20).
    const fn of(band: Band) -> Self {
        if band.beneficial && band.short {
            Self::Song(band.last)
        } else if band.beneficial && band.last {
            Self::Last
        } else {
            Self::Normal
        }
    }
}

/// The finder's view of the table.
struct Table<'a> {
    held: &'a BTreeMap<u32, Spell>,
    top: u32,
    exact: bool,
}

impl Table<'_> {
    fn empty(&self, mut slots: Range<u32>) -> Option<u32> {
        slots.find(|slot| !self.held.contains_key(slot))
    }

    /// The first empty normal slot: named below the known top, unknown
    /// past it unless every slot the top could reach is full.
    fn empty_normal(&self) -> Result<Option<u32>, Unplaced> {
        if let Some(slot) = self.empty(0..self.top) {
            return Ok(Some(slot));
        }
        let most = TITANIUM_NORMAL_TOP + TITANIUM_MOST_ADDED;
        if self.exact || self.empty(self.top..most).is_none() {
            Ok(None)
        } else {
            Err(Unplaced::TopUnknown)
        }
    }

    fn contains(&self, kind: Kind, slot: u32) -> bool {
        match kind {
            Kind::Normal => slot < self.top,
            Kind::Song(_) => TITANIUM_SONG_SLOTS.contains(&slot),
            Kind::Last => slot == TITANIUM_LAST_SLOT,
        }
    }

    /// The band's first empty slot (step 5), a song falling back to the
    /// normal band.
    fn first_empty(&self, kind: Kind) -> Result<Option<u32>, Unplaced> {
        match kind {
            Kind::Last => Ok(self.empty(TITANIUM_LAST_SLOT..TITANIUM_PLAYER_SLOTS)),
            Kind::Song(_) => match self.empty(TITANIUM_SONG_SLOTS) {
                Some(slot) => Ok(Some(slot)),
                None => self.empty_normal(),
            },
            Kind::Normal => self.empty_normal(),
        }
    }

    /// 0x411ea4(band, flag, preferred): the preferred slot inside the band;
    /// else the band's first empty slot; else, for a song-band spell, the
    /// first empty normal slot; else the preferred slot after all.
    fn choose(&self, kind: Kind, preferred: u32) -> Result<u32, Unplaced> {
        if self.contains(kind, preferred) {
            return Ok(preferred);
        }
        let found = match kind {
            // A song-band spell with +0x1c4 doesn't fall back.
            Kind::Song(true) => self.empty(TITANIUM_SONG_SLOTS),
            _ => self.first_empty(kind)?,
        };
        Ok(found.unwrap_or(preferred))
    }
}

/// What step 4 makes of one held buff.
enum Compared {
    /// No conflict.
    Clear,
    /// Replace it.
    Replace,
    /// Refuse the landing.
    Refuse(Rule),
}

/// Step 4's compare of the landing with one held buff, rules 1 to 10 of
/// R49-22e 1 in order.
fn compare(new: &Spell, held: &Spell) -> Result<Compared, Unplaced> {
    let (Some(new_effects), Some(held_effects)) = (new.effects, held.effects) else {
        return Err(Unplaced::SpellFieldsUnknown);
    };
    let new_flag = new.band.map(|band| band.beneficial);
    let held_flag = held.band.map(|band| band.beneficial);
    for (new_effect, held_effect) in new_effects.iter().zip(held_effects.iter()) {
        // 1: a blank on either side ends the compare.
        if held_effect.id == BLANK || new_effect.id == BLANK {
            break;
        }
        // 2.
        if matches!(new_effect.id, 0x2c | 0x2d) {
            return Ok(Compared::Refuse(Rule::NewEffectRefused));
        }
        // 3: a held Bard spell over a new non-Bard one, at these effects.
        if matches!(held_effect.id, 15 | 0 | 100 | 0x14e) {
            let (Some(held_bard), Some(new_bard)) = (held.bard, new.bard) else {
                return Err(Unplaced::SpellFieldsUnknown);
            };
            if held_bard && !new_bard {
                continue;
            }
        }
        // 4 and 5.
        if new_effect.id != held_effect.id || never_conflicts(new_effect.id) {
            continue;
        }
        let effect = new_effect.id;
        // 6.
        if effect == 10 && (new_effect.base == 0 || held_effect.base == 0) {
            continue;
        }
        // 7.
        if matches!(effect, 1 | 0x14e) && new_effect.base < 0 {
            continue;
        }
        // 8: the +0x17d rules (0x4144d9).
        let (Some(new_flag), Some(held_flag)) = (new_flag, held_flag) else {
            return Err(Unplaced::SpellFieldsUnknown);
        };
        if new_flag && !held_flag {
            if effect == 3 {
                continue;
            }
            return Ok(Compared::Refuse(Rule::OverUnflagged));
        }
        if new_flag
            && held_flag
            && new_effects
                .iter()
                .take_while(|effect| effect.id != BLANK)
                .any(|effect| matches!(effect.id, 0x3a | 0x9c))
        {
            return Ok(Compared::Refuse(Rule::FlaggedPair));
        }
        // 9.
        if effect == 0x65 || never_replaced(held.id) {
            return Ok(Compared::Refuse(Rule::HeldKept));
        }
        // 10: the values, each -1 for the listed spells.
        let forced_new = MINUS_ONE.contains(&new.id) || held.id == MINUS_ONE_HELD;
        let forced_held = MINUS_ONE.contains(&held.id) || held.id == MINUS_ONE_HELD;
        let new_value = if forced_new {
            -1
        } else {
            value(*new_effect).ok_or(Unplaced::ValueUnknown)?
        };
        let held_value = if forced_held {
            -1
        } else {
            value(*held_effect).ok_or(Unplaced::ValueUnknown)?
        };
        return Ok(values(effect, held_value, new_value, new.id));
    }
    Ok(Compared::Clear)
}

/// Rule 10's value compare at a conflicting effect.
fn values(effect: u32, held: i64, new: i64, spell: u32) -> Compared {
    let weaker = Compared::Refuse(Rule::Weaker);
    if (held < 0 && 0 < new) || (held > 0 && 0 > new) {
        return match effect {
            3 => Compared::Clear,
            0 if held < 0 => weaker,
            _ => Compared::Replace,
        };
    }
    let replace = |yes: bool| {
        if yes {
            Compared::Replace
        } else {
            Compared::Refuse(Rule::Weaker)
        }
    };
    if effect == 11 {
        // A slow (99 or less) at least as strong replaces anything; a haste
        // can't replace a slow, nor a weaker slow a stronger one (0x4146xx).
        if new <= 99 && new <= held {
            return Compared::Replace;
        }
        if held < 101 {
            return replace(spell == LET_THROUGH);
        }
        return replace(new >= held);
    }
    match held {
        ..0 => replace(new <= held),
        1.. => replace(new >= held),
        0 => replace(spell == LET_THROUGH),
    }
}

/// Step 4 over every held buff, from slot 0. A conflict with a song on
/// either side only nominates the held slot and the finder goes on; any
/// other replace removes the held buff and places the landing at once.
/// After the loop a nominated slot is used [I], and the spell held there,
/// if any, is removed, as the program removes a found slot's other spell.
fn step_four(
    landing: &Landing<'_>,
    table: &Table<'_>,
    kind: Kind,
) -> Result<Option<Placed>, Unplaced> {
    let new = &landing.spell;
    let mut nominated: Option<u32> = None;
    for (slot, held) in landing.held {
        match compare(new, held)? {
            Compared::Clear => (),
            Compared::Refuse(rule) => return Err(Unplaced::Refused(rule)),
            Compared::Replace => {
                let songs = new.band.is_some_and(|band| band.beneficial)
                    || held.band.is_some_and(|band| band.beneficial);
                if songs {
                    if nominated.is_none_or(|slot| !landing.held.contains_key(&slot)) {
                        nominated = Some(*slot);
                    }
                    continue;
                }
                return Ok(Some(Placed {
                    slot: table.choose(kind, *slot)?,
                    removed: Some(*slot),
                    rule: Some(Rule::Replaced),
                }));
            }
        }
    }
    let Some(nominated) = nominated else {
        return Ok(None);
    };
    let slot = table.choose(kind, nominated)?;
    Ok(Some(Placed {
        slot,
        removed: landing
            .held
            .get(&slot)
            .filter(|held| held.id != new.id)
            .map(|_| slot),
        rule: Some(Rule::SongNominated),
    }))
}

/// Step 6: a landing without +0x17d evicts the first held spell, from slot
/// 0, with it, and takes 0x411ea4's slot for the evicted one. The target's
/// +0x256 is taken as 0 for the player [I].
fn step_six(
    landing: &Landing<'_>,
    table: &Table<'_>,
    kind: Kind,
    band: Band,
) -> Result<Placed, Unplaced> {
    if band.beneficial {
        return Err(Unplaced::BandFull);
    }
    for (slot, held) in landing.held {
        match held.band {
            None => return Err(Unplaced::SpellFieldsUnknown),
            Some(held_band) if held_band.beneficial => {
                return Ok(Placed {
                    slot: table.choose(kind, *slot)?,
                    removed: Some(*slot),
                    rule: Some(Rule::Evicted),
                });
            }
            Some(_) => (),
        }
    }
    Err(Unplaced::BandFull)
}

/// The slot the program gives a spell landing on the player.
///
/// # Errors
/// Reports why the slot cannot be named.
pub fn titanium_player(landing: &Landing<'_>) -> Result<Placed, Unplaced> {
    let held = landing.held;
    let spell = landing.spell;
    // Step 3: the spell's first slot, wherever it is. Its refusals, and
    // step 2's, keep the old record where it is (R20 (g)), so the slot is
    // the same either way.
    if let Some((slot, _)) = held.iter().find(|(_, held)| held.id == spell.id) {
        return Ok(Placed::at(*slot));
    }
    if landing.blocker {
        return Err(Unplaced::Blocked);
    }
    let band = spell.band.ok_or(Unplaced::SpellFieldsUnknown)?;
    let kind = Kind::of(band);
    let (top, exact) = normal_top(landing);
    let table = Table { held, top, exact };
    if let Some(placed) = step_four(landing, &table, kind)? {
        return Ok(placed);
    }
    if let Some(slot) = table.first_empty(kind)? {
        return Ok(Placed::at(slot));
    }
    step_six(landing, &table, kind, band)
}

#[cfg(test)]
mod tests;
