//! Handing items to another character: asking an NPC (or a player) to trade,
//! the window their answer opens, and clicking Give or closing it. The trade
//! slots are the inventory's; this feature says whether a window is open and
//! how many of them it has, and only it changes that.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    exchange::{self, ExchangeUpdate, Partner},
    inventory::InventorySlot,
    message::Message,
    world::{SpawnKind, WorldEvent},
};
use std::time::{Duration, Instant};

/// How long the other side has to take a request. An NPC that is fighting
/// never answers: `EQEmu` sends no acknowledgement while it is engaged.
const ANSWER_WINDOW: Duration = Duration::from_secs(3);

/// How far an exchange has come.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Stage {
    /// The player asked at this time, and no answer has come.
    Asked(Instant),
    /// The window is open.
    Open,
    /// The player clicked Give or Trade.
    Accepted,
}

/// A give or trade window, asked for or open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Exchange {
    /// The other side.
    pub(super) with: u16,
    /// Who they are, which decides the window.
    pub(super) partner: Partner,
    /// How far the exchange has come.
    pub(super) stage: Stage,
}

impl Exchange {
    /// How many trade slots the window offers: none until it opens.
    pub(super) const fn trade_slots(&self) -> u8 {
        match self.stage {
            Stage::Asked(_) => 0,
            Stage::Open | Stage::Accepted => self.partner.slots(),
        }
    }
}

/// The exchange under way, if any. Every feature reads it; only this module
/// changes it.
#[derive(Debug, Default)]
pub(super) struct Exchanging(Option<Exchange>);

impl std::ops::Deref for Exchanging {
    type Target = Option<Exchange>;

    fn deref(&self) -> &Option<Exchange> {
        &self.0
    }
}

/// Lets another feature's tests start with a window open.
#[cfg(test)]
impl From<Exchange> for Exchanging {
    fn from(exchange: Exchange) -> Self {
        Self(Some(exchange))
    }
}

/// Asks other characters to trade and carries the window through to Give or
/// its closing.
#[derive(Default)]
pub(super) struct Exchanges;

/// Tells the host why no window opened or why it went away.
fn refused(session_id: u64, reason: &str, out: &mut Out<'_, '_>) -> Result<()> {
    out.log
        .send(ClientEvent::World(WorldEvent::ExchangeRefused {
            session_id,
            reason: reason.into(),
        }))?;
    out.log.diagnostic(format!("Trade refused: {reason}"))
}

impl Exchanges {
    /// Asks a visible character within reach, while the player holds an item
    /// to hand over.
    fn offer(
        with_id: u16,
        session_id: u64,
        world: &mut World,
        out: &mut Out<'_, '_>,
        now: Instant,
    ) -> Result<()> {
        let check = || -> std::result::Result<(u16, Partner), &'static str> {
            if world.exchange.is_some() {
                return Err("Close the open trade first");
            }
            let (own_id, position) = world.player_at().ok_or("Not in the zone yet")?;
            let spawn = world
                .spawns
                .visible(with_id)
                .ok_or("There is nobody there to trade with")?;
            let partner = match spawn.kind {
                SpawnKind::Npc => Partner::Npc,
                SpawnKind::Player => Partner::Player,
                _ => return Err("There is nobody there to trade with"),
            };
            if !exchange::in_reach(position, spawn) {
                return Err("You are too far away to trade");
            }
            if !world.inventory.items().contains_key(&InventorySlot::CURSOR) {
                return Err("Hold an item on the cursor to hand it over");
            }
            Ok((own_id, partner))
        };
        let (own_id, partner) = match check() {
            Ok(checked) => checked,
            Err(reason) => return refused(session_id, reason, out),
        };
        out.send(&exchange::request(own_id, with_id)?)?;
        world.exchange.0 = Some(Exchange {
            with: with_id,
            partner,
            stage: Stage::Asked(now),
        });
        Ok(())
    }

    /// Clicks Give or Trade in the open window.
    fn accept(session_id: u64, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let own_id = world.player.as_ref().map(|player| player.spawn_id);
        let open = world
            .exchange
            .0
            .as_mut()
            .filter(|exchange| exchange.stage == Stage::Open);
        let (Some(exchange), Some(own_id)) = (open, own_id) else {
            return refused(session_id, "No trade window is open", out);
        };
        out.send(&exchange::accept(own_id)?)?;
        exchange.stage = Stage::Accepted;
        Ok(())
    }

    /// Closes the window, or withdraws a request not yet answered; what the
    /// trade slots held comes back from the server.
    fn cancel(world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let own = world.player.as_ref().map(|player| player.spawn_id);
        if let (Some(_), Some(own_id)) = (world.exchange.0.take(), own) {
            out.send(&exchange::cancel(own_id)?)?;
        }
        Ok(())
    }
}

impl Feature for Exchanges {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![Capability::Giving]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::OfferTrade { .. }
                | ClientCommand::AcceptTrade { .. }
                | ClientCommand::CancelTrade { .. }
        )
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match *command {
            ClientCommand::OfferTrade {
                with_id,
                session_id,
                ..
            } => Self::offer(with_id, session_id, world, out, Instant::now()),
            ClientCommand::AcceptTrade { session_id, .. } => Self::accept(session_id, world, out),
            _ => Self::cancel(world, out),
        }
    }

    /// Withdraws a request nobody answered.
    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let unanswered = world.exchange.is_some_and(|exchange| {
            matches!(exchange.stage, Stage::Asked(at) if now.saturating_duration_since(at) >= ANSWER_WINDOW)
        });
        if unanswered {
            world.exchange.0 = None;
            refused(
                world.session_id,
                "Nobody answered the request to trade",
                out,
            )?;
        }
        Ok(())
    }

    /// Follows the other side's answers. A window this player did not ask
    /// for (another player's request) waits for player trades.
    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let Message::Event(event) = message else {
            return Ok(());
        };
        match event {
            WorldEvent::Exchange(ExchangeUpdate::Opened { with }) => {
                if let Some(exchange) = world.exchange.0.as_mut().filter(|exchange| {
                    u32::from(exchange.with) == *with && matches!(exchange.stage, Stage::Asked(_))
                }) {
                    exchange.stage = Stage::Open;
                }
            }
            WorldEvent::Exchange(ExchangeUpdate::Finished | ExchangeUpdate::Cancelled { .. }) => {
                world.exchange.0 = None;
            }
            WorldEvent::Exchange(ExchangeUpdate::Busy { by }) => {
                if world
                    .exchange
                    .is_some_and(|exchange| u32::from(exchange.with) == *by)
                {
                    world.exchange.0 = None;
                    refused(world.session_id, "They are busy", out)?;
                }
            }
            WorldEvent::Exchange(ExchangeUpdate::Requested { from }) => {
                out.log.diagnostic(format!(
                    "Spawn {from} asked to trade; trades between players are not supported yet"
                ))?;
            }
            WorldEvent::Death(death) if world.is_player(death.spawn_id) => {
                world.exchange.0 = None;
            }
            _ => (),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, inventory::Carried};
    use super::*;
    use eq_network_game::{
        inventory::{Inventory, InventoryUpdate},
        world::Position,
    };

    const NPC: u16 = 42;

    /// An admitted player (7) beside an NPC (42) and a corpse (43), holding
    /// an item on the cursor.
    fn beside_npc() -> World {
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        world.own_spawn = Some(7);
        world.spawns.insert(testing::spawn(NPC, SpawnKind::Npc));
        world
            .spawns
            .insert(testing::spawn(43, SpawnKind::NpcCorpse));
        let mut far = testing::spawn(44, SpawnKind::Npc);
        far.position = Position {
            x: exchange::REACH + 1.0,
            ..Position::default()
        };
        world.spawns.insert(far);
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![testing::item(30)]));
        world.inventory = Carried::from(inventory);
        world
    }

    fn offer(with_id: u16) -> ClientCommand {
        ClientCommand::OfferTrade {
            session_id: 5,
            with_id,
            created: Instant::now(),
        }
    }

    fn refusals(events: &[ClientEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::ExchangeRefused { reason, .. }) => {
                    Some(reason.clone())
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn only_a_character_in_reach_is_asked_while_an_item_is_on_the_cursor() {
        let mut exchanges = Exchanges;
        for with_id in [43, 44, 45] {
            let mut world = beside_npc();
            let outcome = testing::run(|out| exchanges.handle(&offer(with_id), &mut world, out));
            outcome.result.unwrap();
            assert!(outcome.sent.is_empty(), "{with_id}");
            assert_eq!(refusals(&outcome.events).len(), 1, "{with_id}");
            assert!(world.exchange.is_none());
        }
        let mut world = beside_npc();
        world.inventory = Carried::default();
        let outcome = testing::run(|out| exchanges.handle(&offer(NPC), &mut world, out));
        assert_eq!(
            refusals(&outcome.events),
            ["Hold an item on the cursor to hand it over"]
        );
        let mut world = beside_npc();
        let outcome = testing::run(|out| exchanges.handle(&offer(NPC), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [exchange::request(7, NPC).unwrap()],
            "the request names the NPC, then the player"
        );
        let exchange = world.exchange.unwrap();
        assert_eq!((exchange.with, exchange.partner), (NPC, Partner::Npc));
        assert_eq!(exchange.trade_slots(), 0, "no slots before the answer");
        // One exchange at a time.
        let outcome = testing::run(|out| exchanges.handle(&offer(NPC), &mut world, out));
        assert_eq!(refusals(&outcome.events), ["Close the open trade first"]);
    }

    #[test]
    fn the_npcs_answer_opens_the_window_and_give_or_closing_ends_it() {
        let mut exchanges = Exchanges;
        let mut world = beside_npc();
        testing::run(|out| exchanges.handle(&offer(NPC), &mut world, out))
            .result
            .unwrap();
        // Give needs an open window.
        let give = ClientCommand::AcceptTrade {
            session_id: 5,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| exchanges.handle(&give, &mut world, out));
        assert!(
            outcome.sent.is_empty(),
            "no Give before the window opens, sent {:?}",
            outcome.sent
        );
        assert_eq!(refusals(&outcome.events), ["No trade window is open"]);
        // Someone else's answer opens nothing.
        let opened = |with| Message::Event(WorldEvent::Exchange(ExchangeUpdate::Opened { with }));
        testing::run(|out| exchanges.observe(&opened(99), &mut world, out))
            .result
            .unwrap();
        assert_eq!(world.exchange.unwrap().trade_slots(), 0);
        testing::run(|out| exchanges.observe(&opened(u32::from(NPC)), &mut world, out))
            .result
            .unwrap();
        assert_eq!(world.exchange.unwrap().trade_slots(), 4);
        let outcome = testing::run(|out| exchanges.handle(&give, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [exchange::accept(7).unwrap()]);
        assert_eq!(world.exchange.unwrap().stage, Stage::Accepted);
        let finished = Message::Event(WorldEvent::Exchange(ExchangeUpdate::Finished));
        testing::run(|out| exchanges.observe(&finished, &mut world, out))
            .result
            .unwrap();
        assert!(world.exchange.is_none());
        // Closing a window sends the cancel; with none open it sends nothing.
        testing::run(|out| exchanges.handle(&offer(NPC), &mut world, out))
            .result
            .unwrap();
        let close = ClientCommand::CancelTrade { session_id: 5 };
        let outcome = testing::run(|out| exchanges.handle(&close, &mut world, out));
        assert_eq!(outcome.sent, [exchange::cancel(7).unwrap()]);
        assert!(world.exchange.is_none());
        let outcome = testing::run(|out| exchanges.handle(&close, &mut world, out));
        assert!(
            outcome.sent.is_empty(),
            "nothing to close, sent {:?}",
            outcome.sent
        );
    }

    #[test]
    fn an_unanswered_request_is_withdrawn() {
        let mut exchanges = Exchanges;
        let mut world = beside_npc();
        testing::run(|out| exchanges.handle(&offer(NPC), &mut world, out))
            .result
            .unwrap();
        let Some(Exchange {
            stage: Stage::Asked(at),
            ..
        }) = *world.exchange
        else {
            panic!("asked");
        };
        let outcome = testing::run(|out| exchanges.tick(at + ANSWER_WINDOW / 2, &mut world, out));
        assert!(outcome.events.is_empty());
        let outcome = testing::run(|out| exchanges.tick(at + ANSWER_WINDOW, &mut world, out));
        assert_eq!(
            refusals(&outcome.events),
            ["Nobody answered the request to trade"]
        );
        assert!(world.exchange.is_none());
    }
}
