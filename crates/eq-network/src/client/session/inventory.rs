//! The player's belongings: what the server says the inventory holds, the
//! player's item moves and the merchant trades that change it. Only this
//! feature changes the inventory that every feature reads.
//!
//! Each move is sent and predicted immediately in submission order. `EQMac`
//! retains uncertainty until authoritative evidence arrives; silence is no ACK. A
//! trade holds the inventory until the merchant echoes it.
//! Items handed over in a give or trade window leave the trade slots when the
//! window closes, which servers do not say item by item.
//!
//! The coins are this feature's too. Servers answer no coin move and say what
//! the purse holds only now and then, so the ledger keeps the coins where the
//! player put them, changes the purse for loot and purchases kind by kind as
//! servers do, and takes each money update as the truth about the purse.
mod meals;
mod merchant;
#[cfg(test)]
mod merchant_outcomes;
mod resync;

use super::{
    actions::Resource,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{Context, Result};
use eq_network_game::{
    exchange::ExchangeUpdate,
    inventory::{
        banker_in_range, Inventory, InventoryActor, InventoryItem, InventoryMove, InventorySlot,
        InventoryUpdate, MoveRules,
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
    // The profile supplies cursor and bank together. Do not turn an absent
    // profile projection into known empty coins for movement or banking.
    let Some(bank) = coins.bank else {
        return Ok(());
    };
    out.log.send(ClientEvent::World(WorldEvent::CoinsElsewhere {
        cursor: coins.cursor,
        bank,
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

/// Drops a stale purse projection without inventing a new balance. Cursor
/// and bank money retain their separate authority.
fn purse_uncertain(world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
    if world.coins.0.purse.take().is_some() {
        out.log
            .send(ClientEvent::World(WorldEvent::PurseUncertain {
                session_id: world.session_id,
            }))?;
    }
    Ok(())
}

/// Follows what a message says about the coins; true when the purse changed
/// without the host hearing it (loot coins, a purchase, a reward's coins,
/// coins TAKP added), false when only coins elsewhere changed, None when
/// nothing did. A money update replaces the purse, and the host hears it as
/// it is.
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
        // A reward's coins, which every server adds to the purse without a
        // money update (inferred: the official client's purse after a
        // reward is unrecorded).
        WorldEvent::Reward(reward) if !reward.coins.is_empty() => {
            wallet.add_to_purse(reward.coins);
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

/// The line telling of an inventory update the zone sent, so that a log
/// shows the server's side of every change; an emptied world container, for
/// one, leaves no other trace.
fn told(update: &InventoryUpdate) -> String {
    let slots = |items: &[InventoryItem]| {
        items
            .iter()
            .map(|item| item.slot.0.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let what = match update {
        InventoryUpdate::DestroyCursor => {
            "a submitted cursor destruction (local prediction)".into()
        }
        InventoryUpdate::Prediction(items) => format!("a prediction for slots {}", slots(items)),
        InventoryUpdate::Consume { slot, charge } => format!(
            "a {} used from slot {}",
            if *charge { "charge" } else { "unit" },
            slot.0
        ),
        InventoryUpdate::Snapshot(items) => format!("the whole inventory, {} items", items.len()),
        InventoryUpdate::Set(items) => format!("slots {} set", slots(items)),
        InventoryUpdate::Cursor(items) => format!("slots {} pushed onto the cursor", slots(items)),
        InventoryUpdate::Remove(slot) => format!("slot {} emptied", slot.0),
        InventoryUpdate::Used(slot) => format!("a unit or charge used from slot {}", slot.0),
        InventoryUpdate::Deduct { slot, quantity } => {
            format!("{quantity} taken from slot {}", slot.0)
        }
        InventoryUpdate::Invalidated => "the inventory needs refreshing".to_owned(),
        InventoryUpdate::Settled => "the moves in flight settled".to_owned(),
        InventoryUpdate::TradeEmptied => "the trade slots emptied".to_owned(),
        InventoryUpdate::WorldEmptied => "the world container emptied".to_owned(),
    };
    format!("Zone sent an inventory update: {what}")
}

/// Changes the inventory and tells the host.
fn change(update: InventoryUpdate, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
    world.inventory.0.apply(update.clone());
    out.log
        .send(ClientEvent::World(WorldEvent::Inventory(update)))
}

/// What a server type lets the player do with their belongings. With no
/// allowances, the feature only follows the server's inventory and coins.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Independent server permissions, not mutually exclusive states"
)]
pub(super) struct Allowances {
    /// Moving items. An observer still keeps authoritative inventory state.
    pub(super) items: bool,
    /// Moving coins between the purse, the cursor, the bank and a trade.
    pub(super) coins: bool,
    /// Buying and selling at merchants.
    pub(super) merchants: bool,
    /// Eating and drinking, by hand and on the session's own.
    pub(super) meals: bool,
}

impl Allowances {
    /// Items, coins, merchants and meals.
    pub(super) const ALL: Self = Self {
        items: true,
        coins: true,
        merchants: true,
        meals: true,
    };
}

/// Fences local projection descendants after a failed submission.
#[derive(Default)]
struct MoveOrder {
    seen: u64,
    accepted: Option<u64>,
}

impl MoveOrder {
    fn check(&mut self, request: &InventoryMove) -> Result<()> {
        let Some(sequence) = request.sequence else {
            return Ok(());
        };
        anyhow::ensure!(sequence.id > self.seen, "Item move was already processed");
        self.seen = sequence.id;
        anyhow::ensure!(
            sequence.after.is_none() || sequence.after == self.accepted,
            "The preceding item move was not submitted; select the item again"
        );
        Ok(())
    }
}

/// The player's belongings and the changes to them in flight.
pub(super) struct Belongings {
    /// What the player may do with them.
    allows: Allowances,
    /// The server type's rules for item moves.
    rules: MoveRules,
    /// Local queue lineage, scoped to this admission's inventory owner.
    moves: MoveOrder,
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
    #[cfg(test)]
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
            MoveRules::Takp | MoveRules::Quarm => eq_network_game::food::TAKP_HUNGRY,
        };
        Self {
            allows,
            rules,
            moves: MoveOrder::default(),
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
        let planned = self.moves.check(request).and_then(|()| {
            actor(world)
                .context("Character equipment data is unavailable")
                .and_then(|actor| world.inventory.0.plan_move_for(self.rules, request, actor))
                .and_then(|update| {
                    let packet = out.encode(&Request::MoveItem {
                        from: request.from,
                        to: request.to,
                        quantity: request.quantity,
                    })?;
                    Ok((update, packet))
                })
        });
        let result = match planned {
            Ok((update, packet)) => {
                // A failed send ends the admission; an uncertain move is
                // never retried.
                out.send(&packet)?;
                world.inventory.0.apply(update.clone());
                self.moves.accepted = request.sequence.map(|sequence| sequence.id);
                Ok(update)
            }
            Err(error) => Err(error),
        };
        let (update, error) = match result {
            Ok(update) => {
                if self.rules.settles_by_silence() {
                    self.settlement.sent(now);
                }
                (Some(update), None)
            }
            Err(error) => (None, Some(error.to_string())),
        };
        out.log
            .send(ClientEvent::World(WorldEvent::InventoryAction {
                session_id: request.session_id,
                revision: request.revision,
                sequence: request.sequence,
                update,
                error,
            }))
    }

    /// Destruction is owned here, once. Transport failure ends admission and
    /// does not commit or retry the prediction. Silence never settles it.
    fn destroy_item(
        &mut self,
        request: &eq_network_game::inventory::InventoryDestroy,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let planned = world
            .inventory
            .plan_destroy(self.rules, request)
            .and_then(|update| Ok((update, out.encode(&Request::DestroyCursor)?)));
        let (update, error) = match planned {
            Ok((update, packet)) => {
                out.send(&packet)?;
                world.inventory.0.apply(update.clone());
                (Some(update), None)
            }
            Err(error) => (None, Some(error.to_string())),
        };
        out.log
            .send(ClientEvent::World(WorldEvent::InventoryAction {
                session_id: request.session_id,
                revision: request.revision,
                sequence: None,
                update,
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
        if let Err(reason) = self.trades.check(command) {
            return merchant_refused(world, reason, out);
        }
        if self.rules == MoveRules::Quarm {
            let inventory = &world.inventory;
            if !inventory.received() || inventory.stale() {
                return merchant_refused(world, "Wait for a current inventory snapshot", out);
            }
            let known = |slot: InventorySlot| {
                !inventory.prediction_origins().any(|(changed, _)| {
                    changed == slot || slot.parent().is_some_and(|(parent, _)| parent == changed)
                })
            };
            match command {
                ClientCommand::Buy { slot, quantity, .. } => {
                    if !self.merchant.quarm_quantity_known(*slot, *quantity) {
                        return merchant_refused(
                            world,
                            "That purchase quantity is not established by the stock",
                            out,
                        );
                    }
                    if inventory.items().contains_key(&InventorySlot::CURSOR)
                        || !known(InventorySlot::CURSOR)
                        || !(22..=29)
                            .map(InventorySlot)
                            .any(|slot| !inventory.items().contains_key(&slot) && known(slot))
                    {
                        return merchant_refused(
                            world,
                            "Keep a known empty cursor and general slot for the purchase",
                            out,
                        );
                    }
                }
                ClientCommand::Sell { slot, quantity, .. } => {
                    let slot = InventorySlot(*slot);
                    let item = inventory.items().get(&slot);
                    let quantity_known = item.is_some_and(|item| {
                        *quantity > 0 && *quantity <= item.stack_count.unwrap_or(1).min(20)
                    });
                    let priced = item
                        .and_then(|item| item.details.price)
                        .and_then(|price| self.merchant.sale_price(self.quotes, price, *quantity))
                        .is_some();
                    if !known(slot)
                        || !(slot.is_carried()
                            || slot.is_equipment()
                            || slot == InventorySlot::CURSOR)
                        || !priced
                        || !quantity_known
                    {
                        return merchant_refused(
                            world,
                            "The sale needs an established carried item and merchant price",
                            out,
                        );
                    }
                }
                _ => (),
            }
        }
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
    /// see will trade, and opens only within the generation's reach where
    /// it is read (TAKP: 3-D, at most 20). Out of reach the program says
    /// nothing of its own and considers the merchant instead; this session
    /// only refuses.
    fn shop(command: &ClientCommand, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        let ClientCommand::Shop {
            merchant_id, open, ..
        } = command
        else {
            return Ok(());
        };
        let Some(merchant) = world
            .spawns
            .visible(*merchant_id)
            .filter(|spawn| spawn.kind == SpawnKind::Npc)
        else {
            return out
                .log
                .diagnostic("Rejected an unavailable merchant".into());
        };
        let reach = out.wire.reaches().merchants.filter(|_| *open);
        let within = reach.is_none_or(|reach| {
            world
                .player_at()
                .is_some_and(|(_, position)| reach.admits(position, merchant.position))
        });
        if !within {
            return out
                .log
                .diagnostic("Rejected a merchant out of reach".into());
        }
        out.command(command).map(drop)
    }
}

impl Feature for Belongings {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        let mut offered = Vec::new();
        if self.allows.items {
            offered.push(Capability::Inventory);
        }
        if self.allows.merchants {
            offered.push(Capability::Trading);
        }
        if self.allows.meals {
            offered.push(Capability::Meals);
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
        self.moves = MoveOrder::default();
        self.trades.clear();
        self.merchant.clear();
        let admission = world.inventory.admission_updates();
        world.inventory = Carried::default();
        for update in admission {
            change(update, world, out)?;
        }
        tell_coins(world, true, out)?;
        if let Some(nourishment) = self.meals.snapshot() {
            out.log
                .send(ClientEvent::World(WorldEvent::Nourishment(nourishment)))?;
        }
        Ok(())
    }

    /// Only a merchant reply holds the inventory. Neither generation's move
    /// routine waits for a previous item move (`EQMac` 0x422b1c, Titanium 0x51a9a8).
    fn holds(&self, _world: &World, _now: Instant) -> Vec<(Resource, &'static str)> {
        if self.trades.active() {
            vec![(Resource::Inventory, "Wait for the merchant to answer")]
        } else {
            Vec::new()
        }
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
            ClientCommand::MoveInventory(_) => self.allows.items,
            ClientCommand::DestroyItem(_) => self.allows.items && self.rules == MoveRules::EqEmu,
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
        if !self.owns(command) {
            return Ok(());
        }
        match command {
            ClientCommand::MoveInventory(request) => self.move_item(request, world, out),
            ClientCommand::DestroyItem(request) => self.destroy_item(request, world, out),
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

    /// Retains the legacy Titanium refusal window. `EQMac` predictions await
    /// server evidence, regardless of how long or how many moves ago they arose.
    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if self.rules.settles_by_silence() {
            if let Some(update) = self.settlement.due(&world.inventory, now) {
                change(update, world, out)?;
            }
        }
        if self.trades.expire(now) {
            purse_uncertain(world, out)?;
            let reason = "The merchant has not answered. The outcome is unknown; wait for the result before trying again.";
            out.log
                .send(ClientEvent::World(WorldEvent::MerchantUnanswered {
                    session_id: world.session_id,
                    reason: reason.into(),
                }))?;
            out.log
                .diagnostic(format!("Merchant trade unanswered: {reason}"))?;
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
        if matches!(message, Message::Event(WorldEvent::Coins(_))) {
            self.trades.coins_replaced();
        }
        let known_coins = match message {
            Message::Event(WorldEvent::Merchant(update)) => {
                let known = self.trades.coins_current(update);
                let coin_bearing = matches!(
                    update,
                    MerchantUpdate::Bought { quantity, price, .. } if *quantity > 0 && *price > 0
                ) || matches!(
                    update,
                    MerchantUpdate::Sold { slot, quantity, price } if *slot >= 0 && *quantity > 0 && *price > 0
                );
                if coin_bearing && !known {
                    purse_uncertain(world, out)?;
                }
                known
            }
            _ => true,
        };
        if known_coins {
            if let Some(purse) = coins_news(message, &mut world.coins.0) {
                tell_coins(world, purse, out)?;
            }
        }
        match message {
            Message::Event(WorldEvent::Inventory(update)) => {
                let resync = !self.rules.settles_by_silence()
                    && resync::contradicts(&world.inventory, update);
                world.inventory.0.apply(update.clone());
                if resync {
                    // EQMac resyncs can send temporary items, incomplete bag
                    // roots and then disconnect ordinary clients. Only a new
                    // snapshot can restore safe action state, never a timer.
                    change(InventoryUpdate::Invalidated, world, out)?;
                }
                out.log.diagnostic(told(update))?;
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
                if matches!(update, MerchantUpdate::Closed) && self.trades.active() {
                    purse_uncertain(world, out)?;
                }
                if self.trades.refused(update) {
                    merchant_refused(world, "The merchant did not accept that offer.", out)?;
                } else if let Some(update) = self.trades.observe(update, &world.inventory) {
                    change(update, world, out)?;
                }
            }
            Message::Event(event) if world.owns_death(event) => {
                self.trades.clear();
                self.merchant.clear();
            }
            Message::Event(WorldEvent::Nourishment(nourishment)) if self.allows.meals => {
                self.meals
                    .nourished(*nourishment, !self.trades.active(), world, out)?;
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

    #[test]
    fn an_unknown_cursor_and_bank_are_not_reported_as_empty_coins() {
        let (_, mut world) = admitted();
        world.coins.0.purse = Some(Coins::default());
        let output = testing::run(|out| tell_coins(&world, true, out));
        output.result.unwrap();
        assert!(matches!(
            output.events[..],
            [ClientEvent::World(WorldEvent::Coins(_))]
        ));
        coins_news(
            &Message::Event(WorldEvent::CoinsElsewhere {
                cursor: Coins::default(),
                bank: Coins::default(),
                given: Coins::default(),
                offered: Coins::default(),
            }),
            &mut world.coins.0,
        );
        let output = testing::run(|out| tell_coins(&world, false, out));
        output.result.unwrap();
        assert!(matches!(
            output.events[..],
            [ClientEvent::World(WorldEvent::CoinsElsewhere { .. })]
        ));
    }

    #[test]
    fn an_observer_keeps_inventory_and_coins_without_sending_actions() {
        use eq_network_game::{command::EncodedCommand, food::Nourishment, world::Capability};
        let (mut belongings, mut world) = fed(6000, 6000);
        belongings.allows = Allowances::default();
        assert_eq!(belongings.capabilities(), Vec::<Capability>::new());
        let coins = Coins {
            copper: 123,
            ..Coins::default()
        };
        belongings
            .admit(&Message::Event(WorldEvent::Coins(coins)), &mut world)
            .unwrap();
        let output = testing::run_on(&super::super::wire::EqMac, |out| -> Result<()> {
            belongings.admitted(&mut world, out)?;
            belongings.observe(
                &Message::Event(WorldEvent::Nourishment(Nourishment {
                    fatigue: None,
                    food: 0,
                    water: 0,
                })),
                &mut world,
                out,
            )?;
            let consume = ClientCommand::Consume {
                session_id: 5,
                slot: InventorySlot(22),
                created: Instant::now(),
            };
            assert!(!belongings.owns(&consume));
            belongings.handle(&consume, &mut world, out)?;
            let move_item = ClientCommand::MoveInventory(InventoryMove {
                sequence: None,
                session_id: 5,
                revision: world.inventory.revision(),
                from: InventorySlot(22),
                to: InventorySlot::CURSOR,
                quantity: MoveQuantity::Whole,
                created: Instant::now(),
            });
            assert!(!belongings.owns(&move_item));
            belongings.handle(&move_item, &mut world, out)?;
            belongings.tick(Instant::now() + Duration::from_secs(60), &mut world, out)
        });
        output.result.unwrap();
        assert_eq!(output.sent, Vec::<EncodedCommand>::new());
        assert_eq!(output.unreliable, Vec::<EncodedCommand>::new());
        assert!(world.inventory.received());
        assert!(!world.inventory.stale());
        assert_eq!(world.inventory.items().len(), 4);
        assert_eq!(world.coins.purse, Some(coins));
        assert!(output.events.iter().any(|event| matches!(event,
            ClientEvent::World(WorldEvent::Inventory(InventoryUpdate::Snapshot(items))) if items.len() == 4)));
        assert!(output.events.iter().any(|event| matches!(event,
            ClientEvent::World(WorldEvent::Coins(reported)) if *reported == coins)));
        let output = testing::run(|out| -> Result<()> {
            belongings.observe(
                &Message::Event(WorldEvent::Inventory(InventoryUpdate::Remove(
                    InventorySlot(22),
                ))),
                &mut world,
                out,
            )?;
            assert_eq!(world.inventory.items().len(), 3);
            belongings.observe(
                &Message::Event(WorldEvent::Inventory(InventoryUpdate::Invalidated)),
                &mut world,
                out,
            )
        });
        output.result.unwrap();
        assert!(world.inventory.stale());
        assert_eq!(output.sent, Vec::<EncodedCommand>::new());
    }

    #[test]
    fn an_observer_distinguishes_an_empty_snapshot_from_no_inventory() {
        let mut belongings = Belongings::under(
            MoveRules::Takp,
            Quotes::of(eq_network_game::GameDialect::EqMac),
            Allowances::default(),
            eq_network_game::food::AutoEat::default(),
        );
        let mut world = World::new(5);
        assert!(!world.inventory.received());
        belongings
            .admit(
                &Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(vec![]))),
                &mut world,
            )
            .unwrap();
        let output = testing::run(|out| belongings.admitted(&mut world, out));
        output.result.unwrap();
        assert!(world.inventory.received());
        assert!(!world.inventory.stale());
        assert!(output.events.iter().any(|event| matches!(event,
            ClientEvent::World(WorldEvent::Inventory(InventoryUpdate::Snapshot(items))) if items.is_empty())));
    }

    #[test]
    fn a_mac_era_merchant_opens_only_within_reach() {
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        let mut merchant = testing::spawn(9, SpawnKind::Npc);
        merchant.position.x = 20.5;
        world.spawns.insert(merchant);
        let shop = |open| ClientCommand::Shop {
            session_id: 5,
            merchant_id: 9,
            own_id: 7,
            open,
            created: Instant::now(),
        };
        let run = |wire, command: &ClientCommand, world: &World| {
            let outcome = testing::run_on(wire, |out| Belongings::shop(command, world, out));
            outcome.result.unwrap();
            outcome.sent.len()
        };
        // TAKP reaches a merchant 20 away in 3-D; Titanium's reach is unread.
        assert_eq!(run(&super::super::wire::EqMac, &shop(true), &world), 0);
        assert_eq!(run(&super::super::wire::Titanium, &shop(true), &world), 1);
        // Closing is never refused.
        assert_eq!(run(&super::super::wire::EqMac, &shop(false), &world), 1);
        let mut merchant = testing::spawn(9, SpawnKind::Npc);
        merchant.position.x = 20.0;
        world.spawns.insert(merchant);
        assert_eq!(run(&super::super::wire::EqMac, &shop(true), &world), 1);
    }

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
    pub(super) fn shopping_at(belongings: &mut Belongings, world: &mut World) {
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

    pub(super) fn admitted() -> (Belongings, World) {
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
                &Message::Event(WorldEvent::Nourishment(Nourishment {
                    fatigue: None,
                    food,
                    water,
                })),
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
        let hungry = |food, water| {
            Message::Event(WorldEvent::Nourishment(Nourishment {
                fatigue: None,
                food,
                water,
            }))
        };
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
            fatigue: None,
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

    pub(super) fn inventory_events(events: &[ClientEvent]) -> Vec<&InventoryUpdate> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(
                    WorldEvent::Inventory(update)
                    | WorldEvent::InventoryAction {
                        update: Some(update),
                        ..
                    },
                ) => Some(update),
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
    fn each_inventory_update_the_zone_sends_is_told_in_one_line() {
        let (mut belongings, mut world) = admitted();
        let mut told = Vec::new();
        for update in [
            InventoryUpdate::Set(vec![item(22), item(23)]),
            InventoryUpdate::WorldEmptied,
        ] {
            let message = Message::Event(WorldEvent::Inventory(update));
            let outcome = testing::run(|out| belongings.observe(&message, &mut world, out));
            outcome.result.unwrap();
            told.extend(outcome.events.into_iter().filter_map(|event| match event {
                ClientEvent::Diagnostic(line) => Some(line),
                _ => None,
            }));
        }
        assert_eq!(
            told,
            [
                "Zone sent an inventory update: slots 22, 23 set",
                "Zone sent an inventory update: the world container emptied",
            ]
        );
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
    fn quarm_merchant_policy_prices_buys_without_inventing_delivery() {
        let setup =
            super::super::servers::Setup::new("Tester", eq_network_game::food::AutoEat::default());
        let mut merchant = super::super::servers::server_type(crate::client::ServerProtocol::Quarm)
            .inventory(&setup)
            .unwrap()
            .feature;
        assert!(merchant
            .capabilities()
            .contains(&crate::world::Capability::Trading));
        let (_, mut world) = admitted_on_takp();
        world.coins.0.purse = Some(Coins::from_copper(1000));
        let mut opened = Message::Event(WorldEvent::Merchant(MerchantUpdate::Opened {
            merchant_id: 9,
            accepted: true,
            rate: 1.25,
        }));
        merchant.explain(&mut opened, &world);
        let stock = Message::MerchantList(vec![eq_network_game::merchant::MerchantItem {
            slot: 3,
            price: 100,
            quantity: 0,
            item: item(3),
        }]);
        let wire = &super::super::wire::EqMac;
        let outcome = testing::run_on(wire, |out| merchant.observe(&stock, &mut world, out));
        outcome.result.unwrap();
        assert!(
            matches!(&outcome.events[..], [ClientEvent::World(WorldEvent::Merchant(MerchantUpdate::Item(item)))] if item.price == 125)
        );
        let before = world.inventory.items().clone();
        let buy = |quantity| ClientCommand::Buy {
            session_id: 5,
            merchant_id: 9,
            own_id: 7,
            slot: 3,
            quantity,
            created: Instant::now(),
        };
        let outcome = testing::run_on(wire, |out| merchant.handle(&buy(2), &mut world, out));
        outcome.result.unwrap();
        assert!(
            outcome.sent.is_empty(),
            "nonstack stock must not accept guessed quantities"
        );
        let outcome = testing::run_on(wire, |out| merchant.handle(&buy(1), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, 0x3540);
        assert_eq!(
            outcome.sent[0].body,
            [9, 0, 7, 0, 3, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(world.inventory.items(), &before);
        assert_eq!(world.coins.purse, Some(Coins::from_copper(1000)));
        let bought = Message::Event(WorldEvent::Merchant(MerchantUpdate::Bought {
            slot: 3,
            quantity: 1,
            price: 125,
        }));
        testing::run_on(wire, |out| merchant.observe(&bought, &mut world, out))
            .result
            .unwrap();
        assert_eq!(world.coins.purse, Some(Coins::from_copper(875)));
        assert_eq!(
            world.inventory.items(),
            &before,
            "a purchase echo is not item delivery"
        );
    }

    #[test]
    fn quarm_purchase_withholds_unknown_capacity_without_coin_or_item_changes() {
        let setup =
            super::super::servers::Setup::new("Tester", eq_network_game::food::AutoEat::default());
        let mut merchant = super::super::servers::server_type(crate::client::ServerProtocol::Quarm)
            .inventory(&setup)
            .unwrap()
            .feature;
        let (_, mut world) = admitted_on_takp();
        world.coins.0.purse = Some(Coins::from_copper(1000));
        let mut opened = Message::Event(WorldEvent::Merchant(MerchantUpdate::Opened {
            merchant_id: 9,
            accepted: true,
            rate: 1.0,
        }));
        merchant.explain(&mut opened, &world);
        let stock = Message::MerchantList(vec![eq_network_game::merchant::MerchantItem {
            slot: 3,
            price: 25,
            quantity: 0,
            item: item(3),
        }]);
        let wire = &super::super::wire::EqMac;
        testing::run_on(wire, |out| merchant.observe(&stock, &mut world, out))
            .result
            .unwrap();
        let buy = ClientCommand::Buy {
            session_id: 5,
            merchant_id: 9,
            own_id: 7,
            slot: 3,
            quantity: 1,
            created: Instant::now(),
        };
        for update in [
            InventoryUpdate::Set(vec![item(30)]),
            InventoryUpdate::Snapshot((22..=29).map(item).collect()),
            InventoryUpdate::Invalidated,
        ] {
            world.inventory.0.apply(update);
            let before = world.inventory.items().clone();
            let outcome = testing::run_on(wire, |out| merchant.handle(&buy, &mut world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent, []);
            assert_eq!(world.inventory.items(), &before);
            assert_eq!(world.coins.purse, Some(Coins::from_copper(1000)));
            assert!(outcome.events.iter().any(|event| matches!(
                event,
                ClientEvent::World(WorldEvent::MerchantRefused { .. })
            )));
        }
    }

    #[test]
    fn quarm_sale_uses_the_server_echo_and_open_rate_for_local_ledger_changes() {
        let setup =
            super::super::servers::Setup::new("Tester", eq_network_game::food::AutoEat::default());
        let mut merchant = super::super::servers::server_type(crate::client::ServerProtocol::Quarm)
            .inventory(&setup)
            .unwrap()
            .feature;
        let (_, mut world) = admitted_on_takp();
        world.coins.0.purse = Some(Coins::from_copper(100));
        let mut item = item(23);
        item.details.price = Some(100);
        world.inventory.0.apply(InventoryUpdate::Set(vec![item]));
        let mut opened = Message::Event(WorldEvent::Merchant(MerchantUpdate::Opened {
            merchant_id: 9,
            accepted: true,
            rate: 1.25,
        }));
        merchant.explain(&mut opened, &world);
        let sell = ClientCommand::Sell {
            session_id: 5,
            merchant_id: 9,
            own_id: 7,
            slot: 23,
            quantity: 1,
            created: Instant::now(),
        };
        let wire = &super::super::wire::EqMac;
        let outcome = testing::run_on(wire, |out| merchant.handle(&sell, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert!(world.inventory.items().contains_key(&InventorySlot(23)));
        assert_eq!(world.coins.purse, Some(Coins::from_copper(100)));
        let mut sold = Message::Event(WorldEvent::Merchant(MerchantUpdate::Sold {
            slot: 23,
            quantity: 1,
            price: 0,
        }));
        merchant.explain(&mut sold, &world);
        testing::run_on(wire, |out| merchant.observe(&sold, &mut world, out))
            .result
            .unwrap();
        assert_eq!(world.coins.purse, Some(Coins::from_copper(180)));
        assert!(!world.inventory.items().contains_key(&InventorySlot(23)));
        assert!(
            merchant.holds(&world, Instant::now()).is_empty(),
            "the trade hold must end"
        );
    }

    #[test]
    fn quarm_zero_quantity_sale_refuses_without_removing_or_invalidating_items() {
        let setup =
            super::super::servers::Setup::new("Tester", eq_network_game::food::AutoEat::default());
        let mut merchant = super::super::servers::server_type(crate::client::ServerProtocol::Quarm)
            .inventory(&setup)
            .unwrap()
            .feature;
        let (_, mut world) = admitted_on_takp();
        world.coins.0.purse = Some(Coins::from_copper(100));
        let mut offered = item(23);
        offered.details.price = Some(100);
        world.inventory.0.apply(InventoryUpdate::Set(vec![offered]));
        let mut opened = Message::Event(WorldEvent::Merchant(MerchantUpdate::Opened {
            merchant_id: 9,
            accepted: true,
            rate: 1.25,
        }));
        merchant.explain(&mut opened, &world);
        let before = world.inventory.items().clone();
        let sell = ClientCommand::Sell {
            session_id: 5,
            merchant_id: 9,
            own_id: 7,
            slot: 23,
            quantity: 1,
            created: Instant::now(),
        };
        let wire = &super::super::wire::EqMac;
        let outcome = testing::run_on(wire, |out| merchant.handle(&sell, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, 0x2740);
        let rejected = Message::Event(WorldEvent::Merchant(MerchantUpdate::Sold {
            slot: 30,
            quantity: 0,
            price: 0,
        }));
        let outcome = testing::run_on(wire, |out| merchant.observe(&rejected, &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::MerchantRefused { .. })
        )));
        assert_eq!(world.inventory.items(), &before);
        assert!(!world.inventory.stale());
        assert_eq!(world.coins.purse, Some(Coins::from_copper(100)));
        assert!(
            merchant.holds(&world, Instant::now()).is_empty(),
            "the trade hold must end"
        );
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
            own_id: 7,
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
        assert_eq!(world.coins.purse, None);
        // The old coin basis expired with the unanswered operation. Only
        // authoritative money news restores a known purse.
        let coins = Message::Event(WorldEvent::Coins(Coins::from_copper(80)));
        testing::run(|out| belongings.observe(&coins, &mut world, out))
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
        let sale = ClientCommand::Sell {
            session_id: 5,
            merchant_id: 9,
            own_id: 7,
            slot: 22,
            quantity: 1,
            created: Instant::now(),
        };
        testing::run(|out| belongings.handle(&sale, &mut world, out))
            .result
            .unwrap();
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
            own_id: 7,
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
                crate::world::Capability::Trading,
                crate::world::Capability::Meals
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
                own_id: 7,
                slot: 22,
                quantity: 1,
                created: Instant::now(),
            },
        ] {
            assert!(belongings.owns(&command), "{command:?}");
        }
        let request = |world: &World, from: i32, to: i32| {
            ClientCommand::MoveInventory(InventoryMove {
                sequence: None,
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
        // The next move goes out immediately, while the earlier one is uncertain.
        let put_down = request(&world, 30, 24);
        let held = Held::new(belongings.holds(&world, Instant::now()));
        assert_eq!(held.conflict(&put_down), None);
        let outcome = testing::run_on(&super::super::wire::EqMac, |out| {
            belongings.handle(&put_down, &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(world.inventory.unresolved_predictions(), 2);
        let later = Instant::now() + REFUSAL_WINDOW;
        assert_eq!(
            Held::new(belongings.holds(&world, later)).conflict(&put_down),
            None
        );
    }

    #[test]
    fn quarm_meals_follow_its_policy_threshold_wire_and_report_barrier() {
        use eq_network_game::food::{eqmac_consume, Meal, Nourishment};
        let setup =
            super::super::servers::Setup::new("Tester", eq_network_game::food::AutoEat::default());
        let mut meals = super::super::servers::server_type(crate::client::ServerProtocol::Quarm)
            .inventory(&setup)
            .unwrap()
            .feature;
        let (_, mut world) = fed(6000, 6000);
        let report = |food| {
            Message::Event(WorldEvent::Nourishment(Nourishment {
                fatigue: Some(0),
                food,
                water: 6000,
            }))
        };
        let wire = &super::super::wire::EqMac;
        let outcome = testing::run_on(wire, |out| meals.observe(&report(3000), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        let outcome = testing::run_on(wire, |out| meals.observe(&report(2999), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [eqmac_consume(InventorySlot(22), Meal::Food, false).unwrap()]
        );
        assert!(!world.inventory.items().contains_key(&InventorySlot(22)));
        let outcome = testing::run_on(wire, |out| meals.observe(&report(2999), &mut world, out));
        outcome.result.unwrap();
        assert!(
            outcome.sent.is_empty(),
            "a report while a bite is pending must not duplicate it"
        );
    }

    #[test]
    fn meals_withhold_stale_moved_food_and_merchant_inventory() {
        let (mut meals, mut world) = fed(1000, 6000);
        let mut projected: Vec<_> = world.inventory.items().values().cloned().collect();
        projected
            .iter_mut()
            .find(|item| item.slot == InventorySlot(22))
            .unwrap()
            .slot = InventorySlot(25);
        world
            .inventory
            .0
            .apply(InventoryUpdate::Prediction(projected));
        let mut owner = meals::Meals::new(eq_network_game::food::AutoEat::default(), 2999);
        let report = eq_network_game::food::Nourishment {
            fatigue: Some(0),
            food: 1000,
            water: 6000,
        };
        let outcome = testing::run(|out| owner.nourished(report, true, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        let mut refused = None;
        testing::run(|out| {
            refused = owner.by_hand(InventorySlot(25), &mut world, out)?;
            Ok::<(), anyhow::Error>(())
        })
        .result
        .unwrap();
        assert!(refused.is_some());
        let (_, mut world) = fed(1000, 6000);
        let outcome = testing::run(|out| owner.nourished(report, false, &mut world, out));
        outcome.result.unwrap();
        assert!(
            outcome.sent.is_empty(),
            "automatic food cannot bypass a merchant hold"
        );
        world.inventory.0.apply(InventoryUpdate::Invalidated);
        let outcome = testing::run(|out| owner.nourished(report, true, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        let command = ClientCommand::Consume {
            session_id: 5,
            slot: InventorySlot(22),
            created: Instant::now(),
        };
        let outcome = testing::run(|out| meals.handle(&command, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::ConsumeRefused { .. }))));
    }

    #[test]
    fn on_takp_the_session_eats_below_3000_in_takps_numbers() {
        use eq_network_game::food::{eqmac_consume, Meal, Nourishment};
        let (mut belongings, mut world) = admitted_on_takp();
        let report = |food| {
            Message::Event(WorldEvent::Nourishment(Nourishment {
                fatigue: None,
                food,
                water: 6000,
            }))
        };
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
            sequence: None,
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
            [ClientEvent::World(WorldEvent::InventoryAction {
                update: Some(InventoryUpdate::Prediction(_)),
                error: None,
                ..
            })]
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
            own_id: 7,
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
    fn an_unanswered_trade_reports_unknown_and_releases_the_inventory() {
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
                ClientEvent::World(WorldEvent::PurseUncertain { session_id: 5 }),
                ClientEvent::World(WorldEvent::MerchantUnanswered { session_id: 5, .. }),
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
            own_id: 7,
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
                sequence: None,
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
        belongings
            .admit(
                &Message::Event(WorldEvent::CoinsElsewhere {
                    cursor: Coins::default(),
                    bank: Coins::default(),
                    given: Coins::default(),
                    offered: Coins::default(),
                }),
                &mut world,
            )
            .unwrap();
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
        shopping_at(&mut belongings, &mut world);
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
            [ClientEvent::World(WorldEvent::Coins(Coins {
                platinum: 1,
                silver: 3,
                ..
            }))]
        ));
        let purchase = ClientCommand::Buy {
            session_id: 5,
            merchant_id: 9,
            own_id: 7,
            slot: 3,
            quantity: 1,
            created: Instant::now(),
        };
        let sent = testing::run(|out| belongings.handle(&purchase, &mut world, out));
        sent.result.unwrap();
        assert_eq!(sent.sent.len(), 1);
        testing::run(|out| {
            belongings.observe(
                &news(WorldEvent::Merchant(MerchantUpdate::Bought {
                    slot: 3,
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
            [ClientEvent::World(WorldEvent::Coins(Coins { gold: 7, .. }))]
        ));
    }

    #[test]
    fn a_rewards_coins_go_into_the_purse_and_are_told() {
        let (mut belongings, mut world) = admitted();
        let purse = Message::Event(WorldEvent::Coins(Coins {
            gold: 2,
            ..Coins::default()
        }));
        testing::run(|out| belongings.observe(&purse, &mut world, out))
            .result
            .unwrap();
        let reward = |coins| {
            Message::Event(WorldEvent::Reward(eq_network_game::rewards::Reward {
                giver_id: Some(581),
                coins,
                experience: 100,
                items: vec![13_005],
            }))
        };
        let given = reward(Coins {
            gold: 5,
            copper: 3,
            ..Coins::default()
        });
        let outcome = testing::run(|out| belongings.observe(&given, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::Coins(Coins {
                gold: 7,
                copper: 3,
                ..
            }))]
        ));
        // A reward with no coins leaves the purse alone.
        let outcome =
            testing::run(|out| belongings.observe(&reward(Coins::default()), &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.events.is_empty());
        assert_eq!(world.coins.purse.unwrap().gold, 7);
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
