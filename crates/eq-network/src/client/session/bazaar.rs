//! The Bazaar on stock `EQEmu`: the player's search and welcome request,
//! and the server's answers (6.550), and the player's browse of one
//! trader's wares (6.558), read only. Nothing here buys or turns trader
//! mode on.
//!
//! `EQEmu` searches the traders of the player's own zone and instance
//! (`Bazaar::GetSearchResults`, `Local_Scope`), so a search outside the
//! Bazaar finds nothing; where the official client lets the player search
//! is not read yet (R49-42), so the session sends one only in the Bazaar.
//! One search or welcome request is in flight at a time, and the next waits
//! out a floor after the last was sent (ours, inferred), since each costs
//! the server a database search. An answer nobody asked for is withheld and
//! told as a gap, by its opcode and length alone: its rows list other
//! players' sales.
//!
//! A browse ([`Browsing`]) clicks only a player whose trader sign the
//! session saw go on, or whom a search answer named as a trader, within
//! reach, with no merchant or other browse open, one click at a time. The
//! server does not check that the player clicked is a trader, and records
//! the player as their one customer until the browse ends
//! (`Client::Handle_OP_TraderShop`, `Client::WithCustomer`), so every
//! browse the session closes itself sends its end, and an approval that
//! comes after its click stopped waiting is ended too.
//!
//! The Mac-era search (6.580) is the same feature over TAKP's and Quarm's
//! own records ([`bazaar::eqmac`]): their handler answers each result in a
//! packet of its own, then the end, so the host hears one
//! [`BazaarEvent::Results`] per row before [`BazaarEvent::SearchDone`], and
//! rows past [`bazaar::MOST_ROWS`] are withheld and told as a gap. Both
//! servers check that a search comes from zone 151 (`Zones::BAZAAR`,
//! `Client::Handle_OP_BazaarSearch`), as stock `EQEmu` searches only the
//! player's zone; the session sends one only there on every server.
//!
//! Offered on TAKP and Quarm from their source, not live yet (6.580a).
//! Built, and not yet offered on stock `EQEmu`: it offers each once its
//! local live row passes (6.550a, 6.558a).
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "trader browsing is offered on stock EQEmu once row 6.558a passes"
    )
)]
use super::{
    actions::{self, Resource},
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    bazaar::{self, BazaarEvent},
    merchant::MerchantUpdate,
    message::Message,
    reach::Reach,
    request::Request,
    world::{SpawnKind, WorldEvent},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

/// How long a search or welcome request waits for its answer before
/// another may go (inferred: `EQEmu` always ends a search, so this only
/// frees one whose end was lost).
const WAIT: Duration = Duration::from_secs(10);
/// The least time between one search or welcome request and the next
/// (ours, inferred).
const FLOOR: Duration = Duration::from_secs(2);

/// What the session is waiting to hear.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Asked {
    Search,
    Welcome,
}

/// A generation's search and answer records, by what the session needs
/// of them: the opcode they share, and the lengths it tells gaps by.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Records {
    /// The search's and every answer's opcode.
    opcode: u16,
    /// One result row's length.
    row: usize,
    /// The end's length, which the welcome shares.
    end: usize,
}

/// Titanium's ([`bazaar::search`], [`bazaar::answer`]).
const TITANIUM: Records = Records {
    opcode: bazaar::SEARCH_OPCODE,
    row: bazaar::ROW_LENGTH,
    end: bazaar::WELCOME_LENGTH,
};

/// The Mac era's ([`bazaar::eqmac`]).
const EQMAC: Records = Records {
    opcode: bazaar::eqmac::SEARCH_OPCODE,
    row: bazaar::eqmac::ROW_LENGTH,
    end: bazaar::eqmac::DONE_LENGTH,
};

/// Why a gap the Bazaar tells was not passed on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Unpassed {
    /// An answer nobody asked for.
    Unasked,
    /// A result row past [`bazaar::MOST_ROWS`] in one search.
    PastMost,
}

/// The Bazaar's search and welcome request.
pub(super) struct Bazaar {
    /// The generation's records.
    records: Records,
    /// The request in flight, and when it went.
    asked: Option<(Asked, Instant)>,
    /// The rows the search in flight has passed on.
    rows: usize,
    /// When the last request went.
    sent: Option<Instant>,
    /// The length of each answer not passed on, and why, to tell as gaps.
    unasked: Vec<(usize, Unpassed)>,
}

impl Bazaar {
    /// Over Titanium's records, as stock `EQEmu` speaks them.
    pub(super) const fn titanium() -> Self {
        Self::over(TITANIUM)
    }

    /// Over the Mac era's records, as TAKP and Quarm speak them.
    pub(super) const fn eqmac() -> Self {
        Self::over(EQMAC)
    }

    const fn over(records: Records) -> Self {
        Self {
            records,
            asked: None,
            rows: 0,
            sent: None,
            unasked: Vec::new(),
        }
    }

    fn waiting(&self, now: Instant) -> Option<Asked> {
        self.asked
            .filter(|(_, at)| now.saturating_duration_since(*at) < WAIT)
            .map(|(asked, _)| asked)
    }
}

impl Feature for Bazaar {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::BazaarSearching]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::BazaarSearch { .. } | ClientCommand::BazaarWelcome { .. }
        )
    }

    /// A new admission forgets any request in flight.
    fn admitted(&mut self, _world: &mut World, _out: &mut Out<'_, '_>) -> Result<()> {
        self.asked = None;
        Ok(())
    }

    fn explain(&mut self, message: &mut Message, _world: &World) {
        let Message::Event(WorldEvent::Bazaar(event)) = message else {
            if matches!(
                message,
                Message::ZoneOffer(_) | Message::ZoneAnswer(_) | Message::Handoff(_)
            ) {
                self.asked = None;
            }
            return;
        };
        let Some(length) = length(event, self.records) else {
            // A browse's, which [`Browsing`] keeps.
            return;
        };
        let waiting = self.waiting(Instant::now());
        match (event, waiting) {
            (BazaarEvent::Results(rows), Some(Asked::Search)) => {
                if self.rows + rows.len() > bazaar::MOST_ROWS {
                    self.unasked.push((length, Unpassed::PastMost));
                    *message = Message::Withheld;
                } else {
                    self.rows += rows.len();
                }
            }
            (BazaarEvent::SearchDone, Some(Asked::Search))
            | (BazaarEvent::Welcome { .. }, Some(Asked::Welcome)) => self.asked = None,
            _ => {
                self.unasked.push((length, Unpassed::Unasked));
                *message = Message::Withheld;
            }
        }
    }

    fn tick(&mut self, _now: Instant, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        for (length, why) in self.unasked.drain(..) {
            let reason = match why {
                Unpassed::Unasked => "a bazaar answer no request asked for",
                Unpassed::PastMost => "a bazaar result past the most one search lists",
            };
            out.log.gap_unsampled(
                self.records.opcode,
                Some("OP_BazaarSearch"),
                length,
                crate::client::GapKind::Unread {
                    reason: reason.to_owned(),
                },
            )?;
        }
        Ok(())
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let now = Instant::now();
        let (request, asked) = match command {
            ClientCommand::BazaarSearch { criteria, .. } => {
                (Request::BazaarSearch(criteria.clone()), Asked::Search)
            }
            ClientCommand::BazaarWelcome { .. } => (Request::BazaarWelcome, Asked::Welcome),
            _ => return Ok(()),
        };
        if world.zone.0 != bazaar::ZONE_ID {
            return actions::refuse(command, "You can search only in the Bazaar.", out.log);
        }
        if self.waiting(now).is_some() {
            return actions::refuse(command, "Wait for the last search to finish.", out.log);
        }
        if self
            .sent
            .is_some_and(|sent| now.saturating_duration_since(sent) < FLOOR)
        {
            return actions::refuse(command, "Wait a moment before searching again.", out.log);
        }
        if let Err(error) = out.request(&request) {
            return actions::refuse(command, &format!("Cannot search: {error:#}."), out.log);
        }
        self.asked = Some((asked, now));
        self.rows = 0;
        self.sent = Some(now);
        Ok(())
    }
}

/// The length of the record a search's answer came in, in the generation's
/// `records`; `None` for a browse's event.
fn length(event: &BazaarEvent, records: Records) -> Option<usize> {
    match event {
        BazaarEvent::Results(rows) => Some(rows.len() * records.row),
        BazaarEvent::SearchDone | BazaarEvent::Welcome { .. } => Some(records.end),
        BazaarEvent::Admitted { .. }
        | BazaarEvent::TraderSign { .. }
        | BazaarEvent::ShopEnded
        | BazaarEvent::Listed(_)
        | BazaarEvent::Delisted { .. }
        | BazaarEvent::BrowseEnded { .. } => None,
    }
}

/// How long a click on a trader waits for the server's admission (ours,
/// inferred: `EQEmu` answers every click on a player in the zone at once).
const CLICK_WAIT: Duration = Duration::from_secs(10);
/// The least time between one click on a trader and the next (ours,
/// inferred), since each records the player as a customer.
const CLICK_FLOOR: Duration = Duration::from_secs(1);
/// How many clicks that stopped waiting are kept, so a late approval of
/// any of them is ended.
const MOST_LAPSED: usize = 8;
/// The reach a click on a trader takes where the generation's merchant
/// reach is unread: 20 in 3-D, as TAKP's program reaches merchants and
/// trader shops alike (inferred for Titanium, R49-42 d). `EQEmu` checks
/// no distance on the click.
const INFERRED_REACH: Reach = Reach {
    distance: 20.0,
    flat: false,
    inclusive: true,
};

/// An open browse of one trader.
#[derive(Debug)]
struct Browse {
    /// The trader.
    trader: u16,
    /// Each item on their list by serial: its price and count, which a
    /// later buy must send exactly.
    list: BTreeMap<u32, (u32, u32)>,
}

/// What the session owes the server or the host once it may send.
#[derive(Debug, PartialEq)]
enum Owed {
    /// The end of a browse the session closed, or of a late approval.
    End(u16),
    /// A browse is over: tell the host.
    Ended(u16),
    /// A gap, by its opcode, name, length and why.
    Gap(u16, &'static str, usize, &'static str),
}

/// Browsing a trader's wares in the Bazaar (6.558).
#[derive(Default)]
pub(super) struct Browsing {
    /// The players the session knows trade now: their sign went on, or a
    /// search named them. Only spawns the session knows, and cleared on
    /// despawn and zoning.
    traders: BTreeSet<u16>,
    /// The click in flight: the trader and when it went.
    click: Option<(u16, Instant)>,
    /// When the last click went.
    clicked: Option<Instant>,
    /// Clicks that stopped waiting, newest last.
    lapsed: Vec<u16>,
    /// The browse open.
    browse: Option<Browse>,
    /// The merchant whose window the server opened, as the session last
    /// heard.
    merchant: Option<u16>,
    /// When the player last asked a merchant to open, until the server
    /// answers.
    shop_asked: Option<Instant>,
    /// The run of merchant item packets after an approval no browse took,
    /// which are that trader's list: withheld, told once as a gap.
    discarding: Option<bool>,
    /// What the session owes, in order.
    owed: Vec<Owed>,
}

impl Browsing {
    fn waiting(&self, now: Instant) -> Option<u16> {
        self.click
            .filter(|(_, at)| now.saturating_duration_since(*at) < CLICK_WAIT)
            .map(|(trader, _)| trader)
    }

    /// Moves a click that stopped waiting to the lapsed.
    fn lapse(&mut self, now: Instant) {
        if self
            .click
            .is_some_and(|(_, at)| now.saturating_duration_since(at) >= CLICK_WAIT)
        {
            self.give_up_click();
        }
    }

    /// Stops waiting for the click in flight, keeping it as lapsed.
    fn give_up_click(&mut self) {
        if let Some((trader, _)) = self.click.take() {
            self.lapsed.retain(|lapsed| *lapsed != trader);
            self.lapsed.push(trader);
            if self.lapsed.len() > MOST_LAPSED {
                self.lapsed.remove(0);
            }
        }
    }

    /// Forgets everything the zone kept, at a new admission: the server
    /// forgets the customer with the zone, so nothing is sent (inferred).
    fn forget(&mut self) {
        if let Some(browse) = self.browse.take() {
            self.owed.push(Owed::Ended(browse.trader));
        }
        self.traders.clear();
        self.click = None;
        self.lapsed.clear();
        self.merchant = None;
        self.shop_asked = None;
        self.discarding = None;
    }

    /// The server offers or answers a zone change, or hands the player
    /// over: an open browse is ended while this zone's connection can still
    /// carry the end, since a refused change keeps the player here as the
    /// trader's customer, and a click in flight is kept as lapsed, so its
    /// late approval is still ended.
    fn leaving(&mut self) {
        if let Some(trader) = self.browse.as_ref().map(|browse| browse.trader) {
            self.close(trader, true);
        }
        self.give_up_click();
    }

    /// Whether the player's request to open a merchant still waits for
    /// the server (inferred: as long as a click waits for a trader).
    fn shop_waiting(&self, now: Instant) -> bool {
        self.shop_asked
            .is_some_and(|asked| now.saturating_duration_since(asked) < CLICK_WAIT)
    }

    /// Closes the open browse of `trader`: with its end when the session
    /// closes it, without when the server or the trader did.
    fn close(&mut self, trader: u16, send_end: bool) {
        if self
            .browse
            .as_ref()
            .is_some_and(|browse| browse.trader == trader)
        {
            self.browse = None;
            if send_end {
                self.owed.push(Owed::End(trader));
            }
            self.owed.push(Owed::Ended(trader));
        }
    }

    /// A trader left: their sign went off or they despawned.
    fn trader_left(&mut self, spawn: u16) {
        self.traders.remove(&spawn);
        self.close(spawn, false);
    }

    fn admitted_answer(&mut self, trader: u16, approved: bool, message: &mut Message) {
        let now = Instant::now();
        self.lapse(now);
        if self.waiting(now) == Some(trader) {
            self.click = None;
            if approved {
                self.browse = Some(Browse {
                    trader,
                    list: BTreeMap::new(),
                });
            }
        } else if let Some(at) = self.lapsed.iter().position(|lapsed| *lapsed == trader) {
            // A late answer: an approval recorded the player as the
            // trader's customer, so it is ended at once, and the list the
            // server sends after it is withheld.
            self.lapsed.remove(at);
            if approved {
                self.owed.push(Owed::End(trader));
                self.discarding = Some(false);
            }
            *message = Message::Withheld;
        } else {
            self.owed.push(Owed::Gap(
                bazaar::BROWSE_OPCODE,
                "OP_TraderShop",
                bazaar::BROWSE_LENGTH,
                "a trader's admission no click asked for",
            ));
            if approved {
                self.discarding = Some(false);
            }
            *message = Message::Withheld;
        }
    }

    /// An item packet while browsing: an entry of the trader's list.
    fn listed(
        &mut self,
        serial: Option<u32>,
        length: usize,
        item: &Result<Box<eq_network_game::merchant::MerchantItem>, String>,
    ) -> Message {
        let Some(browse) = self.browse.as_mut() else {
            return Message::Withheld;
        };
        let trader = browse.trader;
        let (Some(serial), Ok(item)) = (serial, item) else {
            // Never keep a half list a buy could use.
            self.owed.push(Owed::Gap(
                eq_network_game::merchant::ITEM_PACKET_OPCODE,
                "OP_ItemPacket",
                length,
                "a trader's item that could not be read",
            ));
            self.close(trader, true);
            return Message::Withheld;
        };
        if item.price == 0 {
            return if browse.list.remove(&serial).is_some() {
                Message::Event(WorldEvent::Bazaar(BazaarEvent::Delisted { trader, serial }))
            } else {
                Message::Withheld
            };
        }
        if !browse.list.contains_key(&serial) && browse.list.len() >= bazaar::MOST_LISTED {
            self.owed.push(Owed::Gap(
                eq_network_game::merchant::ITEM_PACKET_OPCODE,
                "OP_ItemPacket",
                length,
                "a trader's list past the most a trader lists",
            ));
            self.close(trader, true);
            return Message::Withheld;
        }
        browse.list.insert(serial, (item.price, item.quantity));
        Message::Event(WorldEvent::Bazaar(BazaarEvent::Listed(Box::new(
            bazaar::TraderListing {
                trader,
                serial,
                price: item.price,
                count: item.quantity,
                item: item.item.clone(),
            },
        ))))
    }

    /// Pays what the session owes.
    fn pay(&mut self, out: &mut Out<'_, '_>) -> Result<()> {
        for owed in std::mem::take(&mut self.owed) {
            match owed {
                Owed::End(trader) => out.request(&Request::EndBrowse(trader))?,
                Owed::Ended(trader) => out.log.send(ClientEvent::World(WorldEvent::Bazaar(
                    BazaarEvent::BrowseEnded { trader },
                )))?,
                Owed::Gap(opcode, name, length, reason) => out.log.gap_unsampled(
                    opcode,
                    Some(name),
                    length,
                    crate::client::GapKind::Unread {
                        reason: reason.to_owned(),
                    },
                )?,
            }
        }
        Ok(())
    }

    fn refusal(
        &self,
        trader: u16,
        world: &World,
        out: &Out<'_, '_>,
        now: Instant,
    ) -> Option<&'static str> {
        if world.zone.0 != bazaar::ZONE_ID {
            return Some("You can browse traders only in the Bazaar.");
        }
        let Some(spawn) = world
            .visible_spawn(trader)
            .filter(|spawn| spawn.kind == SpawnKind::Player && self.traders.contains(&trader))
        else {
            return Some("That player is not a trader.");
        };
        let reach = out.wire.reaches().merchants.unwrap_or(INFERRED_REACH);
        if !world
            .player_at()
            .is_some_and(|(_, position)| reach.admits(position, spawn.position))
        {
            return Some("That trader is too far away.");
        }
        if self.browse.is_some() {
            return Some("Close the trader's window first.");
        }
        if self.merchant.is_some() {
            return Some("Close the merchant's window first.");
        }
        if self.shop_waiting(now) {
            return Some("Wait for the merchant to answer.");
        }
        if self.waiting(now).is_some() {
            return Some("Wait for the trader to answer.");
        }
        if self
            .clicked
            .is_some_and(|clicked| now.saturating_duration_since(clicked) < CLICK_FLOOR)
        {
            return Some("Wait a moment before clicking again.");
        }
        None
    }
}

impl Feature for Browsing {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::BazaarBrowsing]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::BrowseTrader { .. } | ClientCommand::EndBrowse { .. }
        )
    }

    /// A merchant may not open over a browse or a click in flight, as the
    /// trader's list and a merchant's come in the same packets.
    fn holds(&self, _world: &World, now: Instant) -> Vec<(Resource, &'static str)> {
        if self.browse.is_some() || self.waiting(now).is_some() {
            vec![(Resource::Shopping, "Close the trader's window first")]
        } else {
            Vec::new()
        }
    }

    /// A request to open a merchant waits for its answer before a click on
    /// a trader may go, as their lists come in the same packets; a close
    /// asks for nothing.
    fn notice(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let ClientCommand::Shop { open, .. } = command {
            self.shop_asked = open.then(Instant::now);
        }
        Ok(())
    }

    /// A new admission forgets the zone's browse, traders and clicks.
    fn admitted(&mut self, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        self.forget();
        self.pay(out)
    }

    fn explain(&mut self, message: &mut Message, world: &World) {
        if let Message::MerchantView { length, .. } = message {
            if let Some(told) = self.discarding.as_mut() {
                if !*told {
                    *told = true;
                    self.owed.push(Owed::Gap(
                        eq_network_game::merchant::ITEM_PACKET_OPCODE,
                        "OP_ItemPacket",
                        *length,
                        "a trader's list after an approval no browse took",
                    ));
                }
                *message = Message::Withheld;
                return;
            }
        } else {
            // Anything else ends the run of that trader's items.
            self.discarding = None;
        }
        match message {
            Message::ZoneOffer(_) | Message::ZoneAnswer(_) | Message::Handoff(_) => self.leaving(),
            Message::Event(WorldEvent::Bazaar(event)) => match event {
                BazaarEvent::TraderSign { spawn, on: true } => {
                    if world
                        .spawns
                        .get(*spawn)
                        .is_some_and(|known| known.kind == SpawnKind::Player)
                    {
                        self.traders.insert(*spawn);
                    }
                }
                BazaarEvent::TraderSign { spawn, on: false } => self.trader_left(*spawn),
                BazaarEvent::Results(rows) => {
                    // The server's word that each trades now, in this zone
                    // (`Local_Scope`; inferred as the program's own).
                    for trader in rows.iter().filter_map(|row| row.trader) {
                        if world
                            .spawns
                            .get(trader)
                            .is_some_and(|known| known.kind == SpawnKind::Player)
                        {
                            self.traders.insert(trader);
                        }
                    }
                }
                BazaarEvent::Admitted { trader, approved } => {
                    let (trader, approved) = (*trader, *approved);
                    self.admitted_answer(trader, approved, message);
                }
                BazaarEvent::ShopEnded => {
                    if let Some(trader) = self.browse.as_ref().map(|browse| browse.trader) {
                        // The trader left trader mode, and the server freed
                        // them.
                        self.close(trader, false);
                    } else {
                        self.owed.push(Owed::Gap(
                            bazaar::SHOP_END_OPCODE,
                            "OP_ShopEnd",
                            0,
                            "a shop's end with no browse open",
                        ));
                    }
                    *message = Message::Withheld;
                }
                BazaarEvent::SearchDone
                | BazaarEvent::Welcome { .. }
                | BazaarEvent::Listed(_)
                | BazaarEvent::Delisted { .. }
                | BazaarEvent::BrowseEnded { .. } => {}
            },
            Message::MerchantView {
                serial,
                length,
                item,
            } if self.browse.is_some() => {
                *message = self.listed(*serial, *length, item);
            }
            Message::MerchantScoped {
                merchant_id: 0,
                update: MerchantUpdate::Removed { slot },
            } => {
                // `OP_TraderDelItem` names no merchant: while browsing, its
                // serial left the trader's list.
                if let Some(browse) = self.browse.as_mut() {
                    let (trader, serial) = (browse.trader, *slot);
                    *message = if browse.list.remove(&serial).is_some() {
                        Message::Event(WorldEvent::Bazaar(BazaarEvent::Delisted { trader, serial }))
                    } else {
                        Message::Withheld
                    };
                }
            }
            _ => {}
        }
    }

    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match message {
            Message::Event(WorldEvent::Merchant(MerchantUpdate::Opened {
                merchant_id,
                accepted,
                ..
            })) => {
                self.shop_asked = None;
                if *accepted {
                    self.merchant = Some(*merchant_id);
                }
            }
            Message::Event(WorldEvent::Merchant(MerchantUpdate::Closed)) => {
                self.shop_asked = None;
                self.merchant = None;
            }
            Message::Event(WorldEvent::Despawn(id)) => {
                if self.merchant == Some(*id) {
                    self.merchant = None;
                }
                self.trader_left(*id);
            }
            Message::Event(WorldEvent::Spawns(spawns)) => {
                // A spawn sent again is someone new in that place.
                for spawn in spawns {
                    if self.merchant == Some(spawn.spawn_id) {
                        self.merchant = None;
                    }
                    self.trader_left(spawn.spawn_id);
                }
            }
            _ => {}
        }
        self.pay(out)
    }

    fn tick(&mut self, now: Instant, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        self.lapse(now);
        self.pay(out)
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let now = Instant::now();
        self.lapse(now);
        match command {
            ClientCommand::BrowseTrader { trader, .. } => {
                if let Some(reason) = self.refusal(*trader, world, out, now) {
                    return actions::refuse(command, reason, out.log);
                }
                if let Err(error) = out.request(&Request::BrowseTrader(*trader)) {
                    return actions::refuse(
                        command,
                        &format!("Cannot browse: {error:#}."),
                        out.log,
                    );
                }
                self.click = Some((*trader, now));
                self.clicked = Some(now);
                self.lapsed.retain(|lapsed| lapsed != trader);
                Ok(())
            }
            ClientCommand::EndBrowse { .. } => {
                let Some(trader) = self.browse.as_ref().map(|browse| browse.trader) else {
                    return actions::refuse(command, "No trader's window is open.", out.log);
                };
                self.close(trader, true);
                self.pay(out)
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;
    use eq_network_game::bazaar::{BazaarCriteria, BazaarListing};

    fn in_bazaar() -> World {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.zone = (bazaar::ZONE_ID, 0);
        world
    }

    fn search() -> ClientCommand {
        ClientCommand::BazaarSearch {
            session_id: 5,
            criteria: BazaarCriteria {
                name: "Bone Chips".into(),
                ..BazaarCriteria::default()
            },
        }
    }

    fn refused(outcome: &testing::Outcome<Result<()>>) -> Option<String> {
        if !outcome.sent.is_empty() {
            return None;
        }
        outcome.events.iter().find_map(|event| match event {
            ClientEvent::World(WorldEvent::BazaarRefused {
                session_id: 5,
                reason,
            }) => Some(reason.clone()),
            _ => None,
        })
    }

    fn results() -> Message {
        Message::Event(WorldEvent::Bazaar(BazaarEvent::Results(vec![
            BazaarListing {
                trader: Some(312),
                item_id: 1001,
                serial: Some(77),
                name: "Bone Chips".into(),
                count: Some(20),
                price: 1250,
                stat: Some(0),
                listings: None,
            },
        ])))
    }

    fn done() -> Message {
        Message::Event(WorldEvent::Bazaar(BazaarEvent::SearchDone))
    }

    #[test]
    fn a_search_goes_only_in_the_bazaar() {
        let mut world = in_bazaar();
        world.zone = (202, 0);
        let mut feature = Bazaar::titanium();
        let outcome = testing::run(|out| feature.handle(&search(), &mut world, out));
        assert_eq!(
            refused(&outcome).as_deref(),
            Some("You can search only in the Bazaar.")
        );
        assert!(feature.asked.is_none() && feature.sent.is_none());
    }

    #[test]
    fn a_search_goes_as_built_and_its_answer_passes() {
        let mut world = in_bazaar();
        let mut feature = Bazaar::titanium();
        let outcome = testing::run(|out| feature.handle(&search(), &mut world, out));
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, bazaar::SEARCH_OPCODE);
        let ClientCommand::BazaarSearch { criteria, .. } = search() else {
            unreachable!()
        };
        assert_eq!(outcome.sent[0].body, bazaar::search(&criteria).unwrap());
        let mut rows = results();
        feature.explain(&mut rows, &world);
        assert!(matches!(rows, Message::Event(_)));
        // Results alone keep the search in flight; its end frees it.
        assert_eq!(feature.waiting(Instant::now()), Some(Asked::Search));
        let mut end = done();
        feature.explain(&mut end, &world);
        assert!(matches!(end, Message::Event(_)));
        assert!(feature.asked.is_none());
    }

    #[test]
    fn one_search_at_a_time_with_a_floor_between() {
        let mut world = in_bazaar();
        let mut feature = Bazaar::titanium();
        assert_eq!(
            testing::run(|out| feature.handle(&search(), &mut world, out))
                .sent
                .len(),
            1
        );
        // In flight.
        assert_eq!(
            refused(&testing::run(|out| feature.handle(
                &search(),
                &mut world,
                out
            )))
            .as_deref(),
            Some("Wait for the last search to finish.")
        );
        // Ended, but too soon after the last.
        feature.explain(&mut done(), &world);
        assert_eq!(
            refused(&testing::run(|out| feature.handle(
                &search(),
                &mut world,
                out
            )))
            .as_deref(),
            Some("Wait a moment before searching again.")
        );
        let welcome = ClientCommand::BazaarWelcome { session_id: 5 };
        assert!(refused(&testing::run(
            |out| feature.handle(&welcome, &mut world, out)
        ))
        .is_some());
        // Past the floor, the next goes.
        feature.sent = Instant::now().checked_sub(FLOOR);
        let outcome = testing::run(|out| feature.handle(&welcome, &mut world, out));
        assert_eq!(
            (outcome.sent[0].opcode, &outcome.sent[0].body),
            (bazaar::SEARCH_OPCODE, &bazaar::welcome())
        );
        // A lost end frees the search after the wait.
        let mut hello = Message::Event(WorldEvent::Bazaar(BazaarEvent::Welcome {
            traders: 2,
            items: 9,
        }));
        feature.explain(&mut hello, &world);
        assert!(matches!(hello, Message::Event(_)) && feature.asked.is_none());
        feature.sent = Instant::now().checked_sub(FLOOR);
        testing::run(|out| feature.handle(&search(), &mut world, out));
        feature.sent = Instant::now().checked_sub(WAIT);
        feature.asked = feature
            .asked
            .map(|(asked, _)| (asked, feature.sent.unwrap()));
        assert_eq!(
            testing::run(|out| feature.handle(&search(), &mut world, out))
                .sent
                .len(),
            1
        );
    }

    #[test]
    fn a_search_the_codec_refuses_is_refused_in_our_words() {
        let mut world = in_bazaar();
        let mut feature = Bazaar::titanium();
        let long = ClientCommand::BazaarSearch {
            session_id: 5,
            criteria: BazaarCriteria {
                name: "a".repeat(64),
                ..BazaarCriteria::default()
            },
        };
        let outcome = testing::run(|out| feature.handle(&long, &mut world, out));
        assert!(refused(&outcome).is_some_and(|reason| reason.starts_with("Cannot search")));
        assert!(feature.asked.is_none() && feature.sent.is_none());
    }

    #[test]
    fn an_answer_nobody_asked_for_is_withheld_and_a_gap_without_its_body() {
        let mut world = in_bazaar();
        let mut feature = Bazaar::titanium();
        let mut rows = results();
        feature.explain(&mut rows, &world);
        assert!(matches!(rows, Message::Withheld));
        // A welcome while a search is in flight is no answer to it.
        testing::run(|out| feature.handle(&search(), &mut world, out));
        let mut hello = Message::Event(WorldEvent::Bazaar(BazaarEvent::Welcome {
            traders: 2,
            items: 9,
        }));
        feature.explain(&mut hello, &world);
        assert!(matches!(hello, Message::Withheld));
        let told = testing::run(|out| feature.tick(Instant::now(), &mut world, out));
        let gaps: Vec<_> = told
            .events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::Gap(gap) => Some(gap),
                _ => None,
            })
            .collect();
        // The first is told at once; the second, of the same kind, is
        // counted as its repeat.
        assert_eq!(gaps.len(), 1);
        assert_eq!(feature.unasked, Vec::<(usize, Unpassed)>::new());
        assert!(gaps
            .iter()
            .all(|gap| gap.sample.is_none() && gap.opcode == bazaar::SEARCH_OPCODE));
        assert_eq!(gaps[0].min_length, bazaar::ROW_LENGTH);
    }

    #[test]
    fn zoning_or_a_new_admission_forgets_the_search_in_flight() {
        let mut world = in_bazaar();
        let mut feature = Bazaar::titanium();
        testing::run(|out| feature.handle(&search(), &mut world, out));
        testing::run(|out| feature.admitted(&mut world, out));
        assert!(feature.asked.is_none());
        let mut end = done();
        feature.explain(&mut end, &world);
        assert!(matches!(end, Message::Withheld));
    }

    mod eqmac {
        use super::super::super::{feature::testing, ClientEvent};
        use super::super::*;
        use super::{done, in_bazaar, refused, search};
        use eq_network_game::bazaar::{BazaarCriteria, BazaarListing};

        fn row(name: &str) -> Message {
            Message::Event(WorldEvent::Bazaar(BazaarEvent::Results(vec![
                BazaarListing {
                    trader: None,
                    item_id: 13073,
                    serial: None,
                    name: name.into(),
                    count: Some(12),
                    price: 1250,
                    stat: None,
                    listings: Some(1),
                },
            ])))
        }

        fn gaps(outcome: &testing::Outcome<Result<()>>) -> Vec<&crate::client::gaps::Gap> {
            outcome
                .events
                .iter()
                .filter_map(|event| match event {
                    ClientEvent::Gap(gap) => Some(&**gap),
                    _ => None,
                })
                .collect()
        }

        #[test]
        fn a_search_goes_in_the_mac_records_only_in_the_bazaar() {
            let mut world = in_bazaar();
            world.zone = (202, 0);
            let mut feature = Bazaar::eqmac();
            let outcome = testing::run_on(&super::super::super::wire::EqMac, |out| {
                feature.handle(&search(), &mut world, out)
            });
            assert_eq!(
                refused(&outcome).as_deref(),
                Some("You can search only in the Bazaar.")
            );
            world.zone = (bazaar::ZONE_ID, 0);
            let outcome = testing::run_on(&super::super::super::wire::EqMac, |out| {
                feature.handle(&search(), &mut world, out)
            });
            let ClientCommand::BazaarSearch { criteria, .. } = search() else {
                unreachable!()
            };
            assert_eq!(
                (outcome.sent[0].opcode, &outcome.sent[0].body),
                (0x1142, &bazaar::eqmac::search(&criteria).unwrap())
            );
            // The floor holds as on Titanium.
            feature.explain(&mut done(), &world);
            let again = testing::run_on(&super::super::super::wire::EqMac, |out| {
                feature.handle(&search(), &mut world, out)
            });
            assert_eq!(
                refused(&again).as_deref(),
                Some("Wait a moment before searching again.")
            );
        }

        #[test]
        fn a_level_bound_is_refused_and_nothing_goes() {
            let mut world = in_bazaar();
            let mut feature = Bazaar::eqmac();
            let bounded = ClientCommand::BazaarSearch {
                session_id: 5,
                criteria: BazaarCriteria {
                    max_level: Some(50),
                    ..BazaarCriteria::default()
                },
            };
            let outcome = testing::run_on(&super::super::super::wire::EqMac, |out| {
                feature.handle(&bounded, &mut world, out)
            });
            assert!(refused(&outcome).is_some_and(|reason| reason.starts_with("Cannot search")));
            assert!(feature.asked.is_none() && feature.sent.is_none());
        }

        #[test]
        fn each_row_passes_until_the_end_and_rows_past_the_most_are_gaps() {
            let mut world = in_bazaar();
            let mut feature = Bazaar::eqmac();
            testing::run_on(&super::super::super::wire::EqMac, |out| {
                feature.handle(&search(), &mut world, out)
            });
            for _ in 0..bazaar::MOST_ROWS {
                let mut one = row("Bone Chips");
                feature.explain(&mut one, &world);
                assert!(matches!(one, Message::Event(_)));
            }
            let mut past = row("Bone Chips");
            feature.explain(&mut past, &world);
            assert!(matches!(past, Message::Withheld));
            let mut end = done();
            feature.explain(&mut end, &world);
            assert!(matches!(end, Message::Event(_)) && feature.asked.is_none());
            let told = testing::run(|out| feature.tick(Instant::now(), &mut world, out));
            let told = gaps(&told);
            assert_eq!(told.len(), 1);
            assert_eq!(
                (told[0].opcode, told[0].min_length, told[0].sample.is_none()),
                (0x1142, bazaar::eqmac::ROW_LENGTH, true)
            );
            // The next search counts its rows afresh.
            feature.sent = Instant::now().checked_sub(FLOOR);
            testing::run_on(&super::super::super::wire::EqMac, |out| {
                feature.handle(&search(), &mut world, out)
            });
            let mut one = row("Bone Chips");
            feature.explain(&mut one, &world);
            assert!(matches!(one, Message::Event(_)));
        }

        #[test]
        fn an_end_alone_is_an_empty_search_and_an_unasked_end_a_gap() {
            let mut world = in_bazaar();
            let mut feature = Bazaar::eqmac();
            let mut unasked = done();
            feature.explain(&mut unasked, &world);
            assert!(matches!(unasked, Message::Withheld));
            let told = testing::run(|out| feature.tick(Instant::now(), &mut world, out));
            assert_eq!(
                gaps(&told)
                    .iter()
                    .map(|gap| (gap.opcode, gap.min_length))
                    .collect::<Vec<_>>(),
                [(0x1142, bazaar::eqmac::DONE_LENGTH)]
            );
            testing::run_on(&super::super::super::wire::EqMac, |out| {
                feature.handle(&search(), &mut world, out)
            });
            let mut end = done();
            feature.explain(&mut end, &world);
            assert!(matches!(end, Message::Event(_)) && feature.asked.is_none());
        }
    }

    mod browsing {
        use super::super::super::{actions, feature::testing, ClientEvent};
        use super::super::*;
        use eq_network_game::{bazaar::BazaarListing, merchant::MerchantItem};

        const TRADER: u16 = 40;

        fn bazaar_world() -> World {
            let mut world = World::new(5);
            world.own_spawn = Some(7);
            world.player.admit(testing::player(7));
            world.zone = (bazaar::ZONE_ID, 0);
            world
                .spawns
                .insert(testing::spawn(TRADER, SpawnKind::Player));
            world
        }

        fn event(event: BazaarEvent) -> Message {
            Message::Event(WorldEvent::Bazaar(event))
        }

        /// A browsing feature that saw the trader's sign go on.
        fn signed(world: &World) -> Browsing {
            let mut feature = Browsing::default();
            feature.explain(
                &mut event(BazaarEvent::TraderSign {
                    spawn: TRADER,
                    on: true,
                }),
                world,
            );
            feature
        }

        fn click() -> ClientCommand {
            ClientCommand::BrowseTrader {
                session_id: 5,
                trader: TRADER,
            }
        }

        fn refused(outcome: &testing::Outcome<Result<()>>) -> Option<String> {
            if !outcome.sent.is_empty() {
                return None;
            }
            outcome.events.iter().find_map(|event| match event {
                ClientEvent::World(WorldEvent::BazaarRefused {
                    session_id: 5,
                    reason,
                }) => Some(reason.clone()),
                _ => None,
            })
        }

        fn ended(outcome: &testing::Outcome<Result<()>>) -> bool {
            outcome.events.iter().any(|event| {
                matches!(
                    event,
                    ClientEvent::World(WorldEvent::Bazaar(BazaarEvent::BrowseEnded {
                        trader: TRADER
                    }))
                )
            })
        }

        fn gaps(outcome: &testing::Outcome<Result<()>>) -> Vec<&crate::client::Gap> {
            outcome
                .events
                .iter()
                .filter_map(|event| match event {
                    ClientEvent::Gap(gap) => Some(&**gap),
                    _ => None,
                })
                .collect()
        }

        fn view(serial: Option<u32>, price: u32) -> Message {
            Message::MerchantView {
                serial,
                length: 400,
                item: Ok(Box::new(MerchantItem {
                    slot: 30,
                    price,
                    quantity: 1,
                    item: testing::item(30),
                })),
            }
        }

        /// A feature browsing the trader, approved.
        fn browsing(world: &mut World) -> Browsing {
            let mut feature = signed(world);
            let outcome = testing::run(|out| feature.handle(&click(), world, out));
            assert_eq!(outcome.sent.len(), 1);
            let mut admitted = event(BazaarEvent::Admitted {
                trader: TRADER,
                approved: true,
            });
            feature.explain(&mut admitted, world);
            assert!(matches!(admitted, Message::Event(_)));
            assert!(feature.browse.is_some());
            feature
        }

        #[test]
        fn a_click_goes_as_built_to_a_signed_trader_in_reach() {
            let mut world = bazaar_world();
            let mut feature = signed(&world);
            let outcome = testing::run(|out| feature.handle(&click(), &mut world, out));
            assert_eq!(
                (outcome.sent[0].opcode, &outcome.sent[0].body[..]),
                (bazaar::BROWSE_OPCODE, &bazaar::browse(TRADER)[..])
            );
            assert_eq!(feature.waiting(Instant::now()), Some(TRADER));
            // A merchant may not open over the click in flight.
            assert_eq!(
                feature.holds(&world, Instant::now()),
                [(Resource::Shopping, "Close the trader's window first")]
            );
            let shop = ClientCommand::Shop {
                session_id: 5,
                merchant_id: 9,
                own_id: 7,
                open: true,
                created: Instant::now(),
            };
            assert_eq!(actions::needs(&shop), [Resource::Shopping]);
        }

        #[test]
        fn a_click_is_refused_in_our_words_with_nothing_sent() {
            let reason = |world: &mut World, feature: &mut Browsing| {
                refused(&testing::run(|out| feature.handle(&click(), world, out)))
            };
            let mut world = bazaar_world();
            world.zone = (202, 0);
            let mut feature = signed(&world);
            assert_eq!(
                reason(&mut world, &mut feature).as_deref(),
                Some("You can browse traders only in the Bazaar.")
            );
            // No sign seen.
            let mut world = bazaar_world();
            let mut unsigned = Browsing::default();
            assert_eq!(
                reason(&mut world, &mut unsigned).as_deref(),
                Some("That player is not a trader.")
            );
            // Out of reach: 20 in 3-D where the merchant reach is unread.
            let mut feature = signed(&world);
            let mut far = testing::spawn(TRADER, SpawnKind::Player);
            far.position.x = 20.5;
            world.spawns.insert(far);
            assert_eq!(
                reason(&mut world, &mut feature).as_deref(),
                Some("That trader is too far away.")
            );
            let mut near = testing::spawn(TRADER, SpawnKind::Player);
            near.position.x = 20.0;
            world.spawns.insert(near);
            // A merchant's window open.
            testing::run(|out| {
                feature.observe(
                    &Message::Event(WorldEvent::Merchant(MerchantUpdate::Opened {
                        merchant_id: 9,
                        accepted: true,
                        rate: 1.0,
                    })),
                    &mut world,
                    out,
                )
            });
            assert_eq!(
                reason(&mut world, &mut feature).as_deref(),
                Some("Close the merchant's window first.")
            );
            testing::run(|out| {
                feature.observe(
                    &Message::Event(WorldEvent::Merchant(MerchantUpdate::Closed)),
                    &mut world,
                    out,
                )
            });
            // In flight, then too soon after the last.
            assert!(reason(&mut world, &mut feature).is_none());
            assert_eq!(
                reason(&mut world, &mut feature).as_deref(),
                Some("Wait for the trader to answer.")
            );
            feature.explain(
                &mut event(BazaarEvent::Admitted {
                    trader: TRADER,
                    approved: false,
                }),
                &world,
            );
            assert_eq!(
                reason(&mut world, &mut feature).as_deref(),
                Some("Wait a moment before clicking again.")
            );
            feature.clicked = Instant::now().checked_sub(CLICK_FLOOR);
            assert!(reason(&mut world, &mut feature).is_none());
        }

        #[test]
        fn a_sign_or_search_counts_only_for_a_known_player() {
            let mut world = bazaar_world();
            world.spawns.insert(testing::spawn(41, SpawnKind::Npc));
            let mut feature = Browsing::default();
            for spawn in [41, 99] {
                feature.explain(
                    &mut event(BazaarEvent::TraderSign { spawn, on: true }),
                    &world,
                );
            }
            assert_eq!(feature.traders, BTreeSet::new());
            let row = |trader| BazaarListing {
                trader,
                item_id: 1001,
                serial: Some(77),
                name: "Bone Chips".into(),
                count: Some(20),
                price: 1250,
                stat: Some(0),
                listings: None,
            };
            // A Mac-era row names no trader out of the zone.
            feature.explain(
                &mut event(BazaarEvent::Results(vec![
                    row(Some(TRADER)),
                    row(Some(99)),
                    row(None),
                ])),
                &world,
            );
            assert_eq!(feature.traders, BTreeSet::from([TRADER]));
            feature.explain(
                &mut event(BazaarEvent::TraderSign {
                    spawn: TRADER,
                    on: false,
                }),
                &world,
            );
            assert_eq!(feature.traders, BTreeSet::new());
        }

        #[test]
        fn a_refused_admission_opens_nothing() {
            let mut world = bazaar_world();
            let mut feature = signed(&world);
            testing::run(|out| feature.handle(&click(), &mut world, out));
            let mut answer = event(BazaarEvent::Admitted {
                trader: TRADER,
                approved: false,
            });
            feature.explain(&mut answer, &world);
            assert!(matches!(answer, Message::Event(_)));
            assert!(feature.browse.is_none() && feature.click.is_none());
            // Items after it stay the merchant's.
            let mut item = view(Some(90), 1250);
            feature.explain(&mut item, &world);
            assert!(matches!(item, Message::MerchantView { .. }));
        }

        #[test]
        fn the_list_follows_the_server_and_merchant_packets_stay_outside() {
            let mut world = bazaar_world();
            let mut feature = browsing(&mut world);
            let mut item = view(Some(90), 1250);
            feature.explain(&mut item, &world);
            let Message::Event(WorldEvent::Bazaar(BazaarEvent::Listed(listing))) = &item else {
                panic!("listed: {item:?}");
            };
            assert_eq!(
                (listing.trader, listing.serial, listing.price, listing.count),
                (TRADER, 90, 1250, 1)
            );
            // A price cut replaces the entry.
            let mut cut = view(Some(90), 1000);
            feature.explain(&mut cut, &world);
            assert!(
                matches!(&cut, Message::Event(WorldEvent::Bazaar(BazaarEvent::Listed(l))) if l.price == 1000)
            );
            assert_eq!(feature.browse.as_ref().unwrap().list[&90], (1000, 1));
            // Removed by OP_TraderDelItem, then by price 0.
            let mut gone = Message::MerchantScoped {
                merchant_id: 0,
                update: MerchantUpdate::Removed { slot: 90 },
            };
            feature.explain(&mut gone, &world);
            assert!(matches!(
                gone,
                Message::Event(WorldEvent::Bazaar(BazaarEvent::Delisted {
                    trader: TRADER,
                    serial: 90
                }))
            ));
            feature.explain(&mut view(Some(91), 5), &world);
            let mut zero = view(Some(91), 0);
            feature.explain(&mut zero, &world);
            assert!(matches!(
                zero,
                Message::Event(WorldEvent::Bazaar(BazaarEvent::Delisted { serial: 91, .. }))
            ));
            assert_eq!(feature.browse.as_ref().unwrap().list, BTreeMap::new());
            // Ended, a merchant's packets are the merchant's again.
            let outcome = testing::run(|out| {
                feature.handle(&ClientCommand::EndBrowse { session_id: 5 }, &mut world, out)
            });
            assert_eq!(
                (outcome.sent[0].opcode, &outcome.sent[0].body[..]),
                (bazaar::TRADER_OPCODE, &bazaar::end_browse(TRADER)[..])
            );
            assert!(ended(&outcome));
            let mut merchant = view(Some(4), 12);
            feature.explain(&mut merchant, &world);
            super::super::super::inventory::merchant_view(&mut merchant);
            assert!(matches!(
                merchant,
                Message::Event(WorldEvent::Merchant(MerchantUpdate::Item(_)))
            ));
            let mut removal = Message::MerchantScoped {
                merchant_id: 0,
                update: MerchantUpdate::Removed { slot: 4 },
            };
            feature.explain(&mut removal, &world);
            assert!(matches!(removal, Message::MerchantScoped { .. }));
            assert_eq!(
                refused(&testing::run(|out| feature.handle(
                    &ClientCommand::EndBrowse { session_id: 5 },
                    &mut world,
                    out
                )))
                .as_deref(),
                Some("No trader's window is open.")
            );
        }

        #[test]
        fn a_list_past_the_cap_or_unreadable_ends_the_browse_with_its_end() {
            for (overflow, unreadable) in [(true, false), (false, true), (false, false)] {
                let mut world = bazaar_world();
                let mut feature = browsing(&mut world);
                if overflow {
                    for serial in 0..80 {
                        feature.explain(&mut view(Some(serial), 10), &world);
                    }
                    // A known serial is a change, not a new entry.
                    feature.explain(&mut view(Some(79), 9), &world);
                    assert!(feature.browse.is_some());
                }
                let mut last = if unreadable {
                    Message::MerchantView {
                        serial: Some(500),
                        length: 20,
                        item: Err("truncated".into()),
                    }
                } else if overflow {
                    view(Some(500), 10)
                } else {
                    view(None, 10)
                };
                feature.explain(&mut last, &world);
                assert!(matches!(last, Message::Withheld));
                assert!(feature.browse.is_none());
                let outcome = testing::run(|out| feature.tick(Instant::now(), &mut world, out));
                assert_eq!(
                    (outcome.sent[0].opcode, &outcome.sent[0].body[..]),
                    (bazaar::TRADER_OPCODE, &bazaar::end_browse(TRADER)[..])
                );
                assert!(ended(&outcome));
                let told = gaps(&outcome);
                assert_eq!(told.len(), 1);
                assert!(told[0].sample.is_none());
            }
        }

        #[test]
        fn the_server_or_the_trader_ends_a_browse_without_our_end() {
            type End = fn(&mut Browsing, &mut World) -> testing::Outcome<Result<()>>;
            let ends: [End; 3] = [
                |feature, world| {
                    let mut end = event(BazaarEvent::ShopEnded);
                    feature.explain(&mut end, world);
                    assert!(matches!(end, Message::Withheld));
                    testing::run(|out| feature.observe(&end, world, out))
                },
                |feature, world| {
                    let mut off = event(BazaarEvent::TraderSign {
                        spawn: TRADER,
                        on: false,
                    });
                    feature.explain(&mut off, world);
                    testing::run(|out| feature.observe(&off, world, out))
                },
                |feature, world| {
                    let gone = Message::Event(WorldEvent::Despawn(TRADER));
                    testing::run(|out| feature.observe(&gone, world, out))
                },
            ];
            for end in ends {
                let mut world = bazaar_world();
                let mut feature = browsing(&mut world);
                let outcome = end(&mut feature, &mut world);
                assert_eq!(outcome.sent, Vec::new());
                assert!(ended(&outcome));
                assert!(feature.browse.is_none());
            }
        }

        #[test]
        fn an_admission_nobody_asked_for_is_withheld_and_a_late_approval_ended() {
            let mut world = bazaar_world();
            let mut feature = signed(&world);
            let mut unasked = event(BazaarEvent::Admitted {
                trader: TRADER,
                approved: true,
            });
            feature.explain(&mut unasked, &world);
            assert!(matches!(unasked, Message::Withheld));
            let outcome = testing::run(|out| feature.tick(Instant::now(), &mut world, out));
            assert_eq!(outcome.sent, Vec::new());
            assert_eq!(gaps(&outcome).len(), 1);
            assert!(feature.browse.is_none());
            // A click that stopped waiting: its approval is ended at once.
            testing::run(|out| feature.handle(&click(), &mut world, out));
            feature.click = feature
                .click
                .map(|(trader, _)| (trader, Instant::now().checked_sub(CLICK_WAIT).unwrap()));
            testing::run(|out| feature.tick(Instant::now(), &mut world, out));
            assert_eq!(feature.lapsed, [TRADER]);
            let mut late = event(BazaarEvent::Admitted {
                trader: TRADER,
                approved: true,
            });
            feature.explain(&mut late, &world);
            assert!(matches!(late, Message::Withheld));
            assert!(feature.browse.is_none());
            let outcome = testing::run(|out| feature.tick(Instant::now(), &mut world, out));
            assert_eq!(
                (outcome.sent[0].opcode, &outcome.sent[0].body[..]),
                (bazaar::TRADER_OPCODE, &bazaar::end_browse(TRADER)[..])
            );
            assert!(gaps(&outcome).is_empty() && feature.lapsed.is_empty());
        }

        #[test]
        fn a_zone_change_ends_the_browse_with_our_end_and_keeps_the_click_lapsed() {
            let mut world = bazaar_world();
            let mut feature = browsing(&mut world);
            let mut handoff = Message::Handoff(Vec::default());
            feature.explain(&mut handoff, &world);
            // Paid as the handoff is heard, on this zone's connection.
            let outcome = testing::run(|out| feature.observe(&handoff, &mut world, out));
            assert_eq!(
                (outcome.sent[0].opcode, &outcome.sent[0].body[..]),
                (bazaar::TRADER_OPCODE, &bazaar::end_browse(TRADER)[..])
            );
            assert!(ended(&outcome) && feature.browse.is_none());
            // A click in flight is kept as lapsed, so its late approval is
            // ended even when the zone change was refused.
            let mut world = bazaar_world();
            let mut feature = signed(&world);
            testing::run(|out| feature.handle(&click(), &mut world, out));
            feature.explain(&mut Message::Handoff(Vec::default()), &world);
            assert_eq!((feature.click, &feature.lapsed[..]), (None, &[TRADER][..]));
            let mut late = event(BazaarEvent::Admitted {
                trader: TRADER,
                approved: true,
            });
            feature.explain(&mut late, &world);
            let outcome = testing::run(|out| feature.tick(Instant::now(), &mut world, out));
            assert_eq!(outcome.sent[0].opcode, bazaar::TRADER_OPCODE);
            // Only a new admission forgets, and sends nothing.
            let mut world = bazaar_world();
            let mut feature = browsing(&mut world);
            let outcome = testing::run(|out| feature.admitted(&mut world, out));
            assert_eq!(outcome.sent, Vec::new());
            assert!(ended(&outcome) && feature.browse.is_none());
        }

        #[test]
        fn the_list_after_a_late_or_unasked_approval_is_withheld_once_as_a_gap() {
            for late in [true, false] {
                let mut world = bazaar_world();
                let mut feature = signed(&world);
                if late {
                    testing::run(|out| feature.handle(&click(), &mut world, out));
                    feature.give_up_click();
                }
                feature.explain(
                    &mut event(BazaarEvent::Admitted {
                        trader: TRADER,
                        approved: true,
                    }),
                    &world,
                );
                testing::run(|out| feature.tick(Instant::now(), &mut world, out));
                for serial in [90, 91] {
                    let mut item = view(Some(serial), 1250);
                    feature.explain(&mut item, &world);
                    assert!(matches!(item, Message::Withheld));
                }
                let outcome = testing::run(|out| feature.tick(Instant::now(), &mut world, out));
                assert_eq!(gaps(&outcome).len(), 1);
                assert!(gaps(&outcome)[0].sample.is_none());
                // Anything else ends the run: a merchant's item is the
                // merchant's again.
                feature.explain(&mut Message::Withheld, &world);
                let mut merchant = view(Some(4), 12);
                feature.explain(&mut merchant, &world);
                assert!(matches!(merchant, Message::MerchantView { .. }));
            }
        }

        #[test]
        fn a_click_waits_for_a_merchant_asked_to_open() {
            let mut world = bazaar_world();
            let mut feature = signed(&world);
            let shop = |open| ClientCommand::Shop {
                session_id: 5,
                merchant_id: 9,
                own_id: 7,
                open,
                created: Instant::now(),
            };
            testing::run(|out| feature.notice(&shop(true), &mut world, out));
            assert_eq!(
                refused(&testing::run(|out| feature.handle(
                    &click(),
                    &mut world,
                    out
                )))
                .as_deref(),
                Some("Wait for the merchant to answer.")
            );
            // A refusal answers it.
            testing::run(|out| {
                feature.observe(
                    &Message::Event(WorldEvent::Merchant(MerchantUpdate::Opened {
                        merchant_id: 9,
                        accepted: false,
                        rate: 1.0,
                    })),
                    &mut world,
                    out,
                )
            });
            assert!(feature.merchant.is_none() && !feature.shop_waiting(Instant::now()));
            testing::run(|out| feature.notice(&shop(true), &mut world, out));
            testing::run(|out| feature.notice(&shop(false), &mut world, out));
            assert_eq!(
                testing::run(|out| feature.handle(&click(), &mut world, out))
                    .sent
                    .len(),
                1
            );
        }

        #[test]
        fn a_shop_end_with_no_browse_is_a_gap() {
            let mut world = bazaar_world();
            let mut feature = Browsing::default();
            let mut end = event(BazaarEvent::ShopEnded);
            feature.explain(&mut end, &world);
            assert!(matches!(end, Message::Withheld));
            let outcome = testing::run(|out| feature.tick(Instant::now(), &mut world, out));
            assert_eq!(gaps(&outcome).len(), 1);
            assert!(!ended(&outcome));
        }
    }
}
