//! The Bazaar on the Mac-era protocol: the player's search and welcome
//! request, and the server's answers (6.580), read only. Source-derived,
//! not live: TAKP's `EQMacEmu` (at 44afcbf) and Quarm's (at fc75fcf) handle
//! them alike, but no live row has checked them yet.
//!
//! Both servers read a search of exactly `sizeof(BazaarSearch_Struct)`
//! and a welcome request of exactly `sizeof(BazaarWelcome_Struct)`
//! (`Client::Handle_OP_BazaarSearch`), and drop any other length. A search
//! is answered by one `OP_BazaarSearch` for each result row, then always by
//! the search's end (`Client::SendBazaarResults`, `zone/trading.cpp`). The
//! welcome request is answered by the counts of traders and items
//! (`Client::SendBazaarWelcome`).
//!
//! A row's price is what the trader listed, in copper. Its item number
//! passes through the server's 16-bit field (`int16 itemid`), so a number
//! past 32767 arrives wrapped and is not to be trusted as an item's.
//!
//! Layout reference: `BazaarSearch_Struct`, `BazaarWelcome_Struct`,
//! `BazaarSearchResults_Struct` and `BazaarReturnDone_Struct`
//! (`common/eq_packet_structs.h`, packed). The Mac patch (`mac.cpp`)
//! translates none of them.

use super::{BazaarCriteria, BazaarEvent, BazaarListing};
use anyhow::{bail, ensure, Context, Result};

/// The Mac-era `OP_BazaarSearch` (0x4211 in TAKP's patch file, which
/// writes the opcode's bytes the other way round).
pub const SEARCH_OPCODE: u16 = 0x1142;

/// The action of a search and of its result rows.
const SEARCH: u16 = 7;
/// The action of the welcome request and its answer.
const WELCOME: u16 = 9;
/// The action of a search's end.
const SEARCH_DONE: u16 = 12;

/// The length of a search (`BazaarSearch_Struct`): the action's two bytes,
/// six short criteria, the 64-byte name, a short, and the two prices.
pub const SEARCH_LENGTH: usize = 2 + 6 * 2 + NAME_LENGTH + 2 + 2 * 4;
/// The length of the welcome request and its answer
/// (`BazaarWelcome_Struct`).
pub const WELCOME_LENGTH: usize = 20;
/// The length of a result row (`BazaarSearchResults_Struct`): the action,
/// the listings grouped, the item, the trader, the price and the name.
pub const ROW_LENGTH: usize = 2 + 2 + 2 + 2 + 4 + NAME_LENGTH;
/// The length of a search's end (`BazaarReturnDone_Struct`).
pub const DONE_LENGTH: usize = 20;
/// The length of an item name's field, its end included.
const NAME_LENGTH: usize = 64;

/// "Any", for a criterion the search leaves open (both servers' 0xFFFF).
const ANY: u16 = u16::MAX;

/// Builds the Mac-era search (`BazaarSearch_Struct`, 88 bytes). The class,
/// race and slot are bit positions the server tests in the item's masks:
/// the class number, the race's place in the race mask counted from 1
/// (Human 1), and the equipment slot counted from 0. The stat is one of
/// the server's 21 (`STAT_STR` 0 to `STAT_DAMAGE_SHIELD` 20) and the type
/// an item type or one of its groups. An open criterion is sent as 0xFFFF,
/// an open trader as 0, and an open price as 0, as the server reads them;
/// what the official client sends for them is not read (inferred).
///
/// # Errors
/// Refuses a level bound, which the record has no room for; a criterion
/// past 16 bits or equal to the server's "any"; and the name and price
/// bounds every search has (as [`super::search`] checks them).
pub fn search(criteria: &BazaarCriteria) -> Result<Vec<u8>> {
    ensure!(
        criteria.min_level.is_none() && criteria.max_level.is_none(),
        "no level bound on this server's Bazaar"
    );
    super::check_name_and_prices(criteria)?;
    let mut body = Vec::with_capacity(SEARCH_LENGTH);
    body.extend_from_slice(&SEARCH.to_le_bytes());
    body.extend_from_slice(&criteria.trader.unwrap_or(0).to_le_bytes());
    for value in [
        criteria.class,
        criteria.race,
        criteria.stat,
        criteria.slot,
        criteria.item_type,
    ] {
        let short = match value {
            None => ANY,
            Some(value) => {
                let short = u16::try_from(value).context("a search criterion past 16 bits")?;
                ensure!(short != ANY, "a search criterion of the server's any");
                short
            }
        };
        body.extend_from_slice(&short.to_le_bytes());
    }
    let mut name = [0_u8; NAME_LENGTH];
    name[..criteria.name.len()].copy_from_slice(criteria.name.as_bytes());
    body.extend_from_slice(&name);
    body.extend_from_slice(&[0, 0]);
    for platinum in [criteria.min_platinum, criteria.max_platinum] {
        body.extend_from_slice(&platinum.unwrap_or(0).to_le_bytes());
    }
    debug_assert_eq!(body.len(), SEARCH_LENGTH);
    Ok(body)
}

/// Builds the welcome request: the action, then zeros to the record's
/// length. The servers read only the action.
#[must_use]
pub fn welcome() -> Vec<u8> {
    let mut body = vec![0; WELCOME_LENGTH];
    body[..2].copy_from_slice(&WELCOME.to_le_bytes());
    body
}

/// What a Mac-era server's `OP_BazaarSearch` says.
#[derive(Debug, PartialEq)]
pub enum Answer {
    /// Something for the host: one result row, the search's end, or the
    /// welcome.
    Event(BazaarEvent),
    /// An action no server is known to send.
    Other(u16),
}

/// Reads a Mac-era server's `OP_BazaarSearch` by its action.
///
/// # Errors
/// Rejects a body without its action; a row, end or welcome of another
/// length; and a row whose name has no end.
pub fn answer(body: &[u8]) -> Result<Answer> {
    ensure!(body.len() >= 2, "bazaar answer without its action");
    Ok(match short(body, 0) {
        SEARCH => Answer::Event(BazaarEvent::Results(vec![row(body)?])),
        SEARCH_DONE => {
            ensure!(
                body.len() == DONE_LENGTH,
                "invalid bazaar search end length"
            );
            Answer::Event(BazaarEvent::SearchDone)
        }
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
        other => Answer::Other(other),
    })
}

/// Reads one result row. The server appends `" (count)"` to the name: the
/// stack's charges, or how many it groups.
fn row(body: &[u8]) -> Result<BazaarListing> {
    if body.len() != ROW_LENGTH {
        bail!("invalid bazaar result length");
    }
    let field = &body[12..12 + NAME_LENGTH];
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .context("bazaar item name without its end")?;
    let (name, count) = super::counted(&String::from_utf8_lossy(&field[..end]));
    let name = match count {
        Some(_) => name.strip_suffix(' ').map_or(name.clone(), str::to_owned),
        None => name,
    };
    let trader = short(body, 6);
    Ok(BazaarListing {
        trader: (trader != 0).then_some(trader),
        item_id: u32::from(short(body, 4)),
        serial: None,
        name,
        count,
        price: word(body, 8),
        stat: None,
        listings: Some(u32::from(short(body, 2))),
    })
}

fn short(body: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([body[at], body[at + 1]])
}

fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(listings: u16, item: u16, trader: u16, price: u32, name: &[u8]) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LENGTH);
        for value in [SEARCH, listings, item, trader] {
            row.extend_from_slice(&value.to_le_bytes());
        }
        row.extend_from_slice(&price.to_le_bytes());
        let mut field = [0_u8; NAME_LENGTH];
        field[..name.len()].copy_from_slice(name);
        row.extend_from_slice(&field);
        row
    }

    #[test]
    fn the_layouts_have_the_handlers_sizes() {
        // `sizeof` of the packed structs both handlers compare against.
        assert_eq!(SEARCH_LENGTH, 88);
        assert_eq!(WELCOME_LENGTH, 20);
        assert_eq!(ROW_LENGTH, 76);
        assert_eq!(DONE_LENGTH, 20);
    }

    #[test]
    fn a_search_places_each_criterion_as_the_servers_read_it() {
        let open = search(&BazaarCriteria::default()).unwrap();
        assert_eq!(open.len(), SEARCH_LENGTH);
        assert_eq!(short(&open, 0), 7);
        assert_eq!(short(&open, 2), 0);
        for at in (4..14).step_by(2) {
            assert_eq!(short(&open, at), 0xffff, "{at}");
        }
        assert!(open[14..80].iter().all(|byte| *byte == 0));
        assert_eq!((word(&open, 80), word(&open, 84)), (0, 0));

        let chosen = search(&BazaarCriteria {
            trader: Some(312),
            class: Some(2),
            race: Some(13),
            stat: Some(14),
            slot: Some(13),
            item_type: Some(46),
            name: "Bone".into(),
            min_platinum: Some(5),
            max_platinum: Some(900),
            ..BazaarCriteria::default()
        })
        .unwrap();
        assert_eq!(
            [4, 6, 8, 10, 12].map(|at| short(&chosen, at)),
            [2, 13, 14, 13, 46]
        );
        assert_eq!(short(&chosen, 2), 312);
        assert_eq!(&chosen[14..18], b"Bone");
        assert_eq!(chosen[18], 0);
        // Whole platinum: the server multiplies by 1000 into copper.
        assert_eq!((word(&chosen, 80), word(&chosen, 84)), (5, 900));
    }

    #[test]
    fn a_search_the_record_cannot_carry_is_refused() {
        let with = |change: fn(&mut BazaarCriteria)| {
            let mut criteria = BazaarCriteria::default();
            change(&mut criteria);
            search(&criteria)
        };
        assert!(with(|c| c.min_level = Some(10)).is_err());
        assert!(with(|c| c.max_level = Some(60)).is_err());
        assert!(with(|c| c.class = Some(0x1_0000)).is_err());
        assert!(with(|c| c.race = Some(0xffff)).is_err());
        assert!(with(|c| c.name = "x".repeat(64)).is_err());
        assert!(with(|c| c.min_platinum = Some(0)).is_err());
        assert!(with(|c| {
            c.min_platinum = Some(10);
            c.max_platinum = Some(9);
        })
        .is_err());
        assert!(with(|c| c.name = "x".repeat(63)).is_ok());
    }

    #[test]
    fn the_welcome_request_is_its_action_and_zeros() {
        let body = welcome();
        assert_eq!(body.len(), WELCOME_LENGTH);
        assert_eq!(short(&body, 0), 9);
        assert!(body[2..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn a_result_row_is_one_listing() {
        let Answer::Event(BazaarEvent::Results(rows)) =
            answer(&result(3, 1001, 312, 1250, b"Bone Chips (20)")).unwrap()
        else {
            panic!("a row");
        };
        assert_eq!(
            rows,
            [BazaarListing {
                trader: Some(312),
                item_id: 1001,
                serial: None,
                name: "Bone Chips".into(),
                count: Some(20),
                price: 1250,
                stat: None,
                listings: Some(3),
            }]
        );
        // A trader the server found in no zone, an id the server wrapped,
        // and a name its field cut short.
        let Answer::Event(BazaarEvent::Results(rows)) = answer(&result(
            1,
            0x8001,
            0,
            9,
            b"A name cut short at the end of its field, sixty-three letters..",
        ))
        .unwrap() else {
            panic!("a row");
        };
        assert_eq!(rows[0].trader, None);
        assert_eq!(rows[0].item_id, 0x8001);
        assert_eq!(rows[0].count, None);
        assert_eq!(rows[0].name.len(), 63);
    }

    #[test]
    fn the_end_and_the_welcome_are_read_and_odd_answers_refused() {
        let mut done = vec![0_u8; DONE_LENGTH];
        done[..2].copy_from_slice(&SEARCH_DONE.to_le_bytes());
        done[4..12].fill(0xff);
        assert_eq!(
            answer(&done).unwrap(),
            Answer::Event(BazaarEvent::SearchDone)
        );
        let mut counts = welcome();
        counts[4..8].copy_from_slice(&7_u32.to_le_bytes());
        counts[8..12].copy_from_slice(&40_u32.to_le_bytes());
        assert_eq!(
            answer(&counts).unwrap(),
            Answer::Event(BazaarEvent::Welcome {
                traders: 7,
                items: 40
            })
        );
        assert!(answer(&done[..DONE_LENGTH - 1]).is_err());
        assert!(answer(&counts[..WELCOME_LENGTH - 1]).is_err());
        let row = result(1, 1, 1, 1, b"Rock (1)");
        assert!(answer(&row[..ROW_LENGTH - 1]).is_err());
        let mut unended = row.clone();
        unended[12..].fill(b'a');
        assert!(answer(&unended).is_err());
        assert!(answer(&[7]).is_err());
        assert_eq!(answer(&[22, 0]).unwrap(), Answer::Other(22));
    }
}
