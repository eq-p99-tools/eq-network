//! The player's disciplines: those they know, the one under way, and their
//! reuse timers; and the reuse timers groups of spells share.
//!
//! `EQEmu` lists the disciplines a Titanium player knows, by spell, in
//! `OP_DisciplineUpdate`: once the zone has admitted a player who knows
//! any, as it leaves the profile's own list empty
//! (`ZoneDatabase::LoadCharacterDisciplines`), and as the list changes, from
//! a tome, a quest or a GM. It tells of a discipline reuse timer in
//! `OP_DisciplineTimer` as a discipline starts it, as a quest refuses a
//! discipline or a reset clears it (0), and at zone-in for each timer still
//! running. Those zone-in copies of timers 11 to 19 carry the server's other
//! reuse timers, the combat abilities', begging's and the first spell
//! groups', since it counts past the 11 discipline timers it keeps
//! (`Client::SendDisciplineTimers`, `common/ptimer.h`). It tells of a timer
//! a group of spells shares in `OP_LinkedReuse` as a memorized spell of the
//! group is cast and as a reset clears it, with the timer's end and start
//! on the zone's own clock, so its wait is the end less the start.
//!
//! TAKP and Quarm tell an `EQMac` player of the discipline they begin, by
//! the servers' own number for it (`common/spdat.h`'s `disc_*`), and of its
//! end, in `OP_DisciplineChange` (`Client::CastDiscipline`,
//! `Client::Process`); one with no lasting effect has no end.
//!
//! Layout reference: `Disciplines_Struct` (100 spell IDs),
//! `DisciplineTimer_Struct` (the timer, its seconds and a field `EQEmu`
//! leaves 0) and `LinkedSpellReuseTimer_Struct` (the timer, its end and its
//! start) in `EQEmu`'s `common/eq_packet_structs.h`, and TAKP's
//! `ClientDiscipline_Struct` (the discipline's number and three bytes of
//! padding).
use crate::{buffs::BuffUpdate, command::EncodedCommand, spells, world::WorldEvent};
use anyhow::{ensure, Result};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

/// `OP_DisciplineUpdate`: the disciplines the player knows.
pub const UPDATE_OPCODE: u16 = 0x7180;
/// `OP_DisciplineTimer`: a discipline reuse timer.
pub const TIMER_OPCODE: u16 = 0x53df;
/// `OP_LinkedReuse`: a reuse timer a group of spells shares.
pub const LINKED_REUSE_OPCODE: u16 = 0x6a00;
/// `EQMac`'s `OP_DisciplineChange`: a discipline begun or ended.
pub const EQMAC_CHANGE_OPCODE: u16 = 0xf241;
/// How many disciplines a Titanium list has room for
/// (`MAX_PP_DISCIPLINES`).
pub const SLOTS: usize = 100;

/// Titanium reserves one buff slot after its 25 long and 12 short buffs
/// (`EQEmu` `common/patches/titanium_limits.h`, `LONG_BUFFS` through `DISC_BUFFS`).
pub const TITANIUM_BUFF_SLOT: u32 = 37;

/// Self-targeted Titanium disciplines from spell definition rows 4503 and
/// 4499. The Warrior level is class field 104, self target is field 98,
/// and the shared timer index is field 167 (see `EQEmu` `common/spdat.h`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Discipline {
    /// Warrior level 52.
    Evasive,
    /// Warrior level 55.
    Defensive,
}

impl Discipline {
    /// The supported activation set in level order.
    pub const SUPPORTED: [Self; 2] = [Self::Evasive, Self::Defensive];

    /// Exact discipline spell ID in the received list.
    #[must_use]
    pub const fn spell(self) -> u32 {
        match self {
            Self::Evasive => 4503,
            Self::Defensive => 4499,
        }
    }

    /// Display name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Evasive => "Evasive",
            Self::Defensive => "Defensive",
        }
    }

    /// Stable binding and command name.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Evasive => "evasive",
            Self::Defensive => "defensive",
        }
    }

    /// Parse a supported name without interpreting a raw spell number.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::SUPPORTED
            .into_iter()
            .find(|discipline| discipline.key().eq_ignore_ascii_case(name))
    }

    /// The received discipline reuse group shared by both actions.
    #[must_use]
    pub const fn timer(self) -> u32 {
        1
    }

    /// Requires a Warrior at the correct level and an explicit list entry.
    #[must_use]
    pub fn learned(self, class: Option<u32>, level: u8, list: Option<&[u32]>) -> bool {
        class == Some(1)
            && level
                >= match self {
                    Self::Evasive => 52,
                    Self::Defensive => 55,
                }
            && list.is_some_and(|known| known.contains(&self.spell()))
    }
}

/// The Titanium item/discipline slot. No item is named for a discipline.
pub const CAST_SLOT: u8 = 10;

/// Cast a supported self-targeted discipline. The caller supplies only the
/// session's own spawn ID after eligibility and reuse checks.
///
/// # Errors
/// Refuses an absent own spawn.
pub fn titanium_cast(discipline: Discipline, own: u16) -> Result<EncodedCommand> {
    ensure!(own != 0, "invalid discipline self target");
    Ok(spells::titanium_cast(CAST_SLOT, discipline.spell(), own))
}

/// An active discipline reported by the server. Titanium's buff slot and
/// `EQMac`'s numbered discipline are distinct authorities.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub enum Active {
    /// No current observation.
    #[default]
    Unknown,
    /// The server reported the end of the observed discipline.
    Inactive,
    /// The spell in the own Titanium discipline buff slot.
    Titanium(u32),
    /// The received `EQMac` discipline number.
    EqMac(u8),
}

impl Active {
    /// Applies an exact slot replacement or fade for the admitted player.
    /// An ordinary fade must name the observed spell. An exact slot
    /// replacement can clear the slot without naming its former spell.
    pub fn observe_titanium(&mut self, own: u16, update: &BuffUpdate) -> bool {
        if update.entity_id != u32::from(own) || update.slot != TITANIUM_BUFF_SLOT {
            return false;
        }
        if let Some(buff) = &update.buff {
            *self = Self::Titanium(buff.spell_id);
        } else if update.replaces_slot
            || matches!(*self, Self::Titanium(spell) if spell == update.spell_id)
        {
            *self = Self::Inactive;
        } else {
            return false;
        }
        true
    }
}

/// Received timer records in one namespace. An unknown timer ID remains
/// addressable. A missing timer and an observed zero have different meanings.
#[derive(Clone, Debug, Default)]
pub struct Timers(BTreeMap<u32, Instant>);

impl Timers {
    /// Replace only the timer named by the server.
    pub fn set(&mut self, reuse: Reuse, now: Instant) {
        if let Some(ready) = now.checked_add(reuse.remaining) {
            self.0.insert(reuse.timer, ready);
        }
    }

    /// Time left on a received timer, including an elapsed timer's zero.
    #[must_use]
    pub fn remaining(&self, timer: u32, now: Instant) -> Option<Duration> {
        Some(self.0.get(&timer)?.saturating_duration_since(now))
    }

    /// Received timer records with the time already spent staging removed.
    pub fn updates(&self, now: Instant) -> impl Iterator<Item = Reuse> + '_ {
        self.0.iter().map(move |(&timer, ready)| Reuse {
            timer,
            remaining: ready.saturating_duration_since(now),
        })
    }
}

/// A reuse timer as the server set it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Reuse {
    /// The timer, by the number the disciplines or spells that share it
    /// name.
    pub timer: u32,
    /// How long until they are ready again; zero when they are ready now.
    pub remaining: Duration,
}

/// What a Titanium discipline message says, if `opcode` is one: the
/// disciplines the player knows, or a discipline's or a group of spells'
/// reuse timer.
///
/// # Errors
/// Rejects a message of the wrong length.
pub(crate) fn titanium(opcode: u16, body: &[u8]) -> Result<Option<WorldEvent>> {
    Ok(Some(match opcode {
        UPDATE_OPCODE => WorldEvent::Disciplines(titanium_list(body)?),
        TIMER_OPCODE => WorldEvent::DisciplineReuse(titanium_timer(body)?),
        LINKED_REUSE_OPCODE => WorldEvent::LinkedReuse(titanium_linked_reuse(body)?),
        _ => return Ok(None),
    }))
}

/// Decodes Titanium's list of the disciplines the player knows: a spell ID
/// in each of its 100 slots, 0 for an empty one. The profile holds the same
/// list.
///
/// # Errors
/// Rejects any length other than 400 bytes.
pub fn titanium_list(body: &[u8]) -> Result<Vec<u32>> {
    ensure!(body.len() == SLOTS * 4, "invalid discipline list length");
    Ok(body
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| u32::from_le_bytes(*bytes))
        .collect())
}

/// Decodes Titanium's `OP_DisciplineTimer`: the timer and its seconds.
///
/// # Errors
/// Rejects any length other than 12 bytes.
pub fn titanium_timer(body: &[u8]) -> Result<Reuse> {
    let [timer, seconds, _] = words(body, "discipline timer")?;
    Ok(Reuse {
        timer,
        remaining: Duration::from_secs(u64::from(seconds)),
    })
}

/// Decodes Titanium's `OP_LinkedReuse`: the timer, waiting from its start
/// to its end.
///
/// # Errors
/// Rejects any length other than 12 bytes.
pub fn titanium_linked_reuse(body: &[u8]) -> Result<Reuse> {
    let [timer, end, start] = words(body, "linked reuse")?;
    Ok(Reuse {
        timer,
        // `EQEmu` sets the end as the start and the wait added in 32 bits.
        remaining: Duration::from_secs(u64::from(end.wrapping_sub(start))),
    })
}

/// Decodes `EQMac`'s `OP_DisciplineChange`: the discipline begun, or none
/// as it ends.
///
/// # Errors
/// Rejects any length other than four bytes.
pub fn eqmac_change(body: &[u8]) -> Result<Option<u8>> {
    ensure!(body.len() == 4, "invalid discipline change length");
    Ok((body[0] != 0).then_some(body[0]))
}

/// The record's 32-bit fields, for a record of exactly `N` of them.
fn words<const N: usize>(body: &[u8], what: &str) -> Result<[u32; N]> {
    let (fields, rest) = body.as_chunks::<4>();
    ensure!(
        fields.len() == N && rest.is_empty(),
        "invalid {what} length"
    );
    Ok(std::array::from_fn(|index| {
        u32::from_le_bytes(fields[index])
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_self_disciplines_use_the_cast_slot_and_share_reuse() {
        assert!(!Discipline::Defensive.learned(Some(1), 54, Some(&[4499])));
        assert!(!Discipline::Defensive.learned(Some(2), 55, Some(&[4499])));
        assert!(!Discipline::Defensive.learned(Some(1), 55, None));
        assert!(Discipline::Defensive.learned(Some(1), 55, Some(&[4503, 4499])));
        assert_eq!(Discipline::Evasive.timer(), Discipline::Defensive.timer());
        let packet = titanium_cast(Discipline::Defensive, 81).unwrap();
        assert_eq!(packet.opcode, 0x304b);
        assert_eq!(packet.body, record(&[10, 4499, u32::MAX, 81, 0]));
    }

    #[test]
    fn reset_and_unknown_group_remain_distinct() {
        let now = Instant::now();
        let mut timers = Timers::default();
        timers.set(
            Reuse {
                timer: 99,
                remaining: Duration::from_secs(17),
            },
            now,
        );
        timers.set(
            Reuse {
                timer: 1,
                remaining: Duration::ZERO,
            },
            now,
        );
        assert_eq!(
            timers.remaining(99, now + Duration::from_secs(1)),
            Some(Duration::from_secs(16))
        );
        assert_eq!(timers.remaining(1, now), Some(Duration::ZERO));
        assert_eq!(timers.remaining(0, now), None);
    }

    fn record(fields: &[u32]) -> Vec<u8> {
        fields
            .iter()
            .flat_map(|field| field.to_le_bytes())
            .collect()
    }

    #[test]
    fn the_list_keeps_its_slots() {
        let mut slots = [0; SLOTS];
        slots[0] = 4585;
        slots[3] = 4587;
        let list = titanium_list(&record(&slots)).unwrap();
        assert_eq!(list.len(), SLOTS);
        assert_eq!(list[..4], [4585, 0, 0, 4587]);
        assert!(titanium_list(&record(&slots[..99])).is_err());
    }

    #[test]
    fn timers_read_their_waits() {
        // Timer 2 set for 1,800 seconds, then cleared.
        assert_eq!(
            titanium_timer(&record(&[2, 1800, 0])).unwrap(),
            Reuse {
                timer: 2,
                remaining: Duration::from_mins(30),
            }
        );
        assert_eq!(
            titanium_timer(&record(&[2, 0, 0])).unwrap().remaining,
            Duration::ZERO
        );
        // A spell group's timer, from 5,000 to 5,030 on the zone's clock.
        assert_eq!(
            titanium_linked_reuse(&record(&[7, 5030, 5000])).unwrap(),
            Reuse {
                timer: 7,
                remaining: Duration::from_secs(30),
            }
        );
        // A reset: the end is the start.
        assert_eq!(
            titanium_linked_reuse(&record(&[7, 5000, 5000]))
                .unwrap()
                .remaining,
            Duration::ZERO
        );
        assert!(titanium_timer(&record(&[2, 1800])).is_err());
        assert!(titanium_linked_reuse(&record(&[7, 5030, 5000, 0])).is_err());
    }

    #[test]
    fn an_eqmac_discipline_begins_and_ends() {
        assert_eq!(eqmac_change(&[30, 0, 0, 0]).unwrap(), Some(30));
        assert_eq!(eqmac_change(&[0; 4]).unwrap(), None);
        assert!(eqmac_change(&[30]).is_err());
    }
}
