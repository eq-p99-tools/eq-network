//! The player's belongings: what the server says the inventory holds, the
//! player's item moves and the merchant trades that change it. Only this
//! feature changes the inventory that every feature reads.
//!
//! A move is predicted as soon as it is sent and settles once the server has
//! let it stand; a trade holds the inventory until the merchant echoes it.
//! Items handed over in a give or trade window leave the trade slots when the
//! window closes, which servers do not say item by item.
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
    message::{Message, Part},
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
    encoder: Encoder,
}

impl Belongings {
    pub(super) fn new(encoder: Encoder) -> Self {
        Self {
            settlement: Settlement::default(),
            trades: MerchantTrades::default(),
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
        let mut transport_failed = false;
        let sink = &mut *out.sink;
        let result = actor(world)
            .context("Character equipment data is unavailable")
            .and_then(|actor| {
                world.inventory.0.submit_move(request, actor, |packet| {
                    let sent = sink.send(packet);
                    transport_failed = sent.is_err();
                    sent
                })
            });
        // A failed send ends the admission; an uncertain move is never retried.
        if transport_failed {
            return result.map(drop);
        }
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

    /// Moves coins. Servers answer no coin move, so nothing waits on one; a
    /// trade window must be open for coins to go into it, and a banker near
    /// for the bank.
    fn move_coins(command: &ClientCommand, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
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
            match transfer.encode() {
                Ok(packet) => return out.send(&packet),
                Err(error) => Some(error.to_string()),
            }
        };
        let reason = refusal.unwrap_or_default();
        out.log.send(ClientEvent::World(WorldEvent::CoinsRefused {
            session_id: *session_id,
            transfer: *transfer,
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

    /// Item updates before admission build the inventory the admission reports.
    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        if let Some(update) = update(message) {
            world.inventory.0.apply(update);
        }
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
        Ok(())
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

    /// Item updates, and the inventory change a sale's echo stands for; the
    /// host hears of item updates with the rest of the zone's news.
    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
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
    use eq_network_game::GameDialect;
    use eq_network_game::{
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
        let mut belongings = Belongings::new(Encoder::new(GameDialect::Titanium, "Tester"));
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
        let mut belongings = Belongings::new(Encoder::new(GameDialect::Titanium, "Tester"));
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

    #[test]
    fn coins_go_into_a_trade_only_while_its_window_is_open_and_into_the_bank_only_near_a_banker() {
        use super::super::exchange::{Exchange, Exchanging, Stage};
        use eq_network_game::{
            exchange::Partner,
            money::{Coin, CoinPlace, CoinTransfer, MOVE_OPCODE as COIN_OPCODE},
        };
        let (mut belongings, mut world) = admitted();
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
        let refused = |events: &[ClientEvent]| {
            events
                .iter()
                .filter_map(|event| match event {
                    ClientEvent::World(WorldEvent::CoinsRefused { reason, .. }) => {
                        Some(reason.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let outcome = testing::run(|out| {
            belongings.handle(
                &move_coins(CoinPlace::Purse, CoinPlace::Cursor),
                &mut world,
                out,
            )
        });
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, COIN_OPCODE);
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
            assert_eq!(refused(&outcome.events), [reason]);
        }
        world.exchange = Exchanging::from(Exchange {
            with: 42,
            partner: Partner::Npc,
            stage: Stage::Open,
        });
        let outcome = testing::run(|out| {
            belongings.handle(
                &move_coins(CoinPlace::Cursor, CoinPlace::Trade),
                &mut world,
                out,
            )
        });
        assert_eq!(outcome.sent.len(), 1);
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
