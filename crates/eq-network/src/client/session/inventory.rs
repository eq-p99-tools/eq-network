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
    feature::{Encoder, Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{Context, Result};
use eq_network_game::{
    exchange::ExchangeUpdate,
    inventory::{banker_in_range, Inventory, InventoryActor, InventoryMove, InventoryUpdate},
    loot::{LootResponse, LootUpdate},
    merchant::MerchantUpdate,
    message::{Message, Part},
    money::Wallet,
    request::Request,
    world::Coins,
    world::{SpawnKind, WorldEvent},
};
use merchant::MerchantTrades;
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

/// Follows what a message says about the coins; true when the purse changed
/// without the host hearing it (loot coins, a purchase), false when only
/// coins elsewhere changed, None when nothing did. A money update replaces
/// the purse, and the host hears it as it is.
fn coins_news(message: &Message, wallet: &mut Wallet) -> Option<bool> {
    let Message::Event(event) = message else {
        return None;
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
        WorldEvent::Merchant(MerchantUpdate::Bought { price, .. }) => {
            wallet.pay(u64::from(*price));
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

/// The player's belongings and the changes to them in flight.
pub(super) struct Belongings {
    /// Moves waiting to settle.
    settlement: Settlement,
    /// A purchase or sale waiting for the merchant.
    trades: MerchantTrades,
    /// How fed and watered the player is, which decides when to eat.
    meals: meals::Meals,
    encoder: Encoder,
}

impl Belongings {
    pub(super) fn new(encoder: Encoder, auto_eat: eq_network_game::food::AutoEat) -> Self {
        Self {
            settlement: Settlement::default(),
            trades: MerchantTrades::default(),
            meals: meals::Meals::new(auto_eat),
            encoder,
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
            .and_then(|actor| world.inventory.0.plan_move(request, actor))
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

    /// Buys or sells; the merchant's echo settles the trade.
    fn trade(&mut self, command: &ClientCommand, out: &mut Out<'_, '_>) -> Result<()> {
        if self.encoder.send(command, out)? {
            self.trades.sent(command, Instant::now());
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
    fn shop(&self, command: &ClientCommand, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
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
        self.encoder.send(command, out).map(drop)
    }
}

impl Feature for Belongings {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![Capability::Inventory, Capability::Trading]
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

    /// A sold item leaves only when the merchant echoes the sale.
    fn holds(&self, _world: &World, _now: Instant) -> Vec<(Resource, &'static str)> {
        self.trades
            .active()
            .then_some((Resource::Inventory, "Wait for the merchant to answer"))
            .into_iter()
            .collect()
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
        matches!(
            command,
            ClientCommand::MoveInventory(_)
                | ClientCommand::MoveCoins { .. }
                | ClientCommand::Shop { .. }
                | ClientCommand::Buy { .. }
                | ClientCommand::Sell { .. }
                | ClientCommand::Consume { .. }
                | ClientCommand::AutoEat { .. }
        )
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
            ClientCommand::Shop { .. } => self.shop(command, world, out),
            ClientCommand::AutoEat { auto_eat, .. } => {
                self.meals.choose(*auto_eat);
                Ok(())
            }
            ClientCommand::Consume {
                session_id, slot, ..
            } => match self.meals.by_hand(*slot, world, out)? {
                Some(reason) => out.log.send(ClientEvent::World(WorldEvent::ConsumeRefused {
                    session_id: *session_id,
                    reason: reason.into(),
                })),
                None => Ok(()),
            },
            _ => self.trade(command, out),
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
            out.log
                .send(ClientEvent::World(WorldEvent::MerchantRefused {
                    session_id: world.session_id,
                    reason: "The merchant did not accept that offer.".into(),
                }))?;
            out.log
                .diagnostic("Merchant trade was not answered; released the inventory".into())?;
        }
        Ok(())
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
            Message::Unreadable {
                part: Part::Inventory,
                ..
            } => change(InventoryUpdate::Invalidated, world, out)?,
            // A sale's echo is the only notice that the item left.
            Message::Event(WorldEvent::Merchant(update)) => {
                if let Some(update) = self.trades.observe(update) {
                    change(update, world, out)?;
                }
            }
            Message::Event(WorldEvent::Death(death)) if world.is_player(death.spawn_id) => {
                self.trades.clear();
            }
            Message::Event(WorldEvent::Nourishment(nourishment)) => {
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
    fn admitted() -> (Belongings, World) {
        let mut belongings = Belongings::new(
            Encoder::new("Tester"),
            eq_network_game::food::AutoEat::default(),
        );
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
        let mut belongings = Belongings::new(
            Encoder::new("Tester"),
            eq_network_game::food::AutoEat::default(),
        );
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
        assert_eq!(
            refusals(&outcome.events),
            ["You could not possibly eat any more, you would explode!"]
        );
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
        let mut belongings = Belongings::new(
            Encoder::new("Tester"),
            eq_network_game::food::AutoEat::default(),
        );
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
