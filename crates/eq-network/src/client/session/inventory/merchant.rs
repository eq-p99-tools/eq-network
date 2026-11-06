//! Merchant purchases and sales in flight; the merchant's echo settles each one.
//!
//! The server deletes a sold item without sending an item update, so the sale's
//! echo is turned into that inventory change here, as the Titanium client does it.
//! Some refusals have no reply, so an unanswered trade releases its hold after a
//! timeout. Silence establishes no outcome. Echoes carry no request id, so each
//! is matched to its trade by slot, and a sale's echo
//! removes units only while the slot still holds the item that was offered,
//! less what earlier echoes removed from it: a sale released unanswered stays
//! remembered for an echo that comes late, and an echo nothing explains leaves
//! the inventory untrusted.
use crate::client::ClientCommand;
use eq_network_game::{
    inventory::{Inventory, InventoryItem, InventorySlot, InventoryUpdate},
    merchant::{MerchantItem, MerchantUpdate, Quotes},
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

/// One request resource on the wire; results contain no transaction id.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TradeKey {
    Purchase(u32),
    Sale(InventorySlot),
}

impl TradeKey {
    fn of(command: &ClientCommand) -> Option<Self> {
        match command {
            ClientCommand::Buy { slot, .. } => Some(Self::Purchase(*slot)),
            ClientCommand::Sell { slot, .. } => Some(Self::Sale(InventorySlot(*slot))),
            _ => None,
        }
    }
}

/// A submitted operation and whether the purse it was sent against is current.
struct Outstanding {
    merchant_id: u16,
    trade: Trade,
    coin_basis: bool,
}

impl Trade {
    /// Whether a positive result answers this trade's kind, slot and quantity.
    fn answers(&self, update: &MerchantUpdate) -> bool {
        match (self, update) {
            (
                Self::Purchase { slot, quantity },
                MerchantUpdate::Bought {
                    slot: bought,
                    quantity: bought_quantity,
                    ..
                },
            ) => slot == bought && *bought_quantity > 0 && bought_quantity <= quantity,
            (Self::Sale(sale), MerchantUpdate::Sold { slot, quantity, .. }) => {
                *slot >= 0 && sale.answered_by(InventorySlot(*slot), *quantity)
            }
            _ => false,
        }
    }

    /// An explicit refusal identifies a kind, but not a reliable request slot.
    fn refused_by(&self, update: &MerchantUpdate) -> bool {
        match (self, update) {
            (Self::Purchase { .. }, MerchantUpdate::Bought { quantity: 0, .. }) => true,
            (Self::Sale(_), MerchantUpdate::Sold { slot, quantity, .. }) => {
                *quantity == 0 || *slot < 0
            }
            _ => false,
        }
    }

    fn key(&self) -> TradeKey {
        match self {
            Self::Purchase { slot, .. } => TradeKey::Purchase(*slot),
            Self::Sale(sale) => TradeKey::Sale(sale.slot),
        }
    }
}

/// The trade waiting for the merchant's echo, if any, and trades released
/// unanswered, whose echoes may still come.
#[derive(Default)]
pub(super) struct MerchantTrades {
    pending: Option<(Outstanding, Instant)>,
    // At most one actual submitted operation per kind/slot. `check` prevents
    // a retry from replacing its unanswered authority, including across shops.
    unanswered: BTreeMap<TradeKey, Outstanding>,
}

impl MerchantTrades {
    /// Whether a purchase or sale is waiting for the merchant.
    pub(super) const fn active(&self) -> bool {
        self.pending.is_some()
    }

    /// Refuses a retry that could mistake an older result for a new trade's.
    /// Other slots remain usable after an unanswered trade releases its hold.
    pub(super) fn check(&self, command: &ClientCommand) -> Result<(), &'static str> {
        if TradeKey::of(command).is_some_and(|key| self.unanswered.contains_key(&key)) {
            Err("An earlier trade from that slot is unanswered; wait for its result before trying again")
        } else {
            Ok(())
        }
    }

    fn records(&self) -> impl Iterator<Item = &Outstanding> {
        self.pending
            .iter()
            .map(|(record, _)| record)
            .chain(self.unanswered.values())
    }

    /// A different merchant's reply cannot settle a transaction even when
    /// the echoed slot and count match. A refusal with multiple candidates
    /// remains ambiguous in the shared settlement path.
    pub(super) fn receipt_is_current(
        &self,
        merchant_id: u16,
        update: &MerchantUpdate,
        open: bool,
    ) -> bool {
        let mut candidates = self
            .records()
            .filter(|record| record.trade.answers(update) || record.trade.refused_by(update))
            .peekable();
        if candidates.peek().is_none() {
            return open;
        }
        candidates.all(|record| record.merchant_id == merchant_id)
    }

    /// A coin delta needs a matching trade and its original known purse basis.
    /// A newer snapshot may already include that effect; do not count it twice.
    pub(super) fn coins_current(&self, update: &MerchantUpdate) -> bool {
        self.records()
            .find(|record| record.trade.answers(update))
            .is_some_and(|record| record.coin_basis)
    }

    /// An absolute purse update supersedes every still-outstanding coin basis.
    pub(super) fn coins_replaced(&mut self) {
        for record in self
            .pending
            .iter_mut()
            .map(|(record, _)| record)
            .chain(self.unanswered.values_mut())
        {
            record.coin_basis = false;
        }
    }

    /// Records a purchase or sale that was handed to transport, and for a sale
    /// what its slot holds.
    pub(super) fn sent(&mut self, command: &ClientCommand, inventory: &Inventory, now: Instant) {
        let (merchant_id, trade) = match command {
            ClientCommand::Buy {
                merchant_id,
                slot,
                quantity,
                ..
            } => (
                *merchant_id,
                Trade::Purchase {
                    slot: *slot,
                    quantity: *quantity,
                },
            ),
            ClientCommand::Sell {
                merchant_id,
                slot,
                quantity,
                ..
            } => {
                let slot = InventorySlot(*slot);
                (
                    *merchant_id,
                    Trade::Sale(Sale {
                        slot,
                        quantity: *quantity,
                        item: inventory.items().get(&slot).cloned().map(Box::new),
                    }),
                )
            }
            _ => return,
        };
        self.pending = Some((
            Outstanding {
                merchant_id,
                trade,
                coin_basis: true,
            },
            now,
        ));
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
            MerchantUpdate::Sold { quantity: 0, .. } => None,
            MerchantUpdate::Sold { slot, quantity, .. } => {
                // A script veto echoes slot -1 so that nothing is removed.
                // Only `refused` can attribute it to an outstanding trade.
                if *slot < 0 {
                    return None;
                }
                let slot = InventorySlot(*slot);
                let Some(Trade::Sale(sale)) = self.answered(update) else {
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
            MerchantUpdate::Bought { .. } => {
                let _ = self.answered(update);
                None
            }
            MerchantUpdate::Closed => {
                self.release();
                None
            }
            _ => None,
        }
    }

    /// Ends the sole outstanding trade of the kind an explicit refusal names.
    /// `EQMac` echoes a refused purchase, and Quarm also rejects special sales
    /// this way. Neither response establishes an item or coin change.
    /// With several candidates the reply cannot identify which was refused;
    /// none is discarded or reported as definitively refused. True when one ended.
    pub(super) fn refused(&mut self, update: &MerchantUpdate) -> bool {
        let candidates = self
            .records()
            .filter(|record| record.trade.refused_by(update))
            .count();
        if candidates != 1 {
            return false;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|(record, _)| record.trade.refused_by(update))
        {
            // This trade has an outcome. Do not retain it as unanswered or
            // discard an older trade whose late result can still arrive.
            self.pending = None;
            return true;
        }
        let key = self
            .unanswered
            .iter()
            .find_map(|(key, record)| record.trade.refused_by(update).then_some(*key));
        if let Some(key) = key {
            self.unanswered.remove(&key);
            return true;
        }
        false
    }

    /// The item a remembered sale offered, which an echo of `sold` units
    /// from `slot` answers: the pending sale first, then the one released
    /// unanswered, as `observe` settles them. None when no remembered sale
    /// is answered, or the slot was empty when it was offered.
    pub(super) fn offered(&self, slot: InventorySlot, sold: u32) -> Option<&InventoryItem> {
        self.records()
            .find_map(|record| match &record.trade {
                Trade::Sale(sale) if sale.answered_by(slot, sold) => Some(sale),
                _ => None,
            })
            .and_then(|sale| sale.item.as_deref())
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
        self.unanswered.clear();
    }

    /// Stops holding the inventory for the pending trade, keeping it for
    /// an echo that may still come.
    fn release(&mut self) {
        if let Some((mut record, _)) = self.pending.take() {
            record.coin_basis = false;
            self.unanswered.insert(record.trade.key(), record);
        }
    }

    /// Takes the trade a positive result answers, including a late result.
    fn answered(&mut self, update: &MerchantUpdate) -> Option<Trade> {
        if let Some((record, _)) = self
            .pending
            .take_if(|(record, _)| record.trade.answers(update))
        {
            return Some(record.trade);
        }
        let key = self
            .unanswered
            .iter()
            .find_map(|(key, record)| record.trade.answers(update).then_some(*key))?;
        self.unanswered.remove(&key).map(|record| record.trade)
    }

    /// Carries units an echo removed into the other sales remembered from the
    /// same slot, so that their echoes still find the item they offered.
    fn removed(&mut self, slot: InventorySlot, sold: u32) {
        for record in self
            .pending
            .iter_mut()
            .map(|(record, _)| record)
            .chain(self.unanswered.values_mut())
        {
            if let Trade::Sale(sale) = &mut record.trade {
                sale.removed(slot, sold);
            }
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
    merchant_id: u16,
    /// The rate the window opened with.
    rate: f32,
    places: BTreeMap<u32, Listed>,
}

struct Listed {
    most_per_unit: u64,
    charged: bool,
    stack_limit: u32,
    stock: u32,
}

impl Window {
    /// Lists an item at its place: a list quoted before the rate gets its
    /// price as the server charges it.
    fn list(&mut self, listed: &mut MerchantItem, quotes: Quotes) {
        let quote = listed.price;
        listed.price = quotes.unit_price(quote, self.rate);
        self.places.insert(
            listed.slot,
            Listed {
                most_per_unit: quotes.most_per_unit(quote, self.rate),
                charged: listed.item.activation.maximum_charges > 1,
                stack_limit: if listed.item.stack_count.is_some() {
                    listed.item.rules.stack_size
                } else {
                    1
                },
                stock: listed.quantity,
            },
        );
    }
}

impl OpenMerchant {
    /// Follows the merchant's news before anyone hears it: the window
    /// opening at its rate, each item listed, a place gone, the window
    /// closing. A list quoted before the rate gets each price as the server
    /// charges it.
    pub(super) fn explain(&mut self, update: &mut MerchantUpdate, quotes: Quotes) {
        match update {
            MerchantUpdate::Opened {
                merchant_id,
                accepted,
                rate,
            } => {
                self.0 = accepted.then(|| Window {
                    merchant_id: *merchant_id,
                    rate: *rate,
                    places: BTreeMap::new(),
                });
            }
            MerchantUpdate::Item(listed) => {
                if let Some(window) = self.0.as_mut() {
                    window.list(listed, quotes);
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

    /// Replaces the list with a whole one, as `EQMac`'s comes, each price as
    /// the server charges it; the places listed before and not now are
    /// returned, to be told as gone. None while no window is open, which
    /// lists nothing: without its rate, the prices are not known.
    pub(super) fn replace(
        &mut self,
        items: &mut [MerchantItem],
        quotes: Quotes,
    ) -> Option<Vec<u32>> {
        let window = self.0.as_mut()?;
        let before = std::mem::take(&mut window.places);
        for listed in items.iter_mut() {
            window.list(listed, quotes);
        }
        Some(
            before
                .into_keys()
                .filter(|place| !window.places.contains_key(place))
                .collect(),
        )
    }

    /// What the server added to the purse for a sale of `units` of an item
    /// whose base price is `price`, where the sale's echo prices nothing and
    /// a window is open ([`Quotes::sale_price`]).
    pub(super) fn sale_price(&self, quotes: Quotes, price: u32, units: u32) -> Option<u32> {
        let window = self.0.as_ref()?;
        quotes.sale_price(price, window.rate, units)
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
        let listed = self
            .listed(slot)
            .ok_or("That item is not on the merchant's list")?;
        let purse = purse.ok_or("Your coins are not known yet")?;
        let cost = if listed.charged {
            listed.most_per_unit
        } else {
            listed.most_per_unit.saturating_mul(u64::from(quantity))
        };
        if purse.total_copper() < cost {
            return Err("You may not have enough coins for that");
        }
        Ok(())
    }

    /// Whether the command names the shop whose stock is in use.
    pub(super) fn is_open(&self, merchant_id: u16) -> bool {
        self.0
            .as_ref()
            .is_some_and(|window| window.merchant_id == merchant_id)
    }

    fn listed(&self, slot: u32) -> Option<&Listed> {
        self.0.as_ref()?.places.get(&slot)
    }

    /// Bounds ordinary Titanium purchases by the decoded item and any finite stock.
    pub(super) fn titanium_quantity_known(&self, slot: u32, quantity: u32) -> bool {
        self.listed(slot).is_some_and(|listed| {
            (1..=listed.stack_limit.max(1)).contains(&quantity)
                && (listed.stock == 0 || quantity <= listed.stock)
        })
    }

    /// Bound ordinary Quarm purchases by decoded stock rather than guessing
    /// quantities for charged, non-stackable or unknown items.
    pub(super) fn quarm_quantity_known(&self, slot: u32, quantity: u32) -> bool {
        self.listed(slot)
            .is_some_and(|listed| (1..=listed.stack_limit.min(20)).contains(&quantity))
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
            own_id: 3,
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
    fn a_zero_sale_does_not_answer_a_purchase_or_invalidate_its_items() {
        let mut trades = MerchantTrades::default();
        let inventory = inventory();
        trades.sent(&buy(3), &inventory, Instant::now());
        let zero = MerchantUpdate::Sold {
            slot: 30,
            quantity: 0,
            price: 0,
        };
        assert!(!trades.refused(&zero));
        assert_eq!(trades.observe(&zero, &inventory), None);
        assert!(trades.active());
    }

    #[test]
    fn a_sale_holds_until_its_echo_removes_the_units() {
        let start = Instant::now();
        let inventory = inventory();
        let mut trades = MerchantTrades::default();
        trades.sent(
            &ClientCommand::SelectTarget {
                request_id: 1,
                session_id: 1,
                spawn_id: None,
                expected_pet: None,
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
        assert!(trades.refused(&echo(-1)));
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
    fn a_whole_list_replaces_the_last_and_names_the_places_gone() {
        let (mut merchant, _) = opened(Quotes::BeforeRate, 1.25, 100);
        // A new list: place 3 at 80 before the rate, and place 5; 4 is gone.
        let mut list = [3, 5].map(|slot| eq_network_game::merchant::MerchantItem {
            slot,
            price: 80,
            quantity: 0,
            item: item(i32::try_from(slot).unwrap()),
        });
        assert_eq!(
            merchant.replace(&mut list, Quotes::BeforeRate),
            Some(vec![4])
        );
        assert_eq!(list.each_ref().map(|listed| listed.price), [100, 100]);
        assert_eq!(
            merchant.check_purchase(5, 1, Some(Coins::from_copper(102))),
            Ok(())
        );
        assert!(merchant
            .check_purchase(4, 1, Some(Coins::from_copper(1000)))
            .is_err());
        // With no window open, nothing is listed.
        let mut closed = OpenMerchant::default();
        assert_eq!(closed.replace(&mut list, Quotes::BeforeRate), None);
        assert_eq!(closed.sale_price(Quotes::BeforeRate, 100, 1), None);
        // What TAKP adds for a sale, at the window's rate.
        assert_eq!(merchant.sale_price(Quotes::BeforeRate, 100, 2), Some(160));
        assert_eq!(merchant.sale_price(Quotes::WithRate, 100, 2), None);
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
    fn the_item_a_remembered_sale_offered_is_what_its_echo_answers() {
        let start = Instant::now();
        let mut trades = MerchantTrades::default();
        trades.sent(&sell(25), &inventory(), start);
        assert_eq!(trades.offered(InventorySlot(25), 2), Some(&item(25)));
        // Another slot, or more units than were offered, answer nothing.
        assert_eq!(trades.offered(InventorySlot(24), 2), None);
        assert_eq!(trades.offered(InventorySlot(25), 3), None);
        // Released unanswered, it is still remembered.
        assert!(trades.expire(start + ANSWER_TIMEOUT));
        assert_eq!(trades.offered(InventorySlot(25), 2), Some(&item(25)));
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
