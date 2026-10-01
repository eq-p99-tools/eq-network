//! Delayed spellbook intentions are revalidated at submission time.
use anyhow::Result;
use eq_network_game::{inventory::Inventory, spells::SpellBook};
use std::time::{Duration, Instant};

/// Prepares immediate book edits against current admission and slot contents.
pub(super) fn edit_packet(
    command: &crate::client::ClientCommand,
    book: Option<&SpellBook>,
    session: u64,
    busy: bool,
    now: Instant,
) -> Option<Result<(u16, [u8; 8])>> {
    use crate::client::ClientCommand;
    use anyhow::{ensure, Context};
    let (requested, created) = match command {
        ClientCommand::DeleteSpell {
            session_id,
            created,
            ..
        }
        | ClientCommand::SwapSpell {
            session_id,
            created,
            ..
        } => (*session_id, *created),
        _ => return None,
    };
    Some((|| {
        ensure!(
            requested == session
                && created <= now
                && now.duration_since(created) < Duration::from_secs(1)
                && !busy,
            "book edit is busy or stale"
        );
        let book = book.context("spellbook unavailable")?;
        match command {
            ClientCommand::DeleteSpell { slot, spell_id, .. } => {
                Ok((0x4f37, book.delete_packet(*slot, *spell_id)?))
            }
            ClientCommand::SwapSpell {
                from,
                to,
                from_spell,
                to_spell,
                ..
            } => Ok((
                0x2126,
                book.swap_packet(*from, *to, *from_spell, *to_spell)?,
            )),
            _ => unreachable!("edit command matched above"),
        }
    })())
}

/// Cancels local preparation once and tells the host why it cannot be submitted.
pub(super) fn cancel_pending(
    pending: &mut Option<PendingBookAction>,
    reason: &str,
    log: &mut super::Events<'_>,
) -> Result<()> {
    if pending.take().is_some() {
        log.send(super::ClientEvent::World(
            crate::world::WorldEvent::BookAction(
                eq_network_game::spells::BookActionStatus::Cancelled(reason.into()),
            ),
        ))?;
    }
    Ok(())
}

pub(super) enum BookIntent {
    Memorize {
        gem: u8,
        spell_id: u32,
    },
    Scribe {
        revision: u64,
        slot: u16,
        spell_id: u32,
    },
}

pub(super) struct PendingBookAction {
    pub started: Instant,
    pub intent: BookIntent,
}

impl PendingBookAction {
    /// Uses the provisional five-second sitting interval until live timing is verified.
    pub fn ready(&self, now: Instant) -> bool {
        now.checked_duration_since(self.started)
            .is_some_and(|elapsed| elapsed >= Duration::from_secs(5))
    }

    /// Rechecks the current book and cursor instead of replaying cached packet bytes.
    pub fn packet(&self, book: &SpellBook, inventory: &Inventory) -> Result<[u8; 16]> {
        match self.intent {
            BookIntent::Memorize { gem, spell_id } => book.memorize_packet(gem, spell_id),
            BookIntent::Scribe {
                revision,
                slot,
                spell_id,
            } => book.scribe_packet(inventory, revision, slot, spell_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediate_book_edits_require_fresh_current_unbusy_admission() {
        let now = Instant::now();
        let mut book = SpellBook::default();
        book.apply(&eq_network_game::spells::SpellUpdate::Slot {
            slot: 0,
            spell_id: 42,
            mode: 0,
        });
        for created in [
            now,
            now.checked_sub(Duration::from_secs(1)).unwrap(),
            now + Duration::from_millis(1),
        ] {
            for command in [
                crate::client::ClientCommand::SwapSpell {
                    session_id: 7,
                    from: 0,
                    to: 1,
                    from_spell: 42,
                    to_spell: None,
                    created,
                },
                crate::client::ClientCommand::DeleteSpell {
                    session_id: 7,
                    slot: 0,
                    spell_id: 42,
                    created,
                },
            ] {
                assert_eq!(
                    edit_packet(&command, Some(&book), 7, false, now)
                        .unwrap()
                        .is_ok(),
                    created == now
                );
                assert!(edit_packet(&command, Some(&book), 8, false, now)
                    .unwrap()
                    .is_err());
                assert!(edit_packet(&command, Some(&book), 7, true, now)
                    .unwrap()
                    .is_err());
                assert!(edit_packet(&command, None, 7, false, now).unwrap().is_err());
            }
        }
    }

    #[test]
    fn cancellation_removes_both_intents_and_notifies_only_once() {
        for intent in [
            BookIntent::Memorize {
                gem: 2,
                spell_id: 73,
            },
            BookIntent::Scribe {
                revision: 4,
                slot: 0,
                spell_id: 73,
            },
        ] {
            let config = crate::client::ClientConfig::new(
                "EXAMPLE_ACCOUNT",
                "EXAMPLE_PASSWORD",
                "Test Server",
                "ExampleCharacter",
            );
            let mut received = Vec::new();
            let mut handler = |event| {
                received.push(event);
                Ok(())
            };
            let mut log = super::super::Events::new(&config, &mut handler);
            let mut pending = Some(PendingBookAction {
                started: Instant::now().checked_sub(Duration::from_secs(5)).unwrap(),
                intent,
            });
            assert!(pending.as_ref().unwrap().ready(Instant::now()));
            cancel_pending(&mut pending, "Server relocated character", &mut log).unwrap();
            assert!(pending.is_none());
            cancel_pending(&mut pending, "Duplicate correction", &mut log).unwrap();
            assert_eq!(received.len(), 1);
            assert!(matches!(&received[0], crate::client::ClientEvent::World(
                crate::world::WorldEvent::BookAction(
                    eq_network_game::spells::BookActionStatus::Cancelled(reason)
                )
            ) if reason == "Server relocated character"));
        }
    }

    #[test]
    fn delayed_action_rechecks_book_and_does_not_run_early() {
        let now = Instant::now();
        let pending = PendingBookAction {
            started: now,
            intent: BookIntent::Memorize {
                gem: 2,
                spell_id: 73,
            },
        };
        assert!(!pending.ready(now));
        assert!(!pending.ready(now + Duration::from_millis(4999)));
        assert!(pending.ready(now + Duration::from_secs(5)));
        assert!(!pending.ready(now.checked_sub(Duration::from_secs(1)).unwrap()));
        let mut profile = vec![0; 19592];
        profile[2312..2316].copy_from_slice(&73u32.to_le_bytes());
        let mut book = SpellBook::titanium_profile(&profile).unwrap();
        let inventory = Inventory::default();
        assert!(pending.packet(&book, &inventory).is_ok());
        book.apply(&eq_network_game::spells::SpellUpdate::Slot {
            slot: 0,
            spell_id: 0xffff,
            mode: 0,
        });
        assert!(pending.packet(&book, &inventory).is_err());
    }
}
