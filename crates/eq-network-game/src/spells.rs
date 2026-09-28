//! Server-driven Titanium spell state.
use anyhow::{ensure, Result};
use serde::Serialize;

/// Local worker progress; submission is not confirmation of a server-side change.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum BookActionStatus {
    /// The sitting interval is running.
    Preparing,
    /// The request was submitted; wait for book/gem updates.
    Submitted,
    /// A tracked edit is awaiting its matching server reply.
    AwaitingReply,
    /// The server confirmed the outstanding tracked edit.
    Confirmed,
    /// A pending action was cancelled before submission.
    Cancelled(String),
    /// Validation rejected a requested action.
    Rejected(String),
}

/// Encodes removal of the spell currently occupying a gem, without deleting its book entry.
///
/// # Errors
/// Rejects invalid gems, empty slots, and stale spell identifiers.
pub fn forget_packet(gems: &[Option<u32>; 8], gem: u8, spell_id: u32) -> Result<[u8; 16]> {
    ensure!(
        !matches!(spell_id, 0 | 0xffff | u32::MAX)
            && gems.get(usize::from(gem)) == Some(&Some(spell_id)),
        "gem is empty, invalid, or changed"
    );
    let mut body = [0; 16];
    body[..4].copy_from_slice(&u32::from(gem).to_le_bytes());
    body[4..8].copy_from_slice(&spell_id.to_le_bytes());
    body[8..12].copy_from_slice(&2u32.to_le_bytes());
    Ok(body)
}

/// Spellbook slots from the admitted profile; empty slots retain their indexes.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SpellBook {
    slots: Vec<Option<u32>>,
}

impl Default for SpellBook {
    fn default() -> Self {
        Self {
            slots: vec![None; 400],
        }
    }
}

impl SpellBook {
    /// Encodes a swap only if both slots still match the user's selection.
    /// Empty destination slots are allowed; the source must contain a spell.
    ///
    /// # Errors
    /// Rejects equal, invalid, empty-source, or changed slots.
    pub fn swap_packet(
        &self,
        from: u16,
        to: u16,
        from_spell: u32,
        to_spell: Option<u32>,
    ) -> Result<[u8; 8]> {
        ensure!(
            from != to && !matches!(from_spell, 0 | 0xffff | u32::MAX),
            "invalid spell swap"
        );
        ensure!(
            self.slots.get(usize::from(from)) == Some(&Some(from_spell))
                && self.slots.get(usize::from(to)) == Some(&to_spell),
            "book slots changed or are invalid"
        );
        let mut body = [0; 8];
        body[..4].copy_from_slice(&u32::from(from).to_le_bytes());
        body[4..].copy_from_slice(&u32::from(to).to_le_bytes());
        Ok(body)
    }

    /// Encodes deletion only when the expected spell still occupies the selected book slot.
    /// This never changes local book contents; the reply determines success.
    ///
    /// # Errors
    /// Rejects invalid, empty, or replaced slots and sentinel spell identifiers.
    pub fn delete_packet(&self, slot: u16, spell_id: u32) -> Result<[u8; 8]> {
        ensure!(
            !matches!(spell_id, 0 | 0xffff | u32::MAX)
                && self.slots.get(usize::from(slot)) == Some(&Some(spell_id)),
            "book slot is empty, invalid, or changed"
        );
        let mut body = [0; 8];
        body[..2].copy_from_slice(&slot.to_le_bytes());
        Ok(body)
    }

    /// Encodes scribing only for a current cursor scroll and an empty book slot.
    /// The server remains responsible for spell-level eligibility and consumption.
    ///
    /// # Errors
    /// Rejects stale inventory, changed scrolls, duplicate spells and occupied book slots.
    pub fn scribe_packet(
        &self,
        inventory: &crate::inventory::Inventory,
        revision: u64,
        slot: u16,
        spell_id: u32,
    ) -> Result<[u8; 16]> {
        use anyhow::Context;
        ensure!(
            inventory.received() && !inventory.stale() && inventory.revision() == revision,
            "inventory changed or is unavailable"
        );
        let scroll = inventory
            .items()
            .get(&crate::inventory::InventorySlot(30))
            .context("put the scroll on the cursor first")?;
        ensure!(
            !matches!(spell_id, 0 | 0xffff | u32::MAX) && scroll.scroll_spell == Some(spell_id),
            "cursor scroll changed or has no spell"
        );
        ensure!(
            self.slots.get(usize::from(slot)) == Some(&None),
            "book slot is not empty"
        );
        ensure!(
            !self.slots.contains(&Some(spell_id)),
            "spell is already scribed"
        );
        let mut body = [0; 16];
        body[..4].copy_from_slice(&u32::from(slot).to_le_bytes());
        body[4..8].copy_from_slice(&spell_id.to_le_bytes());
        Ok(body)
    }
    /// Reads the 400 Titanium book slots from an exact-sized player profile.
    ///
    /// # Errors
    /// Rejects other profile dialects and truncated profiles.
    pub fn titanium_profile(profile: &[u8]) -> Result<Self> {
        ensure!(profile.len() == 19592, "invalid Titanium profile length");
        let slots = profile[2312..3912]
            .chunks_exact(4)
            .map(|bytes| {
                let id = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                (!matches!(id, 0 | 0xffff | u32::MAX)).then_some(id)
            })
            .collect();
        Ok(Self { slots })
    }

    /// Indexed book contents, including empty slots.
    #[must_use]
    pub fn slots(&self) -> &[Option<u32>] {
        &self.slots
    }

    /// Encodes a gem assignment only for spells confirmed in this book.
    ///
    /// # Errors
    /// Rejects invalid gems and spells that have not been scribed.
    pub fn memorize_packet(&self, gem: u8, spell_id: u32) -> Result<[u8; 16]> {
        ensure!(
            gem < 8 && self.slots.contains(&Some(spell_id)),
            "spell is not scribed or gem is invalid"
        );
        let mut body = [0; 16];
        body[..4].copy_from_slice(&u32::from(gem).to_le_bytes());
        body[4..8].copy_from_slice(&spell_id.to_le_bytes());
        body[8..12].copy_from_slice(&1u32.to_le_bytes());
        // No reuse reduction is requested. The capture's nonzero trailing value
        // is not copied because its client-side provenance is unverified.
        Ok(body)
    }

    /// Applies confirmed book changes without changing memorized slots.
    pub fn apply(&mut self, update: &SpellUpdate) {
        if let SpellUpdate::BookSwap { from, to } = *update {
            let (from, to) = (usize::from(from), usize::from(to));
            if from < self.slots.len() && to < self.slots.len() {
                self.slots.swap(from, to);
            }
            return;
        }
        let (slot, value) = match *update {
            SpellUpdate::Slot {
                slot,
                spell_id,
                mode: 0,
            } => (
                slot,
                (!matches!(spell_id, 0 | 0xffff | u32::MAX)).then_some(spell_id),
            ),
            SpellUpdate::BookDeletion {
                slot,
                success: true,
            } => (u32::from(slot), None),
            _ => return,
        };
        if let Ok(index) = usize::try_from(slot) {
            if let Some(entry) = self.slots.get_mut(index) {
                *entry = value;
            }
        }
    }
}

/// Server spell notification, independent of outbound requests.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum SpellUpdate {
    /// Server echo confirming the exchange of two book slots, including an empty destination.
    BookSwap {
        /// Source book index.
        from: u16,
        /// Destination book index.
        to: u16,
    },
    /// Reply to a book-slot deletion; failed replies must preserve the entry.
    BookDeletion {
        /// Indexed Titanium book slot, independent of memorized gems.
        slot: u16,
        /// Whether the server removed the entry.
        success: bool,
    },
    /// Mode-3 spell-bar refresh, carrying the server's reuse-time adjustment.
    /// This does not prove the spell affected its target.
    BarRefresh {
        /// Spell gem or another casting slot; consumers validate their own slots.
        slot: u32,
        /// Spell whose reuse timer should refresh.
        spell_id: u32,
        /// Milliseconds deducted from the local spell's base reuse time.
        reduction_ms: u32,
    },
    /// Server cancellation, including interrupts, fizzles and related casting failures.
    Interrupted {
        /// Caster whose spell stopped; nearby casters must not clear our cast.
        caster_id: u32,
        /// Server string-table identifier, retained without guessing its meaning.
        message_id: u32,
    },
    /// Casting started; duration alone does not prove success.
    Began {
        /// Zone-local caster.
        caster_id: u16,
        /// Spell identifier.
        spell_id: u16,
        /// Casting duration in milliseconds.
        duration_ms: u32,
    },
    /// Spellbook or memorized-slot update.
    Slot {
        /// Slot index.
        slot: u32,
        /// Spell identifier.
        spell_id: u32,
        /// Zero scribes, one memorizes, two clears; unknown modes are retained.
        mode: u32,
    },
    /// Casting fields accompanying a resource update.
    Mana {
        /// Associated spell identifier.
        spell_id: u32,
        /// Resource change must not stop an active cast when true.
        keep_casting: bool,
    },
}

impl SpellUpdate {
    /// Applies valid gem updates; book and unknown modes leave gems unchanged.
    pub fn apply_gems(&self, gems: &mut [Option<u32>; 8]) {
        if let Self::Slot {
            slot,
            spell_id,
            mode,
        } = *self
        {
            if let Ok(index) = usize::try_from(slot) {
                if let Some(gem) = gems.get_mut(index) {
                    match mode {
                        1 => {
                            *gem = (!matches!(spell_id, 0 | 0xffff | u32::MAX)).then_some(spell_id);
                        }
                        2 => *gem = None,
                        _ => (),
                    }
                }
            }
        }
    }
}

/// Decodes spell notifications without interpreting chat as state.
///
/// # Errors
/// Rejects malformed lengths and boolean values.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<SpellUpdate>> {
    let word = |offset| {
        u32::from_le_bytes([
            body[offset],
            body[offset + 1],
            body[offset + 2],
            body[offset + 3],
        ])
    };
    Ok(Some(match opcode {
        0x2126 => {
            ensure!(body.len() == 8, "invalid book-swap length");
            let (from, to) = (word(0), word(4));
            ensure!(from < 400 && to < 400, "invalid book-swap slots");
            SpellUpdate::BookSwap {
                from: u16::try_from(from)?,
                to: u16::try_from(to)?,
            }
        }
        0x4f37 => {
            // EQEmu Titanium DeleteSpell_Struct: signed slot, padding, success, padding.
            ensure!(body.len() == 8, "invalid book-deletion length");
            let slot = u16::from_le_bytes([body[0], body[1]]);
            ensure!(slot < 400 && body[4] <= 1, "invalid book-deletion fields");
            SpellUpdate::BookDeletion {
                slot,
                success: body[4] != 0,
            }
        }
        0x0b97 => {
            // Titanium InterruptCast_Struct has two words and an optional NUL-terminated label.
            ensure!(
                body.len() >= 8 && (body.len() == 8 || body.last() == Some(&0)),
                "invalid cast-interruption length or label"
            );
            SpellUpdate::Interrupted {
                caster_id: word(0),
                message_id: word(4),
            }
        }
        0x3990 => {
            ensure!(body.len() == 8, "invalid begin-cast length");
            SpellUpdate::Began {
                caster_id: u16::from_le_bytes([body[0], body[1]]),
                spell_id: u16::from_le_bytes([body[2], body[3]]),
                duration_ms: word(4),
            }
        }
        0x308e => {
            ensure!(body.len() == 16, "invalid spell-slot length");
            if word(8) == 3 {
                return Ok(Some(SpellUpdate::BarRefresh {
                    slot: word(0),
                    spell_id: word(4),
                    reduction_ms: word(12),
                }));
            }
            SpellUpdate::Slot {
                slot: word(0),
                spell_id: word(4),
                mode: word(8),
            }
        }
        0x4839 => {
            ensure!(
                body.len() == 16 && body[12] <= 1,
                "invalid spell-mana update"
            );
            SpellUpdate::Mana {
                spell_id: word(8),
                keep_casting: body[12] != 0,
            }
        }
        _ => return Ok(None),
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn swap_waits_for_reply_and_preserves_gems_and_empty_slots() {
        let mut book = super::SpellBook::default();
        book.apply(&super::SpellUpdate::Slot {
            slot: 0,
            spell_id: 42,
            mode: 0,
        });
        book.apply(&super::SpellUpdate::Slot {
            slot: 399,
            spell_id: 73,
            mode: 0,
        });
        let body = book.swap_packet(0, 399, 42, Some(73)).unwrap();
        assert_eq!(body, [0, 0, 0, 0, 143, 1, 0, 0]);
        assert_eq!(book.slots()[0], Some(42));
        let update = super::decode(0x2126, &body).unwrap().unwrap();
        book.apply(&update);
        let mut gems = [Some(42); 8];
        update.apply_gems(&mut gems);
        assert_eq!(gems, [Some(42); 8]);
        assert_eq!((book.slots()[0], book.slots()[399]), (Some(73), Some(42)));
        assert!(book.swap_packet(0, 399, 42, Some(73)).is_err());
        assert!(book.swap_packet(0, 0, 73, Some(73)).is_err());
        assert!(book.swap_packet(400, 0, 73, Some(73)).is_err());
        let body = book.swap_packet(399, 1, 42, None).unwrap();
        book.apply(&super::decode(0x2126, &body).unwrap().unwrap());
        assert_eq!((book.slots()[399], book.slots()[1]), (None, Some(42)));
        assert!(super::decode(0x2126, &body[..7]).is_err());
        assert!(super::decode(0x2126, &[255; 8]).is_err());
    }

    #[test]
    fn deletion_reply_changes_only_the_confirmed_book_slot() {
        let mut book = super::SpellBook::default();
        book.apply(&super::SpellUpdate::Slot {
            slot: 399,
            spell_id: 42,
            mode: 0,
        });
        assert_eq!(
            book.delete_packet(399, 42).unwrap(),
            [143, 1, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(book.slots()[399], Some(42));
        for (slot, spell) in [(400, 42), (0, 42), (399, 73), (399, 0), (399, u32::MAX)] {
            assert!(book.delete_packet(slot, spell).is_err());
        }
        let mut gems = [Some(42); 8];
        let mut packet = [0xa5; 8];
        packet[..2].copy_from_slice(&399u16.to_le_bytes());
        packet[4] = 0;
        let denied = super::decode(0x4f37, &packet).unwrap().unwrap();
        book.apply(&denied);
        assert_eq!(book.slots()[399], Some(42));
        packet[4] = 1;
        let accepted = super::decode(0x4f37, &packet).unwrap().unwrap();
        book.apply(&accepted);
        accepted.apply_gems(&mut gems);
        assert_eq!(book.slots()[399], None);
        assert_eq!(gems, [Some(42); 8]);
        book.apply(&accepted);
        assert_eq!(book.slots()[399], None);
        assert!(super::decode(0x4f37, &packet[..7]).is_err());
        packet[4] = 2;
        assert!(super::decode(0x4f37, &packet).is_err());
        packet[4] = 1;
        for invalid in [400u16, 0xffff] {
            packet[..2].copy_from_slice(&invalid.to_le_bytes());
            assert!(super::decode(0x4f37, &packet).is_err());
        }
    }

    use super::*;
    #[test]
    fn bar_refresh_retains_adjustment_without_replacing_gems() {
        let mut body = [0; 16];
        for (index, word) in [2u32, 42, 3, 1500].into_iter().enumerate() {
            body[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        let update = decode(0x308e, &body).unwrap().unwrap();
        assert_eq!(
            update,
            SpellUpdate::BarRefresh {
                slot: 2,
                spell_id: 42,
                reduction_ms: 1500
            }
        );
        let mut gems = [Some(99); 8];
        update.apply_gems(&mut gems);
        assert_eq!(gems, [Some(99); 8]);
        for length in 0..16 {
            assert!(decode(0x308e, &body[..length]).is_err());
        }
    }

    #[test]
    fn interruption_preserves_caster_and_reason_without_exposing_optional_label() {
        let mut body = Vec::from([7, 0, 0, 0, 42, 0, 0, 0]);
        let expected = Some(SpellUpdate::Interrupted {
            caster_id: 7,
            message_id: 42,
        });
        assert_eq!(decode(0x0b97, &body).unwrap(), expected);
        for length in 0..8 {
            assert!(decode(0x0b97, &body[..length]).is_err());
        }
        body.extend_from_slice(b"Synthetic caster");
        assert!(decode(0x0b97, &body).is_err());
        body.push(0);
        assert_eq!(decode(0x0b97, &body).unwrap(), expected);
    }
    #[test]
    fn forgetting_checks_current_gem_and_preserves_book() {
        let mut gems = [None; 8];
        gems[3] = Some(73);
        let mut profile = vec![0; 19592];
        profile[2312..2316].copy_from_slice(&73u32.to_le_bytes());
        let mut book = SpellBook::titanium_profile(&profile).unwrap();
        let before = book.clone();
        let body = forget_packet(&gems, 3, 73).unwrap();
        assert_eq!(body, [3, 0, 0, 0, 73, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
        let update = decode(0x308e, &body).unwrap().unwrap();
        update.apply_gems(&mut gems);
        book.apply(&update);
        assert_eq!(gems[3], None);
        assert_eq!(book, before);
        assert!(forget_packet(&gems, 3, 73).is_err());
        assert!(forget_packet(&gems, 8, 73).is_err());
        gems[3] = Some(42);
        assert!(forget_packet(&gems, 3, 73).is_err());
        for sentinel in [0, 0xffff, u32::MAX] {
            gems[3] = Some(sentinel);
            assert!(forget_packet(&gems, 3, sentinel).is_err());
            SpellUpdate::Slot {
                slot: 3,
                spell_id: sentinel,
                mode: 1,
            }
            .apply_gems(&mut gems);
            assert_eq!(gems[3], None);
        }
    }
    #[test]
    fn profile_book_preserves_indexes_and_only_scribe_changes_the_book() {
        let mut profile = vec![0; 19592];
        profile[2312 + 399 * 4..2312 + 400 * 4].copy_from_slice(&73u32.to_le_bytes());
        let mut book = SpellBook::titanium_profile(&profile).unwrap();
        assert_eq!(book.slots().len(), 400);
        assert_eq!(book.slots()[399], Some(73));
        assert!(book.memorize_packet(8, 73).is_err());
        assert!(book.memorize_packet(0, 99).is_err());
        let packet = book.memorize_packet(2, 73).unwrap();
        assert_eq!(u32::from_le_bytes(packet[..4].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(packet[8..12].try_into().unwrap()), 1);
        assert_eq!(book.slots()[0], None);
        book.apply(&SpellUpdate::Slot {
            slot: 0,
            spell_id: 42,
            mode: 1,
        });
        assert_eq!(book.slots()[0], None);
        book.apply(&SpellUpdate::Slot {
            slot: 0,
            spell_id: 42,
            mode: 0,
        });
        assert_eq!(book.slots()[0], Some(42));
        book.apply(&SpellUpdate::Slot {
            slot: 400,
            spell_id: 99,
            mode: 0,
        });
        assert_eq!(book.slots().len(), 400);
        assert!(SpellBook::titanium_profile(&profile[..19591]).is_err());
    }
    #[test]
    fn book_and_unknown_modes_never_replace_memorized_gems() {
        let mut gems = [None; 8];
        gems[0] = Some(42);
        for (slot, mode) in [(0, 0), (0, 99), (8, 1), (u32::MAX, 1)] {
            SpellUpdate::Slot {
                slot,
                spell_id: 71,
                mode,
            }
            .apply_gems(&mut gems);
            assert_eq!(gems[0], Some(42));
        }
        SpellUpdate::Slot {
            slot: 0,
            spell_id: 71,
            mode: 1,
        }
        .apply_gems(&mut gems);
        assert_eq!(gems[0], Some(71));
        SpellUpdate::Slot {
            slot: 0,
            spell_id: 71,
            mode: 2,
        }
        .apply_gems(&mut gems);
        assert_eq!(gems[0], None);
    }
    #[test]
    fn truncation_and_invalid_boolean_are_rejected() {
        for (op, size) in [(0x3990, 8), (0x308e, 16), (0x4839, 16)] {
            for n in 0..size {
                assert!(decode(op, &vec![0; n]).is_err());
            }
        }
        let mut body = [0; 16];
        body[12] = 2;
        assert!(decode(0x4839, &body).is_err());
        assert_eq!(decode(0xffff, &[]).unwrap(), None);
    }
}
