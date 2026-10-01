//! The player's belongings: what the server says the inventory holds, the
//! player's item moves and the merchant trades that change it. Only this
//! feature changes the inventory that every feature reads.
//!
//! A move is predicted as soon as it is sent and settles once the server has
//! let it stand; a trade holds the inventory until the merchant echoes it.
mod merchant;

use super::{
    actions::Resource,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{Context, Result};
use eq_network_game::{
    command,
    inventory::{banker_in_range, Inventory, InventoryActor, InventoryMove, InventoryUpdate},
    message::{Message, Part},
    world::WorldEvent,
    GameDialect,
};
use merchant::MerchantTrades;
use std::time::{Duration, Instant};

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
    world.inventory.apply(update.clone());
    out.log
        .send(ClientEvent::World(WorldEvent::Inventory(update)))
}

/// The player's belongings and the changes to them in flight.
pub(super) struct Belongings {
    /// Moves waiting to settle.
    settlement: Settlement,
    /// A purchase or sale waiting for the merchant.
    trades: MerchantTrades,
    dialect: GameDialect,
    character: String,
}

impl Belongings {
    pub(super) fn new(dialect: GameDialect, character: &str) -> Self {
        Self {
            settlement: Settlement::default(),
            trades: MerchantTrades::default(),
            dialect,
            character: character.into(),
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
                world
                    .inventory
                    .submit_move(request, world.session_id, actor, now, |packet| {
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
        match command::encode(self.dialect, command, &self.character) {
            Ok(packet) => {
                out.send(&packet)?;
                self.trades.sent(command, Instant::now());
                Ok(())
            }
            Err(error) => out
                .log
                .diagnostic(format!("Rejected invalid outbound client command: {error}")),
        }
    }
}

impl Feature for Belongings {
    /// Item updates before admission build the inventory the admission reports.
    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        if let Some(update) = update(message) {
            world.inventory.apply(update);
        }
        Ok(())
    }

    /// Reports the inventory, replayed from empty so that the host's copy and
    /// this one count the same revisions.
    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let admission = world.inventory.admission_updates();
        world.inventory = Inventory::default();
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

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::MoveInventory(_)
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
            Message::Event(WorldEvent::Inventory(update)) => world.inventory.apply(update.clone()),
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
        inventory::{InventoryItem, InventorySlot, MoveQuantity, MOVE_OPCODE},
        merchant::MerchantUpdate,
    };

    /// A plain item in this slot.
    fn item(slot: i32) -> InventoryItem {
        InventoryItem {
            activation: eq_network_game::inventory::ItemActivation::default(),
            scroll_spell: None,
            rules: eq_network_game::inventory::ItemPlacement::default(),
            slot: InventorySlot(slot),
            icon: 0,
            stack_count: None,
            charges: 0,
            bag_slots: 0,
            details: eq_network_game::items::ItemDetails {
                equipment: None,
                bonuses: None,
                id: 42,
                name: "Synthetic item".into(),
                lore: String::new(),
                weight_tenths: 0,
                slots: 0,
                classes: 0,
                races: 0,
                flags: vec![],
                stats: vec![],
            },
        }
    }

    /// An inventory with one item moved to the cursor by an unanswered prediction.
    fn predicted() -> Inventory {
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item(22)]));
        inventory.apply(InventoryUpdate::Prediction(vec![item(30)]));
        inventory
    }

    /// Admitted player 7, carrying one item in slot 22.
    fn admitted() -> (Belongings, World) {
        let mut belongings = Belongings::new(GameDialect::TitaniumP99, "Tester");
        let mut world = World::new(5);
        let snapshot = Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(vec![
            item(22),
        ])));
        belongings.admit(&snapshot, &mut world).unwrap();
        world.own_spawn = Some(7);
        world.player = Some(testing::player(7));
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
        let mut belongings = Belongings::new(GameDialect::TitaniumP99, "Tester");
        let mut world = World::new(5);
        let snapshot = Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(vec![
            item(22),
        ])));
        belongings.admit(&snapshot, &mut world).unwrap();
        let outcome = testing::run(|out| belongings.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(!inventory_events(&outcome.events).is_empty());
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
        assert!(belongings.holds(&world, Instant::now()).is_empty());
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
        assert!(belongings.holds(&world, Instant::now()).is_empty());
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
