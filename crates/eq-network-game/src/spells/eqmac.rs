//! Spell notices and the spellbook on the `EQMac` wire, as TAKP sends them:
//! its server's own packed structs (`common/eq_packet_structs.h`), which the
//! Mac patch passes on untranslated, with 16-bit fields where Titanium's are
//! 32, a book of 256 slots, no reuse adjustment on a gem's refresh, and
//! interruptions that name no caster. The opcodes are TAKP
//! `utils/patches/patch_Mac.conf`'s, their bytes swapped.
use super::{SpellBook, SpellUpdate};
use anyhow::{ensure, Context, Result};

/// `OP_BeginCast`: a cast beginning, sent to everyone nearby.
pub const EQMAC_BEGIN_OPCODE: u16 = 0xa940;
/// `OP_InterruptCast`: the player's own cast stopping, sent to them alone.
pub const EQMAC_INTERRUPT_OPCODE: u16 = 0x3542;
/// `OP_MemorizeSpell`: a scribe, a memorize or a forget, and a gem's refresh.
pub const EQMAC_MEMORIZE_OPCODE: u16 = 0x8241;
/// `OP_DeleteSpell`: a book entry removed.
pub const EQMAC_DELETE_OPCODE: u16 = 0x4a42;
/// `OP_SwapSpell`: two book entries exchanged, which TAKP echoes.
pub const EQMAC_SWAP_OPCODE: u16 = 0xce41;
/// How many slots `EQMac`'s spellbook has (TAKP
/// `common/patches/mac_limits.h` `SPELLBOOK_SIZE`).
pub const EQMAC_BOOK_SLOTS: usize = 256;

/// Where the book lies in the unpacked profile: a signed 16-bit spell ID per
/// slot at 1846 (`common/patches/mac_structs.h` `PlayerProfile_Struct`
/// `spell_book`).
const PROFILE_BOOK: std::ops::Range<usize> = 1846..1846 + EQMAC_BOOK_SLOTS * 2;

/// The message TAKP's discipline command sends through `OP_InterruptCast` to
/// say a discipline can be used again (`Client::Handle_OP_Discipline`); it
/// ends no cast.
const DISCIPLINE_READY: u16 = 393;

impl SpellBook {
    /// Reads the 256 `EQMac` book slots from an unpacked profile, where an
    /// empty slot holds -1.
    ///
    /// # Errors
    /// Rejects a profile of another size.
    pub fn eqmac_profile(profile: &[u8]) -> Result<Self> {
        ensure!(
            profile.len() == crate::quarm::PROFILE_SIZE,
            "invalid EQMac profile length"
        );
        Ok(Self {
            slots: profile[PROFILE_BOOK]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|bytes| {
                    u32::try_from(i16::from_le_bytes(*bytes))
                        .ok()
                        .filter(|id| *id != 0)
                })
                .collect(),
        })
    }
}

/// Decodes `EQMac`'s spell notices; other opcodes give none.
///
/// A begun cast carries its base cast time, which the client shortens by the
/// player's own focus effects (TAKP `Mob::DoCastSpell`). An interruption names
/// no caster, since TAKP sends it to the caster alone (`Mob::InterruptSpell`):
/// it is read with caster 0, for the session to name the player. A gem's
/// refresh carries no reuse adjustment, so the client times the recast from
/// its own spell data.
///
/// # Errors
/// Rejects malformed lengths, book slots past the book and boolean values.
pub fn decode_eqmac(opcode: u16, body: &[u8]) -> Result<Option<SpellUpdate>> {
    let short = |at: usize| u16::from_le_bytes([body[at], body[at + 1]]);
    let word = |at: usize| u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
    Ok(Some(match opcode {
        EQMAC_BEGIN_OPCODE => {
            ensure!(body.len() == 8, "invalid EQMac begin-cast length");
            SpellUpdate::Began {
                caster_id: short(0),
                spell_id: short(2),
                duration_ms: u32::from(short(4)),
            }
        }
        EQMAC_INTERRUPT_OPCODE => {
            // The message, then the color to show it in.
            ensure!(body.len() == 4, "invalid EQMac cast-interruption length");
            let message_id = short(0);
            if message_id == DISCIPLINE_READY {
                return Ok(None);
            }
            SpellUpdate::Interrupted {
                caster_id: 0,
                message_id: u32::from(message_id),
                caster_name: None,
            }
        }
        EQMAC_MEMORIZE_OPCODE => {
            ensure!(body.len() == 12, "invalid EQMac spell-slot length");
            let (slot, spell_id, mode) = (word(0), word(4), word(8));
            if mode == 3 {
                SpellUpdate::BarRefresh {
                    slot,
                    spell_id,
                    reduction_ms: 0,
                }
            } else {
                SpellUpdate::Slot {
                    slot,
                    spell_id,
                    mode,
                }
            }
        }
        EQMAC_DELETE_OPCODE => {
            // A signed slot, padding, the outcome and padding.
            ensure!(
                body.len() == 8 && body[4] <= 1,
                "invalid EQMac book-deletion fields"
            );
            SpellUpdate::BookDeletion {
                slot: book_slot(u32::from(short(0)))?,
                success: body[4] != 0,
            }
        }
        EQMAC_SWAP_OPCODE => {
            ensure!(body.len() == 8, "invalid EQMac book-swap length");
            SpellUpdate::BookSwap {
                from: book_slot(word(0))?,
                to: book_slot(word(4))?,
            }
        }
        _ => return Ok(None),
    }))
}

/// A slot in the 256-slot book.
fn book_slot(slot: u32) -> Result<u16> {
    u16::try_from(slot)
        .ok()
        .filter(|slot| usize::from(*slot) < EQMAC_BOOK_SLOTS)
        .context("EQMac book slot past the book")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn casts_begin_and_stop_in_sixteen_bit_fields() {
        // Caster 7 begins spell 42, its base cast time 2.5 s.
        assert_eq!(
            decode_eqmac(EQMAC_BEGIN_OPCODE, &[7, 0, 42, 0, 0xc4, 0x09, 0, 0]).unwrap(),
            Some(SpellUpdate::Began {
                caster_id: 7,
                spell_id: 42,
                duration_ms: 2500
            })
        );
        // An interruption names no caster, for the session to name the player.
        assert_eq!(
            decode_eqmac(EQMAC_INTERRUPT_OPCODE, &[173, 0, 13, 0]).unwrap(),
            Some(SpellUpdate::Interrupted {
                caster_id: 0,
                message_id: 173,
                caster_name: None
            })
        );
        // The discipline command's notice ends no cast.
        assert_eq!(
            decode_eqmac(EQMAC_INTERRUPT_OPCODE, &[0x89, 0x01, 15, 0]).unwrap(),
            None
        );
        for (opcode, size) in [
            (EQMAC_BEGIN_OPCODE, 8),
            (EQMAC_INTERRUPT_OPCODE, 4),
            (EQMAC_MEMORIZE_OPCODE, 12),
            (EQMAC_DELETE_OPCODE, 8),
            (EQMAC_SWAP_OPCODE, 8),
        ] {
            assert!(decode_eqmac(opcode, &vec![0; size - 1]).is_err());
            assert!(decode_eqmac(opcode, &vec![0; size + 1]).is_err());
        }
        assert_eq!(decode_eqmac(0xffff, &[]).unwrap(), None);
    }

    #[test]
    fn a_gems_refresh_carries_no_reuse_adjustment_and_other_modes_are_slots() {
        let body = |mode: u32| {
            let mut body = [0; 12];
            for (index, word) in [2u32, 42, mode].into_iter().enumerate() {
                body[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
            }
            body
        };
        assert_eq!(
            decode_eqmac(EQMAC_MEMORIZE_OPCODE, &body(3)).unwrap(),
            Some(SpellUpdate::BarRefresh {
                slot: 2,
                spell_id: 42,
                reduction_ms: 0
            })
        );
        for mode in [0, 1, 2] {
            assert_eq!(
                decode_eqmac(EQMAC_MEMORIZE_OPCODE, &body(mode)).unwrap(),
                Some(SpellUpdate::Slot {
                    slot: 2,
                    spell_id: 42,
                    mode
                })
            );
        }
    }

    #[test]
    fn the_book_has_256_slots_and_its_edits_stay_inside_it() {
        let mut profile = vec![0; crate::quarm::PROFILE_SIZE];
        let mut put = |slot: usize, id: i16| {
            let at = 1846 + slot * 2;
            profile[at..at + 2].copy_from_slice(&id.to_le_bytes());
        };
        put(0, 42);
        put(1, -1);
        put(255, 73);
        // The two bytes after the book are no slot of it.
        put(256, 99);
        let mut book = SpellBook::eqmac_profile(&profile).unwrap();
        assert_eq!(book.slots().len(), 256);
        assert_eq!(
            (book.slots()[0], book.slots()[1], book.slots()[255]),
            (Some(42), None, Some(73))
        );
        assert!(!book.slots().contains(&Some(99)));
        // The checks are bound by the book itself: slot 256 is not in it.
        assert!(book.check_swap(255, 256, 73, None).is_err());
        assert!(book.check_delete(256, 73).is_err());
        assert!(book.check_swap(0, 1, 42, None).is_ok());
        // TAKP echoes a swap.
        let swap = decode_eqmac(EQMAC_SWAP_OPCODE, &[0, 0, 0, 0, 1, 0, 0, 0])
            .unwrap()
            .unwrap();
        book.apply(&swap);
        assert_eq!((book.slots()[0], book.slots()[1]), (None, Some(42)));
        assert!(decode_eqmac(EQMAC_SWAP_OPCODE, &[0, 1, 0, 0, 1, 0, 0, 0]).is_err());
        // A deletion's reply, its padding whatever it is.
        let deleted = decode_eqmac(
            EQMAC_DELETE_OPCODE,
            &[255, 0, 0xa5, 0xa5, 1, 0xa5, 0xa5, 0xa5],
        )
        .unwrap()
        .unwrap();
        book.apply(&deleted);
        assert_eq!(book.slots()[255], None);
        for slot in [256i16, -1] {
            let mut reply = [0; 8];
            reply[..2].copy_from_slice(&slot.to_le_bytes());
            reply[4] = 1;
            assert!(decode_eqmac(EQMAC_DELETE_OPCODE, &reply).is_err());
        }
        assert!(decode_eqmac(EQMAC_DELETE_OPCODE, &[0, 0, 0, 0, 2, 0, 0, 0]).is_err());
        assert!(SpellBook::eqmac_profile(&profile[1..]).is_err());
    }
}
