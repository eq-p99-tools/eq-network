//! Spell notices and the spellbook on the `EQMac` wire, as TAKP sends them:
//! its server's own packed structs (`common/eq_packet_structs.h`), which the
//! Mac patch passes on untranslated, with 16-bit fields where Titanium's are
//! 32, a book of 256 slots, no reuse adjustment on a gem's refresh, and
//! interruptions that name no caster. The opcodes are TAKP
//! `utils/patches/patch_Mac.conf`'s, their bytes swapped.
use super::{SpellBook, SpellUpdate};
use crate::{command::EncodedCommand, inventory::InventorySlot};
use anyhow::{ensure, Context, Result};

/// `OP_CastSpell`: the client asking to cast.
pub const EQMAC_CAST_OPCODE: u16 = 0x7e41;
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

/// The casting slot of an item's click effect (TAKP `CastingSlot::Item`).
const ITEM_CAST: u16 = 10;
/// The item slot of a cast from a gem: none.
const NO_ITEM: u16 = u16::MAX;
/// The item slots TAKP casts click effects from: the worn ones, which on the
/// Mac client have no charm slot, and the general ones. It casts nothing in
/// a bag (`common/patches/mac_limits.h` `AllowClickCastFromBag`,
/// `InventoryProfile::SupportsClickCasting`).
const CLICKABLE: std::ops::RangeInclusive<u16> = 1..=29;

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

/// `EQMac`'s cast of a memorized gem's spell at a target.
///
/// # Errors
/// Refuses a gem past the eighth, a spell 16 bits cannot carry, and no
/// target.
pub fn eqmac_cast(gem: u8, spell_id: u32, target_id: u16) -> Result<EncodedCommand> {
    ensure!(gem < 8, "invalid spell gem");
    cast(u16::from(gem), spell_id, NO_ITEM, target_id)
}

/// `EQMac`'s cast of an item's click effect at a target, already checked
/// against the item.
///
/// # Errors
/// Refuses an item outside the worn and general slots, a spell 16 bits
/// cannot carry, and no target.
pub fn eqmac_item_cast(
    spell_id: u32,
    slot: InventorySlot,
    target_id: u16,
) -> Result<EncodedCommand> {
    let item = u16::try_from(slot.0)
        .ok()
        .filter(|slot| CLICKABLE.contains(slot))
        .context("TAKP casts items only from worn and general slots")?;
    cast(ITEM_CAST, spell_id, item, target_id)
}

/// TAKP's `CastSpell_Struct` (12 bytes, packed): the casting slot, the
/// spell, the item's slot and the target as 16 bits each, then a CRC TAKP
/// never reads (`Client::Handle_OP_CastSpell`), sent as 0: the official
/// client's is unrecorded.
fn cast(slot: u16, spell_id: u32, item: u16, target_id: u16) -> Result<EncodedCommand> {
    let spell = u16::try_from(spell_id)
        .ok()
        .filter(|spell| !matches!(*spell, 0 | u16::MAX))
        .context("invalid EQMac spell")?;
    ensure!(target_id != 0, "a cast needs a target");
    let mut body = Vec::with_capacity(12);
    for value in [slot, spell, item, target_id] {
        body.extend_from_slice(&value.to_le_bytes());
    }
    body.extend_from_slice(&0u32.to_le_bytes());
    Ok(EncodedCommand {
        opcode: EQMAC_CAST_OPCODE,
        body,
    })
}

/// `EQMac`'s request memorizing a spell into a gem, already checked
/// against the book.
#[must_use]
pub fn eqmac_memorize(gem: u8, spell_id: u32) -> EncodedCommand {
    book_packet(u32::from(gem), spell_id, 1)
}

/// `EQMac`'s request forgetting a gem's spell, already checked against the
/// gem. TAKP refuses a forget whose spell the player could not memorize, so
/// it names the gem's own spell.
#[must_use]
pub fn eqmac_forget(gem: u8, spell_id: u32) -> EncodedCommand {
    book_packet(u32::from(gem), spell_id, 2)
}

/// `EQMac`'s request scribing the cursor's scroll into a book slot, already
/// checked against the book and the cursor.
///
/// # Errors
/// Refuses a slot past the 256-slot book, where TAKP would use the scroll
/// up and scribe nothing (`Client::ScribeSpell`).
pub fn eqmac_scribe(slot: u16, spell_id: u32) -> Result<EncodedCommand> {
    let slot = book_slot(u32::from(slot))?;
    Ok(book_packet(u32::from(slot), spell_id, 0))
}

/// `EQMac`'s request exchanging two book slots, already checked against the
/// book; TAKP answers with the same packet.
///
/// # Errors
/// Refuses a slot past the 256-slot book.
pub fn eqmac_swap(from: u16, to: u16) -> Result<EncodedCommand> {
    let mut body = Vec::with_capacity(8);
    for slot in [from, to] {
        body.extend_from_slice(&u32::from(book_slot(u32::from(slot))?).to_le_bytes());
    }
    Ok(EncodedCommand {
        opcode: EQMAC_SWAP_OPCODE,
        body,
    })
}

/// TAKP's `MemorizeSpell_Struct` (12 bytes): the gem or book slot, the
/// spell and the mode, each 32 bits; 0 scribes, 1 memorizes and 2 forgets.
fn book_packet(slot: u32, spell_id: u32, mode: u32) -> EncodedCommand {
    let mut body = Vec::with_capacity(12);
    for value in [slot, spell_id, mode] {
        body.extend_from_slice(&value.to_le_bytes());
    }
    EncodedCommand {
        opcode: EQMAC_MEMORIZE_OPCODE,
        body,
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
    fn a_cast_names_its_gem_or_item_slot_in_sixteen_bit_fields() {
        assert_eq!(
            eqmac_cast(7, 42, 9).unwrap().body,
            [7, 0, 42, 0, 255, 255, 9, 0, 0, 0, 0, 0]
        );
        // An item's click: casting slot 10 and the item's own slot, which
        // TAKP numbers as Titanium does for worn and general slots.
        let item = eqmac_item_cast(73, InventorySlot(23), 9).unwrap();
        assert_eq!(item.opcode, EQMAC_CAST_OPCODE);
        assert_eq!(item.body, [10, 0, 73, 0, 23, 0, 9, 0, 0, 0, 0, 0]);
        assert!(eqmac_item_cast(73, InventorySlot(1), 9).is_ok());
        // No charm slot, cursor or bag.
        for slot in [0, 30, 251, 262] {
            assert!(eqmac_item_cast(73, InventorySlot(slot), 9).is_err());
        }
        assert!(eqmac_cast(8, 42, 9).is_err());
        for spell in [0, 0xffff, 0x1_0000] {
            assert!(eqmac_cast(0, spell, 9).is_err());
        }
        assert!(eqmac_cast(0, 42, 0).is_err());
    }

    #[test]
    fn book_requests_are_takps_packets_and_stay_inside_the_book() {
        let words = |packet: EncodedCommand| {
            packet
                .body
                .as_chunks::<4>()
                .0
                .iter()
                .map(|word| u32::from_le_bytes(*word))
                .collect::<Vec<_>>()
        };
        let memorize = eqmac_memorize(2, 42);
        assert_eq!(memorize.opcode, EQMAC_MEMORIZE_OPCODE);
        assert_eq!(words(memorize), [2, 42, 1]);
        assert_eq!(words(eqmac_forget(2, 42)), [2, 42, 2]);
        assert_eq!(words(eqmac_scribe(255, 42).unwrap()), [255, 42, 0]);
        let swap = eqmac_swap(0, 255).unwrap();
        assert_eq!(swap.opcode, EQMAC_SWAP_OPCODE);
        assert_eq!(words(swap), [0, 255]);
        // TAKP would use a scroll up on slot 256 and scribe nothing.
        assert!(eqmac_scribe(256, 42).is_err());
        assert!(eqmac_swap(0, 256).is_err());
        assert!(eqmac_swap(256, 0).is_err());
        // Each answer reads back as the change it asked for.
        assert_eq!(
            decode_eqmac(EQMAC_MEMORIZE_OPCODE, &eqmac_memorize(2, 42).body).unwrap(),
            Some(SpellUpdate::Slot {
                slot: 2,
                spell_id: 42,
                mode: 1
            })
        );
        assert_eq!(
            decode_eqmac(EQMAC_SWAP_OPCODE, &eqmac_swap(0, 255).unwrap().body).unwrap(),
            Some(SpellUpdate::BookSwap { from: 0, to: 255 })
        );
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
