//! Merchant purchases and sales in flight; the merchant's echo settles each one.
//!
//! The server deletes a sold item without sending an item update, so the sale's
//! echo is turned into that inventory change here, as the Titanium client does it.
//! An offer the server refuses is never answered, so an unanswered trade releases
//! after a timeout instead of holding the inventory forever. Echoes carry no
//! request id, so each is matched to its trade by slot, and a sale's echo
//! removes units only while the slot still holds the item that was offered,
//! less what earlier echoes removed from it: a sale released unanswered stays
//! remembered for an echo that comes late, and an echo nothing explains leaves
//! the inventory untrusted.
use crate::client::ClientCommand;
use eq_network_game::{
    inventory::{Inventory, InventoryItem, InventorySlot, InventoryUpdate},
    merchant::{MerchantUpdate, Quotes},
    world::Coins,
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

/// How long an unanswered purchase or sale holds the inventory.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(3);

/// A sale offered to the merchant: the slot, how many units were offered, and
/// what the slot held then.
struct Sale {
    slot: InventorySlot,
    quantity: u32,
    item: Option<Box<InventoryItem>>,
}

impl Sale {
    /// Whether an echo of this many units sold from this slot answers it.
    fn answered_by(&self, slot: InventorySlot, sold: u32) -> bool {
        self.slot == slot && sold > 0 && sold <= self.quantity
    }

    /// The inventory change an echo answering the sale stands for: the units
    /// leave while the slot still holds the item offered. Once the slot holds
    /// something else, nothing says which item left, so the inventory can no
    /// longer be trusted.
    fn settle(&self, sold: u32, inventory: &Inventory) -> InventoryUpdate {
        let unchanged = !inventory.stale()
            && self
                .item
                .as_deref()
                .is_some_and(|item| inventory.items().get(&self.slot) == Some(item));
        if unchanged {
            InventoryUpdate::Deduct {
                slot: self.slot,
                quantity: sold,
            }
        } else {
            InventoryUpdate::Invalidated
        }
    }

    /// Follows units an echo for another sale removed from this sale's slot,
    /// as the inventory applies the deduction: the stack shrinks, or the slot
    /// empties.
    fn removed(&mut self, slot: InventorySlot, sold: u32) {
        if self.slot != slot {
            return;
        }
        match self.item.as_deref_mut() {
            Some(item) if item.stack_count.is_some_and(|count| count > sold) => {
                item.stack_count = item.stack_count.map(|count| count - sold);
            }
            _ => self.item = None,
        }
    }
}

/// A purchase or sale waiting for the merchant's echo.
enum Trade {
    /// Buying from the merchant's slot; the item arrives in its own item update.
    Purchase { slot: u32, quantity: u32 },
    /// Selling from the player's slot.
    Sale(Sale),
}

/// The trade waiting for the merchant's echo, if any, and the last sale
/// released unanswered, whose echo may still come.
#[derive(Default)]
pub(super) struct MerchantTrades {
    pending: Option<(Trade, Instant)>,
    unanswered: Option<Sale>,
}

impl MerchantTrades {
    /// Whether a purchase or sale is waiting for the merchant.
    pub(super) const fn active(&self) -> bool {
        self.pending.is_some()
    }

    /// Records a purchase or sale that was handed to transport, and for a sale
    /// what its slot holds.
    pub(super) fn sent(&mut self, command: &ClientCommand, inventory: &Inventory, now: Instant) {
        let trade = match command {
            ClientCommand::Buy { slot, quantity, .. } => Trade::Purchase {
                slot: *slot,
                quantity: *quantity,
            },
            ClientCommand::Sell { slot, quantity, .. } => {
                let slot = InventorySlot(*slot);
                Trade::Sale(Sale {
                    slot,
                    quantity: *quantity,
                    item: inventory.items().get(&slot).cloned().map(Box::new),
                })
            }
            _ => return,
        };
        self.pending = Some((trade, now));
    }

    /// Settles the trade a merchant echo answers. A sale's echo also returns
    /// the inventory change the server made without reporting it, even when it
    /// comes after the hold was released; an echo for an item the slot no
    /// longer holds, or one that answers no trade, leaves the inventory
    /// untrusted instead of removing what the slot holds now.
    pub(super) fn observe(
        &mut self,
        update: &MerchantUpdate,
        inventory: &Inventory,
    ) -> Option<InventoryUpdate> {
        match update {
            MerchantUpdate::Sold { slot, quantity, .. } => {
                // A script veto echoes slot -1 so that nothing is removed.
                if *slot < 0 {
                    if matches!(self.pending, Some((Trade::Sale(_), _))) {
                        self.pending = None;
                    }
                    return None;
                }
                let slot = InventorySlot(*slot);
                let Some(sale) = self.answered(slot, *quantity) else {
                    return Some(InventoryUpdate::Invalidated);
                };
                let update = sale.settle(*quantity, inventory);
                if matches!(update, InventoryUpdate::Deduct { .. }) {
                    self.removed(slot, *quantity);
                }
                Some(update)
            }
            // The purchased item arrives in its own item update, so an echo
            // that answers no purchase changes nothing here.
            MerchantUpdate::Bought { slot, quantity, .. } => {
                if matches!(
                    &self.pending,
                    Some((Trade::Purchase { slot: bought, quantity: asked }, _))
                        if bought == slot && *quantity > 0 && quantity <= asked
                ) {
                    self.pending = None;
                }
                None
            }
            MerchantUpdate::Closed => {
                self.release();
                None
            }
            _ => None,
        }
    }

    /// Ends a pending purchase that an echo answers with nothing bought,
    /// which can only mean the merchant refused it: TAKP echoes a refused
    /// purchase with no place, count or price
    /// (`Client::Handle_OP_ShopPlayerBuy`), where `EQEmu` says nothing.
    /// True when one ended.
    pub(super) fn refused(&mut self, update: &MerchantUpdate) -> bool {
        let refused = matches!(update, MerchantUpdate::Bought { quantity: 0, .. })
            && matches!(self.pending, Some((Trade::Purchase { .. }, _)));
        if refused {
            self.pending = None;
        }
        refused
    }

    /// Releases a trade the merchant never answered; true when one was pending.
    pub(super) fn expire(&mut self, now: Instant) -> bool {
        let expired = self
            .pending
            .as_ref()
            .is_some_and(|(_, sent)| now.saturating_duration_since(*sent) >= ANSWER_TIMEOUT);
        if expired {
            self.release();
        }
        expired
    }

    /// Forgets the trades that can no longer be answered, such as after death.
    pub(super) fn clear(&mut self) {
        self.pending = None;
        self.unanswered = None;
    }

    /// Stops holding the inventory for the pending trade, keeping a sale for
    /// an echo that may still come.
    fn release(&mut self) {
        if let Some((Trade::Sale(sale), _)) = self.pending.take() {
            self.unanswered = Some(sale);
        }
    }

    /// Takes the sale an echo of this many units sold from this slot answers:
    /// the pending one first, then the one released unanswered.
    fn answered(&mut self, slot: InventorySlot, sold: u32) -> Option<Sale> {
        let answers = |sale: &Sale| sale.answered_by(slot, sold);
        if let Some((Trade::Sale(sale), _)) = self
            .pending
            .take_if(|(trade, _)| matches!(trade, Trade::Sale(sale) if answers(sale)))
        {
            return Some(sale);
        }
        self.unanswered.take_if(|sale| answers(sale))
    }

    /// Carries units an echo removed into the other sales remembered from the
    /// same slot, so that their echoes still find the item they offered.
    fn removed(&mut self, slot: InventorySlot, sold: u32) {
        let pending = match &mut self.pending {
            Some((Trade::Sale(sale), _)) => Some(sale),
            _ => None,
        };
        for sale in pending.into_iter().chain(self.unanswered.as_mut()) {
            sale.removed(slot, sold);
        }
    }
}

/// The merchant whose window is open: the rate it charges at, and the most
/// each place on its list can cost, which a purchase is checked against
/// before it goes out. TAKP logs a purchase it refuses for want of coins as
/// a possible hack, so nothing the purse might not cover is sent.
#[derive(Default)]
pub(super) struct OpenMerchant(Option<Window>);

/// An open merchant window.
struct Window {
    /// The rate the window opened with.
    rate: f32,
    /// By place on the list: the most a unit costs, and whether the item is
    /// priced once whatever the count, as a charged item is.
    places: BTreeMap<u32, (u64, bool)>,
}

impl OpenMerchant {
    /// Follows the merchant's news before anyone hears it: the window
    /// opening at its rate, each item listed, a place gone, the window
    /// closing. A list quoted before the rate gets each price as the server
    /// charges it.
    pub(super) fn explain(&mut self, update: &mut MerchantUpdate, quotes: Quotes) {
        match update {
            MerchantUpdate::Opened { accepted, rate, .. } => {
                self.0 = accepted.then(|| Window {
                    rate: *rate,
                    places: BTreeMap::new(),
                });
            }
            MerchantUpdate::Item(listed) => {
                if let Some(window) = self.0.as_mut() {
                    let quote = listed.price;
                    listed.price = quotes.unit_price(quote, window.rate);
                    window.places.insert(
                        listed.slot,
                        (
                            quotes.most_per_unit(quote, window.rate),
                            listed.item.activation.maximum_charges > 1,
                        ),
                    );
                }
            }
            MerchantUpdate::Removed { slot } => {
                if let Some(window) = self.0.as_mut() {
                    window.places.remove(slot);
                }
            }
            MerchantUpdate::Closed => self.0 = None,
            MerchantUpdate::Bought { .. } | MerchantUpdate::Sold { .. } => (),
        }
    }

    /// Checks a purchase against the purse: the most it can cost must be in
    /// it. Both servers price a charged item once whatever the count.
    ///
    /// # Errors
    /// Refuses a place not on the open merchant's list, a purse not known
    /// yet, and a cost the purse may not cover.
    pub(super) fn check_purchase(
        &self,
        slot: u32,
        quantity: u32,
        purse: Option<Coins>,
    ) -> Result<(), &'static str> {
        let (most, once) = self
            .0
            .as_ref()
            .and_then(|window| window.places.get(&slot))
            .copied()
            .ok_or("That item is not on the merchant's list")?;
        let purse = purse.ok_or("Your coins are not known yet")?;
        let cost = if once {
            most
        } else {
            most.saturating_mul(u64::from(quantity))
        };
        if purse.total_copper() < cost {
            return Err("You may not have enough coins for that");
        }
        Ok(())
    }

    /// Forgets the window, as after death.
    pub(super) fn clear(&mut self) {
        self.0 = None;
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

    fn buy(slot: u32) -> ClientCommand {
        ClientCommand::Buy {
            session_id: 1,
            merchant_id: 7,
            own_id: 3,
            slot,
            quantity: 1,
            created: Instant::now(),
        }
    }

    /// Two items, in slots 24 and 25.
    fn inventory() -> Inventory {
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item(24), item(25)]));
        inventory
    }

    fn echo(slot: i32) -> MerchantUpdate {
        MerchantUpdate::Sold {
            slot,
            quantity: 2,
            price: 40,
        }
    }

    fn deduct(slot: i32) -> InventoryUpdate {
        InventoryUpdate::Deduct {
            slot: InventorySlot(slot),
            quantity: 2,
        }
    }

    #[test]
    fn a_sale_holds_until_its_echo_removes_the_units() {
        let start = Instant::now();
        let inventory = inventory();
        let mut trades = MerchantTrades::default();
        trades.sent(
            &ClientCommand::SelectTarget {
                session_id: 1,
                spawn_id: None,
            },
            &inventory,
            start,
        );
        assert!(!trades.active());
        trades.sent(&sell(25), &inventory, start);
        assert!(trades.active());
        assert!(!trades.expire(start + Duration::from_secs(1)));
        assert_eq!(trades.observe(&echo(25), &inventory), Some(deduct(25)));
        assert!(!trades.active());
    }

    #[test]
    fn vetoed_and_unanswered_trades_release_without_removing_anything() {
        let start = Instant::now();
        let inventory = inventory();
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), &inventory, start);
        assert_eq!(trades.observe(&echo(-1), &inventory), None);
        assert!(!trades.active());
        trades.sent(&buy(3), &inventory, start);
        assert!(!trades.expire(start + Duration::from_secs(1)));
        assert!(trades.active());
        assert!(trades.expire(start + ANSWER_TIMEOUT));
        assert!(!trades.active());
        assert!(!trades.expire(start + ANSWER_TIMEOUT * 2));
    }

    #[test]
    fn a_late_echo_removes_the_units_while_the_slot_holds_the_item_sold() {
        let start = Instant::now();
        let inventory = inventory();
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), &inventory, start);
        assert!(trades.expire(start + ANSWER_TIMEOUT));
        assert!(!trades.active());
        assert_eq!(trades.observe(&echo(25), &inventory), Some(deduct(25)));
        // That sale is settled: another echo for the slot answers nothing.
        assert_eq!(
            trades.observe(&echo(25), &inventory),
            Some(InventoryUpdate::Invalidated)
        );
    }

    #[test]
    fn a_late_echo_never_removes_what_took_the_items_place() {
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
    fn late_echoes_for_two_sales_from_one_stack_each_remove_their_units() {
        let start = Instant::now();
        let mut stack = item(25);
        stack.stack_count = Some(5);
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![stack]));
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), &inventory, start);
        trades.expire(start + ANSWER_TIMEOUT);
        trades.sent(&sell(25), &inventory, start + ANSWER_TIMEOUT);
        for _ in 0..2 {
            let update = trades.observe(&echo(25), &inventory).unwrap();
            assert_eq!(update, deduct(25));
            inventory.apply(update);
        }
        assert!(!trades.active());
        assert!(!inventory.stale());
        assert_eq!(inventory.items()[&InventorySlot(25)].stack_count, Some(1));
        // Both sales are settled: a third echo answers nothing.
        assert_eq!(
            trades.observe(&echo(25), &inventory),
            Some(InventoryUpdate::Invalidated)
        );
    }

    #[test]
    fn an_echo_never_removes_an_identical_item_put_where_a_sold_one_was() {
        let start = Instant::now();
        let mut inventory = inventory();
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), &inventory, start);
        trades.expire(start + ANSWER_TIMEOUT);
        trades.sent(&sell(25), &inventory, start + ANSWER_TIMEOUT);
        let update = trades.observe(&echo(25), &inventory).unwrap();
        assert_eq!(update, deduct(25));
        inventory.apply(update);
        // An identical item put in its place is not the one the other sale
        // offered, which has left.
        inventory.apply(InventoryUpdate::Set(vec![item(25)]));
        assert_eq!(
            trades.observe(&echo(25), &inventory),
            Some(InventoryUpdate::Invalidated)
        );
    }

    #[test]
    fn a_late_echo_settles_its_own_sale_and_leaves_a_newer_trade_held() {
        let start = Instant::now();
        let inventory = inventory();
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), &inventory, start);
        trades.expire(start + ANSWER_TIMEOUT);
        trades.sent(&sell(24), &inventory, start + ANSWER_TIMEOUT);
        assert_eq!(trades.observe(&echo(25), &inventory), Some(deduct(25)));
        assert!(trades.active(), "the newer sale's hold was released");
        assert_eq!(trades.observe(&echo(24), &inventory), Some(deduct(24)));
        assert!(!trades.active());
    }

    #[test]
    fn echoes_that_answer_no_trade_never_settle_the_pending_one() {
        let inventory = inventory();
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), &inventory, Instant::now());
        assert_eq!(
            trades.observe(&echo(24), &inventory),
            Some(InventoryUpdate::Invalidated)
        );
        assert!(trades.active());
        // A purchase's item comes in its own update; its echo changes nothing.
        let bought = MerchantUpdate::Bought {
            slot: 25,
            quantity: 2,
            price: 1,
        };
        assert_eq!(trades.observe(&bought, &inventory), None);
        assert!(trades.active());
        // More units than were offered answer nothing either.
        let more = MerchantUpdate::Sold {
            slot: 25,
            quantity: 3,
            price: 60,
        };
        assert_eq!(
            trades.observe(&more, &inventory),
            Some(InventoryUpdate::Invalidated)
        );
        assert!(trades.active());
    }

    #[test]
    fn a_purchase_echo_releases_only_its_own_purchase() {
        let inventory = inventory();
        let mut trades = MerchantTrades::default();
        trades.sent(&buy(3), &inventory, Instant::now());
        let other = MerchantUpdate::Bought {
            slot: 4,
            quantity: 1,
            price: 10,
        };
        assert_eq!(trades.observe(&other, &inventory), None);
        assert!(trades.active());
        let bought = MerchantUpdate::Bought {
            slot: 3,
            quantity: 1,
            price: 10,
        };
        assert_eq!(trades.observe(&bought, &inventory), None);
        assert!(!trades.active());
    }

    #[test]
    fn closing_the_merchant_releases_the_hold_and_a_late_echo_still_settles() {
        let inventory = inventory();
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), &inventory, Instant::now());
        assert_eq!(trades.observe(&MerchantUpdate::Closed, &inventory), None);
        assert!(!trades.active());
        assert_eq!(trades.observe(&echo(25), &inventory), Some(deduct(25)));
    }

    #[test]
    fn an_echo_of_nothing_bought_ends_a_pending_purchase_as_refused() {
        let start = Instant::now();
        let mut trades = MerchantTrades::default();
        let nothing = MerchantUpdate::Bought {
            slot: 0,
            quantity: 0,
            price: 0,
        };
        // Answering no purchase, it ends nothing.
        assert!(!trades.refused(&nothing));
        trades.sent(&sell(25), &inventory(), start);
        assert!(!trades.refused(&nothing));
        assert!(trades.active());
        trades.clear();
        trades.sent(&buy(4), &inventory(), start);
        assert!(trades.refused(&nothing));
        assert!(!trades.active());
        // A purchase echoed with units bought is not a refusal.
        trades.sent(&buy(4), &inventory(), start);
        assert!(!trades.refused(&MerchantUpdate::Bought {
            slot: 4,
            quantity: 1,
            price: 25
        }));
    }

    /// A merchant opened at `rate`, listing an item at place 3 for `quote`,
    /// and a charged one at place 4.
    fn opened(quotes: Quotes, rate: f32, quote: u32) -> (OpenMerchant, MerchantUpdate) {
        let mut merchant = OpenMerchant::default();
        merchant.explain(
            &mut MerchantUpdate::Opened {
                merchant_id: 9,
                accepted: true,
                rate,
            },
            quotes,
        );
        let mut listed = MerchantUpdate::Item(Box::new(eq_network_game::merchant::MerchantItem {
            slot: 3,
            price: quote,
            quantity: 0,
            item: item(3),
        }));
        merchant.explain(&mut listed, quotes);
        let mut charged = item(4);
        charged.activation.maximum_charges = 5;
        merchant.explain(
            &mut MerchantUpdate::Item(Box::new(eq_network_game::merchant::MerchantItem {
                slot: 4,
                price: quote,
                quantity: 0,
                item: charged,
            })),
            quotes,
        );
        (merchant, listed)
    }

    #[test]
    fn a_list_quoted_before_the_rate_is_told_at_the_price_charged() {
        let (_, listed) = opened(Quotes::BeforeRate, 1.25, 100);
        assert!(matches!(listed, MerchantUpdate::Item(item) if item.price == 125));
        let (_, listed) = opened(Quotes::WithRate, 1.25, 100);
        assert!(matches!(listed, MerchantUpdate::Item(item) if item.price == 100));
    }

    #[test]
    fn a_purchase_goes_out_only_when_the_purse_covers_the_most_it_can_cost() {
        let gold = |gold| {
            Some(Coins {
                gold,
                ..Coins::default()
            })
        };
        // EQEmu charges what it lists: two at 100 copper each.
        let (merchant, _) = opened(Quotes::WithRate, 1.0, 100);
        assert_eq!(merchant.check_purchase(3, 2, gold(2)), Ok(()));
        assert!(merchant.check_purchase(3, 3, gold(2)).is_err());
        // A charged item costs one unit's price whatever the count.
        assert_eq!(merchant.check_purchase(4, 5, gold(1)), Ok(()));
        // TAKP: 100 before a rate of 1.25 costs at most 127 a unit.
        let (merchant, _) = opened(Quotes::BeforeRate, 1.25, 100);
        assert!(merchant
            .check_purchase(3, 2, Some(Coins::from_copper(253)))
            .is_err());
        assert_eq!(
            merchant.check_purchase(3, 2, Some(Coins::from_copper(254))),
            Ok(())
        );
        // An unlisted place, an unknown purse, and a closed window.
        assert!(merchant.check_purchase(5, 1, gold(10)).is_err());
        assert!(merchant.check_purchase(3, 1, None).is_err());
        let (mut merchant, _) = opened(Quotes::WithRate, 1.0, 100);
        merchant.explain(&mut MerchantUpdate::Removed { slot: 3 }, Quotes::WithRate);
        assert!(merchant.check_purchase(3, 1, gold(10)).is_err());
        merchant.explain(&mut MerchantUpdate::Closed, Quotes::WithRate);
        assert!(merchant.check_purchase(4, 1, gold(10)).is_err());
        // A refused opening opens nothing.
        let mut merchant = OpenMerchant::default();
        merchant.explain(
            &mut MerchantUpdate::Opened {
                merchant_id: 9,
                accepted: false,
                rate: 1.0,
            },
            Quotes::WithRate,
        );
        assert!(merchant.check_purchase(3, 1, gold(10)).is_err());
    }

    #[test]
    fn after_death_an_echo_answers_nothing() {
        let inventory = inventory();
        let start = Instant::now();
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), &inventory, start);
        trades.expire(start + ANSWER_TIMEOUT);
        trades.sent(&sell(24), &inventory, start + ANSWER_TIMEOUT);
        trades.clear();
        assert!(!trades.active());
        assert_eq!(
            trades.observe(&echo(25), &inventory),
            Some(InventoryUpdate::Invalidated)
        );
    }
}
