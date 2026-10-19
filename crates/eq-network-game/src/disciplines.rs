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
use crate::world::WorldEvent;
use anyhow::{ensure, Result};
use serde::Serialize;
use std::time::Duration;

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
                remaining: Duration::from_secs(1800),
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
