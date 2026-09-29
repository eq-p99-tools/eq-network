//! Merchant purchases and sales in flight; the merchant's echo settles each one.
//!
//! The server deletes a sold item without sending an item update, so the sale's
//! echo is turned into that inventory change here, as the Titanium client does it.
//! An offer the server refuses is never answered, so an unanswered trade releases
//! after a timeout instead of holding the inventory forever.
use super::ClientCommand;
use eq_network_game::{
    inventory::{InventorySlot, InventoryUpdate},
    merchant::MerchantUpdate,
};
use std::time::{Duration, Instant};

/// How long an unanswered purchase or sale holds the inventory.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(3);

/// The purchase or sale waiting for the merchant's echo, if any.
#[derive(Default)]
pub(super) struct MerchantTrades {
    sent: Option<Instant>,
}

impl MerchantTrades {
    /// Whether a purchase or sale is waiting for the merchant.
    pub(super) const fn active(&self) -> bool {
        self.sent.is_some()
    }

    /// Records a purchase or sale that was handed to transport.
    pub(super) fn sent(&mut self, command: &ClientCommand, now: Instant) {
        if matches!(
            command,
            ClientCommand::Buy { .. } | ClientCommand::Sell { .. }
        ) {
            self.sent = Some(now);
        }
    }

    /// Settles the pending trade from a merchant echo. A sale also returns the
    /// inventory change the server made without reporting it; the echo is
    /// authoritative even when it arrives after the hold timed out.
    pub(super) fn observe(&mut self, update: &MerchantUpdate) -> Option<InventoryUpdate> {
        match update {
            MerchantUpdate::Sold { slot, quantity, .. } => {
                self.sent = None;
                // A script veto echoes slot -1 so that nothing is removed.
                (*slot >= 0).then_some(InventoryUpdate::Deduct {
                    slot: InventorySlot(*slot),
                    quantity: *quantity,
                })
            }
            // A purchased item arrives in its own item update.
            MerchantUpdate::Bought { .. } | MerchantUpdate::Closed => {
                self.sent = None;
                None
            }
            _ => None,
        }
    }

    /// Releases a trade the merchant never answered; true when one was pending.
    pub(super) fn expire(&mut self, now: Instant) -> bool {
        let expired = self
            .sent
            .is_some_and(|sent| now.saturating_duration_since(sent) >= ANSWER_TIMEOUT);
        if expired {
            self.sent = None;
        }
        expired
    }

    /// Forgets a trade that can no longer be answered, such as after death.
    pub(super) fn clear(&mut self) {
        self.sent = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sell(slot: i32) -> ClientCommand {
        ClientCommand::Sell {
            session_id: 1,
            merchant_id: 7,
            slot,
            quantity: 2,
            created: Instant::now(),
        }
    }

    #[test]
    fn a_sale_holds_until_its_echo_removes_the_units() {
        let start = Instant::now();
        let mut trades = MerchantTrades::default();
        trades.sent(
            &ClientCommand::SelectTarget {
                session_id: 1,
                spawn_id: None,
            },
            start,
        );
        assert!(!trades.active());
        trades.sent(&sell(25), start);
        assert!(trades.active());
        assert_eq!(
            trades.observe(&MerchantUpdate::Sold {
                slot: 25,
                quantity: 2,
                price: 40
            }),
            Some(InventoryUpdate::Deduct {
                slot: InventorySlot(25),
                quantity: 2
            })
        );
        assert!(!trades.active());
        // A late echo still reports the server's deletion.
        assert!(trades
            .observe(&MerchantUpdate::Sold {
                slot: 24,
                quantity: 1,
                price: 3
            })
            .is_some());
    }

    #[test]
    fn vetoed_and_unanswered_trades_release_without_removing_anything() {
        let start = Instant::now();
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), start);
        assert_eq!(
            trades.observe(&MerchantUpdate::Sold {
                slot: -1,
                quantity: 0,
                price: 0
            }),
            None
        );
        assert!(!trades.active());
        trades.sent(&sell(25), start);
        assert!(!trades.expire(start + Duration::from_secs(1)));
        assert!(trades.active());
        assert!(trades.expire(start + ANSWER_TIMEOUT));
        assert!(!trades.active());
        assert!(!trades.expire(start + ANSWER_TIMEOUT * 2));
    }
}
