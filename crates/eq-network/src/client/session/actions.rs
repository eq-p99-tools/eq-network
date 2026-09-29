//! One table of what in-flight actions hold and what new commands need.
//!
//! Controllers still decide when their own action starts and ends (a cast begins
//! and is interrupted, a book edit is answered, a scribe consumes its scroll). This
//! module only turns their current state into held resources and refuses any
//! command that needs one of them, so conflicts are decided in one place instead
//! of by per-action guards.
use super::{
    book_edits::BookEdits,
    casting::{self, CastGuard},
    merchant::MerchantTrades,
    spellbook::{BookIntent, PendingBookAction},
    ClientCommand, ClientEvent, Events,
};
use anyhow::Result;
use eq_network_game::{spells::BookActionStatus, world::WorldEvent};

/// Game state an in-flight action may hold exclusively.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Resource {
    /// Casting a spell or item click effect.
    Casting,
    /// The spellbook and memorized gems.
    Spellbook,
    /// Inventory contents, including the cursor.
    Inventory,
}

/// Resources held right now, each with the reason shown to a refused command.
pub(super) struct Held(Vec<(Resource, &'static str)>);

impl Held {
    /// Derives holds from the controllers that track each action's lifetime.
    pub(super) fn from_state(
        cast_guard: &CastGuard,
        book_edits: &BookEdits,
        pending: Option<&PendingBookAction>,
        trades: &MerchantTrades,
    ) -> Self {
        let mut held = Vec::new();
        if cast_guard.active() {
            held.push((
                Resource::Casting,
                "Wait for the current cast to finish or interrupt it",
            ));
        }
        if trades.active() {
            // A sold item leaves only when the merchant echoes the sale.
            held.push((Resource::Inventory, "Wait for the merchant to answer"));
        }
        let scribing = book_edits.scribing()
            || pending.is_some_and(|pending| matches!(pending.intent, BookIntent::Scribe { .. }));
        if scribing {
            // The server consumes the cursor scroll when it answers; moving items
            // meanwhile desynchronized P99's inventory and logged the character out.
            held.push((Resource::Inventory, "Wait for scribing to finish"));
        }
        if scribing || pending.is_some() || book_edits.outstanding() {
            held.push((Resource::Spellbook, "Wait for the current spellbook change"));
        }
        Self(held)
    }

    /// The reason a command must wait, if it needs anything held.
    pub(super) fn conflict(&self, command: &ClientCommand) -> Option<&'static str> {
        let needs = needs(command);
        self.0
            .iter()
            .find(|(resource, _)| needs.contains(resource))
            .map(|(_, reason)| *reason)
    }
}

/// What a command needs exclusively; unlisted commands need nothing here.
pub(super) fn needs(command: &ClientCommand) -> &'static [Resource] {
    use Resource::{Casting, Inventory, Spellbook};
    match command {
        ClientCommand::CastSpell { .. } => &[Casting],
        ClientCommand::UseItem(_) => &[Casting, Inventory],
        ClientCommand::ScribeSpell { .. } => &[Casting, Spellbook, Inventory],
        ClientCommand::MemorizeSpell { .. }
        | ClientCommand::ForgetSpell { .. }
        | ClientCommand::DeleteSpell { .. }
        | ClientCommand::SwapSpell { .. } => &[Casting, Spellbook],
        ClientCommand::MoveInventory(_)
        | ClientCommand::Buy { .. }
        | ClientCommand::Sell { .. } => &[Inventory],
        _ => &[],
    }
}

/// Reports a refused command through the result event its caller waits for.
pub(super) fn refuse(command: &ClientCommand, reason: &str, log: &mut Events<'_>) -> Result<()> {
    match command {
        ClientCommand::CastSpell { .. } => {
            if let Some(event) = casting::rejected(command, reason) {
                log.send(ClientEvent::World(event))?;
            }
        }
        ClientCommand::UseItem(request) => {
            log.send(ClientEvent::World(WorldEvent::ItemUseAction {
                session_id: request.session_id,
                request_id: request.request_id,
                error: Some(reason.into()),
            }))?;
        }
        ClientCommand::MoveInventory(request) => {
            log.send(ClientEvent::World(WorldEvent::InventoryAction {
                session_id: request.session_id,
                revision: request.revision,
                error: Some(reason.into()),
            }))?;
        }
        ClientCommand::Buy { session_id, .. } | ClientCommand::Sell { session_id, .. } => {
            log.send(ClientEvent::World(WorldEvent::MerchantRefused {
                session_id: *session_id,
                reason: reason.into(),
            }))?;
        }
        _ => log.send(ClientEvent::World(WorldEvent::BookAction(
            BookActionStatus::Rejected(reason.into()),
        )))?,
    }
    log.diagnostic(reason.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn cast() -> ClientCommand {
        ClientCommand::CastSpell {
            session_id: 1,
            gem: 0,
            spell_id: 42,
            target_id: 7,
            created: Instant::now(),
        }
    }

    fn memorize() -> ClientCommand {
        ClientCommand::MemorizeSpell {
            session_id: 1,
            gem: 0,
            spell_id: 42,
            created: Instant::now(),
        }
    }

    #[test]
    fn holds_refuse_only_commands_that_need_them() {
        let idle_trades = MerchantTrades::default();
        let idle = Held::from_state(
            &CastGuard::default(),
            &BookEdits::default(),
            None,
            &idle_trades,
        );
        assert_eq!(idle.conflict(&cast()), None);
        assert_eq!(idle.conflict(&memorize()), None);

        let mut guard = CastGuard::default();
        guard.submitted(42, Instant::now());
        let casting = Held::from_state(&guard, &BookEdits::default(), None, &idle_trades);
        assert!(casting.conflict(&cast()).is_some());
        assert!(casting.conflict(&memorize()).is_some());
        assert_eq!(
            casting.conflict(&ClientCommand::SelectTarget {
                session_id: 1,
                spawn_id: Some(7)
            }),
            None
        );

        let scribe = PendingBookAction {
            started: Instant::now(),
            intent: BookIntent::Scribe {
                revision: 1,
                slot: 0,
                spell_id: 42,
            },
        };
        let scribing = Held::from_state(
            &CastGuard::default(),
            &BookEdits::default(),
            Some(&scribe),
            &idle_trades,
        );
        assert_eq!(scribing.conflict(&cast()), None);
        assert_eq!(
            scribing.conflict(&memorize()),
            Some("Wait for the current spellbook change")
        );
        let memorizing = PendingBookAction {
            started: Instant::now(),
            intent: BookIntent::Memorize {
                gem: 0,
                spell_id: 42,
            },
        };
        let held = Held::from_state(
            &CastGuard::default(),
            &BookEdits::default(),
            Some(&memorizing),
            &idle_trades,
        );
        assert!(held.conflict(&memorize()).is_some());
        assert!(!held
            .0
            .iter()
            .any(|(resource, _)| *resource == Resource::Inventory));
        assert_eq!(
            scribing
                .0
                .iter()
                .find(|(resource, _)| *resource == Resource::Inventory)
                .map(|(_, reason)| *reason),
            Some("Wait for scribing to finish")
        );
    }

    #[test]
    fn a_pending_trade_holds_the_inventory_but_not_casting() {
        let sell = ClientCommand::Sell {
            session_id: 1,
            merchant_id: 7,
            slot: 24,
            quantity: 1,
            created: Instant::now(),
        };
        let mut trades = MerchantTrades::default();
        let idle = Held::from_state(&CastGuard::default(), &BookEdits::default(), None, &trades);
        assert_eq!(idle.conflict(&sell), None);
        trades.sent(&sell, Instant::now());
        let trading = Held::from_state(&CastGuard::default(), &BookEdits::default(), None, &trades);
        assert_eq!(
            trading.conflict(&sell),
            Some("Wait for the merchant to answer")
        );
        assert_eq!(trading.conflict(&cast()), None);
    }
}
