//! The player's belongings: what the server says the inventory holds, the
//! player's item moves and the merchant trades that change it. Only this
//! feature changes the inventory that every feature reads.
//!
//! A move is predicted as soon as it is sent and settles once the server has
//! let it stand; a trade holds the inventory until the merchant echoes it.
//! Items handed over in a give or trade window leave the trade slots when the
//! window closes, which servers do not say item by item.
//!
//! The coins are this feature's too. Servers answer no coin move and say what
//! the purse holds only now and then, so the ledger keeps the coins where the
//! player put them, changes the purse for loot and purchases kind by kind as
//! servers do, and takes each money update as the truth about the purse.
mod meals;
mod merchant;

use super::{
    actions::Resource,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{Context, Result};
use eq_network_game::{
    exchange::ExchangeUpdate,
    inventory::{
        banker_in_range, Inventory, InventoryActor, InventoryMove, InventorySlot, InventoryUpdate,
        MoveRules,
    },
    loot::{LootResponse, LootUpdate},
    merchant::{MerchantUpdate, Quotes},
    message::{Message, Part},
    money::Wallet,
    request::Request,
    training::TrainingUpdate,
    world::Coins,
    world::{SpawnKind, WorldEvent},
};
use merchant::{MerchantTrades, OpenMerchant};
use std::time::{Duration, Instant};

/// The player's inventory as the session knows it. Every feature reads it;
/// only this module changes it, so a second writer does not compile.
#[derive(Default)]
pub(super) struct Carried(Inventory);

impl std::ops::Deref for Carried {
    type Target = Inventory;

    fn deref(&self) -> &Inventory {
        &self.0
    }
}

/// Lets another feature's tests start from an inventory of their choosing.
#[cfg(test)]
impl From<Inventory> for Carried {
    fn from(inventory: Inventory) -> Self {
        Self(inventory)
    }
}

/// The player's coins as the session keeps them. Every feature reads it;
/// only this module changes it.
#[derive(Debug, Default)]
pub(super) struct Ledger(Wallet);

impl std::ops::Deref for Ledger {
    type Target = Wallet;

    fn deref(&self) -> &Wallet {
        &self.0
    }
}

/// Lets another feature's tests start with coins of their choosing.
#[cfg(test)]
impl From<Wallet> for Ledger {
    fn from(wallet: Wallet) -> Self {
        Self(wallet)
    }
}

/// Tells the host where the coins are: the purse as `Coins`, the rest as
/// `CoinsElsewhere`.
fn tell_coins(world: &World, purse: bool, out: &mut Out<'_, '_>) -> Result<()> {
    let coins = &world.coins;
    if purse {
        if let Some(coins) = coins.purse {
            out.log.send(ClientEvent::World(WorldEvent::Coins(coins)))?;
        }
    }
    out.log.send(ClientEvent::World(WorldEvent::CoinsElsewhere {
        cursor: coins.cursor,
        bank: coins.bank.unwrap_or_default(),
        given: coins.given,
        offered: coins.offered,
    }))
}

/// Tells the host the merchant refused a trade, or that the session did on
/// its behalf.
fn merchant_refused(world: &World, reason: &str, out: &mut Out<'_, '_>) -> Result<()> {
    out.log
        .send(ClientEvent::World(WorldEvent::MerchantRefused {
            session_id: world.session_id,
            reason: reason.into(),
        }))?;
    out.log
        .diagnostic(format!("Merchant trade refused: {reason}"))
}

/// Follows what a message says about the coins; true when the purse changed
/// without the host hearing it (loot coins, a purchase, coins TAKP added),
/// false when only
/// coins elsewhere changed, None when nothing did. A money update replaces
/// the purse, and the host hears it as it is.
fn coins_news(message: &Message, wallet: &mut Wallet) -> Option<bool> {
    let event = match message {
        Message::Event(event) => event,
        // What the server added to the purse, as TAKP announces it.
        Message::PurseAdded(coins) => {
            wallet.add_to_purse(*coins);
            return Some(true);
        }
        _ => return None,
    };
    match event {
        WorldEvent::Coins(coins) => {
            wallet.purse = Some(*coins);
            None
        }
        WorldEvent::CoinsElsewhere { cursor, bank, .. } => {
            wallet.cursor = *cursor;
            wallet.bank = Some(*bank);
            Some(false)
        }
        WorldEvent::Loot(LootUpdate::Opened {
            response: LootResponse::Normal,
            coins,
        }) if !coins.is_empty() => {
            wallet.add_to_purse(*coins);
            Some(true)
        }
        WorldEvent::Merchant(MerchantUpdate::Bought { price, .. }) if *price > 0 => {
            wallet.pay(u64::from(*price));
            Some(true)
        }
        // A sale's price, which TAKP adds to the purse without a money
        // update; `EQEmu`'s update right after replaces the purse with the
        // same coins. TAKP's echo prices nothing, so the session has priced
        // it from the sale it sent before this hears it (inferred: the
        // official client's purse after a sale is unrecorded).
        WorldEvent::Merchant(MerchantUpdate::Sold { price, .. }) if *price > 0 => {
            wallet.add_to_purse(Coins::from_copper(*price));
            Some(true)
        }
        // The other player's coins, which the server reports as added.
        WorldEvent::Exchange(ExchangeUpdate::Coins { coin, amount }) => {
            let offered = wallet.offered.of_mut(*coin);
            *offered = offered.saturating_add(*amount);
            Some(false)
        }
        // The server takes a practice's cost without saying so.
        WorldEvent::Training(TrainingUpdate::Trained { cost, .. }) if *cost > 0 => {
            wallet.pay(*cost);
            Some(true)
        }
        // What the window held was handed over, or comes back with the
        // server's money update.
        WorldEvent::Exchange(ExchangeUpdate::Finished | ExchangeUpdate::Cancelled { .. }) => {
            wallet.given = Coins::default();
            wallet.offered = Coins::default();
            Some(false)
        }
        _ => None,
    }
}

/// How long the server has to refuse a move. `EQEmu` refuses at once, by
/// resending the move's slots, and never acknowledges a success.
const REFUSAL_WINDOW: Duration = Duration::from_secs(2);

/// Settles moves the server has not refused in time.
#[derive(Default)]
struct Settlement(Option<Instant>);

impl Settlement {
    /// Notes a move sent at `now`; every move restarts the wait.
    fn sent(&mut self, now: Instant) {
        self.0 = Some(now);
    }

    /// Whether the last move sent is still unanswered at `now`: the server
    /// has not refused it, and the window to refuse it has not passed.
    fn in_flight(&self, now: Instant) -> bool {
        self.0
            .is_some_and(|sent| now.saturating_duration_since(sent) < REFUSAL_WINDOW)
    }

    /// The settling update, once the last move has gone unrefused for the window
    /// and predictions remain.
    fn due(&mut self, inventory: &Inventory, now: Instant) -> Option<InventoryUpdate> {
        let sent = self.0?;
        if now.saturating_duration_since(sent) < REFUSAL_WINDOW {
            return None;
        }
        self.0 = None;
        inventory.predicted().then_some(InventoryUpdate::Settled)
    }
}

/// What the inventory's rules need to know about the player as they are now,
/// including whether a banker stands near enough to reach the bank.
fn actor(world: &World) -> Option<InventoryActor> {
    let player = world.player.as_ref()?;
    let (_, position) = world.player_at()?;
    Some(InventoryActor {
        bank_access: world
            .spawns
            .all()
            .any(|spawn| banker_in_range(position, spawn)),
        deity: player.deity,
        dual_wield: player
            .skills
            .as_ref()
            .and_then(|skills| skills.get(22))
            .copied(),
        class: player.class,
        race: player.race,
        level: player.level,
        trade_slots: world
            .exchange
            .as_ref()
            .map_or(0, super::exchange::Exchange::trade_slots),
        trade_no_drop: world
            .exchange
            .as_ref()
            .is_none_or(|exchange| exchange.partner == eq_network_game::exchange::Partner::Npc),
        world_container: world.container.is_open(),
    })
}

/// The inventory change a message reports, if any; an unreadable item update
/// leaves the inventory untrustworthy.
fn update(message: &Message) -> Option<InventoryUpdate> {
    match message {
        Message::Event(WorldEvent::Inventory(update)) => Some(update.clone()),
        Message::Unreadable {
            part: Part::Inventory,
            ..
        } => Some(InventoryUpdate::Invalidated),
        _ => None,
    }
}

/// Changes the inventory and tells the host.
fn change(update: InventoryUpdate, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
    world.inventory.0.apply(update.clone());
    out.log
        .send(ClientEvent::World(WorldEvent::Inventory(update)))
}

/// What a server type lets the player do with their belongings beside
/// moving items, each as checked on that kind of server.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Allowances {
    /// Moving coins between the purse, the cursor, the bank and a trade.
    pub(super) coins: bool,
    /// Buying and selling at merchants.
    pub(super) merchants: bool,
    /// Eating and drinking, by hand and on the session's own.
    pub(super) meals: bool,
}

impl Allowances {
    /// Coins, merchants and meals.
    pub(super) const ALL: Self = Self {
        coins: true,
        merchants: true,
        meals: true,
    };
}

/// The player's belongings and the changes to them in flight.
pub(super) struct Belongings {
    /// What the player may do with them beside moving items.
    allows: Allowances,
    /// The server type's rules for item moves.
    rules: MoveRules,
    /// Moves waiting to settle.
    settlement: Settlement,
    /// A purchase or sale waiting for the merchant.
    trades: MerchantTrades,
    /// The merchant whose window is open, and what its list costs.
    merchant: OpenMerchant,
    /// How the server type's merchant packets price trades.
    quotes: Quotes,
    /// How fed and watered the player is, which decides when to eat.
    meals: meals::Meals,
}

impl Belongings {
    /// Belongings under `EQEmu`'s rules on Titanium's wire, with coins,
    /// merchants and meals.
    pub(super) fn new(auto_eat: eq_network_game::food::AutoEat) -> Self {
        Self::under(
            MoveRules::EqEmu,
            Quotes::WithRate,
            Allowances::ALL,
            auto_eat,
        )
    }

    /// Belongings whose items the player moves under a server type's
    /// `rules`, with what else it `allows`, and merchant packets priced as
    /// `quotes` says. Without meals the session never eats; with them it
    /// eats and drinks when the server's own client would, by the rules'
    /// threshold.
    pub(super) fn under(
        rules: MoveRules,
        quotes: Quotes,
        allows: Allowances,
        auto_eat: eq_network_game::food::AutoEat,
    ) -> Self {
        let hungry = match rules {
            MoveRules::EqEmu => eq_network_game::food::HUNGRY,
            MoveRules::Takp => eq_network_game::food::TAKP_HUNGRY,
        };
        Self {
            allows,
            rules,
            settlement: Settlement::default(),
            trades: MerchantTrades::default(),
            merchant: OpenMerchant::default(),
            quotes,
            meals: meals::Meals::new(auto_eat, hungry),
        }
    }

    /// Moves an item, predicting where it lands until the server refuses the
    /// move or lets it stand.
    fn move_item(
        &mut self,
        request: &InventoryMove,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let now = Instant::now();
        let planned = actor(world)
            .context("Character equipment data is unavailable")
            .and_then(|actor| world.inventory.0.plan_move_for(self.rules, request, actor))
            .and_then(|update| {
                let packet = out.encode(&Request::MoveItem {
                    from: request.from,
                    to: request.to,
                    quantity: request.quantity,
                })?;
                Ok((update, packet))
            });
        let result = match planned {
            Ok((update, packet)) => {
                // A failed send ends the admission; an uncertain move is
                // never retried.
                out.send(&packet)?;
                world.inventory.0.apply(update.clone());
                Ok(update)
            }
            Err(error) => Err(error),
        };
        let error = match result {
            Ok(update) => {
                self.settlement.sent(now);
                out.log
                    .send(ClientEvent::World(WorldEvent::Inventory(update)))?;
                None
            }
            Err(error) => Some(error.to_string()),
        };
        out.log
            .send(ClientEvent::World(WorldEvent::InventoryAction {
                session_id: request.session_id,
                revision: request.revision,
                error,
            }))
    }

    /// Buys or sells; the merchant's echo settles the trade. A purchase the
    /// purse may not cover is refused before it goes out, as is selling a
    /// NO DROP item, which both servers ignore without a word.
    fn trade(
        &mut self,
        command: &ClientCommand,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let refusal = match command {
            ClientCommand::Buy { slot, quantity, .. } => self
                .merchant
                .check_purchase(*slot, *quantity, world.coins.purse)
                .err(),
            ClientCommand::Sell { slot, .. } => world
                .inventory
                .items()
                .get(&InventorySlot(*slot))
                .is_some_and(|item| item.details.flags.iter().any(|flag| flag == "NO DROP"))
                .then_some("NO DROP items cannot be sold"),
            _ => None,
        };
        if let Some(reason) = refusal {
            return merchant_refused(world, reason, out);
        }
        if out.command(command)? {
            self.trades.sent(command, &world.inventory, Instant::now());
        }
        Ok(())
    }

    /// Moves coins and keeps them where they went. Servers answer no coin
    /// move, and log one that takes more than a place holds, so the ledger
    /// refuses that before anything is sent; a trade window must be open for
    /// coins to go into it, and a banker near for the bank.
    fn move_coins(command: &ClientCommand, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        use eq_network_game::money::CoinPlace;
        let ClientCommand::MoveCoins {
            session_id,
            transfer,
            ..
        } = command
        else {
            return Ok(());
        };
        let touches = |place| transfer.from == place || transfer.to == place;
        let refusal = if touches(CoinPlace::Trade)
            && world
                .exchange
                .is_none_or(|exchange| exchange.trade_slots() == 0)
        {
            Some("No trade window is open".to_owned())
        } else if transfer.from == CoinPlace::Trade {
            // `EQEmu` ignores such a move, which would part the ledger from
            // the server's count.
            Some("Coins in a trade stay there until it closes".to_owned())
        } else if touches(CoinPlace::Bank) && !actor(world).is_some_and(|actor| actor.bank_access) {
            Some("Stand near a banker to use the bank".to_owned())
        } else {
            let mut after = *world.coins;
            match after
                .apply(*transfer)
                .map_err(str::to_owned)
                .and_then(|()| {
                    out.encode(&Request::MoveCoins(*transfer))
                        .map_err(|error| error.to_string())
                }) {
                Ok(packet) => {
                    out.send(&packet)?;
                    world.coins.0 = after;
                    let purse = touches(CoinPlace::Purse);
                    return tell_coins(world, purse, out);
                }
                Err(reason) => Some(reason),
            }
        };
        let reason = refusal.unwrap_or_default();
        out.log.send(ClientEvent::World(WorldEvent::CoinsRefused {
            session_id: *session_id,
            reason: reason.clone(),
        }))?;
        out.log.diagnostic(format!("Coin move refused: {reason}"))
    }

    /// Opens or closes a merchant's window; only a merchant the player can
    /// see will trade.
    fn shop(command: &ClientCommand, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        let ClientCommand::Shop { merchant_id, .. } = command else {
            return Ok(());
        };
        let merchant = world
            .spawns
            .visible(*merchant_id)
            .is_some_and(|spawn| spawn.kind == SpawnKind::Npc);
        if !merchant {
            return out
                .log
                .diagnostic("Rejected an unavailable merchant".into());
        }
        out.command(command).map(drop)
    }
}

impl Feature for Belongings {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        let mut offered = vec![Capability::Inventory];
        if self.allows.merchants {
            offered.push(Capability::Trading);
        }
        offered
    }

    /// Item updates and the profile's coins before admission build the
    /// inventory and the ledger the admission reports.
    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        if let Some(update) = update(message) {
            world.inventory.0.apply(update);
        }
        if let Message::Event(WorldEvent::Nourishment(nourishment)) = message {
            self.meals.admit(*nourishment);
        }
        coins_news(message, &mut world.coins.0);
        Ok(())
    }

    /// Reports the inventory, replayed from empty so that the host's copy and
    /// this one count the same revisions.
    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let admission = world.inventory.admission_updates();
        world.inventory = Carried::default();
        for update in admission {
            change(update, world, out)?;
        }
        tell_coins(world, true, out)
    }

    /// A sold item leaves only when the merchant echoes the sale. Under
    /// TAKP's rules one move is in flight at a time, which is ours and
    /// stricter than the official client, which moves items without
    /// waiting: TAKP answers only a refused move, so a move is answered once
    /// it settles, and holding the next keeps a resync from landing on top
    /// of other predictions. A move onto a cursor TAKP may still refill
    /// (from a queue it said nothing of) waits with it.
    fn holds(&self, _world: &World, now: Instant) -> Vec<(Resource, &'static str)> {
        let mut holds = Vec::new();
        if self.trades.active() {
            holds.push((Resource::Inventory, "Wait for the merchant to answer"));
        }
        if self.rules == MoveRules::Takp && self.settlement.in_flight(now) {
            holds.push((
                Resource::Inventory,
                "Wait for the server to settle the last move",
            ));
        }
        holds
    }

    /// Closing a give or trade window empties the trade slots: the server
    /// sends their items back without emptying them itself.
    fn notice(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if matches!(command, ClientCommand::CancelTrade { .. }) && world.exchange.is_some() {
            change(InventoryUpdate::TradeEmptied, world, out)?;
        }
        Ok(())
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        match command {
            ClientCommand::MoveInventory(_) => true,
            ClientCommand::MoveCoins { .. } => self.allows.coins,
            ClientCommand::Shop { .. } | ClientCommand::Buy { .. } | ClientCommand::Sell { .. } => {
                self.allows.merchants
            }
            ClientCommand::Consume { .. } | ClientCommand::AutoEat { .. } => self.allows.meals,
            _ => false,
        }
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match command {
            ClientCommand::MoveInventory(request) => self.move_item(request, world, out),
            ClientCommand::MoveCoins { .. } => Self::move_coins(command, world, out),
            ClientCommand::Shop { .. } => Self::shop(command, world, out),
            ClientCommand::AutoEat { auto_eat, .. } => {
                self.meals.choose(*auto_eat);
                Ok(())
            }
            ClientCommand::Consume {
                session_id, slot, ..
            } => match self.meals.by_hand(*slot, world, out)? {
                Some((reason, string_id)) => {
                    out.log.send(ClientEvent::World(WorldEvent::ConsumeRefused {
                        session_id: *session_id,
                        reason: reason.into(),
                        string_id,
                    }))
                }
                None => Ok(()),
            },
            _ => self.trade(command, world, out),
        }
    }

    /// Settles moves the server let stand, and releases a trade the merchant
    /// never answered.
    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        // Servers answer only a refused move, so silence settles the rest.
        if let Some(update) = self.settlement.due(&world.inventory, now) {
            change(update, world, out)?;
        }
        if self.trades.expire(now) {
            merchant_refused(world, "The merchant did not accept that offer.", out)?;
        }
        Ok(())
    }

    /// Follows the open merchant's list before anyone hears it, so a list
    /// quoted before the merchant's rate reaches the host at the price
    /// charged; and where a sale's echo prices nothing, prices it as the
    /// server added it to the purse, from the item the sale it answers
    /// offered, before the ledger and the host hear it (inferred: TAKP's
    /// echo price comes from padding). An echo answering no sale the
    /// session remembers adds nothing, so the purse never rises past what
    /// the server added.
    fn explain(&mut self, message: &mut Message, _world: &World) {
        let Message::Event(WorldEvent::Merchant(update)) = message else {
            return;
        };
        if let MerchantUpdate::Sold {
            slot,
            quantity,
            price,
        } = update
        {
            let base = self
                .trades
                .offered(InventorySlot(*slot), *quantity)
                .and_then(|item| item.details.price);
            if let Some(added) =
                base.and_then(|base| self.merchant.sale_price(self.quotes, base, *quantity))
            {
                *price = added;
            }
        }
        self.merchant.explain(update, self.quotes);
    }

    /// Item updates, the inventory change a sale's echo stands for, and what
    /// the news says about the coins; the host hears of item updates and
    /// money updates with the rest of the zone's news.
    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Some(purse) = coins_news(message, &mut world.coins.0) {
            tell_coins(world, purse, out)?;
        }
        match message {
            Message::Event(WorldEvent::Inventory(update)) => {
                world.inventory.0.apply(update.clone());
            }
            // A whole list: the places gone, then each item, told as the
            // merchant news a list sent item by item would be.
            Message::MerchantList(items) => {
                let mut items = items.clone();
                // Without an open window the prices are not known.
                let Some(gone) = self.merchant.replace(&mut items, self.quotes) else {
                    return out
                        .log
                        .diagnostic("Merchant list with no merchant window open dropped".into());
                };
                for slot in gone {
                    out.log.send(ClientEvent::World(WorldEvent::Merchant(
                        MerchantUpdate::Removed { slot },
                    )))?;
                }
                for listed in items {
                    out.log.send(ClientEvent::World(WorldEvent::Merchant(
                        MerchantUpdate::Item(Box::new(listed)),
                    )))?;
                }
            }
            Message::Unreadable {
                part: Part::Inventory,
                ..
            } => change(InventoryUpdate::Invalidated, world, out)?,
            // A sale's echo is the only notice that the item left, and an echo
            // of nothing bought the only one that a purchase was refused.
            Message::Event(WorldEvent::Merchant(update)) => {
                if self.trades.refused(update) {
                    merchant_refused(world, "The merchant did not accept that offer.", out)?;
                } else if let Some(update) = self.trades.observe(update, &world.inventory) {
                    change(update, world, out)?;
                }
            }
            Message::Event(WorldEvent::Death(death)) if world.is_player(death.spawn_id) => {
                self.trades.clear();
                self.merchant.clear();
            }
            Message::Event(WorldEvent::Nourishment(nourishment)) if self.allows.meals => {
                self.meals.nourished(*nourishment, world, out)?;
            }
            // The window closed: what the trade slots held was handed over, or
            // comes back as item updates.
            Message::Event(WorldEvent::Exchange(
                ExchangeUpdate::Finished | ExchangeUpdate::Cancelled { .. },
            )) => change(InventoryUpdate::TradeEmptied, world, out)?,
            _ => (),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{actions::Held, feature::testing};
    use super::*;
    use eq_network_game::{
        food::Shortage,
        inventory::{InventorySlot, MoveQuantity, MOVE_OPCODE},
        merchant::MerchantUpdate,
    };
    use testing::item;

    /// An inventory with one item moved to the cursor by an unanswered prediction.
    fn predicted() -> Inventory {
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item(22)]));
        inventory.apply(InventoryUpdate::Prediction(vec![item(30)]));
        inventory
    }

    /// Admitted player 7, carrying one item in slot 22.
    /// Merchant 9 open at a rate of 1, listing place 3 for 25 copper, with
    /// a platinum in the purse.
    fn shopping_at(belongings: &mut Belongings, world: &mut World) {
        let purse = Message::Event(WorldEvent::Coins(Coins {
            platinum: 1,
            ..Coins::default()
        }));
        testing::run(|out| belongings.observe(&purse, world, out))
            .result
            .unwrap();
        for mut news in [
            MerchantUpdate::Opened {
                merchant_id: 9,
                accepted: true,
                rate: 1.0,
            },
            MerchantUpdate::Item(Box::new(eq_network_game::merchant::MerchantItem {
                slot: 3,
                price: 25,
                quantity: 0,
                item: item(3),
            })),
        ]
        .map(|update| Message::Event(WorldEvent::Merchant(update)))
        {
            belongings.explain(&mut news, world);
        }
    }

    fn admitted() -> (Belongings, World) {
        let mut belongings = Belongings::new(eq_network_game::food::AutoEat::default());
        let mut world = World::new(5);
        let snapshot = Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(vec![
            item(22),
        ])));
        belongings.admit(&snapshot, &mut world).unwrap();
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        testing::run(|out| belongings.admitted(&mut world, out))
            .result
            .unwrap();
        (belongings, world)
    }

    /// Admitted player 7, carrying a ration (22), a bag (23) holding a
    /// drink, and the profile's word on how fed and watered they are.
    fn fed(food: u32, water: u32) -> (Belongings, World) {
        use eq_network_game::food::Nourishment;
        let mut belongings = Belongings::new(eq_network_game::food::AutoEat::default());
        let mut world = World::new(5);
        let typed = |slot, item_type| {
            let mut item = item(slot);
            item.rules.item_type = item_type;
            item
        };
        let mut bag = item(23);
        bag.bag_slots = 4;
        let drink = InventorySlot(23).child(0).unwrap().0;
        let snapshot = Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(vec![
            typed(22, 14),
            bag,
            typed(drink, 15),
            typed(24, 11),
        ])));
        belongings.admit(&snapshot, &mut world).unwrap();
        belongings
            .admit(
                &Message::Event(WorldEvent::Nourishment(Nourishment { food, water })),
                &mut world,
            )
            .unwrap();
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        testing::run(|out| belongings.admitted(&mut world, out))
            .result
            .unwrap();
        (belongings, world)
    }

    #[test]
    fn a_hungry_or_thirsty_player_eats_and_drinks_what_they_carry() {
        use eq_network_game::food::{consume, Meal, Nourishment};
        let (mut belongings, mut world) = fed(6000, 6000);
        let hungry =
            |food, water| Message::Event(WorldEvent::Nourishment(Nourishment { food, water }));
        // Fed and watered: nothing happens.
        let outcome = testing::run(|out| belongings.observe(&hungry(3001, 6000), &mut world, out));
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        // Both low: the ration, then the drink in the bag, each one taken.
        let drink = InventorySlot(23).child(0).unwrap();
        let outcome = testing::run(|out| belongings.observe(&hungry(3000, 2000), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [
                consume(InventorySlot(22), Meal::Food, false),
                consume(drink, Meal::Drink, false)
            ]
        );
        assert_eq!(
            inventory_events(&outcome.events),
            [
                &InventoryUpdate::Deduct {
                    slot: InventorySlot(22),
                    quantity: 1
                },
                &InventoryUpdate::Deduct {
                    slot: drink,
                    quantity: 1
                }
            ]
        );
        // The answer to the bite of food still counts the drink short, and
        // the answer to the drink comes next: neither is eaten twice.
        for answer in [hungry(4500, 2000), hungry(4500, 3500)] {
            let outcome = testing::run(|out| belongings.observe(&answer, &mut world, out));
            outcome.result.unwrap();
            assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        }
        // With the ration gone, a hungry player hears there is nothing to eat.
        let outcome = testing::run(|out| belongings.observe(&hungry(100, 6000), &mut world, out));
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::NothingToEat {
                food: Some(Shortage::Nothing),
                water: None
            })]
        ));
    }

    #[test]
    fn food_with_modifiers_waits_for_the_player_unless_anything_goes() {
        use eq_network_game::food::{consume, AutoEat, Meal, Nourishment};
        let (mut belongings, mut world) = fed(6000, 6000);
        let mut ration = item(22);
        ration.rules.item_type = 14;
        ration.details.stats.push(eq_network_game::items::ItemStat {
            label: "STR".into(),
            value: 1,
        });
        world.inventory.0.apply(InventoryUpdate::Set(vec![ration]));
        let hungry = Message::Event(WorldEvent::Nourishment(Nourishment {
            food: 2000,
            water: 6000,
        }));
        let outcome = testing::run(|out| belongings.observe(&hungry, &mut world, out));
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::NothingToEat {
                food: Some(Shortage::OnlyModified),
                water: None
            })]
        ));
        // The host lets anything go.
        let anything = ClientCommand::AutoEat {
            session_id: 5,
            auto_eat: AutoEat::Anything,
        };
        assert!(belongings.owns(&anything));
        testing::run(|out| belongings.handle(&anything, &mut world, out))
            .result
            .unwrap();
        let outcome = testing::run(|out| belongings.observe(&hungry, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [consume(InventorySlot(22), Meal::Food, false)]
        );
    }

    #[test]
    fn a_bite_by_hand_counts_and_a_full_player_is_told_so() {
        use eq_network_game::food::{consume, Meal};
        let eat = |slot| ClientCommand::Consume {
            session_id: 5,
            slot: InventorySlot(slot),
            created: Instant::now(),
        };
        let refusals = |events: &[ClientEvent]| -> Vec<String> {
            events
                .iter()
                .filter_map(|event| match event {
                    ClientEvent::World(WorldEvent::ConsumeRefused { reason, .. }) => {
                        Some(reason.clone())
                    }
                    _ => None,
                })
                .collect()
        };
        let (mut belongings, mut world) = fed(6000, 4000);
        let outcome = testing::run(|out| belongings.handle(&eat(22), &mut world, out));
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert_eq!(refusals(&outcome.events), ["You are too full to eat more"]);
        let outcome = testing::run(|out| belongings.handle(&eat(24), &mut world, out));
        assert_eq!(refusals(&outcome.events), ["You cannot eat or drink that"]);
        let drink = InventorySlot(23).child(0).unwrap();
        let outcome = testing::run(|out| belongings.handle(&eat(drink.0), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [consume(drink, Meal::Drink, true)]);
        assert!(!world.inventory.items().contains_key(&drink));
    }

    fn inventory_events(events: &[ClientEvent]) -> Vec<&InventoryUpdate> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::Inventory(update)) => Some(update),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn moves_settle_once_the_last_one_goes_unrefused_for_the_window() {
        let start = Instant::now();
        let mut inventory = predicted();
        let mut settlement = Settlement::default();
        assert_eq!(settlement.due(&inventory, start), None);
        settlement.sent(start);
        settlement.sent(start + Duration::from_secs(1));
        assert_eq!(settlement.due(&inventory, start + REFUSAL_WINDOW), None);
        let due = start + Duration::from_secs(1) + REFUSAL_WINDOW;
        assert_eq!(
            settlement.due(&inventory, due),
            Some(InventoryUpdate::Settled)
        );
        assert_eq!(settlement.due(&inventory, due), None);
        // Nothing left to settle: moves the server already answered stay quiet.
        settlement.sent(start);
        inventory.apply(InventoryUpdate::Settled);
        assert_eq!(settlement.due(&inventory, due), None);
    }

    #[test]
    fn the_admission_reports_the_inventory_staged_before_it() {
        let (_, world) = admitted();
        assert!(world.inventory.items().contains_key(&InventorySlot(22)));
        let mut belongings = Belongings::new(eq_network_game::food::AutoEat::default());
        let mut world = World::new(5);
        let snapshot = Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(vec![
            item(22),
        ])));
        belongings.admit(&snapshot, &mut world).unwrap();
        let outcome = testing::run(|out| belongings.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(
            !inventory_events(&outcome.events).is_empty(),
            "admission should report the inventory"
        );
        assert!(world.inventory.received());
    }

    /// Admitted player 7 on TAKP, carrying a ration (22) and another item
    /// (23).
    fn admitted_on_takp() -> (Belongings, World) {
        let mut belongings = Belongings::under(
            MoveRules::Takp,
            Quotes::BeforeRate,
            Allowances::ALL,
            eq_network_game::food::AutoEat::default(),
        );
        let mut world = World::new(5);
        let mut ration = item(22);
        ration.rules.item_type = 14;
        let snapshot = Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(vec![
            ration,
            item(23),
        ])));
        belongings.admit(&snapshot, &mut world).unwrap();
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        testing::run_on(&super::super::wire::EqMac, |out| {
            belongings.admitted(&mut world, out)
        })
        .result
        .unwrap();
        (belongings, world)
    }

    #[test]
    fn on_takp_a_merchant_is_told_at_its_charge_and_a_purchase_waits_for_coins_to_cover_it() {
        let eqmac = &super::super::wire::EqMac;
        let (mut belongings, mut world) = admitted_on_takp();
        let purse = |copper| Message::Event(WorldEvent::Coins(Coins::from_copper(copper)));
        testing::run(|out| belongings.observe(&purse(253), &mut world, out))
            .result
            .unwrap();
        // TAKP opens at its rate and lists 100 copper before it: told as 125.
        let mut opened = Message::Event(WorldEvent::Merchant(MerchantUpdate::Opened {
            merchant_id: 9,
            accepted: true,
            rate: 1.25,
        }));
        belongings.explain(&mut opened, &world);
        let listed = |slot| eq_network_game::merchant::MerchantItem {
            slot,
            price: 100,
            quantity: 0,
            item: item(3),
        };
        let list =
            |slots: &[u32]| Message::MerchantList(slots.iter().map(|slot| listed(*slot)).collect());
        let outcome = testing::run_on(eqmac, |out| {
            belongings.observe(&list(&[3, 4]), &mut world, out)
        });
        outcome.result.unwrap();
        assert!(matches!(
            &outcome.events[..],
            [
                ClientEvent::World(WorldEvent::Merchant(MerchantUpdate::Item(first))),
                ClientEvent::World(WorldEvent::Merchant(MerchantUpdate::Item(second))),
            ] if (first.slot, first.price, second.slot, second.price) == (3, 125, 4, 125)
        ));
        // A new whole list without place 4 says it is gone.
        let outcome = testing::run_on(eqmac, |out| {
            belongings.observe(&list(&[3]), &mut world, out)
        });
        outcome.result.unwrap();
        assert!(matches!(
            &outcome.events[..],
            [
                ClientEvent::World(WorldEvent::Merchant(MerchantUpdate::Removed { slot: 4 })),
                ClientEvent::World(WorldEvent::Merchant(MerchantUpdate::Item(item))),
            ] if item.slot == 3
        ));
        // Two may cost up to 254: refused before anything goes out.
        let buy = ClientCommand::Buy {
            session_id: 5,
            merchant_id: 9,
            own_id: 7,
            slot: 3,
            quantity: 2,
            created: Instant::now(),
        };
        let outcome = testing::run_on(eqmac, |out| belongings.handle(&buy, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert!(matches!(
            outcome.events[..],
            [
                ClientEvent::World(WorldEvent::MerchantRefused { .. }),
                ClientEvent::Diagnostic(_)
            ]
        ));
        // With a copper more it goes out, in TAKP's packet, and holds.
        testing::run(|out| belongings.observe(&purse(254), &mut world, out))
            .result
            .unwrap();
        let outcome = testing::run_on(eqmac, |out| belongings.handle(&buy, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, 0x3540);
        assert!(belongings.trades.active());
        // TAKP refuses it with an echo of nothing bought: refused at once.
        let nothing = Message::Event(WorldEvent::Merchant(MerchantUpdate::Bought {
            slot: 0,
            quantity: 0,
            price: 0,
        }));
        let outcome = testing::run_on(eqmac, |out| belongings.observe(&nothing, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            outcome.events[..],
            [
                ClientEvent::World(WorldEvent::MerchantRefused { .. }),
                ClientEvent::Diagnostic(_)
            ]
        ));
        assert!(!belongings.trades.active());
        assert_eq!(world.coins.purse, Some(Coins::from_copper(254)));
    }

    #[test]
    fn on_takp_a_sale_adds_what_takp_added_though_its_echo_prices_nothing() {
        let (mut belongings, mut world) = admitted_on_takp();
        let purse = Message::Event(WorldEvent::Coins(Coins::default()));
        testing::run(|out| belongings.observe(&purse, &mut world, out))
            .result
            .unwrap();
        let mut opened = Message::Event(WorldEvent::Merchant(MerchantUpdate::Opened {
            merchant_id: 9,
            accepted: true,
            rate: 1.25,
        }));
        belongings.explain(&mut opened, &world);
        // Item 23, worth 100 copper: TAKP adds 100 / 1.25 + 0.5, cut, for one.
        let worth = |price| {
            let mut worth = item(23);
            worth.details.price = Some(price);
            Message::Event(WorldEvent::Inventory(InventoryUpdate::Set(vec![worth])))
        };
        testing::run(|out| belongings.observe(&worth(100), &mut world, out))
            .result
            .unwrap();
        let sell = ClientCommand::Sell {
            session_id: 5,
            merchant_id: 9,
            slot: 23,
            quantity: 1,
            created: Instant::now(),
        };
        let echo = |slot| {
            Message::Event(WorldEvent::Merchant(MerchantUpdate::Sold {
                slot,
                quantity: 1,
                price: 0,
            }))
        };
        // An echo answering no sale adds nothing.
        let mut stray = echo(23);
        belongings.explain(&mut stray, &world);
        assert!(matches!(
            stray,
            Message::Event(WorldEvent::Merchant(MerchantUpdate::Sold { price: 0, .. }))
        ));
        let eqmac = &super::super::wire::EqMac;
        testing::run_on(eqmac, |out| belongings.handle(&sell, &mut world, out))
            .result
            .unwrap();
        // Released unanswered, with a pricier item put in its place: the
        // late echo is priced from the item the sale offered.
        let later = Instant::now() + Duration::from_secs(3);
        testing::run(|out| belongings.tick(later, &mut world, out))
            .result
            .unwrap();
        testing::run(|out| belongings.observe(&worth(1000), &mut world, out))
            .result
            .unwrap();
        let mut sold = echo(23);
        belongings.explain(&mut sold, &world);
        assert!(matches!(
            sold,
            Message::Event(WorldEvent::Merchant(MerchantUpdate::Sold { price: 80, .. }))
        ));
        testing::run(|out| belongings.observe(&sold, &mut world, out))
            .result
            .unwrap();
        assert_eq!(world.coins.purse, Some(Coins::from_copper(80)));
    }

    #[test]
    fn a_list_with_no_window_open_is_dropped() {
        let (mut belongings, mut world) = admitted_on_takp();
        let list = Message::MerchantList(vec![eq_network_game::merchant::MerchantItem {
            slot: 0,
            price: 100,
            quantity: 0,
            item: item(3),
        }]);
        let outcome = testing::run(|out| belongings.observe(&list, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(outcome.events[..], [ClientEvent::Diagnostic(_)]));
    }

    #[test]
    fn a_sale_adds_its_price_to_the_purse_and_no_drop_is_never_offered() {
        let (mut belongings, mut world) = admitted();
        shopping_at(&mut belongings, &mut world);
        let sold = Message::Event(WorldEvent::Merchant(MerchantUpdate::Sold {
            slot: 22,
            quantity: 1,
            price: 1234,
        }));
        let outcome = testing::run(|out| belongings.observe(&sold, &mut world, out));
        outcome.result.unwrap();
        // A platinum, and 1 platinum 2 gold 3 silver 4 copper from the sale.
        assert_eq!(
            world.coins.purse,
            Some(Coins {
                platinum: 2,
                gold: 2,
                silver: 3,
                copper: 4
            })
        );
        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::Coins(_)))));
        // A NO DROP item is refused before anything goes out.
        let mut bound = item(23);
        bound.details.flags.push("NO DROP".into());
        let snapshot = Message::Event(WorldEvent::Inventory(InventoryUpdate::Set(vec![bound])));
        testing::run(|out| belongings.observe(&snapshot, &mut world, out))
            .result
            .unwrap();
        let sell = ClientCommand::Sell {
            session_id: 5,
            merchant_id: 9,
            slot: 23,
            quantity: 1,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| belongings.handle(&sell, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert!(matches!(
            outcome.events[..],
            [
                ClientEvent::World(WorldEvent::MerchantRefused { .. }),
                ClientEvent::Diagnostic(_)
            ]
        ));
        assert!(!belongings.trades.active());
    }

    #[test]
    fn on_takp_items_move_one_at_a_time_beside_meals_and_merchants() {
        use eq_network_game::food::AutoEat;
        let (mut belongings, mut world) = admitted_on_takp();
        assert_eq!(
            belongings.capabilities(),
            [
                crate::world::Capability::Inventory,
                crate::world::Capability::Trading
            ]
        );
        for command in [
            ClientCommand::AutoEat {
                session_id: 5,
                auto_eat: AutoEat::Anything,
            },
            ClientCommand::Consume {
                session_id: 5,
                slot: InventorySlot(22),
                created: Instant::now(),
            },
            ClientCommand::Sell {
                session_id: 5,
                merchant_id: 9,
                slot: 22,
                quantity: 1,
                created: Instant::now(),
            },
        ] {
            assert!(belongings.owns(&command), "{command:?}");
        }
        let request = |world: &World, from: i32, to: i32| {
            ClientCommand::MoveInventory(InventoryMove {
                session_id: 5,
                revision: world.inventory.revision(),
                from: InventorySlot(from),
                to: InventorySlot(to),
                quantity: MoveQuantity::Whole,
                created: Instant::now(),
            })
        };
        let pick_up = request(&world, 23, 30);
        assert!(belongings.owns(&pick_up));
        let outcome = testing::run_on(&super::super::wire::EqMac, |out| {
            belongings.handle(&pick_up, &mut world, out)
        });
        outcome.result.unwrap();
        // In EQMac's numbers, where the cursor is 0.
        assert_eq!(
            outcome.sent,
            [eq_network_game::inventory::eqmac_move(
                InventorySlot(23),
                InventorySlot::CURSOR,
                MoveQuantity::Whole
            )
            .unwrap()]
        );
        // The next move waits for TAKP to settle this one.
        let put_down = request(&world, 30, 24);
        let held = Held::new(belongings.holds(&world, Instant::now()));
        assert_eq!(
            held.conflict(&put_down),
            Some("Wait for the server to settle the last move")
        );
        let later = Instant::now() + REFUSAL_WINDOW;
        assert_eq!(
            Held::new(belongings.holds(&world, later)).conflict(&put_down),
            None
        );
    }

    #[test]
    fn on_takp_the_session_eats_below_3000_in_takps_numbers() {
        use eq_network_game::food::{eqmac_consume, Meal, Nourishment};
        let (mut belongings, mut world) = admitted_on_takp();
        let report =
            |food| Message::Event(WorldEvent::Nourishment(Nourishment { food, water: 6000 }));
        // TAKP's own client waits until below 3000.
        let outcome = testing::run_on(&super::super::wire::EqMac, |out| {
            belongings.observe(&report(3000), &mut world, out)
        });
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        let outcome = testing::run_on(&super::super::wire::EqMac, |out| {
            belongings.observe(&report(2999), &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [eqmac_consume(InventorySlot(22), Meal::Food, false).unwrap()]
        );
    }

    #[test]
    fn a_move_is_predicted_when_sent_and_settles_when_the_server_lets_it_stand() {
        let (mut belongings, mut world) = admitted();
        let request = ClientCommand::MoveInventory(InventoryMove {
            session_id: 5,
            revision: world.inventory.revision(),
            from: InventorySlot(22),
            to: InventorySlot(30),
            quantity: MoveQuantity::Whole,
            created: Instant::now(),
        });
        let outcome = testing::run(|out| belongings.handle(&request, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, MOVE_OPCODE);
        assert!(matches!(
            outcome.events[..],
            [
                ClientEvent::World(WorldEvent::Inventory(InventoryUpdate::Prediction(_))),
                ClientEvent::World(WorldEvent::InventoryAction { error: None, .. })
            ]
        ));
        assert!(world.inventory.predicted());
        let later = Instant::now() + REFUSAL_WINDOW;
        let outcome = testing::run(|out| belongings.tick(later, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            inventory_events(&outcome.events),
            [&InventoryUpdate::Settled]
        );
        assert!(!world.inventory.predicted());
    }

    #[test]
    fn a_sale_holds_the_inventory_until_the_merchant_echoes_it() {
        let (mut belongings, mut world) = admitted();
        let sell = ClientCommand::Sell {
            session_id: 5,
            merchant_id: 9,
            slot: 22,
            quantity: 1,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| belongings.handle(&sell, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        let held = Held::new(belongings.holds(&world, Instant::now()));
        assert_eq!(
            held.conflict(&sell),
            Some("Wait for the merchant to answer")
        );
        let cast = ClientCommand::CastSpell {
            session_id: 5,
            gem: 0,
            spell_id: 202,
            target_id: 7,
            created: Instant::now(),
        };
        assert_eq!(held.conflict(&cast), None);
        // The echo is the only notice that the item left.
        let echo = Message::Event(WorldEvent::Merchant(MerchantUpdate::Sold {
            slot: 22,
            quantity: 1,
            price: 40,
        }));
        let outcome = testing::run(|out| belongings.observe(&echo, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(inventory_events(&outcome.events).len(), 1);
        assert!(!world.inventory.items().contains_key(&InventorySlot(22)));
        assert!(
            belongings.holds(&world, Instant::now()).is_empty(),
            "a hold remains: {:?}",
            belongings.holds(&world, Instant::now())
        );
    }

    #[test]
    fn an_unanswered_trade_is_refused_and_releases_the_inventory() {
        let (mut belongings, mut world) = admitted();
        shopping_at(&mut belongings, &mut world);
        let buy = ClientCommand::Buy {
            session_id: 5,
            merchant_id: 9,
            own_id: 7,
            slot: 3,
            quantity: 1,
            created: Instant::now(),
        };
        testing::run(|out| belongings.handle(&buy, &mut world, out))
            .result
            .unwrap();
        let later = Instant::now() + Duration::from_secs(3);
        let outcome = testing::run(|out| belongings.tick(later, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            outcome.events[..],
            [
                ClientEvent::World(WorldEvent::MerchantRefused { session_id: 5, .. }),
                ClientEvent::Diagnostic(_)
            ]
        ));
        assert!(
            belongings.holds(&world, Instant::now()).is_empty(),
            "a hold remains: {:?}",
            belongings.holds(&world, Instant::now())
        );
    }

    #[test]
    fn a_late_sale_echo_never_removes_what_took_the_items_place() {
        let (mut belongings, mut world) = admitted();
        let sell = ClientCommand::Sell {
            session_id: 5,
            merchant_id: 9,
            slot: 22,
            quantity: 1,
            created: Instant::now(),
        };
        testing::run(|out| belongings.handle(&sell, &mut world, out))
            .result
            .unwrap();
        let later = Instant::now() + Duration::from_secs(3);
        testing::run(|out| belongings.tick(later, &mut world, out))
            .result
            .unwrap();
        assert_eq!(belongings.holds(&world, later), []);
        // Released, the slot takes another item before the echo comes.
        let mut replacement = item(22);
        replacement.details.id += 1;
        let arrived = Message::Event(WorldEvent::Inventory(InventoryUpdate::Set(vec![
            replacement.clone(),
        ])));
        testing::run(|out| belongings.observe(&arrived, &mut world, out))
            .result
            .unwrap();
        let echo = Message::Event(WorldEvent::Merchant(MerchantUpdate::Sold {
            slot: 22,
            quantity: 1,
            price: 40,
        }));
        let outcome = testing::run(|out| belongings.observe(&echo, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            inventory_events(&outcome.events),
            [&InventoryUpdate::Invalidated]
        );
        assert!(world.inventory.stale());
        assert_eq!(world.inventory.items()[&InventorySlot(22)], replacement);
    }

    #[test]
    fn handing_over_fills_only_an_open_windows_slots_and_its_closing_empties_them() {
        use super::super::exchange::{Exchange, Exchanging, Stage};
        use eq_network_game::exchange::{ExchangeUpdate, Partner};
        let (mut belongings, mut world) = admitted();
        let pick_up = |world: &World, from, to| {
            ClientCommand::MoveInventory(InventoryMove {
                session_id: 5,
                revision: world.inventory.revision(),
                from: InventorySlot(from),
                to: InventorySlot(to),
                quantity: MoveQuantity::Whole,
                created: Instant::now(),
            })
        };
        testing::run(|out| belongings.handle(&pick_up(&world, 22, 30), &mut world, out))
            .result
            .unwrap();
        // No window, no trade slots.
        let outcome =
            testing::run(|out| belongings.handle(&pick_up(&world, 30, 3000), &mut world, out));
        assert!(
            outcome.sent.is_empty(),
            "no trade slots without a window, sent {:?}",
            outcome.sent
        );
        world.exchange = Exchanging::from(Exchange {
            with: 42,
            partner: Partner::Npc,
            stage: Stage::Open,
        });
        let outcome =
            testing::run(|out| belongings.handle(&pick_up(&world, 30, 3000), &mut world, out));
        assert_eq!(outcome.sent.len(), 1);
        assert!(world.inventory.items().contains_key(&InventorySlot(3000)));
        // Closing the window empties the slots; the server sends the item back.
        let close = ClientCommand::CancelTrade { session_id: 5 };
        let outcome = testing::run(|out| belongings.notice(&close, &mut world, out));
        assert_eq!(
            inventory_events(&outcome.events),
            [&InventoryUpdate::TradeEmptied]
        );
        assert!(world.inventory.items().is_empty());
        // So does the exchange going through.
        world.inventory = Carried::from({
            let mut inventory = Inventory::default();
            inventory.apply(InventoryUpdate::Snapshot(vec![item(3001)]));
            inventory
        });
        let finished = Message::Event(WorldEvent::Exchange(ExchangeUpdate::Finished));
        testing::run(|out| belongings.observe(&finished, &mut world, out))
            .result
            .unwrap();
        assert!(world.inventory.items().is_empty());
    }

    /// An admitted player whose purse holds five gold, and a move of two gold.
    fn with_gold() -> (
        Belongings,
        World,
        impl Fn(eq_network_game::money::CoinPlace, eq_network_game::money::CoinPlace) -> ClientCommand,
    ) {
        use eq_network_game::money::{Coin, CoinTransfer};
        let (mut belongings, mut world) = admitted();
        let purse = Message::Event(WorldEvent::Coins(Coins {
            gold: 5,
            ..Coins::default()
        }));
        testing::run(|out| belongings.observe(&purse, &mut world, out))
            .result
            .unwrap();
        let move_coins = |from, to| ClientCommand::MoveCoins {
            session_id: 5,
            transfer: CoinTransfer {
                from,
                to,
                coin: Coin::Gold,
                into: Coin::Gold,
                amount: 2,
            },
            created: Instant::now(),
        };
        (belongings, world, move_coins)
    }

    /// The reasons the host heard for refused coin moves.
    fn coins_refused(events: &[ClientEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::CoinsRefused { reason, .. }) => Some(reason.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn on_takp_coins_move_in_its_own_packet_and_the_bank_waits_for_a_banker() {
        use eq_network_game::{
            money::{Coin, CoinPlace, CoinTransfer, EQMAC_MOVE_OPCODE},
            world::SpawnKind,
        };
        let eqmac = &super::super::wire::EqMac;
        let (mut belongings, mut world) = admitted_on_takp();
        let purse = Message::Event(WorldEvent::Coins(Coins {
            gold: 12,
            ..Coins::default()
        }));
        let elsewhere = Message::Event(WorldEvent::CoinsElsewhere {
            cursor: Coins::default(),
            bank: Coins::default(),
            given: Coins::default(),
            offered: Coins::default(),
        });
        for news in [purse, elsewhere] {
            testing::run(|out| belongings.observe(&news, &mut world, out))
                .result
                .unwrap();
        }
        // 11 gold from the purse into platinum on the cursor: TAKP takes 10
        // and adds 1.
        let move_coins = |from, to| ClientCommand::MoveCoins {
            session_id: 5,
            transfer: CoinTransfer {
                from,
                to,
                coin: Coin::Gold,
                into: Coin::Platinum,
                amount: 11,
            },
            created: Instant::now(),
        };
        let outcome = testing::run_on(eqmac, |out| {
            belongings.handle(
                &move_coins(CoinPlace::Purse, CoinPlace::Cursor),
                &mut world,
                out,
            )
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, EQMAC_MOVE_OPCODE);
        assert_eq!(
            world.coins.purse,
            Some(Coins {
                gold: 2,
                ..Coins::default()
            })
        );
        assert_eq!(world.coins.cursor.platinum, 1);
        // Into the bank only beside a banker, which TAKP's spawns now name.
        let to_bank = ClientCommand::MoveCoins {
            session_id: 5,
            transfer: CoinTransfer {
                from: CoinPlace::Cursor,
                to: CoinPlace::Bank,
                coin: Coin::Platinum,
                into: Coin::Platinum,
                amount: 1,
            },
            created: Instant::now(),
        };
        assert!(belongings.owns(&to_bank));
        let outcome = testing::run_on(eqmac, |out| belongings.handle(&to_bank, &mut world, out));
        assert_eq!(outcome.sent, []);
        assert_eq!(
            coins_refused(&outcome.events),
            ["Stand near a banker to use the bank"]
        );
        let mut banker = testing::spawn(9, SpawnKind::Npc);
        banker.class = Some(40);
        world.spawns.insert(banker);
        let outcome = testing::run_on(eqmac, |out| belongings.handle(&to_bank, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(world.coins.bank.map(|bank| bank.platinum), Some(1));
    }

    #[test]
    fn a_coin_move_is_sent_kept_and_told_and_goes_nowhere_a_place_is_closed() {
        use eq_network_game::money::{CoinPlace, MOVE_OPCODE as COIN_OPCODE};
        let (mut belongings, mut world, move_coins) = with_gold();
        let outcome = testing::run(|out| {
            belongings.handle(
                &move_coins(CoinPlace::Purse, CoinPlace::Cursor),
                &mut world,
                out,
            )
        });
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, COIN_OPCODE);
        // The host hears where the coins went: the purse, then the rest.
        assert!(matches!(
            outcome.events[..],
            [
                ClientEvent::World(WorldEvent::Coins(Coins { gold: 3, .. })),
                ClientEvent::World(WorldEvent::CoinsElsewhere {
                    cursor: Coins { gold: 2, .. },
                    ..
                })
            ]
        ));
        for (place, reason) in [
            (CoinPlace::Trade, "No trade window is open"),
            (CoinPlace::Bank, "Stand near a banker to use the bank"),
        ] {
            let outcome = testing::run(|out| {
                belongings.handle(&move_coins(CoinPlace::Cursor, place), &mut world, out)
            });
            assert!(
                outcome.sent.is_empty(),
                "nothing sent to {place:?}, sent {:?}",
                outcome.sent
            );
            assert_eq!(coins_refused(&outcome.events), [reason]);
        }
    }

    #[test]
    fn coins_go_into_an_open_window_and_never_past_what_a_place_holds() {
        use super::super::exchange::{Exchange, Exchanging, Stage};
        use eq_network_game::{exchange::Partner, money::CoinPlace};
        let (mut belongings, mut world, move_coins) = with_gold();
        world.exchange = Exchanging::from(Exchange {
            with: 42,
            partner: Partner::Npc,
            stage: Stage::Open,
        });
        for (from, to) in [
            (CoinPlace::Purse, CoinPlace::Cursor),
            (CoinPlace::Cursor, CoinPlace::Trade),
        ] {
            let outcome =
                testing::run(|out| belongings.handle(&move_coins(from, to), &mut world, out));
            assert_eq!(outcome.sent.len(), 1, "{from:?} to {to:?}");
        }
        assert_eq!(world.coins.given.gold, 2);
        // Never more than a place holds: servers log that as a hack.
        let mut greedy = move_coins(CoinPlace::Purse, CoinPlace::Cursor);
        if let ClientCommand::MoveCoins { transfer, .. } = &mut greedy {
            transfer.amount = 4;
        }
        let outcome = testing::run(|out| belongings.handle(&greedy, &mut world, out));
        assert!(
            outcome.sent.is_empty(),
            "nothing sent past the purse, sent {:?}",
            outcome.sent
        );
        assert_eq!(
            coins_refused(&outcome.events),
            ["You do not have that many coins there"]
        );
        // The window going through takes the given coins.
        let finished = Message::Event(WorldEvent::Exchange(
            eq_network_game::exchange::ExchangeUpdate::Finished,
        ));
        testing::run(|out| belongings.observe(&finished, &mut world, out))
            .result
            .unwrap();
        assert_eq!(world.coins.given, Coins::default());
    }

    #[test]
    fn another_players_coins_are_told_and_coins_put_in_stay_there() {
        use super::super::exchange::{Exchange, Exchanging, Stage};
        use eq_network_game::{
            exchange::{ExchangeUpdate, Partner},
            money::{Coin, CoinPlace},
        };
        let (mut belongings, mut world, move_coins) = with_gold();
        world.exchange = Exchanging::from(Exchange {
            with: 50,
            partner: Partner::Player,
            stage: Stage::Open,
        });
        // The server reports each addition, which the ledger sums.
        let added = Message::Event(WorldEvent::Exchange(ExchangeUpdate::Coins {
            coin: Coin::Gold,
            amount: 3,
        }));
        for _ in 0..2 {
            testing::run(|out| belongings.observe(&added, &mut world, out))
                .result
                .unwrap();
        }
        let outcome = testing::run(|out| belongings.observe(&added, &mut world, out));
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::CoinsElsewhere {
                offered: Coins { gold: 9, .. },
                ..
            })]
        ));
        // `EQEmu` ignores a move out of the trade, so none is sent.
        for (from, to) in [
            (CoinPlace::Purse, CoinPlace::Cursor),
            (CoinPlace::Cursor, CoinPlace::Trade),
        ] {
            testing::run(|out| belongings.handle(&move_coins(from, to), &mut world, out))
                .result
                .unwrap();
        }
        let outcome = testing::run(|out| {
            belongings.handle(
                &move_coins(CoinPlace::Trade, CoinPlace::Cursor),
                &mut world,
                out,
            )
        });
        assert!(
            outcome.sent.is_empty(),
            "coins stay in the trade, sent {:?}",
            outcome.sent
        );
        assert_eq!(
            coins_refused(&outcome.events),
            ["Coins in a trade stay there until it closes"]
        );
        assert_eq!(world.coins.given.gold, 2);
    }

    #[test]
    fn loot_coins_and_purchases_move_the_purse_until_a_money_update_says_otherwise() {
        use eq_network_game::{loot::LootResponse, merchant::MerchantUpdate};
        let (mut belongings, mut world) = admitted();
        let news = |event| Message::Event(event);
        testing::run(|out| {
            belongings.observe(
                &news(WorldEvent::Coins(Coins {
                    platinum: 1,
                    ..Coins::default()
                })),
                &mut world,
                out,
            )
        })
        .result
        .unwrap();
        let looted = testing::run(|out| {
            belongings.observe(
                &news(WorldEvent::Loot(LootUpdate::Opened {
                    response: LootResponse::Normal,
                    coins: Coins {
                        silver: 3,
                        ..Coins::default()
                    },
                })),
                &mut world,
                out,
            )
        });
        assert!(matches!(
            looted.events[..],
            [
                ClientEvent::World(WorldEvent::Coins(Coins {
                    platinum: 1,
                    silver: 3,
                    ..
                })),
                ClientEvent::World(WorldEvent::CoinsElsewhere { .. })
            ]
        ));
        testing::run(|out| {
            belongings.observe(
                &news(WorldEvent::Merchant(MerchantUpdate::Bought {
                    slot: 1,
                    quantity: 1,
                    price: 25,
                })),
                &mut world,
                out,
            )
        })
        .result
        .unwrap();
        // The price came from the silver, the change back in copper.
        assert_eq!(
            world.coins.purse,
            Some(Coins {
                platinum: 1,
                copper: 5,
                ..Coins::default()
            })
        );
        // The server's word replaces the estimate, and the host hears it as it is.
        let update = testing::run(|out| {
            belongings.observe(&news(WorldEvent::Coins(Coins::default())), &mut world, out)
        });
        assert!(update.events.is_empty());
        assert_eq!(world.coins.purse, Some(Coins::default()));
    }

    #[test]
    fn coins_takp_adds_to_the_purse_are_told_as_the_purse_they_make() {
        let (mut belongings, mut world) = admitted();
        let purse = Message::Event(WorldEvent::Coins(Coins {
            gold: 2,
            ..Coins::default()
        }));
        testing::run(|out| belongings.observe(&purse, &mut world, out))
            .result
            .unwrap();
        let added = Message::PurseAdded(Coins {
            gold: 5,
            ..Coins::default()
        });
        let outcome = testing::run(|out| belongings.observe(&added, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            outcome.events[..],
            [
                ClientEvent::World(WorldEvent::Coins(Coins { gold: 7, .. })),
                ClientEvent::World(WorldEvent::CoinsElsewhere { .. })
            ]
        ));
    }

    #[test]
    fn an_unreadable_item_update_leaves_the_inventory_untrusted() {
        let (mut belongings, mut world) = admitted();
        let unreadable = Message::Unreadable {
            part: Part::Inventory,
            error: "truncated".into(),
        };
        let outcome = testing::run(|out| belongings.observe(&unreadable, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            inventory_events(&outcome.events),
            [&InventoryUpdate::Invalidated]
        );
        assert!(world.inventory.stale());
    }
}
