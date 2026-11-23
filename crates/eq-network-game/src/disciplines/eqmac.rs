//! `/disc` on the `EQMac` wire, as TAKP's and Quarm's own
//! `Client::Handle_OP_Discipline` reads it (TAKP 44afcbf, Quarm fc75fcf;
//! 6.603 G1, source-derived, not yet checked live).
//!
//! `OP_Discipline` (`ClientDiscipline_Struct`) is the servers' number for
//! the discipline and three bytes of padding. The number 0 asks when a
//! discipline may next be used; the server answers with a line either way.
//! Any other number uses that discipline: the server checks the class, the
//! level, the reuse timer and any discipline still active, and answers each
//! refusal with a line of its own. Neither server sends a Mac player a list
//! of disciplines or their timers, so the session keeps neither.
//!
//! What the `EQMac` program itself sends, and what it refuses before
//! sending, is unread (row 10a).
use crate::command::EncodedCommand;

/// `EQMac`'s `OP_Discipline`: a discipline used, or the reuse asked about.
pub const OPCODE: u16 = 0xe641;

/// The number that asks when a discipline may next be used.
pub const REUSE_QUESTION: u8 = 0;

/// Warrior, Paladin, Ranger, Shadow Knight, Monk, Bard, Rogue and Beastlord,
/// by the servers' class numbers.
const WARRIOR: u32 = 1;
const PALADIN: u32 = 3;
const RANGER: u32 = 4;
const SHADOW_KNIGHT: u32 = 5;
const MONK: u32 = 7;
const BARD: u32 = 8;
const ROGUE: u32 = 9;
const BEASTLORD: u32 = 15;

/// Every class `disc_resistant` and `disc_fearless` serve.
const MELEE: &[u32] = &[
    WARRIOR,
    PALADIN,
    RANGER,
    SHADOW_KNIGHT,
    MONK,
    BARD,
    ROGUE,
    BEASTLORD,
];

/// The servers' disciplines: each number (`common/spdat.h`'s `disc_*`),
/// the spell it casts and the classes it casts it for, as
/// `Client::CastDiscipline` chooses the spell and
/// `Client::DisciplineUseLevel` the classes (`zone/effects.cpp`, the same
/// on both servers). One number casts a different spell for each class
/// that shares it.
const TABLE: &[(u8, u32, &[u32])] = &[
    (1, 4498, &[WARRIOR]),
    (2, 4501, &[WARRIOR]),
    (3, 4499, &[WARRIOR]),
    (4, 4503, &[WARRIOR]),
    (5, 4508, &[MONK]),
    (6, 4674, &[WARRIOR]),
    (6, 4509, &[MONK]),
    (6, 4673, &[ROGUE]),
    (11, 4510, &[MONK]),
    (11, 4671, &[BEASTLORD]),
    (12, 4511, &[MONK]),
    (13, 4670, &[WARRIOR]),
    (13, 4502, &[MONK]),
    (14, 4675, &[WARRIOR]),
    (14, 4512, &[MONK]),
    (14, 4676, &[ROGUE]),
    (14, 4678, &[BEASTLORD]),
    (15, 4513, &[MONK]),
    (15, 4677, &[ROGUE]),
    (16, 4672, &[WARRIOR]),
    (16, 4505, &[ROGUE]),
    (17, 4514, &[WARRIOR]),
    (19, 4515, &[ROGUE]),
    (20, 4507, &[MONK]),
    (21, 4517, &[ROGUE]),
    (22, 4500, &[PALADIN]),
    (23, 4518, &[PALADIN]),
    (24, 4506, &[RANGER]),
    (25, 4519, &[RANGER]),
    (26, 4520, &[SHADOW_KNIGHT]),
    (27, 4504, &[SHADOW_KNIGHT]),
    (28, 4516, &[BARD]),
    (29, 4586, &[BARD]),
    (30, 4585, MELEE),
    (31, 4587, MELEE),
];

/// The servers' number for a discipline spell the player's class casts;
/// None for a spell outside the class's row, which the servers would
/// refuse. The level is left to the server, whose own line answers it.
#[must_use]
pub fn number(spell_id: u32, class: u32) -> Option<u8> {
    TABLE
        .iter()
        .find(|(_, spell, classes)| *spell == spell_id && classes.contains(&class))
        .map(|(number, ..)| *number)
}

/// Encodes `OP_Discipline`: the number, then three zero bytes (what the
/// program writes there is row 10a). [`REUSE_QUESTION`] asks about the
/// reuse timer.
#[must_use]
pub fn encode(number: u8) -> EncodedCommand {
    EncodedCommand {
        opcode: OPCODE,
        body: vec![number, 0, 0, 0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_discipline_is_its_number_and_three_zero_bytes() {
        assert_eq!(encode(4).opcode, 0xe641);
        assert_eq!(encode(4).body, [4, 0, 0, 0]);
        assert_eq!(encode(REUSE_QUESTION).body, [0, 0, 0, 0]);
        assert_eq!(encode(31).body, [31, 0, 0, 0]);
    }

    #[test]
    fn a_spell_maps_to_its_number_only_for_a_class_that_casts_it() {
        // Evasive is a Warrior's only.
        assert_eq!(number(4503, WARRIOR), Some(4));
        assert_eq!(number(4503, MONK), None);
        // Number 6 is Furious, Whirlwind or Counterattack by class.
        assert_eq!(number(4674, WARRIOR), Some(6));
        assert_eq!(number(4509, MONK), Some(6));
        assert_eq!(number(4673, ROGUE), Some(6));
        assert_eq!(number(4674, ROGUE), None);
        // Number 14 has a spell for each of four classes.
        for (spell, class) in [
            (4675, WARRIOR),
            (4512, MONK),
            (4676, ROGUE),
            (4678, BEASTLORD),
        ] {
            assert_eq!(number(spell, class), Some(14));
        }
        // Resistant and Fearless serve every melee class and no caster.
        for class in MELEE {
            assert_eq!(number(4585, *class), Some(30));
            assert_eq!(number(4587, *class), Some(31));
        }
        for caster in [2, 6, 10, 11, 12, 13, 14] {
            assert!(TABLE
                .iter()
                .all(|(_, spell, _)| number(*spell, caster).is_none()));
        }
        // No spell is two numbers, and 0 is never a discipline.
        for (index, (_, spell, _)) in TABLE.iter().enumerate() {
            assert!(TABLE[index + 1..]
                .iter()
                .all(|(_, other, _)| other != spell));
        }
        assert!(TABLE.iter().all(|(number, ..)| *number != REUSE_QUESTION));
        assert_eq!(number(0, WARRIOR), None);
    }
}
