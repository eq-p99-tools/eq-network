//! The Bazaar on stock `EQEmu`, Titanium generation: the player's search
//! for traders' items and the server's answers (6.550), and the player's
//! browse of one trader's wares (6.558), read only. No buy and no trader
//! mode is sent from here.
//!
//! Stock `EQEmu` answers a search (`Client::Handle_OP_BazaarSearch`,
//! `Client::DoBazaarSearch`, `zone/trading.cpp`) with one `OP_BazaarSearch`
//! of packed result rows when anything matched, then always with the
//! search's end (`Client::SendBazaarDone`), and after rows the parcel
//! delivery costs (`Client::SendBazaarDeliveryCosts`), which only `RoF`
//! and later use. It searches only the traders in the player's own zone
//! and instance when the search names no trader (`Local_Scope`,
//! `common/bazaar.cpp` `Bazaar::GetSearchResults`), so outside the Bazaar
//! a search finds nothing and is answered by its end alone. It answers the
//! welcome request (`Client::SendBazaarWelcome`) with the counts of traders
//! and items.
//!
//! A row's price is what the trader listed, in copper. Stock `EQEmu`'s
//! buy charges the price the client sends, and in the code read never
//! compares it with this one, so a later buy must send this price exactly.
//!
//! Layout reference: `EQEmu`'s Titanium `BazaarSearch_Struct`,
//! `BazaarSearchResults_Struct` and `BazaarWelcome_Struct`
//! (`common/patches/titanium_structs.h`), its Titanium encoder and decoder
//! of `OP_BazaarSearch` (`common/patches/titanium.cpp`), and its
//! `BazaarReturnDone_Struct`, `BazaarWelcome_Struct` and
//! `BazaarDeliveryCost_Struct` (`common/eq_packet_structs.h`), packed.

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;

/// Titanium's `OP_BazaarSearch`: the player's search and welcome request,
/// and the server's results, end of search, welcome and delivery costs.
pub const SEARCH_OPCODE: u16 = 0x1ee9;

/// The Bazaar's zone id, where stock `EQEmu` keeps its traders.
pub const ZONE_ID: u16 = 151;

/// The action word of a search and of its result rows.
const SEARCH: u32 = 7;
/// The action word of the welcome request and its answer.
const WELCOME: u32 = 9;
/// The action word of a search's end.
const SEARCH_DONE: u32 = 12;
/// The action word of the parcel delivery costs, which only `RoF` and
/// later use.
const DELIVERY_COSTS: u32 = 29;

/// The length of a search (`BazaarSearch_Struct`): the action, six words,
/// the 64-byte name and four words.
pub const SEARCH_LENGTH: usize = 4 + 6 * 4 + NAME_LENGTH + 4 * 4;
/// The length of a result row (`BazaarSearchResults_Struct`): the action,
/// the trader's spawn, a word, the item, its serial number, a word, the
/// 64-byte name, the price and the stat.
pub const ROW_LENGTH: usize = 4 + 4 + 4 + 4 + 4 + 4 + NAME_LENGTH + 4 + 4;
/// The length of the welcome request and its answer
/// (`BazaarWelcome_Struct`, alike on both sides).
pub const WELCOME_LENGTH: usize = 20;
/// The length of a search's end (`BazaarReturnDone_Struct`).
const DONE_LENGTH: usize = 20;
/// The length of the delivery costs (`BazaarDeliveryCost_Struct`, packed:
/// a word, a short, a float and a word).
const DELIVERY_COSTS_LENGTH: usize = 14;
/// The length of an item name's field, its end included.
const NAME_LENGTH: usize = 64;

/// The most result rows an answer may carry: well past stock `EQEmu`'s
/// default `Bazaar:MaxSearchResults` of 50 items, as several traders may
/// list one item (ours, inferred). An answer with more is read as nothing.
pub const MOST_ROWS: usize = 1000;

/// "Any", for a criterion the search leaves open.
const ANY: u32 = u32::MAX;
/// The level bounds stock `EQEmu` takes as no bound.
const LOWEST_LEVEL: u32 = 1;
/// See [`LOWEST_LEVEL`].
const HIGHEST_LEVEL: u32 = 100;

/// What a Bazaar search looks for. Every criterion left `None` is open.
/// The numbers are the server's own (`Bazaar::GetSearchResults`): a class
/// as the class number, a race as the player race value, a slot as an
/// equipment slot, an item type and a stat as `EQEmu`'s numbers.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct BazaarCriteria {
    /// One trader's spawn in the zone; `None` searches every trader.
    pub trader: Option<u16>,
    /// The class that can use the item.
    pub class: Option<u32>,
    /// The race that can use the item.
    pub race: Option<u32>,
    /// The stat the item must have.
    pub stat: Option<u32>,
    /// The slot the item must fit.
    pub slot: Option<u32>,
    /// The item's type.
    pub item_type: Option<u32>,
    /// Part of the item's name; empty for any. At most 63 bytes of
    /// printable ASCII.
    pub name: String,
    /// The least price, in whole platinum.
    pub min_platinum: Option<u32>,
    /// The most price, in whole platinum.
    pub max_platinum: Option<u32>,
    /// The least recommended level.
    pub min_level: Option<u32>,
    /// The most recommended level.
    pub max_level: Option<u32>,
}

/// Builds `OP_BazaarSearch`'s search (`BazaarSearch_Struct`, 108 bytes).
/// An open criterion is sent as `EQEmu` reads "any": all ones for a
/// number, 0 for a price, 1 and 100 for the levels. What the official
/// client sends for an open criterion is not read yet (R49-42a): these are
/// the values the server takes as open (inferred).
///
/// # Errors
/// Refuses a name of more than 63 bytes or of anything but printable
/// ASCII, a price or level whose least is past its most, a price of 0
/// platinum (the server's "no bound"), a price the server's copper cannot
/// hold, and a level of 0 or past 100.
pub fn search(criteria: &BazaarCriteria) -> Result<Vec<u8>> {
    ensure!(
        criteria.name.len() < NAME_LENGTH,
        "an item name of at most 63 letters"
    );
    ensure!(
        criteria
            .name
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' '),
        "an item name of plain letters"
    );
    for platinum in [criteria.min_platinum, criteria.max_platinum]
        .into_iter()
        .flatten()
    {
        // `EQEmu` takes 0 as no bound, and multiplies the least price into
        // 32-bit copper.
        ensure!(
            (1..=u32::MAX / 1000).contains(&platinum),
            "a price of at least 1 platinum"
        );
    }
    if let (Some(least), Some(most)) = (criteria.min_platinum, criteria.max_platinum) {
        ensure!(least <= most, "a least price no more than the most");
    }
    for level in [criteria.min_level, criteria.max_level]
        .into_iter()
        .flatten()
    {
        ensure!(
            (LOWEST_LEVEL..=HIGHEST_LEVEL).contains(&level),
            "a level from 1 to 100"
        );
    }
    let min_level = criteria.min_level.unwrap_or(LOWEST_LEVEL);
    let max_level = criteria.max_level.unwrap_or(HIGHEST_LEVEL);
    ensure!(
        min_level <= max_level,
        "a least level no more than the most"
    );
    let mut body = Vec::with_capacity(SEARCH_LENGTH);
    body.extend_from_slice(&SEARCH.to_le_bytes());
    for value in [
        criteria.trader.map_or(0, u32::from),
        criteria.class.unwrap_or(ANY),
        criteria.race.unwrap_or(ANY),
        criteria.stat.unwrap_or(ANY),
        criteria.slot.unwrap_or(ANY),
        criteria.item_type.unwrap_or(ANY),
    ] {
        body.extend_from_slice(&value.to_le_bytes());
    }
    let mut name = [0_u8; NAME_LENGTH];
    name[..criteria.name.len()].copy_from_slice(criteria.name.as_bytes());
    body.extend_from_slice(&name);
    for value in [
        criteria.min_platinum.unwrap_or(0),
        criteria.max_platinum.unwrap_or(0),
        min_level,
        max_level,
    ] {
        body.extend_from_slice(&value.to_le_bytes());
    }
    debug_assert_eq!(body.len(), SEARCH_LENGTH);
    Ok(body)
}

/// Builds the welcome request: the action, then zeros to the welcome's
/// length. `EQEmu` reads only the action; the length is the welcome's
/// record (inferred, R49-42a).
#[must_use]
pub fn welcome() -> Vec<u8> {
    let mut body = vec![0; WELCOME_LENGTH];
    body[..4].copy_from_slice(&WELCOME.to_le_bytes());
    body
}

/// One trader's item, as a search found it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BazaarListing {
    /// The trader's spawn in the zone; a host names the trader from its
    /// own spawn list.
    pub trader: u16,
    /// The item's number.
    pub item_id: u32,
    /// The item's serial number, which names this one item of the trader's.
    pub serial: u32,
    /// The item's name, without the count the server adds.
    pub name: String,
    /// How many: the stack's charges, or 1; `None` where the server's name
    /// carried no count it could fit.
    pub count: Option<u32>,
    /// The trader's price for it, in copper.
    pub price: u32,
    /// The searched stat's value on the item, or 0.
    pub stat: u32,
}

/// One item on the trader's list while the player browses them, as the
/// server listed it: the price a later buy must send exactly.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TraderListing {
    /// The trader's spawn in the zone.
    pub trader: u16,
    /// The item's serial number, which names this one item of the trader's.
    pub serial: u32,
    /// The trader's price for one, in copper.
    pub price: u32,
    /// How many: the stack's count, or 1.
    pub count: u32,
    /// The item, as the server sent it.
    pub item: crate::inventory::InventoryItem,
}

/// What the server says of the Bazaar: a search's answers, and a browse
/// of one trader.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum BazaarEvent {
    /// Items a search found, in the server's order.
    Results(Vec<BazaarListing>),
    /// The search is over: after its results, or alone when it found
    /// nothing.
    SearchDone,
    /// The answer to the welcome request: how many traders, and how many
    /// items they sell.
    Welcome {
        /// Traders.
        traders: u32,
        /// Items for sale.
        items: u32,
    },
    /// The server's answer to the player's click on a trader
    /// ([`browse`]): whether it lets the player browse them. A refusal's
    /// own line comes as chat.
    Admitted {
        /// The trader clicked.
        trader: u16,
        /// Whether the browse is open.
        approved: bool,
    },
    /// Another player's trader sign went on or off
    /// (`OP_BecomeTrader`). Their name is not read: the host has the
    /// spawn's.
    TraderSign {
        /// The player's spawn.
        spawn: u16,
        /// Whether they are a trader now.
        on: bool,
    },
    /// The server's `OP_ShopEnd`: the trader being browsed left trader
    /// mode (`Client::TraderEndTrader`). The session tells the host
    /// [`BazaarEvent::BrowseEnded`] in its place.
    ShopEnded,
    /// An item on the list of the trader being browsed, new or changed: a
    /// price cut or a re-list replaces the entry of the same serial.
    Listed(Box<TraderListing>),
    /// An item left the list of the trader being browsed: sold, withdrawn
    /// or priced at 0.
    Delisted {
        /// The trader.
        trader: u16,
        /// The item's serial number.
        serial: u32,
    },
    /// The browse of a trader is over, and their list is forgotten.
    BrowseEnded {
        /// The trader.
        trader: u16,
    },
}

/// What a server's `OP_BazaarSearch` says.
#[derive(Debug, PartialEq)]
pub enum Answer {
    /// Something for the host.
    Event(BazaarEvent),
    /// The parcel delivery costs, which Titanium has no use for.
    DeliveryCosts,
    /// An action no server is known to send Titanium.
    Other(u32),
}

/// Reads the server's `OP_BazaarSearch` by its action word.
///
/// # Errors
/// Rejects a body without its action; result rows that are not whole,
/// more than [`MOST_ROWS`], or that do not each start with the search's
/// action, name a spawn past 16 bits or have a name without its end; and a
/// welcome, end or delivery costs of another length.
pub fn answer(body: &[u8]) -> Result<Answer> {
    ensure!(body.len() >= 4, "bazaar answer without its action");
    Ok(match word(body, 0) {
        SEARCH => Answer::Event(BazaarEvent::Results(results(body)?)),
        WELCOME => {
            ensure!(
                body.len() == WELCOME_LENGTH,
                "invalid bazaar welcome length"
            );
            Answer::Event(BazaarEvent::Welcome {
                traders: word(body, 4),
                items: word(body, 8),
            })
        }
        SEARCH_DONE => {
            ensure!(
                body.len() == DONE_LENGTH,
                "invalid bazaar search end length"
            );
            Answer::Event(BazaarEvent::SearchDone)
        }
        DELIVERY_COSTS => {
            ensure!(
                body.len() == DELIVERY_COSTS_LENGTH,
                "invalid bazaar delivery costs length"
            );
            Answer::DeliveryCosts
        }
        other => Answer::Other(other),
    })
}

/// Reads packed result rows: no count, one row after another.
fn results(body: &[u8]) -> Result<Vec<BazaarListing>> {
    ensure!(
        body.len().is_multiple_of(ROW_LENGTH),
        "bazaar results not whole rows"
    );
    let rows = body.len() / ROW_LENGTH;
    ensure!(rows <= MOST_ROWS, "more bazaar results than a search lists");
    body.as_chunks::<ROW_LENGTH>()
        .0
        .iter()
        .map(|row| {
            ensure!(word(row, 0) == SEARCH, "bazaar result of another action");
            let trader = u16::try_from(word(row, 4)).context("bazaar trader past a spawn id")?;
            let field = &row[24..24 + NAME_LENGTH];
            let end = field
                .iter()
                .position(|byte| *byte == 0)
                .context("bazaar item name without its end")?;
            let (name, count) = counted(&String::from_utf8_lossy(&field[..end]));
            Ok(BazaarListing {
                trader,
                item_id: word(row, 12),
                serial: word(row, 16),
                name,
                count,
                price: word(row, 24 + NAME_LENGTH),
                stat: word(row, 28 + NAME_LENGTH),
            })
        })
        .collect()
}

/// Splits the server's `"Name(count)"`; a name the field cut short keeps
/// all it has and no count.
fn counted(text: &str) -> (String, Option<u32>) {
    text.strip_suffix(')')
        .and_then(|rest| rest.rsplit_once('('))
        .and_then(|(name, count)| {
            (!count.is_empty() && count.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| {
                    count
                        .parse()
                        .ok()
                        .map(|count| (name.to_owned(), Some(count)))
                })
                .flatten()
        })
        .unwrap_or_else(|| (text.to_owned(), None))
}

/// Titanium's `OP_TraderShop`: the player's click on a trader, and the
/// server's admission.
pub const BROWSE_OPCODE: u16 = 0x35e8;
/// Titanium's `OP_Trader`: here, the player's end of a browse.
pub const TRADER_OPCODE: u16 = 0x524e;
/// Titanium's `OP_BecomeTrader`: a player's trader sign going on or off.
pub const SIGN_OPCODE: u16 = 0x2844;
/// Titanium's `OP_ShopEnd`: from the server, the trader being browsed left
/// trader mode.
pub const SHOP_END_OPCODE: u16 = crate::merchant::END_OPCODE;

/// The length of the click and its answer (`TraderClick_Struct`).
pub const BROWSE_LENGTH: usize = 16;
/// The length of a browse's end (`Trader_ShowItems_Struct`).
pub const END_BROWSE_LENGTH: usize = 20;
/// The length of a trader sign (`BecomeTrader_Struct`): the spawn, the
/// action and the 64-byte name.
pub const SIGN_LENGTH: usize = 4 + 4 + 64;
/// `OP_Trader`'s action that ends a browse (`EndTransaction`).
const END_TRANSACTION: u32 = 4;
/// The most items a trader lists (`EQEmu`'s `BAZAAR_SIZE`).
pub const MOST_LISTED: usize = 80;

/// Builds the click on a trader (`OP_TraderShop`, Titanium's
/// `TraderClick_Struct`): the trader's spawn, action 0, a word 0 and no
/// approval. Titanium's decoder reads every body as a click
/// (`DECODE(OP_TraderShop)`), and the server does not check that the
/// spawn is a trader (`Client::Handle_OP_TraderShop`), so only a spawn
/// whose sign is on may be sent.
#[must_use]
pub fn browse(trader: u16) -> [u8; BROWSE_LENGTH] {
    let mut body = [0; BROWSE_LENGTH];
    body[..4].copy_from_slice(&u32::from(trader).to_le_bytes());
    body
}

/// Builds the end of a browse (`OP_Trader` action 4, Titanium's
/// `Trader_ShowItems_Struct`): the action, the trader's spawn and three
/// words 0. The server frees the trader (`WithCustomer(0)`) and answers
/// nothing a Titanium client reads; it refuses another customer while
/// one is recorded, the same one too, so every browse must end with it.
#[must_use]
pub fn end_browse(trader: u16) -> [u8; END_BROWSE_LENGTH] {
    let mut body = [0; END_BROWSE_LENGTH];
    body[..4].copy_from_slice(&END_TRANSACTION.to_le_bytes());
    body[4..8].copy_from_slice(&u32::from(trader).to_le_bytes());
    body
}

/// Reads the server's `OP_TraderShop`: the admission to browse a trader.
/// Titanium's encoder writes the trader, action 0 and the approval, 0 or
/// 1 (`ENCODE(OP_TraderShop)`).
///
/// # Errors
/// Rejects another length (a buy's echo, which is not read yet), another
/// action, a trader past a spawn id and an approval other than 0 or 1.
pub fn admission(body: &[u8]) -> Result<BazaarEvent> {
    ensure!(
        body.len() == BROWSE_LENGTH,
        "trader shop answer of another length"
    );
    ensure!(word(body, 4) == 0, "trader shop answer of another action");
    let trader = u16::try_from(word(body, 0)).context("trader past a spawn id")?;
    let approved = match word(body, 12) {
        0 => false,
        1 => true,
        _ => bail!("trader shop approval past 1"),
    };
    Ok(BazaarEvent::Admitted { trader, approved })
}

/// Reads `OP_BecomeTrader`: action 1 is a sign going on, 0 going off
/// (Titanium's `ENCODE(OP_BecomeTrader)`). The name is not read.
///
/// # Errors
/// Rejects another length, another action and a spawn past 16 bits.
pub fn sign(body: &[u8]) -> Result<BazaarEvent> {
    ensure!(body.len() == SIGN_LENGTH, "trader sign of another length");
    let spawn = u16::try_from(word(body, 0)).context("trader sign past a spawn id")?;
    let on = match word(body, 4) {
        0 => false,
        1 => true,
        _ => bail!("trader sign of another action"),
    };
    Ok(BazaarEvent::TraderSign { spawn, on })
}

/// Reads the server's `OP_ShopEnd`, which is empty.
///
/// # Errors
/// Rejects a body.
pub fn shop_end(body: &[u8]) -> Result<BazaarEvent> {
    ensure!(body.is_empty(), "shop end with a body");
    Ok(BazaarEvent::ShopEnded)
}

fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(trader: u32, item: u32, serial: u32, name: &[u8], price: u32) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LENGTH);
        for value in [SEARCH, trader, 0, item, serial, 0] {
            row.extend_from_slice(&value.to_le_bytes());
        }
        let mut field = [0_u8; NAME_LENGTH];
        field[..name.len()].copy_from_slice(name);
        row.extend_from_slice(&field);
        row.extend_from_slice(&price.to_le_bytes());
        row.extend_from_slice(&3_u32.to_le_bytes());
        row
    }

    #[test]
    fn the_layouts_have_the_servers_lengths() {
        assert_eq!(SEARCH_LENGTH, 108);
        assert_eq!(ROW_LENGTH, 96);
        assert_eq!(WELCOME_LENGTH, 20);
    }

    #[test]
    fn an_open_search_sends_the_servers_any() {
        let body = search(&BazaarCriteria::default()).unwrap();
        let mut expected = Vec::new();
        for value in [7, 0, ANY, ANY, ANY, ANY, ANY] {
            expected.extend_from_slice(&value.to_le_bytes());
        }
        expected.extend_from_slice(&[0; 64]);
        for value in [0_u32, 0, 1, 100] {
            expected.extend_from_slice(&value.to_le_bytes());
        }
        assert_eq!(body, expected);
    }

    #[test]
    fn a_full_search_sends_each_criterion_in_its_place() {
        let body = search(&BazaarCriteria {
            trader: Some(312),
            class: Some(4),
            race: Some(2),
            stat: Some(11),
            slot: Some(13),
            item_type: Some(10),
            name: "Rusty Short Sword".into(),
            min_platinum: Some(5),
            max_platinum: Some(250),
            min_level: Some(10),
            max_level: Some(40),
        })
        .unwrap();
        assert_eq!(body.len(), SEARCH_LENGTH);
        let words =
            |at: usize, n: usize| (0..n).map(|i| word(&body, at + 4 * i)).collect::<Vec<_>>();
        assert_eq!(words(0, 7), [7, 312, 4, 2, 11, 13, 10]);
        assert_eq!(&body[28..45], b"Rusty Short Sword");
        assert!(body[45..92].iter().all(|byte| *byte == 0));
        assert_eq!(words(92, 4), [5, 250, 10, 40]);
    }

    #[test]
    fn a_search_out_of_bounds_is_refused() {
        let with = |change: fn(&mut BazaarCriteria)| {
            let mut criteria = BazaarCriteria::default();
            change(&mut criteria);
            search(&criteria)
        };
        assert!(with(|c| c.name = "a".repeat(63)).is_ok());
        assert!(with(|c| c.name = "a".repeat(64)).is_err());
        assert!(with(|c| c.name = "Sword\0".into()).is_err());
        assert!(with(|c| c.name = "Épée".into()).is_err());
        assert!(with(|c| {
            c.min_platinum = Some(9);
            c.max_platinum = Some(8);
        })
        .is_err());
        assert!(with(|c| c.min_platinum = Some(0)).is_err());
        assert!(with(|c| c.max_platinum = Some(u32::MAX / 1000 + 1)).is_err());
        assert!(with(|c| c.min_level = Some(0)).is_err());
        assert!(with(|c| c.max_level = Some(101)).is_err());
        assert!(with(|c| {
            c.min_level = Some(30);
            c.max_level = Some(20);
        })
        .is_err());
        // An open most level is 100: a least of 60 alone is fine.
        assert!(with(|c| c.min_level = Some(60)).is_ok());
    }

    #[test]
    fn the_welcome_request_is_its_action_and_zeros() {
        let body = welcome();
        assert_eq!(body.len(), WELCOME_LENGTH);
        assert_eq!(word(&body, 0), 9);
        assert!(body[4..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn result_rows_carry_each_listing() {
        let mut body = row(312, 1001, 77, b"Bone Chips(20)", 1250);
        body.extend(row(40, 2002, 78, b"Cloth Cap(1)", 3));
        body.extend(row(
            40,
            2003,
            79,
            b"A name cut short at the end of its fiel",
            9,
        ));
        let Answer::Event(BazaarEvent::Results(rows)) = answer(&body).unwrap() else {
            panic!("results");
        };
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            BazaarListing {
                trader: 312,
                item_id: 1001,
                serial: 77,
                name: "Bone Chips".into(),
                count: Some(20),
                price: 1250,
                stat: 3,
            }
        );
        assert_eq!(
            (rows[1].name.as_str(), rows[1].count),
            ("Cloth Cap", Some(1))
        );
        assert_eq!(
            (rows[2].name.as_str(), rows[2].count),
            ("A name cut short at the end of its fiel", None)
        );
    }

    #[test]
    fn results_not_whole_too_many_or_odd_are_refused() {
        let one = row(312, 1001, 77, b"Bone Chips(20)", 1250);
        assert!(answer(&one[..ROW_LENGTH - 1]).is_err());
        let mut trailing = one.clone();
        trailing.extend_from_slice(&[7, 0, 0, 0]);
        assert!(answer(&trailing).is_err());
        assert!(answer(&one.repeat(MOST_ROWS)).is_ok());
        assert!(answer(&one.repeat(MOST_ROWS + 1)).is_err());
        let mut second_other = one.clone();
        second_other.extend(row(1, 1, 1, b"x(1)", 1));
        second_other[ROW_LENGTH] = 12;
        assert!(answer(&second_other).is_err());
        let mut far = one.clone();
        far[4..8].copy_from_slice(&70_000_u32.to_le_bytes());
        assert!(answer(&far).is_err());
        let mut unended = one;
        unended[24..24 + NAME_LENGTH].fill(b'a');
        assert!(answer(&unended).is_err());
        assert!(answer(&[7, 0, 0]).is_err());
    }

    #[test]
    fn the_end_welcome_and_delivery_costs_are_read_by_their_lengths() {
        let mut done = vec![0; 20];
        done[..4].copy_from_slice(&12_u32.to_le_bytes());
        done[8..].fill(0xff);
        assert_eq!(
            answer(&done).unwrap(),
            Answer::Event(BazaarEvent::SearchDone)
        );
        assert!(answer(&done[..16]).is_err());
        let mut hello = vec![0; 20];
        hello[..4].copy_from_slice(&9_u32.to_le_bytes());
        hello[4..8].copy_from_slice(&6_u32.to_le_bytes());
        hello[8..12].copy_from_slice(&140_u32.to_le_bytes());
        assert_eq!(
            answer(&hello).unwrap(),
            Answer::Event(BazaarEvent::Welcome {
                traders: 6,
                items: 140
            })
        );
        assert!(answer(&hello[..12]).is_err());
        let mut costs = vec![0; 14];
        costs[..4].copy_from_slice(&29_u32.to_le_bytes());
        assert_eq!(answer(&costs).unwrap(), Answer::DeliveryCosts);
        assert!(answer(&costs[..13]).is_err());
        assert_eq!(answer(&22_u32.to_le_bytes()).unwrap(), Answer::Other(22));
    }

    #[test]
    fn the_click_and_the_end_have_the_servers_layouts() {
        assert_eq!(
            (BROWSE_LENGTH, END_BROWSE_LENGTH, SIGN_LENGTH),
            (16, 20, 72)
        );
        let click = browse(312);
        assert_eq!(
            [
                word(&click, 0),
                word(&click, 4),
                word(&click, 8),
                word(&click, 12)
            ],
            [312, 0, 0, 0]
        );
        let end = end_browse(312);
        assert_eq!(word(&end, 0), 4);
        assert_eq!(word(&end, 4), 312);
        assert!(end[8..].iter().all(|byte| *byte == 0));
    }

    fn answered(trader: u32, action: u32, approval: u32) -> Vec<u8> {
        [trader, action, 0, approval]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    #[test]
    fn an_admission_is_read_by_its_length_action_and_approval() {
        assert_eq!(
            admission(&answered(312, 0, 1)).unwrap(),
            BazaarEvent::Admitted {
                trader: 312,
                approved: true
            }
        );
        assert_eq!(
            admission(&answered(312, 0, 0)).unwrap(),
            BazaarEvent::Admitted {
                trader: 312,
                approved: false
            }
        );
        assert!(admission(&answered(312, 0, 2)).is_err());
        assert!(admission(&answered(312, 7, 1)).is_err());
        assert!(admission(&answered(70_000, 0, 1)).is_err());
        assert!(admission(&answered(312, 0, 1)[..12]).is_err());
        // A buy's echo is another record, not read here.
        assert!(admission(&[0; 32]).is_err());
    }

    #[test]
    fn a_sign_is_read_without_its_name() {
        let mut body = vec![0; SIGN_LENGTH];
        body[..4].copy_from_slice(&40_u32.to_le_bytes());
        body[4..8].copy_from_slice(&1_u32.to_le_bytes());
        // A name with no end is not read, so it is no fault.
        body[8..].fill(b'a');
        assert_eq!(
            sign(&body).unwrap(),
            BazaarEvent::TraderSign {
                spawn: 40,
                on: true
            }
        );
        body[4..8].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            sign(&body).unwrap(),
            BazaarEvent::TraderSign {
                spawn: 40,
                on: false
            }
        );
        body[4..8].copy_from_slice(&22_u32.to_le_bytes());
        assert!(sign(&body).is_err());
        assert!(sign(&body[..71]).is_err());
        assert_eq!(shop_end(&[]).unwrap(), BazaarEvent::ShopEnded);
        assert!(shop_end(&[0]).is_err());
    }
}
