//! `RoF2`'s spell book and gems, as stock `EQEmu` sends them, inferred for
//! `RoF2` (6.532).
//!
//! None of these messages has a `RoF2` encoder (`common/patches/rof2_ops.h`),
//! so `RoF2` gets the server's own structs, read by Titanium's readers. The
//! server's casting slots and book are `RoF2`'s (`emu_constants.h` takes
//! them from `RoF2::spells`), and its `SpellbookSize` for a `RoF2` client is
//! 720 (`rof2_limits.h`), where Titanium's is 400.
//!
//! What the `RoF2` program does with a bar refresh's casting slot and
//! reduction, and whether it reads the deletion's padding, is not read
//! (6.532).
use crate::spells::SpellUpdate;
use anyhow::Result;

/// `OP_MemorizeSpell` (`utils/patches/patch_RoF2.conf`; inferred, 6.532).
pub const MEMORIZE_OPCODE: u16 = 0x217c;
/// `OP_SwapSpell` (`utils/patches/patch_RoF2.conf`; inferred, 6.532).
pub const SWAP_OPCODE: u16 = 0x0efa;
/// `OP_DeleteSpell` (`utils/patches/patch_RoF2.conf`; inferred, 6.532).
pub const DELETE_OPCODE: u16 = 0x3358;

/// `RoF2`'s spell book: 720 slots.
const BOOK: u32 = crate::spells::book_size(crate::GameDialect::Rof2);

/// What one of this module's messages says; None for another opcode.
///
/// A memorize message is Titanium's: a slot's update, or for mode 3 the
/// spell bar's refresh. Its slot is the server's own number, a casting
/// slot for mode 3 (gems 0 to 11, the item 12, the discipline 13), where
/// Titanium's encoder renumbers the casting slot; and it carries the
/// server's reduction, which Titanium's encoder does not copy. A mode past
/// 3 is a slot's update with that mode, as Titanium's. A swap or a deletion
/// is Titanium's within the 720-slot book, and a negative deletion slot
/// refuses, as Titanium's.
///
/// # Errors
/// Refuses what Titanium's readers refuse, with the book at 720 slots.
pub fn read(opcode: u16, body: &[u8]) -> Result<Option<SpellUpdate>> {
    Ok(Some(match opcode {
        MEMORIZE_OPCODE => crate::spells::memorized(body)?,
        SWAP_OPCODE => crate::spells::book_swap(body, BOOK)?,
        DELETE_OPCODE => crate::spells::book_deletion(body, BOOK)?,
        _ => return Ok(None),
    }))
}

#[cfg(test)]
mod tests;
