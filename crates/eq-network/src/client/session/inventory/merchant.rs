//! Reconciles merchant replies without treating silence as a refused transaction.
use crate::client::ClientCommand;
use eq_network_game::{
    inventory::{Inventory, InventoryItem, InventorySlot, InventoryUpdate},
    merchant::MerchantUpdate,
};
use std::time::{Duration, Instant};

const ANSWER_TIMEOUT: Duration = Duration::from_secs(3);

enum Operation {
    Purchase {
        slot: u32,
        quantity: u32,
    },
    Sale {
        slot: InventorySlot,
        quantity: u32,
        item: Option<Box<InventoryItem>>,
    },
}

enum Answer {
    Waiting(Instant),
    Uncertain,
}

struct Pending {
    operation: Operation,
    answer: Answer,
}

/// Only one transaction can be correlated: merchant echoes carry no request ID.
#[derive(Default)]
pub(super) struct MerchantTrades {
    pending: Option<Pending>,
}

impl MerchantTrades {
    pub(super) const fn active(&self) -> bool {
        self.pending.is_some()
    }

    /// Retains the source item until a matching echo, including after a timeout.
    pub(super) fn sent(&mut self, command: &ClientCommand, inventory: &Inventory, now: Instant) {
        // The session holds the inventory. Never replace an unresolved operation.
        if self.active() {
            return;
        }
        let operation = match command {
            ClientCommand::Buy { slot, quantity, .. } => Operation::Purchase {
                slot: *slot,
                quantity: *quantity,
            },
            ClientCommand::Sell { slot, quantity, .. } => {
                let slot = InventorySlot(*slot);
                Operation::Sale {
                    slot,
                    quantity: *quantity,
                    item: inventory.items().get(&slot).cloned().map(Box::new),
                }
            }
            _ => return,
        };
        self.pending = Some(Pending {
            operation,
            answer: Answer::Waiting(now),
        });
    }

    /// Applies a sale only to its original item. An unrelated reply cannot
    /// settle another operation; ambiguous replies invalidate the projection.
    pub(super) fn observe(
        &mut self,
        update: &MerchantUpdate,
        inventory: &Inventory,
    ) -> Option<InventoryUpdate> {
        match update {
            MerchantUpdate::Sold { slot, quantity, .. } => {
                if let Some(Pending {
                    operation:
                        Operation::Sale {
                            slot: expected,
                            quantity: requested,
                            item,
                        },
                    ..
                }) = &self.pending
                {
                    // A script veto echoes a negative slot, without deleting anything.
                    if *slot < 0 {
                        self.pending = None;
                        return None;
                    }
                    if *expected == InventorySlot(*slot) && *quantity > 0 && *quantity <= *requested
                    {
                        let unchanged = item
                            .as_deref()
                            .is_some_and(|item| inventory.items().get(expected) == Some(item));
                        self.pending = None;
                        return Some(if unchanged && !inventory.stale() {
                            InventoryUpdate::Deduct {
                                slot: InventorySlot(*slot),
                                quantity: *quantity,
                            }
                        } else {
                            InventoryUpdate::Invalidated
                        });
                    }
                }
                (*slot >= 0).then_some(InventoryUpdate::Invalidated)
            }
            MerchantUpdate::Bought { slot, quantity, .. } => {
                if matches!(&self.pending, Some(Pending { operation: Operation::Purchase { slot: expected, quantity: requested }, .. }) if slot == expected && *quantity > 0 && quantity <= requested)
                {
                    self.pending = None;
                    None // The purchased item arrives in a separate item update.
                } else {
                    Some(InventoryUpdate::Invalidated)
                }
            }
            MerchantUpdate::Closed => self.clear().then_some(InventoryUpdate::Invalidated),
            _ => None,
        }
    }

    /// Reports uncertainty once, retaining the hold: silence is not a rejection.
    pub(super) fn expire(&mut self, now: Instant) -> bool {
        let Some(pending) = &mut self.pending else {
            return false;
        };
        if matches!(pending.answer, Answer::Waiting(sent) if now.saturating_duration_since(sent) >= ANSWER_TIMEOUT)
        {
            pending.answer = Answer::Uncertain;
            true
        } else {
            false
        }
    }

    /// Abandons correlation on a reset. An unresolved operation requires the
    /// caller to invalidate or replace the inventory.
    pub(super) fn clear(&mut self) -> bool {
        self.pending.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::session::feature::testing::item;

    fn sell(slot: i32) -> ClientCommand {
        ClientCommand::Sell {
            session_id: 1,
            merchant_id: 7,
            slot,
            quantity: 2,
            created: Instant::now(),
        }
    }

    fn inventory() -> Inventory {
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item(25)]));
        inventory
    }

    fn echo(slot: i32) -> MerchantUpdate {
        MerchantUpdate::Sold {
            slot,
            quantity: 2,
            price: 40,
        }
    }

    #[test]
    fn late_matching_sale_settles_the_original_hold() {
        let start = Instant::now();
        let inventory = inventory();
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), &inventory, start);
        assert!(!trades.expire(start + Duration::from_secs(1)));
        assert!(trades.expire(start + ANSWER_TIMEOUT));
        assert!(trades.active());
        assert!(!trades.expire(start + ANSWER_TIMEOUT * 2));
        assert_eq!(
            trades.observe(&echo(25), &inventory),
            Some(InventoryUpdate::Deduct {
                slot: InventorySlot(25),
                quantity: 2
            })
        );
        assert!(!trades.active());
    }

    #[test]
    fn delayed_sale_never_deletes_replacement_contents() {
        let start = Instant::now();
        let mut inventory = inventory();
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), &inventory, start);
        trades.expire(start + ANSWER_TIMEOUT);
        let mut replacement = item(25);
        replacement.details.id += 1;
        inventory.apply(InventoryUpdate::Set(vec![replacement.clone()]));
        inventory.apply(trades.observe(&echo(25), &inventory).unwrap());
        assert!(inventory.stale());
        assert_eq!(inventory.items()[&InventorySlot(25)], replacement);
    }

    #[test]
    fn unrelated_echo_does_not_clear_another_hold() {
        let inventory = inventory();
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), &inventory, Instant::now());
        assert_eq!(
            trades.observe(&echo(24), &inventory),
            Some(InventoryUpdate::Invalidated)
        );
        assert!(trades.active());
        assert_eq!(
            trades.observe(
                &MerchantUpdate::Bought {
                    slot: 25,
                    quantity: 2,
                    price: 1
                },
                &inventory
            ),
            Some(InventoryUpdate::Invalidated)
        );
        assert!(trades.active());
    }

    #[test]
    fn timeout_cannot_be_replaced_by_a_new_transaction() {
        let inventory = inventory();
        let start = Instant::now();
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), &inventory, start);
        trades.expire(start + ANSWER_TIMEOUT);
        trades.sent(&sell(24), &inventory, start + ANSWER_TIMEOUT);
        assert_eq!(
            trades.observe(&echo(24), &inventory),
            Some(InventoryUpdate::Invalidated)
        );
        assert!(trades.active());
        assert!(matches!(
            trades.observe(&echo(25), &inventory),
            Some(InventoryUpdate::Deduct { .. })
        ));
    }

    #[test]
    fn closing_an_uncertain_trade_invalidates_and_late_echo_cannot_deduct() {
        let inventory = inventory();
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), &inventory, Instant::now());
        assert_eq!(
            trades.observe(&MerchantUpdate::Closed, &inventory),
            Some(InventoryUpdate::Invalidated)
        );
        assert!(!trades.active());
        assert_eq!(
            trades.observe(&echo(25), &inventory),
            Some(InventoryUpdate::Invalidated)
        );
    }

    #[test]
    fn explicit_script_veto_releases_without_invalidating() {
        let inventory = inventory();
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), &inventory, Instant::now());
        assert_eq!(trades.observe(&echo(-1), &inventory), None);
        assert!(!trades.active());
    }
}
